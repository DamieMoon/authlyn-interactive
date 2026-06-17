//! Guild icon upload + the server-derived per-server accent (M6/P1, effect G).
//!
//! `PUT /guilds/{id}/icon` points `guild.icon` at an already-uploaded media blob
//! (the client POSTs the file to `/media` first, then sends the id here), then
//! re-derives the per-server accent (`guild.accent_color`) from the icon image
//! server-side. Manager-gated like every other guild mutation; the media id is
//! existence-checked (privacy-404) exactly as persona `set_avatar` does.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::{SetGuildIconRequest, SyncEvent};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::require_manager;
use crate::server::state::AppState;

/// PUT /guilds/{id}/icon — set the guild's icon and re-derive its per-server
/// accent from the image (manager-gated). The body carries a media id from a
/// prior `POST /media`.
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn set_guild_icon(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<SetGuildIconRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    // Owner/admin only; non-members get a privacy-404, plain members 403, and a
    // soft-deleted guild is rejected (require_manager checks liveness).
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    // Existence-check the media id (privacy-404), same contract as persona
    // set_avatar. (Accent derivation from the bytes lands in M6/P1.3.)
    match media_exists(&state, &req.media_id).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "media not found"),
        Err(e) => {
            tracing::error!(error = %e, "media_exists failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    if let Err(e) = state
        .db
        .query("UPDATE type::record('guild', $gid) SET icon = type::record('media_blob', $mid);")
        .bind(("gid", gid.clone()))
        .bind(("mid", req.media_id.clone()))
        .await
        .and_then(|r| r.check())
    {
        tracing::error!(error = %e, "set_guild_icon update failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }
    // Icon (and, from P1.3, the derived accent) are part of every member's rail
    // render → broadcast so all members refetch (id-only frame).
    state.emit(SyncEvent::ListsChanged);
    StatusCode::NO_CONTENT.into_response()
}

/// True iff a `media_blob` row exists for `mid` (mirrors the persona-gallery
/// probe; the path is server-minted so a bad id is a 404, never a 500).
async fn media_exists(state: &AppState, mid: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query("SELECT meta::id(id) AS id_key FROM type::record('media_blob', $mid);")
        .bind(("mid", mid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}
