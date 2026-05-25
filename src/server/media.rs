//! `POST /media` and `GET /media/{id}` — server-visible blob storage
//! (phase-1 build step 4).
//!
//! Server-trusted: stores plaintext images (avatars, persona art, gallery)
//! and serves them back with their stored MIME so `<img src="/media/{id}">`
//! works. Carries over the path-traversal defense + per-route 16 MiB cap
//! from the retired E2EE attachment store; auth is now the session
//! ([`AuthAccount`]) and the uploader is an `account`.
//!
//! Path-traversal: the on-disk filename derives from a server-minted random
//! id (no user input touches the path on POST); on GET the stored path is
//! canonicalized and verified to live inside [`AppState::media_dir`] before
//! any read. There is no per-blob ACL in phase 1 — any authenticated account
//! may fetch any id (avatars are meant to be seen by co-members).

use std::path::PathBuf;

use axum::body::Bytes;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use surrealdb::types::SurrealValue;
use tokio::fs;

use crate::protocol::{ErrorBody, MediaUploadResponse};
use crate::server::auth::AuthAccount;
use crate::server::state::AppState;

/// Multipart field carrying the file bytes. Other fields are ignored.
const FILE_FIELD: &str = "file";
const STORAGE_EXT: &str = "bin";
const DEFAULT_MIME: &str = "application/octet-stream";
/// Reject absurd MIME strings (a sane image type is well under this).
const MAX_MIME_LEN: usize = 255;

// ---------------------------------------------------------------------------
// POST /media
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(uploader = %account.0))]
pub async fn upload_media(
    State(state): State<AppState>,
    account: AuthAccount,
    mut multipart: Multipart,
) -> Response {
    let (mime, bytes) = match next_file_field(&mut multipart).await {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    if bytes.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "file must not be empty");
    }

    let id = random_media_id();
    let path = state.media_dir.join(format!("{id}.{STORAGE_EXT}"));
    if let Err(e) = fs::write(&path, &bytes).await {
        tracing::error!(error = %e, path = %path.display(), "media write failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }
    let size = match i64::try_from(bytes.len()) {
        Ok(n) => n,
        Err(_) => {
            let _ = fs::remove_file(&path).await;
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let path_str = path.to_string_lossy().to_string();

    if let Err(e) = persist_media_row(&state, &id, &account.0, &mime, size, &path_str).await {
        tracing::error!(error = %e, "media row insert failed");
        let _ = fs::remove_file(&path).await;
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }

    (StatusCode::CREATED, Json(MediaUploadResponse { id })).into_response()
}

/// Drain multipart fields until the `file` field; return `(mime, bytes)`.
/// The MIME is captured from the part's `Content-Type` (defaulting to
/// octet-stream) before the bytes are consumed.
async fn next_file_field(multipart: &mut Multipart) -> Result<(String, Bytes), Response> {
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => {
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    "missing 'file' multipart field",
                ))
            }
            Err(e) => {
                tracing::warn!(error = %e, "multipart frame parse failed");
                return Err(error_response(
                    StatusCode::BAD_REQUEST,
                    "malformed multipart body",
                ));
            }
        };
        if field.name() == Some(FILE_FIELD) {
            let mime = match field.content_type() {
                Some(ct) if ct.len() <= MAX_MIME_LEN => ct.to_string(),
                _ => DEFAULT_MIME.to_string(),
            };
            let bytes = field.bytes().await.map_err(|e| {
                tracing::warn!(error = %e, "multipart body read failed");
                error_response(StatusCode::BAD_REQUEST, "could not read multipart body")
            })?;
            return Ok((mime, bytes));
        }
    }
}

async fn persist_media_row(
    state: &AppState,
    id: &str,
    uploader: &str,
    mime: &str,
    size: i64,
    storage_path: &str,
) -> surrealdb::Result<()> {
    state
        .db
        .query(
            r#"
            CREATE type::record("media_blob", $id) SET
                uploader     = type::record("account", $uploader),
                mime         = $mime,
                size_bytes   = $size,
                storage_path = $path;
            "#,
        )
        .bind(("id", id.to_string()))
        .bind(("uploader", uploader.to_string()))
        .bind(("mime", mime.to_string()))
        .bind(("size", size))
        .bind(("path", storage_path.to_string()))
        .await?
        .check()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// GET /media/{id}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, media = %id))]
pub async fn download_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    account: AuthAccount,
) -> Response {
    let _ = &account; // any authenticated account may fetch any blob (phase 1)

    let row = match load_media_row(&state, &id).await {
        Ok(Some(r)) => r,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "media not found"),
        Err(e) => {
            tracing::error!(error = %e, "load_media_row failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    // Path-traversal defense in depth: the stored path must canonicalize
    // inside the (already-canonical) media_dir. A missing file canonicalizes
    // to an error → 404 (same as an unknown id).
    let canonical = match PathBuf::from(&row.storage_path).canonicalize() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %row.storage_path, "media file missing on disk");
            return error_response(StatusCode::NOT_FOUND, "media not found");
        }
    };
    if !canonical.starts_with(state.media_dir.as_ref()) {
        tracing::error!(path = %canonical.display(), "media path escapes media_dir");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }

    let bytes = match fs::read(&canonical).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "media read failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    // Serve the stored MIME so the browser renders images directly. A
    // malformed stored value falls back to octet-stream.
    let content_type = if row.mime.is_empty() {
        DEFAULT_MIME.to_string()
    } else {
        row.mime
    };
    ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
}

#[derive(SurrealValue)]
struct MediaRow {
    storage_path: String,
    mime: String,
}

async fn load_media_row(state: &AppState, id: &str) -> surrealdb::Result<Option<MediaRow>> {
    let mut resp = state
        .db
        .query(r#"SELECT storage_path, mime FROM type::record("media_blob", $id);"#)
        .bind(("id", id.to_string()))
        .await?
        .check()?;
    resp.take(0)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn random_media_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(msg))).into_response()
}
