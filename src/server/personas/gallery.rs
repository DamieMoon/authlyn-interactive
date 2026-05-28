//! Persona avatar + gallery: set avatar, add/remove gallery image, the
//! media-id validation helper. Split from `server/personas.rs` in Wave 3;
//! behavior preserved verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::{AddGalleryImageRequest, AddGalleryImageResponse, SetAvatarRequest};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::can_edit_persona;
use crate::server::state::AppState;

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
