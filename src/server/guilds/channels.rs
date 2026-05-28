//! Channel CRUD + soft-delete trash/restore. Split from `server/guilds.rs` in
//! Wave 3; behavior preserved verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    ChannelListResponse, ChannelSummary, CreateChannelRequest, PatchChannelRequest,
};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::require_manager;
use crate::server::state::AppState;
use crate::server::validate::validate_name;

const CHANNEL_KINDS: [&str; 2] = ["text", "lorebook"];

// ---------------------------------------------------------------------------
// POST /guilds/{id}/channels
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn create_channel(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<CreateChannelRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    let name = req.name.trim().to_string();
    if let Err(msg) = validate_name(&name) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }
    if !CHANNEL_KINDS.contains(&req.kind.as_str()) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "channel kind must be text or lorebook",
        );
    }

    match insert_channel(&state, &gid, &name, &req.kind).await {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "insert_channel failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn insert_channel(
    state: &AppState,
    gid: &str,
    name: &str,
    kind: &str,
) -> surrealdb::Result<ChannelSummary> {
    // Append at the end: next position = current max + 1 (0 if no channels).
    let mut pos_resp = state
        .db
        .query(
            "SELECT VALUE position FROM channel
                WHERE guild = type::record('guild', $gid) ORDER BY position DESC LIMIT 1;",
        )
        .bind(("gid", gid.to_string()))
        .await?
        .check()?;
    let position = pos_resp.take::<Option<i64>>(0)?.map_or(0, |m| m + 1);

    let mut resp = state
        .db
        .query(
            "CREATE channel SET
                guild    = type::record('guild', $gid),
                name     = $name,
                kind     = $kind,
                position = $position
                RETURN meta::id(id) AS id_key;",
        )
        .bind(("gid", gid.to_string()))
        .bind(("name", name.to_string()))
        .bind(("kind", kind.to_string()))
        .bind(("position", position))
        .await?
        .check()?;
    let row: Option<IdRow> = resp.take(0)?;
    let id = row
        .map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("insert_channel produced no row".to_string()))?;
    Ok(ChannelSummary {
        id,
        name: name.to_string(),
        kind: kind.to_string(),
        position,
    })
}

// ---------------------------------------------------------------------------
// PATCH /guilds/{id}/channels/{cid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, channel = %cid))]
pub async fn patch_channel(
    State(state): State<AppState>,
    Path((gid, cid)): Path<(String, String)>,
    account: AuthAccount,
    payload: Result<Json<PatchChannelRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    match channel_in_guild(&state, &gid, &cid).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "channel not found"),
        Err(e) => {
            tracing::error!(error = %e, "channel_in_guild failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let mut sets: Vec<&str> = Vec::new();
    if let Some(ref raw) = req.name {
        if validate_name(raw.trim()).is_err() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "channel name must be 1–100 characters",
            );
        }
        sets.push("name = $name");
    }
    if req.position.is_some() {
        sets.push("position = $position");
    }
    if sets.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let sql = format!(
        "UPDATE type::record('channel', $cid) SET {};",
        sets.join(", ")
    );
    let mut q = state.db.query(&sql).bind(("cid", cid));
    if let Some(raw) = req.name {
        q = q.bind(("name", raw.trim().to_string()));
    }
    if let Some(position) = req.position {
        q = q.bind(("position", position));
    }
    match q.await.and_then(|r| r.check()) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "patch_channel update failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /guilds/{id}/channels/{cid}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, channel = %cid))]
pub async fn delete_channel(
    State(state): State<AppState>,
    Path((gid, cid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    match channel_in_guild(&state, &gid, &cid).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "channel not found"),
        Err(e) => {
            tracing::error!(error = %e, "channel_in_guild failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    // Soft-delete (#22): hidden by the deleted_at = NONE read filters; the
    // purge sweep removes it + its messages after the 1d window.
    match state
        .db
        .query("UPDATE type::record('channel', $cid) SET deleted_at = time::now();")
        .bind(("cid", cid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete_channel failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Channel trash + restore
// ---------------------------------------------------------------------------

/// GET /guilds/{id}/trash/channels — soft-deleted channels in a guild (manager).
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn list_deleted_channels(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
) -> Response {
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    #[derive(SurrealValue)]
    struct ChanRow {
        id_key: String,
        name: String,
        kind: String,
        position: i64,
    }
    let mut resp = match state
        .db
        .query(
            "SELECT meta::id(id) AS id_key, name, kind, position FROM channel
                WHERE guild = type::record('guild', $gid)
                  AND deleted_at != NONE ORDER BY position;",
        )
        .bind(("gid", gid))
        .await
        .and_then(|r| r.check())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "list_deleted_channels failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let channels = match resp.take::<Vec<ChanRow>>(0) {
        Ok(rows) => rows
            .into_iter()
            .map(|c| ChannelSummary {
                id: c.id_key,
                name: c.name,
                kind: c.kind,
                position: c.position,
            })
            .collect(),
        Err(e) => {
            tracing::error!(error = %e, "list_deleted_channels decode failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    (StatusCode::OK, Json(ChannelListResponse { channels })).into_response()
}

/// POST /guilds/{id}/channels/{cid}/restore — un-delete a channel (manager).
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, channel = %cid))]
pub async fn restore_channel(
    State(state): State<AppState>,
    Path((gid, cid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    // Scope the update to this guild so a manager can't revive an unrelated id.
    match state
        .db
        .query(
            "UPDATE type::record('channel', $cid) SET deleted_at = NONE
                WHERE guild = type::record('guild', $gid);",
        )
        .bind(("cid", cid))
        .bind(("gid", gid))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "restore_channel failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

async fn channel_in_guild(state: &AppState, gid: &str, cid: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM channel
                WHERE id = type::record('channel', $cid)
                  AND guild = type::record('guild', $gid)
                  AND deleted_at = NONE;",
        )
        .bind(("cid", cid.to_string()))
        .bind(("gid", gid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}
