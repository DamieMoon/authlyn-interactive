//! `POST /channels/{cid}/messages` + the persist + attachment-existence check.
//! Split from `server/messages.rs` in Wave 3; behavior preserved verbatim.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::{SendMessageRequest, SendMessageResponse};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::state::AppState;

use super::{channel_access, AccessOutcome, MAX_ATTACHMENTS, MAX_BODY_CHARS};

// ---------------------------------------------------------------------------
// POST /channels/{cid}/messages
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn post_message(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<SendMessageRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    let body = req.body.trim_end().to_string();
    // Attachments: dedupe (preserve order), bound the count. A message is valid
    // with text, with images, or both — but not empty of both.
    let mut attachments: Vec<String> = Vec::new();
    for id in req.attachment_ids {
        if !attachments.contains(&id) {
            attachments.push(id);
        }
    }
    if body.trim().is_empty() && attachments.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "message must have text or an image",
        );
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return error_response(StatusCode::BAD_REQUEST, "message body too long");
    }
    if attachments.len() > MAX_ATTACHMENTS {
        return error_response(StatusCode::BAD_REQUEST, "too many attachments");
    }
    // Reject unknown media ids so a row never stores a dangling attachment.
    match all_media_exist(&state, &attachments).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::BAD_REQUEST, "unknown attachment"),
        Err(e) => {
            tracing::error!(error = %e, "attachment existence check failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    let access = match channel_access(&state, &cid, &account.0).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "channel_access failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let stored_persona = match access {
        AccessOutcome::Ok(ctx) => {
            if ctx.kind != "text" {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "cannot post messages to a non-text channel",
                );
            }
            ctx.active_persona
        }
        AccessOutcome::ChannelNotFound | AccessOutcome::NotMember => {
            return error_response(StatusCode::NOT_FOUND, "channel not found");
        }
    };
    // Attribution is decided at send time, never racing the per-channel wear
    // write: trust the persona the client says it's wearing here AFTER checking
    // the caller may use it; if absent/invalid, fall back to the stored
    // per-channel persona (`channel_active_persona`), else speak as the account.
    let active_persona = match req.persona_id.as_deref() {
        Some(pid) => {
            match crate::server::permissions::can_edit_persona(&state, pid, &account.0).await {
                Ok(true) => Some(pid.to_string()),
                Ok(false) => stored_persona,
                Err(e) => {
                    tracing::error!(error = %e, "can_edit_persona failed");
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
                }
            }
        }
        None => stored_persona,
    };

    match persist_message(
        &state,
        &cid,
        &account.0,
        active_persona.as_deref(),
        &body,
        &attachments,
    )
    .await
    {
        Ok(id) => {
            // Fire-and-forget Web Push to the guild's other members (#30). Never
            // blocks or fails the send; a no-op when push is disabled.
            crate::server::push::notify_new_message(state.clone(), id.clone(), account.0.clone());
            (StatusCode::CREATED, Json(SendMessageResponse { id })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "persist_message failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

async fn persist_message(
    state: &AppState,
    cid: &str,
    author: &str,
    persona: Option<&str>,
    body: &str,
    attachments: &[String],
) -> surrealdb::Result<String> {
    // `persona` is optional; only set the field when the caller is wearing
    // one, so a personaless author leaves it NONE.
    let sql = if persona.is_some() {
        // Snapshot the worn persona's name/description onto the row so the
        // message survives the persona being renamed or deleted later.
        "CREATE message SET
            channel = type::record('channel', $cid),
            author  = type::record('account', $author),
            persona = type::record('persona', $persona),
            persona_name = (SELECT VALUE name FROM ONLY type::record('persona', $persona)),
            persona_description = (SELECT VALUE description FROM ONLY type::record('persona', $persona)),
            persona_color = (SELECT VALUE color FROM ONLY type::record('persona', $persona)),
            persona_avatar = (SELECT VALUE avatar FROM ONLY type::record('persona', $persona)),
            body    = $body,
            attachments = $attachments
            RETURN meta::id(id) AS id_key;"
    } else {
        "CREATE message SET
            channel = type::record('channel', $cid),
            author  = type::record('account', $author),
            body    = $body,
            attachments = $attachments
            RETURN meta::id(id) AS id_key;"
    };
    let mut q = state
        .db
        .query(sql)
        .bind(("cid", cid.to_string()))
        .bind(("author", author.to_string()))
        .bind(("body", body.to_string()))
        .bind(("attachments", attachments.to_vec()));
    if let Some(persona) = persona {
        q = q.bind(("persona", persona.to_string()));
    }
    let mut resp = q.await?.check()?;
    let row: Option<IdRow> = resp.take(0)?;
    row.map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("CREATE message returned no row".to_string()))
}

/// True when every id in `ids` names an existing `media_blob` (empty → true).
/// Stops a message from persisting a dangling attachment reference.
async fn all_media_exist(state: &AppState, ids: &[String]) -> surrealdb::Result<bool> {
    if ids.is_empty() {
        return Ok(true);
    }
    let mut resp = state
        .db
        .query("SELECT VALUE meta::id(id) FROM media_blob WHERE meta::id(id) IN $ids;")
        .bind(("ids", ids.to_vec()))
        .await?
        .check()?;
    let found: Vec<String> = resp.take(0)?;
    Ok(ids.iter().all(|id| found.contains(id)))
}
