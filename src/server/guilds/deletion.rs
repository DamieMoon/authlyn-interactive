//! Guild soft-delete and restore (#22). Split from `server/guilds.rs` in
//! Wave 3; behavior preserved verbatim.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::server::auth::AuthAccount;
use crate::server::errors::error_response;
use crate::server::permissions::require_owner;
use crate::server::retry::with_write_conflict_retry;
use crate::server::state::AppState;

// ---------------------------------------------------------------------------
// DELETE /guilds/{id}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn delete_guild(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
) -> Response {
    if let Err(r) = require_owner(&state, &gid, &account.0).await {
        return r;
    }
    // Soft-delete (#22): stamp deleted_at and leave channels/members/messages
    // intact so a restore brings the whole guild back. It vanishes from every
    // read (all filter deleted_at = NONE); the purge sweep hard-deletes it +
    // its channels/members after the 30d window.
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query("UPDATE type::record('guild', $gid) SET deleted_at = time::now();")
            .bind(("gid", gid.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete_guild failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// POST /guilds/{id}/restore
// ---------------------------------------------------------------------------

/// POST /guilds/{id}/restore — un-delete a guild the caller owns (its
/// channels/members were left intact, so it returns whole).
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn restore_guild(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
) -> Response {
    // require_owner reads guild_member, which survives a soft-delete.
    if let Err(r) = require_owner(&state, &gid, &account.0).await {
        return r;
    }
    match state
        .db
        .query("UPDATE type::record('guild', $gid) SET deleted_at = NONE;")
        .bind(("gid", gid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "restore_guild failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}
