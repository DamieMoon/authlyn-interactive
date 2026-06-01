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
    // Dedupe via a HashSet so the work is O(n), not the prior O(n²) linear scan
    // over the fully-untrusted attachment_ids vector — the MAX_ATTACHMENTS cap is
    // checked after dedup, so without this a body packed with distinct ids (up to
    // the 512 KiB limit) cost quadratic CPU before the cap fired (review F-D12-4).
    let mut seen = std::collections::HashSet::new();
    let mut attachments: Vec<String> = Vec::new();
    for id in req.attachment_ids {
        if seen.insert(id.clone()) {
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

    // Reply target (L-3): the parent must exist, live in THIS channel, and not be
    // soft-deleted — else 400. Validated before any write so a reply never stores
    // a dangling / cross-channel / deleted-parent link. NONE when not a reply.
    let reply_to = match req.reply_to_id.as_deref().map(str::trim) {
        Some(rid) if !rid.is_empty() => match reply_target_valid(&state, &cid, rid).await {
            Ok(true) => Some(rid.to_string()),
            Ok(false) => return error_response(StatusCode::BAD_REQUEST, "invalid reply target"),
            Err(e) => {
                tracing::error!(error = %e, "reply target check failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        },
        _ => None,
    };

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
    // Attribution is decided at send time and re-derived server-side on EVERY
    // send. The client may SUGGEST persona_id; we honor it only if the caller may
    // still edit it, else fall back to the stored per-channel wear — but that
    // stored `channel_active_persona` value is ALSO re-checked here, never
    // trusted: a revoked editor or a deleted persona must not keep stamping via a
    // stale wear row (the row is cleared on revoke/leave/delete, and this re-gate
    // is the defense-in-depth that holds even if a cleanup path is ever missed).
    // Final fallback: speak as the bare account.
    let mut active_persona: Option<String> = None;
    for candidate in [req.persona_id.as_deref(), stored_persona.as_deref()]
        .into_iter()
        .flatten()
    {
        match crate::server::permissions::can_edit_persona(&state, candidate, &account.0).await {
            Ok(true) => {
                active_persona = Some(candidate.to_string());
                break;
            }
            Ok(false) => continue,
            Err(e) => {
                tracing::error!(error = %e, "can_edit_persona failed");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
            }
        }
    }

    match persist_message(
        &state,
        &cid,
        &account.0,
        active_persona.as_deref(),
        &body,
        &attachments,
        reply_to.as_deref(),
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
    reply_to: Option<&str>,
) -> surrealdb::Result<String> {
    // `persona` is optional; only set the field when the caller is wearing
    // one, so a personaless author leaves it NONE. `reply_to` is likewise only
    // set when this is a reply, so a non-reply leaves it NONE. Both are spliced
    // as static column fragments (no user values in the SQL text); the values
    // ride in via `.bind()`.
    let persona_set = if persona.is_some() {
        // Snapshot the worn persona's name/description onto the row so the
        // message survives the persona being renamed or deleted later.
        ",
            persona = type::record('persona', $persona),
            persona_name = (SELECT VALUE name FROM ONLY type::record('persona', $persona)),
            persona_description = (SELECT VALUE description FROM ONLY type::record('persona', $persona)),
            persona_color = (SELECT VALUE color FROM ONLY type::record('persona', $persona)),
            persona_avatar = (SELECT VALUE avatar FROM ONLY type::record('persona', $persona))"
    } else {
        ""
    };
    let reply_set = if reply_to.is_some() {
        ",
            reply_to = type::record('message', $reply_to)"
    } else {
        ""
    };
    let sql = format!(
        "CREATE message SET
            channel = type::record('channel', $cid),
            author  = type::record('account', $author),
            body    = $body,
            attachments = $attachments{persona_set}{reply_set}
            RETURN meta::id(id) AS id_key;"
    );
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
    if let Some(reply_to) = reply_to {
        q = q.bind(("reply_to", reply_to.to_string()));
    }
    let mut resp = q.await?.check()?;
    let row: Option<IdRow> = resp.take(0)?;
    row.map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("CREATE message returned no row".to_string()))
}

/// True iff `rid` names a message that exists, lives in channel `cid`, and is
/// not soft-deleted (L-3 reply target validation). Parameterized via
/// `type::record` / `.bind`; a missing row, a cross-channel row, or a
/// soft-deleted row all return false (caller maps to a 400).
async fn reply_target_valid(state: &AppState, cid: &str, rid: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query(
            "SELECT VALUE meta::id(id) FROM type::record('message', $rid)
                WHERE channel = type::record('channel', $cid)
                  AND deleted_at = NONE;",
        )
        .bind(("rid", rid.to_string()))
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    let found: Vec<String> = resp.take(0)?;
    Ok(!found.is_empty())
}

/// True when every id in `ids` names an existing `media_blob` (empty → true).
/// Stops a message from persisting a dangling attachment reference.
///
/// W5/H4: binds the ids as `RecordId`s and reads them via `FROM $records` so
/// SurrealDB plans a per-record `RecordIdScan` (Union of PK lookups, gated
/// by `id IS NOT NONE` to drop missing rows) instead of a full `TableScan`
/// — which was the actual plan for `WHERE meta::id(id) IN $ids` on
/// 3.1.0-beta.3 (verified via `EXPLAIN`).
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
