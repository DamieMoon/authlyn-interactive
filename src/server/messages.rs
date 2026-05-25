//! `POST /channels/{cid}/messages` and `GET /channels/{cid}/messages`
//! (phase-1 build step 3).
//!
//! Channel-scoped, server-trusted (plaintext) messages with the proven
//! `(sent_at, id)` composite-cursor pagination. The author comes from the
//! session ([`AuthAccount`]); the "speaking-as" persona is resolved
//! server-side from the caller's `guild_member.active_persona` for the
//! channel's guild — never trusted from the body. `body` is stored verbatim
//! (it may contain [`crate::markup`] formatting, rendered client-side).
//!
//! ## Privacy 404s
//! Unknown channel and caller-not-a-member-of-the-channel's-guild both
//! surface as `404 "channel not found"` — membership stays non-leaky.
//!
//! ## Composite cursor (SurrealDB 3.1.0-beta.3)
//! Carried over verbatim from the retired room messages: bind `$since`
//! through `type::datetime(...)` (a plain string compares lexically and
//! re-delivers the boundary row), project `sent_at` RAW (never `<string>`
//! cast — that lex-mis-orders at sub-second format boundaries; see
//! `server::datetime`), and `ORDER BY` the projected aliases.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use surrealdb::types::{Datetime, SurrealValue};

use crate::protocol::{
    ErrorBody, ListMessagesResponse, MessageEnvelope, SendMessageRequest, SendMessageResponse,
};
use crate::server::auth::AuthAccount;
use crate::server::datetime::to_rfc3339_fixed;
use crate::server::state::AppState;

/// Max messages returned per `GET`. Callers iterate with the cursor for more.
/// `i64` to bind directly to SurrealQL `int`.
const MESSAGES_PAGE_LIMIT: i64 = 100;

/// Max characters in a message body (markup included).
const MAX_BODY_CHARS: usize = 4000;

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
    if body.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "message body must not be empty");
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return error_response(StatusCode::BAD_REQUEST, "message body too long");
    }

    let access = match channel_access(&state, &cid, &account.0).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "channel_access failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    let active_persona = match access {
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

    match persist_message(&state, &cid, &account.0, active_persona.as_deref(), &body).await {
        Ok(id) => (StatusCode::CREATED, Json(SendMessageResponse { id })).into_response(),
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
) -> surrealdb::Result<String> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    // `persona` is optional; only set the field when the caller is wearing
    // one, so a personaless author leaves it NONE.
    let sql = if persona.is_some() {
        "CREATE message SET
            channel = type::record('channel', $cid),
            author  = type::record('account', $author),
            persona = type::record('persona', $persona),
            body    = $body
            RETURN meta::id(id) AS id_key;"
    } else {
        "CREATE message SET
            channel = type::record('channel', $cid),
            author  = type::record('account', $author),
            body    = $body
            RETURN meta::id(id) AS id_key;"
    };
    let mut q = state
        .db
        .query(sql)
        .bind(("cid", cid.to_string()))
        .bind(("author", author.to_string()))
        .bind(("body", body.to_string()));
    if let Some(persona) = persona {
        q = q.bind(("persona", persona.to_string()));
    }
    let mut resp = q.await?.check()?;
    let row: Option<IdRow> = resp.take(0)?;
    row.map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("CREATE message returned no row".to_string()))
}

// ---------------------------------------------------------------------------
// GET /channels/{cid}/messages
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub since: Option<String>,
    pub after_id: Option<String>,
}

#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn list_messages(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    Query(cursor): Query<ListMessagesQuery>,
    account: AuthAccount,
) -> Response {
    let parsed_cursor = match parse_cursor(&cursor) {
        Ok(c) => c,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };

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

    match load_messages(&state, &cid, parsed_cursor).await {
        Ok(messages) => (StatusCode::OK, Json(ListMessagesResponse { messages })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_messages failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

enum CursorState {
    None,
    Both { since: String, after_id: String },
}

fn parse_cursor(q: &ListMessagesQuery) -> Result<CursorState, &'static str> {
    match (&q.since, &q.after_id) {
        (None, None) => Ok(CursorState::None),
        (Some(since), Some(after_id)) => {
            let since = since.trim();
            let after_id = after_id.trim();
            if !is_rfc3339(since) {
                return Err("since must be RFC3339 datetime");
            }
            if after_id.is_empty() {
                return Err("after_id must not be empty");
            }
            Ok(CursorState::Both {
                since: since.to_string(),
                after_id: after_id.to_string(),
            })
        }
        _ => Err("since and after_id must be provided together"),
    }
}

/// Necessary-condition RFC 3339 shape probe (maps malformed cursors to a
/// typed 400 instead of letting SurrealDB's parse error bubble to a 500).
fn is_rfc3339(s: &str) -> bool {
    if s.len() < 20 {
        return false;
    }
    let b = s.as_bytes();
    b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':'
}

#[derive(SurrealValue)]
struct MessageRow {
    id_key: String,
    author_key: String,
    persona_id: Option<String>,
    persona_name: Option<String>,
    body: String,
    tier: String,
    sent_at: Datetime,
}

impl MessageRow {
    fn into_envelope(self) -> MessageEnvelope {
        MessageEnvelope {
            id: self.id_key,
            author_id: self.author_key,
            persona_id: self.persona_id,
            persona_name: self.persona_name,
            body: self.body,
            tier: self.tier,
            sent_at: to_rfc3339_fixed(self.sent_at),
        }
    }
}

async fn load_messages(
    state: &AppState,
    cid: &str,
    cursor: CursorState,
) -> surrealdb::Result<Vec<MessageEnvelope>> {
    // `persona_id` is null-safe (the IF guard avoids meta::id(NONE));
    // `persona.name` traversal yields NONE when there's no persona.
    const PROJECTION: &str = "
        meta::id(id)     AS id_key,
        meta::id(author) AS author_key,
        (IF persona != NONE THEN meta::id(persona) ELSE NONE END) AS persona_id,
        persona.name     AS persona_name,
        body,
        tier,
        sent_at";

    let (sql, bound) = match cursor {
        CursorState::None => (
            format!(
                "SELECT {PROJECTION} FROM message
                    WHERE channel = type::record('channel', $cid)
                    ORDER BY sent_at ASC, id_key ASC LIMIT $page_limit;"
            ),
            None,
        ),
        CursorState::Both { since, after_id } => (
            format!(
                "SELECT {PROJECTION} FROM message
                    WHERE channel = type::record('channel', $cid)
                      AND (sent_at > type::datetime($since)
                           OR (sent_at = type::datetime($since) AND meta::id(id) > $after_id))
                    ORDER BY sent_at ASC, id_key ASC LIMIT $page_limit;"
            ),
            Some((since, after_id)),
        ),
    };

    let mut q = state
        .db
        .query(sql)
        .bind(("cid", cid.to_string()))
        .bind(("page_limit", MESSAGES_PAGE_LIMIT));
    if let Some((since, after_id)) = bound {
        q = q.bind(("since", since)).bind(("after_id", after_id));
    }
    let mut resp = q.await?.check()?;
    let rows: Vec<MessageRow> = resp.take(0)?;
    Ok(rows.into_iter().map(MessageRow::into_envelope).collect())
}

// ---------------------------------------------------------------------------
// Shared: channel access (membership gate + kind + active persona)
// ---------------------------------------------------------------------------

struct ChannelCtx {
    kind: String,
    active_persona: Option<String>,
}

enum AccessOutcome {
    Ok(ChannelCtx),
    ChannelNotFound,
    NotMember,
}

/// Resolve a channel to its guild + kind, then check the caller's membership
/// of that guild and read their active persona for it. The two unknowns
/// (no such channel / caller not a member) are distinct internally but the
/// handlers collapse both to a privacy-404.
async fn channel_access(
    state: &AppState,
    cid: &str,
    account: &str,
) -> surrealdb::Result<AccessOutcome> {
    #[derive(SurrealValue)]
    struct ChanRow {
        guild_key: String,
        kind: String,
    }
    #[derive(SurrealValue)]
    struct MemRow {
        persona_id: Option<String>,
    }

    let mut resp = state
        .db
        .query("SELECT meta::id(guild) AS guild_key, kind FROM type::record('channel', $cid);")
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    let Some(chan) = resp.take::<Option<ChanRow>>(0)? else {
        return Ok(AccessOutcome::ChannelNotFound);
    };

    let mut resp = state
        .db
        .query(
            "SELECT (IF active_persona != NONE THEN meta::id(active_persona) ELSE NONE END)
                AS persona_id
                FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $account);",
        )
        .bind(("gid", chan.guild_key))
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    let Some(mem) = resp.take::<Option<MemRow>>(0)? else {
        return Ok(AccessOutcome::NotMember);
    };

    Ok(AccessOutcome::Ok(ChannelCtx {
        kind: chan.kind,
        active_persona: mem.persona_id,
    }))
}

// ---------------------------------------------------------------------------
// Shaping
// ---------------------------------------------------------------------------

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(msg))).into_response()
}

fn json_rejection_response(rej: JsonRejection) -> Response {
    let reason: &'static str = match rej {
        JsonRejection::JsonDataError(_) => "invalid JSON body shape",
        JsonRejection::JsonSyntaxError(_) => "malformed JSON",
        JsonRejection::MissingJsonContentType(_) => "missing Content-Type: application/json",
        JsonRejection::BytesRejection(_) => "could not read request body",
        _ => "invalid JSON request",
    };
    error_response(StatusCode::BAD_REQUEST, reason)
}
