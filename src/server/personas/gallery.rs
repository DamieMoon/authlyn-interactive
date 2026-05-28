//! Persona avatar + gallery: set avatar, add/remove gallery image, the
//! media-id validation helper. Split from `server/personas.rs` in Wave 3;
//! behavior preserved verbatim.

use std::collections::HashSet;
use std::fmt::Write as _;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::{
    AddGalleryImageRequest, AddGalleryImageResponse, AddGalleryImagesBatchRequest,
    AddGalleryImagesBatchResponse, SetAvatarRequest,
};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::can_edit_persona;
use crate::server::retry::with_write_conflict_retry;
use crate::server::state::AppState;

/// Cap on `media_ids` in a single batch gallery upload. Matches the per-message
/// attachment cap in [`crate::server::messages`] so paste-many in chat and
/// paste-many in the persona gallery land on the same number (W7/B1/B3).
const MAX_BATCH_IMAGES: usize = 100;

// ---------------------------------------------------------------------------
// PUT /personas/{id}/avatar
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn set_avatar(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<SetAvatarRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(resp) =
        require_editable_persona_and_media(&state, &pid, &account.0, &req.media_id).await
    {
        return resp;
    }
    match state
        .db
        .query(
            "UPDATE type::record('persona', $pid) SET avatar = type::record('media_blob', $mid);",
        )
        .bind(("pid", pid))
        .bind(("mid", req.media_id))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "set_avatar failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /personas/{id}/gallery  +  DELETE /personas/{id}/gallery/{img}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn add_gallery_image(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<AddGalleryImageRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(resp) =
        require_editable_persona_and_media(&state, &pid, &account.0, &req.media_id).await
    {
        return resp;
    }

    match insert_gallery_image(&state, &pid, &req.media_id).await {
        Ok(id) => (StatusCode::CREATED, Json(AddGalleryImageResponse { id })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "insert_gallery_image failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn insert_gallery_image(
    state: &AppState,
    pid: &str,
    media_id: &str,
) -> surrealdb::Result<String> {
    let mut pos_resp = state
        .db
        .query(
            "SELECT VALUE position FROM persona_image
                WHERE persona = type::record('persona', $pid) ORDER BY position DESC LIMIT 1;",
        )
        .bind(("pid", pid.to_string()))
        .await?
        .check()?;
    let position = pos_resp.take::<Option<i64>>(0)?.map_or(0, |m| m + 1);

    let mut resp = state
        .db
        .query(
            "CREATE persona_image SET
                persona = type::record('persona', $pid),
                media = type::record('media_blob', $mid),
                position = $position
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("pid", pid.to_string()))
        .bind(("mid", media_id.to_string()))
        .bind(("position", position))
        .await?
        .check()?;
    resp.take::<Option<IdRow>>(0)?
        .map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("insert_gallery_image produced no row".to_string()))
}

// ---------------------------------------------------------------------------
// POST /personas/{id}/gallery/batch — atomic multi-image upload (W7/B3)
// ---------------------------------------------------------------------------

/// Add multiple gallery images to a persona in one atomic SurrealDB transaction.
///
/// Mirrors [`add_gallery_image`]'s 404/permission shape exactly (owner-or-editor
/// via [`can_edit_persona`], privacy-404 when the caller can't edit). The new
/// rows' `position`s are sequential and contiguous starting from the persona's
/// current `MAX(position) + 1` so the gallery view sees the batch as one block
/// preserving the client's input order. The returned `ids` parallel the input
/// `media_ids` 1:1, in order.
///
/// ## Atomicity
/// The whole batch lives inside ONE `BEGIN TRANSACTION; … COMMIT TRANSACTION;`
/// — if any CREATE fails the transaction rolls back, so the client can retry
/// cleanly without a partial gallery.
///
/// The starting position is read OUTSIDE the transaction with the same
/// `SELECT VALUE position … ORDER BY position DESC LIMIT 1` shape as
/// [`add_gallery_image`]. That means TWO concurrent batches CAN race the
/// SELECT-MAX (the spec accepts this) — what we guarantee is that EACH batch's
/// own positions are internally contiguous. The whole thing is wrapped in
/// [`with_write_conflict_retry`] so a transaction-level MVCC race on either
/// the SELECT MAX or the CREATEs is retried against a fresh snapshot.
///
/// ## Validation order
/// `JsonRejection` → empty `media_ids` (400) → too many (400) → duplicates
/// (400) → permission (404 privacy) → each media exists (404 "media not found"
/// — same as the single-id endpoint, where the same check rejects unknown
/// ids).
#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid))]
pub async fn add_gallery_images_batch(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<AddGalleryImagesBatchRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if req.media_ids.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "no media ids");
    }
    if req.media_ids.len() > MAX_BATCH_IMAGES {
        return error_response(StatusCode::BAD_REQUEST, "too many images");
    }
    // HashSet dedup: any duplicate id in the input is a client bug. Reject
    // rather than silently dedup so positions can't be "off" relative to what
    // the caller passed.
    let mut seen: HashSet<&str> = HashSet::with_capacity(req.media_ids.len());
    for id in &req.media_ids {
        if !seen.insert(id.as_str()) {
            return error_response(StatusCode::BAD_REQUEST, "duplicate media id");
        }
    }

    // Permission gate — privacy-404, same as the single-id endpoint.
    match can_edit_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "can_edit_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    // Existence-check ALL media ids in one round-trip — same shape as the
    // message-attachment check (`all_media_exist` in `messages::posting`):
    // bind as `RecordId`s and `FROM $records WHERE id IS NOT NONE` so the
    // planner does a per-record PK lookup rather than a table scan.
    match all_media_exist(&state, &req.media_ids).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "media not found"),
        Err(e) => {
            tracing::error!(error = %e, "all_media_exist failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    // The whole batch (SELECT MAX + N CREATEs) is retried as one unit on
    // SurrealDB write conflicts so two concurrent inviters/uploaders
    // de-stampede cleanly. `with_write_conflict_retry` is `FnMut` so the
    // closure must be able to be called multiple times — clone the captures
    // into the async block each pass.
    let media_ids = req.media_ids.clone();
    let pid_owned = pid.clone();
    match with_write_conflict_retry(|| {
        let state = state.clone();
        let pid = pid_owned.clone();
        let media_ids = media_ids.clone();
        async move { insert_gallery_images_batch(&state, &pid, &media_ids).await }
    })
    .await
    {
        Ok(ids) => (
            StatusCode::CREATED,
            Json(AddGalleryImagesBatchResponse { ids }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "insert_gallery_images_batch failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// True when every id in `ids` names an existing `media_blob`. Mirrors the
/// shape of `messages::posting::all_media_exist` (W5/H4: record-id bind +
/// `FROM $records WHERE id IS NOT NONE` triggers a per-record PK plan rather
/// than a table scan). Empty input is a caller bug — handler rejects it before
/// we get here — but we still defend.
async fn all_media_exist(state: &AppState, ids: &[String]) -> surrealdb::Result<bool> {
    if ids.is_empty() {
        return Ok(true);
    }
    let records: Vec<surrealdb::types::RecordId> = ids
        .iter()
        .map(|id| surrealdb::types::RecordId::new("media_blob", id.as_str()))
        .collect();
    let mut resp = state
        .db
        .query("SELECT VALUE meta::id(id) FROM $records WHERE id IS NOT NONE;")
        .bind(("records", records))
        .await?
        .check()?;
    let found: Vec<String> = resp.take(0)?;
    Ok(ids.iter().all(|id| found.contains(id)))
}

/// Atomic insert of N gallery rows. Reads the current max `position` for the
/// persona, then issues all N `CREATE persona_image`s inside one
/// `BEGIN TRANSACTION; … COMMIT TRANSACTION;` with positions
/// `max + 1, max + 2, …` — preserving the input's order so the client can
/// correlate the returned ids 1:1 with `media_ids`.
///
/// Returns the new `persona_image` row ids, in the same order as `media_ids`.
async fn insert_gallery_images_batch(
    state: &AppState,
    pid: &str,
    media_ids: &[String],
) -> surrealdb::Result<Vec<String>> {
    // Current max position (None when the gallery is empty → start at 0,
    // matching `insert_gallery_image`). Read outside the transaction; a
    // concurrent batch can still slip a row in between this SELECT and our
    // CREATEs — the spec's promise is single-batch contiguity, not
    // cross-batch ordering.
    let mut pos_resp = state
        .db
        .query(
            "SELECT VALUE position FROM persona_image
                WHERE persona = type::record('persona', $pid)
                ORDER BY position DESC LIMIT 1;",
        )
        .bind(("pid", pid.to_string()))
        .await?
        .check()?;
    let start_pos = pos_resp.take::<Option<i64>>(0)?.map_or(0, |m| m + 1);

    // Build one transaction with N stacked CREATE-as-LET statements; each
    // binds its own media id parameter (`$m_0`, `$m_1`, …) so values never
    // interpolate into SQL. The position is a Rust-computed literal — safe
    // because it's a plain integer derived from start_pos + index. At the
    // end one RETURN collects every new row's id into an ordered array, so
    // we read the batch via a single typed `take(...)` at the RETURN's
    // statement index (avoiding per-CREATE index counting for N=100). The
    // statement-index layout follows the same convention as
    // `persist_create_guild` in `server::guilds::mod`: BEGIN and COMMIT each
    // occupy one slot, every LET / RETURN at the transaction's top level
    // occupies one slot — so the layout here is 0 BEGIN, 1..=N LET $c_i,
    // N+1 RETURN, N+2 COMMIT.
    let n = media_ids.len();
    let mut sql = String::with_capacity(n * 240 + 96);
    sql.push_str("BEGIN TRANSACTION;\n");
    for i in 0..n {
        let _ = writeln!(
            sql,
            "LET $c_{i} = (CREATE persona_image SET \
                persona = type::record('persona', $pid), \
                media = type::record('media_blob', $m_{i}), \
                position = {pos} \
                RETURN meta::id(id) AS id_key)[0].id_key;",
            pos = start_pos + i as i64,
        );
    }
    sql.push_str("RETURN [");
    for i in 0..n {
        if i > 0 {
            sql.push_str(", ");
        }
        let _ = write!(sql, "$c_{i}");
    }
    sql.push_str("];\nCOMMIT TRANSACTION;\n");

    let mut q = state.db.query(sql).bind(("pid", pid.to_string()));
    for (i, mid) in media_ids.iter().enumerate() {
        q = q.bind((format!("m_{i}"), mid.to_string()));
    }
    let mut resp = q.await?.check()?;
    // RETURN [list] inside a transaction is a single statement that produces
    // a Vec — `take::<Vec<T>>` flattens that into the user-facing list
    // directly (`take::<Option<T>>` would reject "multiple results" since
    // the wire shape is an array, not a single value). Layout: BEGIN(0),
    // LET $c_0..$c_{n-1} at 1..=n, RETURN at n+1, COMMIT at n+2.
    let return_idx = 1 + n;
    let ids: Vec<String> = resp.take(return_idx)?;
    if ids.len() != n {
        return Err(surrealdb::Error::thrown(format!(
            "batch transaction RETURN produced {} ids, expected {n}",
            ids.len()
        )));
    }
    Ok(ids)
}

#[tracing::instrument(skip_all, fields(account = %account.0, persona = %pid, image = %img))]
pub async fn remove_gallery_image(
    State(state): State<AppState>,
    Path((pid, img)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    match can_edit_persona(&state, &pid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
        Err(e) => {
            tracing::error!(error = %e, "can_edit_persona failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    match state
        .db
        .query(
            "DELETE FROM persona_image
                WHERE id = type::record('persona_image', $img)
                  AND persona = type::record('persona', $pid);",
        )
        .bind(("img", img))
        .bind(("pid", pid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "remove_gallery_image failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

async fn media_exists(state: &AppState, mid: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query("SELECT meta::id(id) AS id_key FROM type::record('media_blob', $mid);")
        .bind(("mid", mid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}

/// Edit-check the persona and existence-check the media id; map either miss
/// to the appropriate 404. Uses the same owner-or-editor rule as `patch_persona`
/// so anyone who may edit the persona may also set its picture.
async fn require_editable_persona_and_media(
    state: &AppState,
    pid: &str,
    account: &str,
    media_id: &str,
) -> Result<(), Response> {
    match can_edit_persona(state, pid, account).await {
        Ok(true) => {}
        Ok(false) => return Err(error_response(StatusCode::NOT_FOUND, "persona not found")),
        Err(e) => {
            tracing::error!(error = %e, "can_edit_persona failed");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ));
        }
    }
    match media_exists(state, media_id).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(error_response(StatusCode::NOT_FOUND, "media not found")),
        Err(e) => {
            tracing::error!(error = %e, "media_exists failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ))
        }
    }
}
