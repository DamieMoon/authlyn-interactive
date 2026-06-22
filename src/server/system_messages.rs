//! `POST /admin/system-message` — app-admin broadcast of a "Nova DOT" system
//! message into every live guild's default channel. ssr-only.
//!
//! The endpoint is admin-gated (`is_admin`, fail-closed → 403). The fan-out core
//! ([`broadcast_system_message`]) is auth-free and exposed so integration tests
//! can exercise it directly — the admin-ALLOWED path can't be driven through HTTP
//! because `is_admin` reads process env that races the parallel test workers (see
//! `tests/feedback.rs`).

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use surrealdb::types::SurrealValue;

use crate::protocol::{SendSystemMessageRequest, SystemBroadcastResult};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::is_admin;
use crate::server::state::AppState;

/// Reserved bot account id (seeded in `schema.surql`) that authors system msgs.
const SYSTEM_ACCOUNT: &str = "nova_dot";
/// Upper bound on a broadcast body, in characters (mirrors the feedback bound).
const MAX_BODY_CHARS: usize = 4000;

/// Validate + trim a broadcast body: 1..=4000 chars after trimming. Pure (no DB),
/// so it's unit-testable without the admin-gated route.
pub fn validate_broadcast_body(body: &str) -> Result<String, &'static str> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("message body must not be empty");
    }
    if trimmed.chars().count() > MAX_BODY_CHARS {
        return Err("message body too long");
    }
    Ok(trimmed.to_string())
}

/// POST /admin/system-message — admin-only. Broadcasts a Nova DOT system message
/// to every live guild's default (first live text) channel.
pub async fn send_system_message(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<SendSystemMessageRequest>, JsonRejection>,
) -> Response {
    match is_admin(&state, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::FORBIDDEN, "forbidden"),
        Err(e) => {
            tracing::error!(error = %e, "admin check failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let body = match validate_broadcast_body(&req.body) {
        Ok(b) => b,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };

    match broadcast_system_message(&state, &body).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "system broadcast failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// Fan out a Nova DOT system message into every live guild's default (first live
/// text) channel. Auth-free core — the HTTP handler gates admin. Best-effort: a
/// per-guild failure aborts the whole transaction via `?` (callers map to 500),
/// but a guild with no live text channel is counted as skipped, not an error.
pub async fn broadcast_system_message(
    state: &AppState,
    body: &str,
) -> surrealdb::Result<SystemBroadcastResult> {
    // Live guilds only — a soft-deleted guild is excluded entirely.
    let mut resp = state
        .db
        .query("SELECT VALUE meta::id(id) FROM guild WHERE deleted_at = NONE;")
        .await?
        .check()?;
    let guild_ids: Vec<String> = resp.take(0)?;
    let guilds_targeted = guild_ids.len();
    let mut messages_sent = 0usize;
    let mut guilds_skipped = 0usize;

    for gid in guild_ids {
        // The guild's default channel: the first live text channel by position.
        // `position` is projected (not just ordered-by) because SurrealDB requires
        // the ORDER idiom to appear in the selection.
        #[derive(SurrealValue)]
        #[allow(dead_code)] // `position` exists only to satisfy ORDER BY.
        struct FirstChannel {
            cid: String,
            position: i64,
        }
        let mut resp = state
            .db
            .query(
                "SELECT meta::id(id) AS cid, position FROM channel
                 WHERE guild = type::record('guild', $gid)
                   AND kind = 'text' AND deleted_at = NONE
                 ORDER BY position ASC LIMIT 1;",
            )
            .bind(("gid", gid.clone()))
            .await?
            .check()?;
        let first: Option<FirstChannel> = resp.take(0)?;
        let Some(first) = first else {
            guilds_skipped += 1;
            continue;
        };
        let cid = first.cid;

        // Authored by the reserved Nova DOT bot; kind='system' drives rendering.
        // attachments/pinged_users/persona take their schema defaults.
        let mut resp = state
            .db
            .query(
                "CREATE message SET
                    channel = type::record('channel', $cid),
                    author  = type::record('account', $author),
                    body    = $body,
                    kind    = 'system'
                 RETURN meta::id(id) AS id_key;",
            )
            .bind(("cid", cid.clone()))
            .bind(("author", SYSTEM_ACCOUNT.to_string()))
            .bind(("body", body.to_string()))
            .await?
            .check()?;
        // A plain CREATE on an auto-id table always returns its row; a missing
        // one means something is wrong (and would silently break the
        // `targeted == sent + skipped` count invariant), so surface it as a 500
        // rather than under-reporting success. Mirrors `create_account`.
        let created: IdRow = resp.take::<Option<IdRow>>(0)?.ok_or_else(|| {
            surrealdb::Error::thrown("system broadcast CREATE produced no row".to_string())
        })?;
        messages_sent += 1;
        // Best-effort Web Push to that guild's members (the author — nova_dot —
        // is excluded by the fan-out query and has no subscription anyway).
        crate::server::push::notify_new_message(
            state.clone(),
            created.id_key,
            SYSTEM_ACCOUNT.to_string(),
        );
        // SSE bus: a broadcast row is a message like any other — live
        // subscribers learn of it the same way (Task 6/7 review carry-over).
        state.emit(crate::protocol::SyncEvent::MessageCreated {
            channel_id: cid.clone(),
        });
    }

    Ok(SystemBroadcastResult {
        guilds_targeted,
        messages_sent,
        guilds_skipped,
    })
}
