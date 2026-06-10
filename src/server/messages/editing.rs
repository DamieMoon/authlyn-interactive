//! Per-message mutations: edit own, delete (soft, #22), restore, and the
//! trash listing. Split from `server/messages.rs` in Wave 3; behavior preserved
//! verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{EditMessageRequest, ListMessagesResponse, MessageEnvelope};
use crate::server::auth::AuthAccount;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::retry::with_write_conflict_retry;
use crate::server::state::AppState;

use super::reading::{MessageRow, MESSAGES_PAGE_LIMIT, MSG_PROJECTION};
use super::{channel_access, AccessOutcome, MAX_BODY_CHARS};

// ---------------------------------------------------------------------------
// PATCH /channels/{cid}/messages/{mid}  — edit own message body
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid, message = %mid))]
pub async fn edit_message(
    State(state): State<AppState>,
    Path((cid, mid)): Path<(String, String)>,
    account: AuthAccount,
    payload: Result<Json<EditMessageRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let body = req.body.trim_end().to_string();
    if body.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "message body must not be empty");
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return error_response(StatusCode::BAD_REQUEST, "message body too long");
    }

    // Membership gate first (privacy 404 for non-members / unknown channel),
    // then the author check (403). The message must live in this channel.
    if let Err(resp) = require_own_message(&state, &cid, &mid, &account.0).await {
        return resp;
    }

    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query("UPDATE type::record('message', $mid) SET body = $body;")
            .bind(("mid", mid.clone()))
            .bind(("body", body.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => {
            state.emit(crate::protocol::SyncEvent::MessageEdited {
                channel_id: cid.clone(),
                message_id: mid.clone(),
            });
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "edit_message update failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /channels/{cid}/messages/{mid}  — delete own message
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid, message = %mid))]
pub async fn delete_message(
    State(state): State<AppState>,
    Path((cid, mid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    if let Err(resp) = require_own_message(&state, &cid, &mid, &account.0).await {
        return resp;
    }

    // Soft-delete (#22): hidden by the deleted_at = NONE read filters; the
    // purge sweep removes it after the 1h window. Restorable until then.
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query("UPDATE type::record('message', $mid) SET deleted_at = time::now();")
            .bind(("mid", mid.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => {
            state.emit(crate::protocol::SyncEvent::MessageDeleted {
                channel_id: cid.clone(),
                message_id: mid.clone(),
            });
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "delete_message failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Soft-delete trash + restore (#22)
// ---------------------------------------------------------------------------

/// POST /channels/{cid}/messages/{mid}/restore — un-delete the caller's own
/// soft-deleted message (the channel must still be live).
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid, message = %mid))]
pub async fn restore_message(
    State(state): State<AppState>,
    Path((cid, mid)): Path<(String, String)>,
    account: AuthAccount,
) -> Response {
    if let Err(resp) = require_own_message(&state, &cid, &mid, &account.0).await {
        return resp;
    }
    let result = with_write_conflict_retry(|| async {
        state
            .db
            .query("UPDATE type::record('message', $mid) SET deleted_at = NONE;")
            .bind(("mid", mid.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await;
    match result {
        Ok(()) => {
            // A restored message reappears — notify-and-fetch treats it as new arrival.
            state.emit(crate::protocol::SyncEvent::MessageCreated {
                channel_id: cid.clone(),
            });
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "restore_message failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// GET /channels/{cid}/messages/trash — the channel's soft-deleted messages,
/// recoverable until the 1h purge. Any member may view the trash (mirrors
/// normal message visibility).
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn list_deleted_messages(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
) -> Response {
    match channel_access(&state, &cid, &account.0).await {
        Ok(AccessOutcome::Ok(_)) => {}
        Ok(AccessOutcome::ChannelNotFound) | Ok(AccessOutcome::NotMember) => {
            return error_response(StatusCode::NOT_FOUND, "channel not found");
        }
        Err(e) => {
            tracing::error!(error = %e, "channel_access failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    match load_deleted_messages(&state, &cid, &account.0).await {
        Ok(messages) => (
            StatusCode::OK,
            Json(ListMessagesResponse {
                messages,
                typing: Vec::new(),
                active_persona: None,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_deleted_messages failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn load_deleted_messages(
    state: &AppState,
    cid: &str,
    caller: &str,
) -> surrealdb::Result<Vec<MessageEnvelope>> {
    let sql = format!(
        "SELECT {MSG_PROJECTION} FROM message
            WHERE channel = type::record('channel', $cid)
              AND deleted_at != NONE
            ORDER BY sent_at ASC, id_key ASC LIMIT $page_limit;"
    );
    let mut resp = state
        .db
        .query(sql)
        // `$caller` feeds MSG_PROJECTION's `is_pinged` arm (L-4).
        .bind(("cid", cid.to_string()))
        .bind(("caller", caller.to_string()))
        .bind(("page_limit", MESSAGES_PAGE_LIMIT))
        .await?
        .check()?;
    let rows: Vec<MessageRow> = resp.take(0)?;
    Ok(rows.into_iter().map(MessageRow::into_envelope).collect())
}

/// Gate for the per-message mutations (edit/delete): the caller must be a
/// member of the channel's guild (else privacy-404) *and* the message must
/// exist in this channel and be authored by the caller (else 403). The two
/// "not yours" cases — a stranger's message vs. a missing message — both
/// collapse to 403 so a member can't probe which message ids exist by edit.
async fn require_own_message(
    state: &AppState,
    cid: &str,
    mid: &str,
    account: &str,
) -> Result<(), Response> {
    match channel_access(state, cid, account).await {
        Ok(AccessOutcome::Ok(_)) => {}
        Ok(AccessOutcome::ChannelNotFound) | Ok(AccessOutcome::NotMember) => {
            return Err(error_response(StatusCode::NOT_FOUND, "channel not found"));
        }
        Err(e) => {
            tracing::error!(error = %e, "channel_access failed");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ));
        }
    }

    match message_author(state, cid, mid).await {
        Ok(Some(author)) if author == account => Ok(()),
        Ok(Some(_)) | Ok(None) => Err(error_response(
            StatusCode::FORBIDDEN,
            "you can only modify your own messages",
        )),
        Err(e) => {
            tracing::error!(error = %e, "message_author lookup failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage error",
            ))
        }
    }
}

/// The author account id of a message *scoped to a channel*, or `None` when
/// no such message exists in that channel.
async fn message_author(
    state: &AppState,
    cid: &str,
    mid: &str,
) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        author_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(author) AS author_key FROM type::record('message', $mid)
                WHERE channel = type::record('channel', $cid);",
        )
        .bind(("mid", mid.to_string()))
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<Row>>(0)?.map(|r| r.author_key))
}
