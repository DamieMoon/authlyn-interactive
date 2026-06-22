//! "Worn" persona endpoints — per-guild (deprecated path) and per-channel
//! (current path). Split from `server/personas.rs` in Wave 3; behavior
//! preserved verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::SetActivePersonaRequest;
use crate::server::access::{resolve_membership, Membership};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::can_edit_persona;
use crate::server::state::AppState;

// ---------------------------------------------------------------------------
// PUT /guilds/{id}/active-persona
// ---------------------------------------------------------------------------

/// PUT /guilds/{id}/active-persona — **legacy** per-guild worn persona, stored on
/// `guild_member.active_persona`. The current path is the per-channel
/// [`set_channel_active_persona`], which is what `server::messages` actually reads
/// at send time; this one is retained for back-compat. Caller must be a guild
/// member (privacy-404 otherwise); a non-null `persona_id` is gated by
/// `can_edit_persona` (owner OR redeemed editor — you can't wear someone else's),
/// and that wear is re-checked again at send time, never trusted. A null
/// `persona_id` clears the worn persona. (`tests/personas.rs::cannot_wear_someone_elses_persona`.)
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn set_active_persona(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<SetActivePersonaRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    // Caller must be a member of the guild (privacy-404 otherwise).
    match is_guild_member(&state, &gid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "guild not found"),
        Err(e) => {
            tracing::error!(error = %e, "is_guild_member failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    if let Some(ref pid) = req.persona_id {
        // Editors (key-redeemed) may also wear the persona, not just the owner.
        match can_edit_persona(&state, pid, &account.0).await {
            Ok(true) => {}
            Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
            Err(e) => {
                tracing::error!(error = %e, "can_edit_persona failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        }
    }

    let outcome = match req.persona_id {
        Some(pid) => state
            .db
            .query(
                "UPDATE guild_member SET active_persona = type::record('persona', $pid)
                        WHERE guild = type::record('guild', $gid)
                          AND account = type::record('account', $account);",
            )
            .bind(("pid", pid))
            .bind(("gid", gid))
            .bind(("account", account.0))
            .await
            .and_then(|r| r.check()),
        None => state
            .db
            .query(
                "UPDATE guild_member SET active_persona = NONE
                        WHERE guild = type::record('guild', $gid)
                          AND account = type::record('account', $account);",
            )
            .bind(("gid", gid))
            .bind(("account", account.0))
            .await
            .and_then(|r| r.check()),
    };
    match outcome {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "set_active_persona failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// PUT /channels/{cid}/active-persona  — per-channel worn persona (#persona)
// ---------------------------------------------------------------------------

/// True when `account` is a member of the guild that owns channel `cid` (and
/// the channel/guild aren't soft-deleted). Channel-scoped membership gate; the
/// resolve + membership check is the shared [`crate::server::access`] core
/// (soft-delete filter on, matching the previous behavior). Unknown channel and
/// non-member both collapse to `false`, as before.
async fn is_channel_member(state: &AppState, cid: &str, account: &str) -> surrealdb::Result<bool> {
    Ok(matches!(
        resolve_membership(state, cid, account, true).await?,
        Membership::Member { .. }
    ))
}

/// PUT /channels/{cid}/active-persona — the **current** per-channel worn persona
/// (`channel_active_persona`), the value `server::messages` reads to stamp each
/// post. Caller must be a channel member (privacy-404 otherwise); a non-null
/// `persona_id` is gated by `can_edit_persona` (owner OR redeemed editor), and a
/// null clears it. The set is an idempotent DELETE-then-CREATE on the UNIQUE
/// `(account, channel)` pair inside `with_write_conflict_retry`, so two
/// concurrent re-wears (double-tap / slow-network retry) converge to one row
/// instead of 500-ing the MVCC loser.
/// (`tests/personas.rs::concurrent_channel_wear_converges_to_one_row`,
/// `cannot_wear_someone_elses_persona`.)
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn set_channel_active_persona(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<SetActivePersonaRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };

    // Caller must be a member of the channel's guild (privacy-404 otherwise).
    match is_channel_member(&state, &cid, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "channel not found"),
        Err(e) => {
            tracing::error!(error = %e, "is_channel_member failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    if let Some(ref pid) = req.persona_id {
        // Editors (key-redeemed) may also wear the persona, not just the owner.
        match can_edit_persona(&state, pid, &account.0).await {
            Ok(true) => {}
            Ok(false) => return error_response(StatusCode::NOT_FOUND, "persona not found"),
            Err(e) => {
                tracing::error!(error = %e, "can_edit_persona failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        }
    }

    // Pure per-channel state: delete any existing row for (account, channel)
    // then, if wearing, create the new one — in one transaction so a wear is
    // never observed as "both rows" or "no row".
    let outcome = match req.persona_id {
        // Idempotent DELETE-then-CREATE on the (account, channel) UNIQUE pair,
        // wrapped in the write-conflict retry: two concurrent re-wears (double-tap
        // / retry-on-slow-network) converge to one row instead of 500ing the MVCC
        // loser (inv13).
        Some(pid) => {
            crate::server::retry::with_write_conflict_retry(|| async {
                state
                    .db
                    .query(
                        "BEGIN TRANSACTION;
                         DELETE FROM channel_active_persona
                            WHERE account = type::record('account', $account)
                              AND channel = type::record('channel', $cid);
                         CREATE channel_active_persona SET
                            account = type::record('account', $account),
                            channel = type::record('channel', $cid),
                            persona = type::record('persona', $pid);
                         COMMIT TRANSACTION;",
                    )
                    .bind(("pid", pid.clone()))
                    .bind(("cid", cid.clone()))
                    .bind(("account", account.0.clone()))
                    .await?
                    .check()?;
                Ok(())
            })
            .await
        }
        None => state
            .db
            .query(
                "DELETE FROM channel_active_persona
                    WHERE account = type::record('account', $account)
                      AND channel = type::record('channel', $cid);",
            )
            .bind(("cid", cid))
            .bind(("account", account.0))
            .await
            .and_then(|r| r.check())
            .map(|_| ()),
    };
    match outcome {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "set_channel_active_persona failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

async fn is_guild_member(state: &AppState, gid: &str, account: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(id) AS id_key FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $account);",
        )
        .bind(("gid", gid.to_string()))
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}
