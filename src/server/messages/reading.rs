//! `GET /channels/{cid}/messages` + the composite-cursor pagination + the
//! shared MSG_PROJECTION / MessageRow / mime-batch / typing-name resolution.
//! Split from `server/messages.rs` in Wave 3; behavior preserved verbatim.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use surrealdb::types::{Datetime, SurrealValue};

use crate::protocol::{Attachment, ListMessagesResponse, MessageEnvelope};
use crate::server::auth::AuthAccount;
use crate::server::datetime::to_rfc3339_fixed;
use crate::server::errors::error_response;
use crate::server::state::AppState;

use super::typing::TYPING_TTL;
use super::{channel_access, AccessOutcome};

/// Max messages returned per `GET`. Callers iterate with the cursor for more.
/// `i64` to bind directly to SurrealQL `int`.
pub(super) const MESSAGES_PAGE_LIMIT: i64 = 100;

// ---------------------------------------------------------------------------
// GET /channels/{cid}/messages
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub since: Option<String>,
    pub after_id: Option<String>,
    pub before: Option<String>,
    pub before_id: Option<String>,
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

    let active_persona = match channel_access(&state, &cid, &account.0).await {
        Ok(AccessOutcome::Ok(ctx)) => ctx.active_persona,
        Ok(AccessOutcome::ChannelNotFound) | Ok(AccessOutcome::NotMember) => {
            return error_response(StatusCode::NOT_FOUND, "channel not found");
        }
        Err(e) => {
            tracing::error!(error = %e, "channel_access failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    let messages = match load_messages(&state, &cid, parsed_cursor).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "load_messages failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    // Collect the account ids still actively typing in this channel, pruning
    // expired entries opportunistically. Tiny critical section: lock → read +
    // prune → drop. The name resolution that follows (a DB read) happens AFTER
    // the lock is released, so the mutex is never held across an `.await`.
    let typing_accounts: Vec<String> = {
        let now = std::time::Instant::now();
        let mut map = state.typing.lock().expect("typing mutex poisoned");
        let mut live = Vec::new();
        if let Some(chan) = map.get_mut(&cid) {
            chan.retain(|_acct, ts| now.duration_since(*ts) < TYPING_TTL);
            // Exclude the caller — you never see "you are typing".
            live = chan
                .keys()
                .filter(|acct| *acct != &account.0)
                .cloned()
                .collect();
            if chan.is_empty() {
                map.remove(&cid);
            }
        }
        live
    };

    let typing = match resolve_typing_names(&state, &cid, &typing_accounts).await {
        Ok(names) => names,
        Err(e) => {
            // A failed name lookup must not fail the poll — degrade to no
            // indicator rather than a 500 that breaks message delivery.
            tracing::warn!(error = %e, "resolve_typing_names failed; dropping typing list");
            Vec::new()
        }
    };

    (
        StatusCode::OK,
        Json(ListMessagesResponse {
            messages,
            typing,
            active_persona,
        }),
    )
        .into_response()
}

/// Resolve typing account ids to display names, preferring each typist's worn
/// persona name IN THIS CHANNEL (`channel_active_persona` → `persona.name`) so
/// the indicator matches how their messages are attributed, falling back to the
/// account's `display_name`/`username`. Order is not significant (the client
/// formats "A and B are typing").
///
/// One round-trip total: a two-statement query batches every typist
/// (`account` IN-list + `channel_active_persona` IN-list scoped to this
/// channel), then a HashMap merge in Rust resolves the persona-or-fallback
/// preference. W5/H2 — was N round-trips (one query per typist); the
/// previous comment about avoiding a correlated sub-SELECT inside the
/// projection (3.1.0-beta.3 unevenness) is honored: this batch keeps the
/// two table reads as independent top-level SELECTs.
async fn resolve_typing_names(
    state: &AppState,
    cid: &str,
    accounts: &[String],
) -> surrealdb::Result<Vec<String>> {
    if accounts.is_empty() {
        return Ok(Vec::new());
    }
    #[derive(SurrealValue)]
    struct AccountRow {
        acct_id: String,
        fallback: String,
    }
    #[derive(SurrealValue)]
    struct PersonaRow {
        acct_id: String,
        persona_name: Option<String>,
    }
    let mut resp = state
        .db
        .query(
            "SELECT
                meta::id(id) AS acct_id,
                (IF display_name != '' THEN display_name ELSE username END) AS fallback
             FROM account
             WHERE meta::id(id) IN $accts;

             SELECT
                meta::id(account) AS acct_id,
                persona.name AS persona_name
             FROM channel_active_persona
             WHERE channel = type::record('channel', $cid)
               AND meta::id(account) IN $accts;",
        )
        .bind(("cid", cid.to_string()))
        .bind(("accts", accounts.to_vec()))
        .await?
        .check()?;
    let accounts_rows: Vec<AccountRow> = resp.take(0)?;
    let persona_rows: Vec<PersonaRow> = resp.take(1)?;

    // Build acct_id → persona-name map (only entries whose persona still has
    // a name; a since-deleted persona surfaces as `persona_name = None`, in
    // which case we fall through to the account fallback — preserving the
    // original `?? display_name ?? username` chain).
    let persona_by_acct: std::collections::HashMap<String, String> = persona_rows
        .into_iter()
        .filter_map(|r| r.persona_name.map(|n| (r.acct_id, n)))
        .collect();
    // Vanished accounts (no `AccountRow`) are silently dropped, matching the
    // prior per-account behavior (`Option::None` → skip).
    Ok(accounts_rows
        .into_iter()
        .map(|r| {
            persona_by_acct
                .get(&r.acct_id)
                .cloned()
                .unwrap_or(r.fallback)
        })
        .collect())
}

enum CursorState {
    /// No cursor: the newest page (most recent `MESSAGES_PAGE_LIMIT`), ASC.
    None,
    /// Forward from a cursor (the poll): messages newer than `(since, after_id)`.
    Both { since: String, after_id: String },
    /// Older history (scroll-up backfill): messages older than `(before, before_id)`.
    Before { before: String, before_id: String },
}

fn parse_cursor(q: &ListMessagesQuery) -> Result<CursorState, &'static str> {
    // Older-history page (scroll-up) takes precedence when present.
    match (&q.before, &q.before_id) {
        (Some(before), Some(before_id)) => {
            let before = before.trim();
            let before_id = before_id.trim();
            if !is_rfc3339(before) {
                return Err("before must be RFC3339 datetime");
            }
            if before_id.is_empty() {
                return Err("before_id must not be empty");
            }
            return Ok(CursorState::Before {
                before: before.to_string(),
                before_id: before_id.to_string(),
            });
        }
        (None, None) => {}
        _ => return Err("before and before_id must be provided together"),
    }
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

/// Necessary-condition RFC 3339 probe: maps a malformed cursor to a typed 400
/// instead of letting SurrealDB's `type::datetime` parse error bubble to a 500.
/// A full chrono parse (not just a separator-position check) rejects a value
/// with the right separators but a malformed sub-second/offset tail, e.g.
/// `2026-05-22T12:00:00Xbogus`, which the old positional probe let through
/// (review F-D4-3).
fn is_rfc3339(s: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(s).is_ok()
}

#[derive(SurrealValue)]
pub(super) struct MessageRow {
    pub id_key: String,
    pub author_key: String,
    pub author_name: String,
    pub author_display: String,
    pub persona_id: Option<String>,
    pub persona_name: Option<String>,
    pub persona_description: Option<String>,
    pub persona_color: Option<String>,
    pub persona_avatar_id: Option<String>,
    pub body: String,
    pub attachments: Vec<String>,
    pub tier: String,
    pub sent_at: Datetime,
}

impl MessageRow {
    pub(super) fn into_envelope(self) -> MessageEnvelope {
        MessageEnvelope {
            id: self.id_key,
            author_id: self.author_key,
            author_name: self.author_name,
            author_display: self.author_display,
            persona_id: self.persona_id,
            persona_name: self.persona_name,
            persona_description: self.persona_description,
            persona_color: self.persona_color,
            persona_avatar_id: self.persona_avatar_id,
            body: self.body,
            // Mimes are resolved in a single batch query after the page is
            // built (see `resolve_attachment_mimes`); placeholder empty mime
            // until then (falls back to image render if a row is missing).
            attachments: self
                .attachments
                .into_iter()
                .map(|id| Attachment {
                    id,
                    mime: String::new(),
                })
                .collect(),
            tier: self.tier,
            sent_at: to_rfc3339_fixed(self.sent_at),
        }
    }
}

// Shared SELECT projection for message rows — the channel list and the
// soft-delete trash list both build MessageRows from it. `persona_id` is
// null-safe (the IF guard avoids meta::id(NONE)); name/description/color come
// from the send-time snapshot with a `?? persona.*` fallback for legacy rows
// whose persona still exists (deleted personas keep their frozen snapshot).
pub(super) const MSG_PROJECTION: &str = "
        meta::id(id)     AS id_key,
        meta::id(author) AS author_key,
        author.username  AS author_name,
        (author.display_name ?: author.username) AS author_display,
        (IF persona != NONE THEN meta::id(persona) ELSE NONE END) AS persona_id,
        (persona_name ?? persona.name)               AS persona_name,
        (persona_description ?? persona.description)  AS persona_description,
        (persona_color ?? persona.color)             AS persona_color,
        (IF persona_avatar != NONE THEN meta::id(persona_avatar)
         ELSE (IF persona.avatar != NONE THEN meta::id(persona.avatar) ELSE NONE END) END)
            AS persona_avatar_id,
        body,
        (attachments ?? []) AS attachments,
        tier,
        sent_at";

async fn load_messages(
    state: &AppState,
    cid: &str,
    cursor: CursorState,
) -> surrealdb::Result<Vec<MessageEnvelope>> {
    // `bound` carries the named (datetime, id) params for the cursor arms.
    // `reverse` is set for the DESC-ordered arms (the newest page and the
    // older-history page): they ORDER BY DESC so LIMIT keeps the rows nearest
    // "now" / nearest the cursor, then we flip the page back to ASC for display.
    let (sql, bound, reverse) = match cursor {
        CursorState::None => (
            format!(
                "SELECT {MSG_PROJECTION} FROM message
                    WHERE channel = type::record('channel', $cid)
                      AND deleted_at = NONE
                    ORDER BY sent_at DESC, id_key DESC LIMIT $page_limit;"
            ),
            None,
            true,
        ),
        CursorState::Both { since, after_id } => (
            format!(
                "SELECT {MSG_PROJECTION} FROM message
                    WHERE channel = type::record('channel', $cid)
                      AND deleted_at = NONE
                      AND (sent_at > type::datetime($since)
                           OR (sent_at = type::datetime($since) AND meta::id(id) > $after_id))
                    ORDER BY sent_at ASC, id_key ASC LIMIT $page_limit;"
            ),
            Some(("since", since, "after_id", after_id)),
            false,
        ),
        CursorState::Before { before, before_id } => (
            format!(
                "SELECT {MSG_PROJECTION} FROM message
                    WHERE channel = type::record('channel', $cid)
                      AND deleted_at = NONE
                      AND (sent_at < type::datetime($before)
                           OR (sent_at = type::datetime($before) AND meta::id(id) < $before_id))
                    ORDER BY sent_at DESC, id_key DESC LIMIT $page_limit;"
            ),
            Some(("before", before, "before_id", before_id)),
            true,
        ),
    };

    let mut q = state
        .db
        .query(sql)
        .bind(("cid", cid.to_string()))
        .bind(("page_limit", MESSAGES_PAGE_LIMIT));
    if let Some((k1, v1, k2, v2)) = bound {
        q = q.bind((k1, v1)).bind((k2, v2));
    }
    let mut resp = q.await?.check()?;
    let rows: Vec<MessageRow> = resp.take(0)?;
    let mut out: Vec<MessageEnvelope> = rows.into_iter().map(MessageRow::into_envelope).collect();
    if reverse {
        out.reverse();
    }
    resolve_attachment_mimes(state, &mut out).await?;
    Ok(out)
}

/// Fill in each envelope's attachment `mime` from `media_blob` in ONE batch
/// query across the whole page. The persisted message row only carries the
/// media ids (`array<string>`); the MIME lives on the blob. Missing ids keep
/// their placeholder empty mime (client falls back to image render). Order is
/// preserved per envelope.
///
/// W5/H4: binds the deduped ids as `RecordId`s and reads them via
/// `FROM $records` (Union of PK `RecordIdScan`s, gated by `id IS NOT NONE`)
/// instead of `WHERE meta::id(id) IN $ids` — which `EXPLAIN` revealed plans
/// as a full `TableScan` of `media_blob` on 3.1.0-beta.3.
pub(super) async fn resolve_attachment_mimes(
    state: &AppState,
    envelopes: &mut [MessageEnvelope],
) -> surrealdb::Result<()> {
    let ids: Vec<String> = {
        let mut v: Vec<String> = envelopes
            .iter()
            .flat_map(|e| e.attachments.iter().map(|a| a.id.clone()))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    if ids.is_empty() {
        return Ok(());
    }
    #[derive(SurrealValue)]
    struct MimeRow {
        id: String,
        mime: String,
    }
    let records: Vec<surrealdb::types::RecordId> = ids
        .iter()
        .map(|id| surrealdb::types::RecordId::new("media_blob", id.as_str()))
        .collect();
    let mut resp = state
        .db
        .query("SELECT meta::id(id) AS id, mime FROM $records WHERE id IS NOT NONE;")
        .bind(("records", records))
        .await?
        .check()?;
    let rows: Vec<MimeRow> = resp.take(0)?;
    let map: std::collections::HashMap<String, String> =
        rows.into_iter().map(|r| (r.id, r.mime)).collect();
    for env in envelopes.iter_mut() {
        for att in env.attachments.iter_mut() {
            if let Some(mime) = map.get(&att.id) {
                att.mime = mime.clone();
            }
        }
    }
    Ok(())
}
