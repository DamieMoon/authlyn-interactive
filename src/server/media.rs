//! `POST /media` and `GET /media/{id}` — server-visible blob storage.
//!
//! Server-trusted: stores plaintext images (avatars, persona art, gallery)
//! and serves them back with their stored MIME so `<img src="/media/{id}">`
//! works. Auth is the session ([`AuthAccount`]) and the uploader is an
//! `account`.
//!
//! Path-traversal: the on-disk filename derives from a server-minted random
//! id (no user input touches the path on POST); on GET the stored path is
//! canonicalized and verified to live inside [`AppState::media_dir`] before
//! any read. There is no per-blob ACL in phase 1 — any authenticated account
//! may fetch any id (avatars are meant to be seen by co-members).

use std::path::PathBuf;

use axum::body::Bytes;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use serde::Deserialize;
use surrealdb::types::SurrealValue;
use tokio::fs;

use crate::protocol::MediaUploadResponse;
use crate::server::auth::AuthAccount;
use crate::server::errors::error_response;
use crate::server::state::AppState;

/// Multipart field carrying the file bytes. Other fields are ignored.
const FILE_FIELD: &str = "file";
const STORAGE_EXT: &str = "bin";
const DEFAULT_MIME: &str = "application/octet-stream";
/// Reject absurd MIME strings (a sane image type is well under this).
const MAX_MIME_LEN: usize = 255;

/// Inline-renderable raster image MIME allowlist. These are the only types
/// served back with their stored `Content-Type` so `<img src="/media/{id}">`
/// renders; every other allowed type is forced to an attachment download (see
/// [`serve_original`]). SVG is deliberately excluded — it is XML and can carry
/// script, so serving it inline would be a stored-XSS vector (review F-D8-3).
const INLINE_IMAGE_MIMES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Non-image MIME types accepted for upload but never served inline — each is
/// neutralized to an `octet-stream` attachment on GET (see [`serve_original`]).
/// SCRIPT-CAPABLE types are intentionally absent (`text/html`,
/// `image/svg+xml`, `application/javascript`, `application/xhtml+xml`, any
/// executable): they are REJECTED at upload so attacker-controlled active
/// content can never be stored and later served from our own origin (F-D8-3).
const ALLOWED_DOWNLOAD_MIMES: [&str; 8] = [
    "video/mp4",
    "video/webm",
    "audio/mpeg",
    "audio/mp4",
    "audio/wav",
    "application/pdf",
    "application/zip",
    "text/plain",
];

/// Base type of `mime`, lowercased — strips any `; charset=…` parameters.
fn mime_base(mime: &str) -> &str {
    mime.split(';').next().unwrap_or("").trim()
}

/// True if `mime`'s base type is an inline-safe raster image. SVG is excluded
/// (it is XML and can carry script); these are served with their stored type.
fn is_inline_image_mime(mime: &str) -> bool {
    let base = mime_base(mime);
    INLINE_IMAGE_MIMES
        .iter()
        .any(|allowed| base.eq_ignore_ascii_case(allowed))
}

/// True if `mime` may be uploaded: an inline-safe image OR one of the
/// download-only types. Everything else (script-capable / unknown) is rejected.
/// Inline-renderable images keep their type on serve; download types are forced
/// to a safe attachment by [`serve_original`].
fn is_allowed_attachment_mime(mime: &str) -> bool {
    if is_inline_image_mime(mime) {
        return true;
    }
    let base = mime_base(mime);
    ALLOWED_DOWNLOAD_MIMES
        .iter()
        .any(|allowed| base.eq_ignore_ascii_case(allowed))
}

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
    // Allowlist: images render inline, the other allowed types download as a
    // neutralized attachment. SCRIPT-CAPABLE types (text/html, image/svg+xml,
    // application/javascript, …) are rejected so attacker-controlled active
    // content can never be stored and later served from our origin (F-D8-3).
    // The serve path additionally hardens legacy rows predating this check.
    if !is_allowed_attachment_mime(&mime) {
        return error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported media type (images, MP4/WebM video, MP3/M4A/WAV audio, PDF, ZIP, or plain text)",
        );
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

/// Optional `?w=N` query: serve a JPEG thumbnail at most N px wide instead of
/// the full blob. Used for avatars/cards so the chat doesn't pull multi-MB
/// originals. Absent → full original (back-compat).
#[derive(Debug, Deserialize)]
pub struct MediaQuery {
    pub w: Option<u32>,
}

#[tracing::instrument(skip_all, fields(account = %account.0, media = %id))]
pub async fn download_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<MediaQuery>,
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

    // Thumbnail fast path: `?w=N` on an image blob serves a cached/just-built
    // JPEG downscaled to ≤N px wide. The cache file lives next to the original
    // (`{id}.w{N}.v2.jpg`), keyed on the clamped width, so each size is built once.
    // The `v2` tag is a cache-buster: it bumps when the thumbnail PIPELINE changes
    // (v2 = Lanczos3 + JPEG q85) so existing soft thumbnails regenerate sharp
    // without a manual cache wipe. Old `{id}.w{N}.jpg` files are left orphaned
    // (harmless small JPEGs; a future sweep can remove the un-versioned ones).
    if let Some(w) = q.w {
        let w = w.clamp(THUMB_MIN_W, THUMB_MAX_W);
        if row.mime.starts_with("image/") {
            let thumb_path = state.media_dir.join(format!("{id}.w{w}.v2.jpg"));
            if let Ok(cached) = fs::read(&thumb_path).await {
                return jpeg_response(cached);
            }
            if let Ok(orig) = fs::read(&canonical).await {
                let for_blocking = orig.clone();
                match tokio::task::spawn_blocking(move || make_thumb(&for_blocking, w)).await {
                    Ok(Ok(jpg)) => {
                        // Best-effort cache; a write failure just rebuilds next time.
                        let _ = fs::write(&thumb_path, &jpg).await;
                        return jpeg_response(jpg);
                    }
                    // Undecodable/unsupported → fall back to the original bytes.
                    _ => return serve_original(orig, row.mime),
                }
            }
            // Original unreadable → fall through to the normal read+error path.
        }
    }

    let bytes = match fs::read(&canonical).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "media read failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    serve_original(bytes, row.mime)
}

/// Clamp bounds for the `?w=` thumbnail width (avatars are tiny; cards modest).
const THUMB_MIN_W: u32 = 16;
const THUMB_MAX_W: u32 = 512;

/// Serve raw blob bytes. Always sends `X-Content-Type-Options: nosniff` so the
/// browser cannot sniff stored bytes into active content. Inline-safe raster
/// images are served with their stored type; anything else (legacy rows, svg,
/// text/html, …) is forced to an `octet-stream` attachment so it can never
/// render as active content on our origin (review F-D8-3).
fn serve_original(bytes: Vec<u8>, mime: String) -> Response {
    if is_inline_image_mime(&mime) {
        (
            [
                (header::CONTENT_TYPE, mime),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            ],
            bytes,
        )
            .into_response()
    } else {
        (
            [
                (header::CONTENT_TYPE, DEFAULT_MIME.to_string()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                (header::CONTENT_DISPOSITION, "attachment".to_string()),
            ],
            bytes,
        )
            .into_response()
    }
}

fn jpeg_response(bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}

/// Decode `bytes`, downscale to ≤`max_w` px wide (preserving aspect, never
/// upscaling), and re-encode as JPEG. CPU-bound — call inside spawn_blocking.
/// Alpha is flattened to RGB: the only consumers are circle-masked avatars and
/// cards where corners are clipped anyway.
///
/// Uses Lanczos3 resampling (not bilinear `Triangle`) + JPEG quality 85: avatars
/// and cards looked soft/low-res with the default bilinear filter + default
/// quality (~75), especially on hi-DPR displays. Thumbnails are cached on disk
/// (`{id}.w{N}.jpg`, built once per width), so the sharper filter costs nothing
/// after the first build.
fn make_thumb(bytes: &[u8], max_w: u32) -> Result<Vec<u8>, image::ImageError> {
    use std::io::Cursor;
    let img = image::load_from_memory(bytes)?;
    let resized = if img.width() > max_w {
        let nh = ((u64::from(img.height()) * u64::from(max_w)) / u64::from(img.width())).max(1);
        img.resize_exact(max_w, nh as u32, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
    image::DynamicImage::ImageRgb8(resized.to_rgb8()).write_with_encoder(encoder)?;
    Ok(out.into_inner())
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
