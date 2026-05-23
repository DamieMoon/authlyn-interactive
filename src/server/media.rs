//! `POST /media` and `GET /media/{id}` — encrypted attachment storage
//! (routing-plan step 9).
//!
//! Dumb relay over opaque ciphertext bytes, same posture as
//! `server::messages`. The server stores byte-for-byte what the client
//! uploaded; the AES key, IV, and SHA-256 ride inside the
//! Megolm-encrypted message body (carried by
//! [`crate::crypto::EncryptedFileRef`]) and the server never sees them.
//! GET hands the same bytes back; verification + decryption happen
//! client-side.
//!
//! ## Stance
//!
//! - **Adversarial.** The id in the GET URL is attacker-controlled and
//!   the storage layout is filesystem-backed, so path traversal is the
//!   first thing to defeat. Defense in depth: file paths are derived
//!   *server-side* from a freshly minted random id (no user input touches
//!   the path on POST); on GET, the resolved storage_path is canonicalized
//!   and verified to live inside [`AppState::media_dir`] before any
//!   `tokio::fs::read` opens it.
//! - **Defensive at the HTTP boundary.** Auth (`X-Device-Id` →
//!   `load_caller_user`) runs *before* the multipart body is read on
//!   POST, so an unauthenticated request never spends disk or CPU.
//!   `multipart::MultipartError` and the bytes-read failure both map to
//!   typed 400s rather than bubbling out as 500s.
//!
//! ## Auth gate on GET
//!
//! Matrix `m.encrypted` v2 treats media URLs as unauthenticated locators
//! (security comes from the key+iv+hash, which only authorised recipients
//! can decrypt). This module additionally requires `X-Device-Id` on the
//! GET so a leaked URL can't be used as a free public CDN — a leaked URL
//! still hands an attacker only ciphertext, but at least it doesn't hand
//! it to an anonymous one. There is no per-room ACL: any authenticated
//! device can fetch any blob id (matches dumb-relay posture).
//!
//! ## Write order and orphans
//!
//! Order on POST: write file → CREATE row → success. If CREATE fails the
//! file is deleted best-effort; if the response is dropped after CREATE
//! lands the row is reachable but the client doesn't know the id (it
//! never reached them) — the row is orphaned. Both forms of orphan are
//! cheap to detect and GC later (out of scope for v1, per the routing
//! plan's "Media garbage collection is out of scope").

use std::path::PathBuf;

use axum::body::Bytes;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use surrealdb::types::SurrealValue;
use tokio::fs;

use crate::protocol::{ErrorBody, MediaUploadResponse};
use crate::server::keys::extract_device_id;
use crate::server::state::AppState;

/// Multipart field name the upload handler reads. Anything else in the
/// multipart body is ignored (drained but not stored), so clients can
/// add metadata fields later without a wire break.
const CIPHERTEXT_FIELD: &str = "ciphertext";

/// File extension for stored ciphertext. Informational only — the file
/// is opaque bytes; the mimetype lives in the `EncryptedFileRef` inside
/// the Megolm-encrypted message body.
const STORAGE_EXT: &str = "bin";

// ---------------------------------------------------------------------------
// POST /media
// ---------------------------------------------------------------------------

/// Persist one encrypted attachment ciphertext.
///
/// Wire contract:
///
/// - Auth: `X-Device-Id` → caller's `device` row → caller's `user`.
/// - Body: multipart/form-data with a single `ciphertext` field
///   (additional fields are tolerated and ignored).
/// - Success: `201 Created` + [`MediaUploadResponse`].
///
/// Validation table:
///
/// | Failure | Status | Body |
/// |---|---|---|
/// | Missing X-Device-Id | 401 | `missing X-Device-Id header` |
/// | Unknown caller device | 401 | `unknown caller device` |
/// | Malformed multipart frame | 400 | `malformed multipart body` |
/// | Missing `ciphertext` field | 400 | `missing 'ciphertext' multipart field` |
/// | Empty `ciphertext` field | 400 | `ciphertext must not be empty` |
/// | `Content-Length` > 16 MiB | 413 | (tower-http default — layered upstream) |
/// | Chunked body exceeds 16 MiB mid-stream | 400 | `malformed multipart body` |
///
/// The two body-size branches surface differently: tower-http's
/// [`RequestBodyLimitLayer`] short-circuits to 413 when the client's
/// `Content-Length` header is set and exceeds the cap, but a chunked
/// upload that grows past the cap mid-stream fails inside
/// `multipart.next_field()`, which `next_ciphertext_field` maps to a
/// 400 `"malformed multipart body"`. Spec-bothness is intentional —
/// the 400 is the typed error our handler controls; the 413 is the
/// layer's contract.
#[tracing::instrument(skip_all, fields(caller_device, caller_user))]
pub async fn upload_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let device_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("caller_device", &tracing::field::display(&device_id));

    // Resolve caller first — an unauth request never reads the multipart
    // body, so an unauthenticated 16 MiB upload costs us no disk + ~0 CPU.
    let uploader = match load_caller_user(&state, &device_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "unknown caller device"),
        Err(e) => {
            tracing::error!(error = %e, "load_caller_user failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    tracing::Span::current().record("caller_user", &tracing::field::display(&uploader));

    // v1 trade-off: `field.bytes()` buffers the whole ciphertext (up
    // to the 16 MiB per-route cap) into a single `Bytes` allocation per
    // request. N concurrent authenticated uploads = ~16·N MiB resident;
    // on the 8 GB Pi target this gives ~500 concurrent uploads as the
    // memory-pressure ceiling, with no per-user concurrency throttle in
    // v1. Acceptable because the v1 test target is a 1 MiB blob and the
    // current user count is one; streaming via `field.chunk()` into a
    // `tokio::fs::File` is the deferred mitigation when these
    // assumptions stop holding.
    let ciphertext = match next_ciphertext_field(&mut multipart).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    if ciphertext.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "ciphertext must not be empty");
    }

    let id = random_media_id();
    let path = state.media_dir.join(format!("{id}.{STORAGE_EXT}"));

    // Write file first; on CREATE failure attempt cleanup so we don't
    // accumulate orphan files in the common-case error path. A crash
    // between these two steps still leaves an orphan file — GC is out
    // of scope for v1.
    if let Err(e) = fs::write(&path, &ciphertext).await {
        tracing::error!(error = %e, path = %path.display(), "media file write failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }

    // Bound `i64::try_from` failure to a defensive 500 even though
    // `RequestBodyLimitLayer` ensures `ciphertext.len()` always fits.
    let size = match i64::try_from(ciphertext.len()) {
        Ok(n) => n,
        Err(_) => {
            let _ = fs::remove_file(&path).await;
            tracing::error!(len = ciphertext.len(), "ciphertext length overflows i64");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let path_str = path.to_string_lossy().to_string();

    if let Err(e) = persist_media_row(&state, &id, &uploader, size, &path_str).await {
        tracing::error!(error = %e, "media row insert failed");
        let _ = fs::remove_file(&path).await;
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }

    (StatusCode::CREATED, Json(MediaUploadResponse { id })).into_response()
}

/// Drain multipart fields until we hit the `ciphertext` field; return its
/// bytes. Other fields are skipped (their bodies are discarded by axum
/// when we move on without calling `.bytes()`). If we exhaust the
/// multipart with no match, return the 400 typed error.
async fn next_ciphertext_field(multipart: &mut Multipart) -> Result<Bytes, Response> {
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    "missing 'ciphertext' multipart field",
                ));
            }
            Err(e) => {
                tracing::warn!(error = %e, "multipart frame parse failed");
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    "malformed multipart body",
                ));
            }
        };
        if field.name() == Some(CIPHERTEXT_FIELD) {
            return field.bytes().await.map_err(|e| {
                tracing::warn!(error = %e, "multipart body read failed");
                error_response(StatusCode::BAD_REQUEST, "could not read multipart body")
            });
        }
        // Other field — fall through to the next iteration; axum drops
        // the unread `Field` bodies for us.
    }
}

async fn persist_media_row(
    state: &AppState,
    id: &str,
    uploader: &str,
    size: i64,
    storage_path: &str,
) -> surrealdb::Result<()> {
    state
        .db
        .query(
            r#"
            CREATE type::record("media_blob", $id) SET
                uploader     = type::record("user", $uploader),
                size_bytes   = $size,
                storage_path = $path;
            "#,
        )
        .bind(("id", id.to_string()))
        .bind(("uploader", uploader.to_string()))
        .bind(("size", size))
        .bind(("path", storage_path.to_string()))
        .await?
        .check()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// GET /media/{id}
// ---------------------------------------------------------------------------

/// Serve one stored ciphertext blob.
///
/// Wire contract:
///
/// - Auth: same as POST.
/// - Path: `{id}` is the opaque id [`upload_media`] minted.
/// - Success: `200 OK`, `Content-Type: application/octet-stream`, body =
///   ciphertext bytes.
///
/// Validation table:
///
/// | Failure | Status | Body |
/// |---|---|---|
/// | Missing X-Device-Id | 401 | `missing X-Device-Id header` |
/// | Unknown caller device | 401 | `unknown caller device` |
/// | Unknown id OR file missing on disk | 404 | `media not found` |
#[tracing::instrument(skip_all, fields(caller_device, media_id = %id))]
pub async fn download_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let device_id = match extract_device_id(&headers) {
        Some(d) => d,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("caller_device", &tracing::field::display(&device_id));

    match load_caller_user(&state, &device_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "unknown caller device"),
        Err(e) => {
            tracing::error!(error = %e, "load_caller_user failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    let row = match load_media_row(&state, &id).await {
        Ok(Some(r)) => r,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "media not found"),
        Err(e) => {
            tracing::error!(error = %e, "load_media_row failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    // Path-traversal defense in depth. storage_path was written by the
    // server, so we trust it — but verifying it canonicalizes inside
    // `media_dir` before opening makes a future DB-write compromise
    // less useful. `state.media_dir` is already canonical (resolved at
    // AppState construction), so the comparison is a free path-component
    // check rather than a per-request stat-chain. `canonicalize()` on
    // the row path requires the file to exist, so a missing file
    // surfaces as 404 (same as an unknown id, privacy-conservative).
    let path = PathBuf::from(&row.storage_path);
    let canonical = match path.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "media file missing on disk");
            return error_response(StatusCode::NOT_FOUND, "media not found");
        }
    };
    if !canonical.starts_with(state.media_dir.as_ref()) {
        tracing::error!(
            path = %canonical.display(),
            root = %state.media_dir.display(),
            "media path escapes media_dir — refusing to serve"
        );
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }

    // Same v1 trade-off as upload_media: `fs::read` buffers the full
    // blob (up to 16 MiB per the upload cap) into a `Vec<u8>` before
    // shipping. Switch to `Body::from_stream` over `tokio::fs::File`
    // when the upload path goes streaming.
    let bytes = match fs::read(&canonical).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "media file read failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    )
        .into_response()
}

#[derive(SurrealValue)]
struct MediaRow {
    storage_path: String,
}

async fn load_media_row(state: &AppState, id: &str) -> surrealdb::Result<Option<MediaRow>> {
    let mut resp = state
        .db
        .query(r#"SELECT storage_path FROM type::record("media_blob", $id);"#)
        .bind(("id", id.to_string()))
        .await?
        .check()?;
    let row: Option<MediaRow> = resp.take(0)?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve a device id (from `X-Device-Id`) to its owning user id.
/// Returns `Ok(None)` when the device row doesn't exist — handlers map
/// that to `401 "unknown caller device"`. Same shape as
/// `messages::load_caller_user` (`messages.rs:596-609`) and
/// `rooms::load_caller_user` (`rooms.rs:600-613`); the duplication is
/// intentional, keeping each handler module's auth helper inline with
/// the handler so renames or auth-stub swaps stay scoped.
async fn load_caller_user(state: &AppState, device_id: &str) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        user_key: String,
    }
    let mut resp = state
        .db
        .query("SELECT meta::id(user) AS user_key FROM type::record('device', $device_id);")
        .bind(("device_id", device_id.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| r.user_key))
}

/// 16-byte hex string. Used as the `media_blob` row id and the on-disk
/// filename base; the same value goes back to the client as
/// `MediaUploadResponse::id`. Same shape as the test harness's
/// `random_id()` so it interoperates cleanly with the existing tests.
fn random_media_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(msg))).into_response()
}
