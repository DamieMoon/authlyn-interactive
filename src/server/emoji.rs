//! Custom emoji — per-guild Discord-style shortcodes backed by media_blob ids.
//!
//! Flow: the client uploads the image via `POST /media` (returns a media id),
//! then POSTs `{ name, media_id }` here. The emoji row links to the existing
//! media_blob; no new upload pipeline is required.
//!
//! ## Authorization
//! All three endpoints require the caller to be a guild member (any role),
//! matching the Discord model where members can see + create emoji but deletion
//! is gated at manager (owner/admin) level to prevent abuse.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::{Datetime, SurrealValue};

use crate::protocol::{CreateEmojiRequest, CustomEmoji, ListEmojiResponse};
use crate::server::auth::AuthAccount;
use crate::server::datetime::to_rfc3339_fixed;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::guilds;
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

/// `^[a-z0-9_]{2,32}$` validated in Rust (no regex dependency needed).
fn validate_emoji_name(name: &str) -> Result<(), &'static str> {
    let n = name.len();
    if n < 2 {
        return Err("emoji name must be at least 2 characters");
    }
    if n > 32 {
        return Err("emoji name must be at most 32 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err("emoji name must match [a-z0-9_]");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// POST /guilds/{id}/emoji
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn create_emoji(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<CreateEmojiRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    // Member gate: any role suffices.
    match guilds::caller_role(&state, &gid, &account.0).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "guild not found"),
        Err(e) => {
            tracing::error!(error = %e, "caller_role failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    if let Err(msg) = validate_emoji_name(&req.name) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }

    match insert_emoji(&state, &gid, &req.name, &req.media_id, &account.0).await {
        Ok(emoji) => (StatusCode::CREATED, Json(emoji)).into_response(),
        Err(e) if is_unique_violation(&e) => error_response(
            StatusCode::CONFLICT,
            "emoji name already exists in this guild",
        ),
        Err(e) => {
            tracing::error!(error = %e, "insert_emoji failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn insert_emoji(
    state: &AppState,
    gid: &str,
    name: &str,
    media_id: &str,
    creator: &str,
) -> surrealdb::Result<CustomEmoji> {
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        name: String,
        media_key: String,
        creator_key: String,
        created_at: Datetime,
    }

    let row: Option<Row> = with_write_conflict_retry(|| async {
        let mut resp = state
            .db
            .query(
                "CREATE custom_emoji SET
                    guild      = type::record('guild', $gid),
                    name       = $name,
                    media      = type::record('media_blob', $media_id),
                    creator    = type::record('account', $creator)
                    RETURN
                        meta::id(id)      AS id_key,
                        name,
                        meta::id(media)   AS media_key,
                        meta::id(creator) AS creator_key,
                        created_at;",
            )
            .bind(("gid", gid.to_string()))
            .bind(("name", name.to_string()))
            .bind(("media_id", media_id.to_string()))
            .bind(("creator", creator.to_string()))
            .await?
            .check()?;
        resp.take::<Option<Row>>(0)
    })
    .await?;

    let r =
        row.ok_or_else(|| surrealdb::Error::thrown("insert_emoji produced no row".to_string()))?;
    Ok(CustomEmoji {
        id: r.id_key,
        name: r.name,
        media_id: r.media_key,
        creator_id: r.creator_key,
        created_at: to_rfc3339_fixed(r.created_at),
    })
}

// ---------------------------------------------------------------------------
// GET /guilds/{id}/emoji
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn list_emoji(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
) -> Response {
    // Member gate.
    match guilds::caller_role(&state, &gid, &account.0).await {
        Ok(Some(_)) => {}
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "guild not found"),
        Err(e) => {
            tracing::error!(error = %e, "caller_role failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    match load_emoji(&state, &gid).await {
        Ok(emoji) => (StatusCode::OK, Json(ListEmojiResponse { emoji })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_emoji failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_emoji(state: &AppState, gid: &str) -> surrealdb::Result<Vec<CustomEmoji>> {
    #[derive(SurrealValue)]
    struct Row {
        id_key: String,
        name: String,
        media_key: String,
        creator_key: String,
        created_at: Datetime,
    }

    let mut resp = state
        .db
        .query(
            "SELECT
                meta::id(id)      AS id_key,
                name,
                meta::id(media)   AS media_key,
                meta::id(creator) AS creator_key,
                created_at
            FROM custom_emoji
            WHERE guild = type::record('guild', $gid)
            ORDER BY name;",
        )
        .bind(("gid", gid.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = resp.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| CustomEmoji {
            id: r.id_key,
            name: r.name,
            media_id: r.media_key,
            creator_id: r.creator_key,
            created_at: to_rfc3339_fixed(r.created_at),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// DELETE /guilds/{id}/emoji/{name}
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid, emoji = %ename))]
pub async fn delete_emoji(
    State(state): State<AppState>,
    Path((gid, ename)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    // Manager gate (owner or admin): mirrors the pattern in guilds.rs.
    match guilds::caller_role(&state, &gid, &account.0).await {
        Ok(Some(role)) if role == "owner" || role == "admin" => {}
        Ok(Some(_)) => return error_response(StatusCode::FORBIDDEN, "admin only"),
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "guild not found"),
        Err(e) => {
            tracing::error!(error = %e, "caller_role failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    match state
        .db
        .query(
            "DELETE FROM custom_emoji
                WHERE guild = type::record('guild', $gid)
                  AND name  = $name;",
        )
        .bind(("gid", gid))
        .bind(("name", ename))
        .await
        .and_then(|r| r.check())
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete_emoji failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}
