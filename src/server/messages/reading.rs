//! `GET /channels/{cid}/messages` + the composite-cursor pagination + the
//! shared MSG_PROJECTION / MessageRow / typing-name resolution. Attachment
//! MIMEs join the page projection itself (T10) — no second round-trip.
//! Split from `server/messages.rs` in Wave 3; behavior preserved verbatim.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use surrealdb::types::{Datetime, SurrealValue};

use crate::protocol::{Attachment, ListMessagesResponse, MessageEnvelope, ReplyPreview};
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

    let messages = match load_messages(&state, &cid, &account.0, parsed_cursor).await {
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
/// persona name IN THIS CHANNEL — a thin wrapper over
/// [`resolve_display_names`] that drops the account-id half (the indicator
/// only formats names; Ghost Quill's drafts endpoint needs the pairs).
async fn resolve_typing_names(
    state: &AppState,
    cid: &str,
    accounts: &[String],
) -> surrealdb::Result<Vec<String>> {
    Ok(resolve_display_names(state, cid, accounts)
        .await?
        .into_iter()
        .map(|(_acct, name)| name)
        .collect())
}

/// Resolve account ids to `(account_id, display_name)` pairs, preferring each
/// account's worn persona name IN THIS CHANNEL (`channel_active_persona` →
/// `persona.name`) so the name matches how their messages are attributed,
/// falling back to the account's `display_name`/`username`. Order is not
/// significant (callers format or sort as needed). Shared by the typing
/// indicator ([`resolve_typing_names`]) and the Ghost Quill drafts endpoint
/// (`typing::typing_drafts`, W4/T7) so the persona-aware resolution can never
/// diverge between the two.
///
/// One round-trip total: a two-statement query batches every account, then a
/// HashMap merge in Rust resolves the persona-or-fallback preference. W5/H2 —
/// was N round-trips (one query per typist); the previous comment about
/// avoiding a correlated sub-SELECT inside the projection (3.1.0-beta.3
/// unevenness) is honored: this batch keeps the two table reads as
/// independent top-level SELECTs.
///
/// Review M-38: both halves used `meta::id(x) IN $accts`, the documented
/// TableScan anti-pattern this branch already fixed in
/// `posting.rs::all_media_exist` and the `attachment_mimes` arm — left here
/// on the per-keystroke Ghost Quill path. The ids now bind as `RecordId`s:
/// the account half point-reads `FROM $acct_records` (`id IS NOT NONE` drops
/// since-vanished accounts, preserving the silently-skip behavior), and the
/// persona half's `account IN $acct_records` is an equality set on the
/// PREFIX of the `(account, channel)` UNIQUE index — EXPLAIN FULL on the
/// 3.1.3 dev binary plans one `channel_active_persona_pair` IndexScan branch
/// per account (vs the prior TableScan), with the channel equality filtered
/// on those few rows.
pub(super) async fn resolve_display_names(
    state: &AppState,
    cid: &str,
    accounts: &[String],
) -> surrealdb::Result<Vec<(String, String)>> {
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
    let acct_records: Vec<surrealdb::types::RecordId> = accounts
        .iter()
        .map(|id| surrealdb::types::RecordId::new("account", id.as_str()))
        .collect();
    let mut resp = state
        .db
        .query(
            "SELECT
                meta::id(id) AS acct_id,
                (IF display_name != '' THEN display_name ELSE username END) AS fallback
             FROM $acct_records
             WHERE id IS NOT NONE;

             SELECT
                meta::id(account) AS acct_id,
                persona.name AS persona_name
             FROM channel_active_persona
             WHERE account IN $acct_records
               AND channel = type::record('channel', $cid);",
        )
        .bind(("cid", cid.to_string()))
        .bind(("acct_records", acct_records))
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
            let name = persona_by_acct
                .get(&r.acct_id)
                .cloned()
                .unwrap_or(r.fallback);
            (r.acct_id, name)
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

/// The reply-preview sub-object joined in by MSG_PROJECTION's `reply_to` arm
/// (L-3). Built from the parent message LIVE at read time; the projection only
/// emits it when the parent exists and is not soft-deleted (else the arm is
/// NONE → `Option` here is `None`), so a since-deleted parent degrades to no
/// quote rather than a dangling reference.
#[derive(SurrealValue)]
pub(super) struct ReplyPreviewRow {
    pub id: String,
    pub author_display: String,
    pub body_snippet: String,
}

/// One `{id, mime}` pair from MSG_PROJECTION's `attachment_mimes` subquery —
/// the per-row `media_blob` join that replaced the post-page mime batch (T10).
#[derive(SurrealValue)]
pub(super) struct AttachmentMimeRow {
    pub id: String,
    pub mime: String,
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
    /// `{id, mime}` pairs for this row's attachments, joined from `media_blob`
    /// inside MSG_PROJECTION. May omit since-vanished blobs; merge by id.
    pub attachment_mimes: Vec<AttachmentMimeRow>,
    pub tier: String,
    pub sent_at: Datetime,
    pub reply_to: Option<ReplyPreviewRow>,
    /// Whether the READING caller is `@`-mentioned by this message (L-4) — the
    /// projection evaluates `$caller IN pinged_users`, so it's per-reader.
    pub is_pinged: bool,
    /// `"user"` or `"system"` (Nova DOT admin broadcast). Coalesced to `"user"`
    /// in the projection so legacy rows are safe.
    pub kind: String,
    /// Delivery effect (W4/T5): `whisper`/`shout`/`spell`, or `None` for an
    /// ordinary message and on every legacy row (`option<>` field — NONE is
    /// valid, no coalesce needed).
    pub effect: Option<String>,
}

impl MessageRow {
    pub(super) fn into_envelope(self) -> MessageEnvelope {
        // Mimes ride the row itself via MSG_PROJECTION's `attachment_mimes`
        // subquery; merge by id so attachment ORDER follows `attachments`
        // (send order), not the join. A since-deleted blob keeps the empty
        // mime (client falls back to image render) — same degradation the
        // old post-page batch had.
        let mimes: std::collections::HashMap<String, String> = self
            .attachment_mimes
            .into_iter()
            .map(|r| (r.id, r.mime))
            .collect();
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
            attachments: self
                .attachments
                .into_iter()
                .map(|id| {
                    let mime = mimes.get(&id).cloned().unwrap_or_default();
                    Attachment { id, mime }
                })
                .collect(),
            tier: self.tier,
            sent_at: to_rfc3339_fixed(self.sent_at),
            reply_to: self.reply_to.map(|r| ReplyPreview {
                id: r.id,
                author_display: r.author_display,
                body_snippet: r.body_snippet,
            }),
            is_pinged: self.is_pinged,
            kind: self.kind,
            effect: self.effect,
        }
    }
}

// Shared SELECT projection for message rows — the channel list and the
// soft-delete trash list both build MessageRows from it. `persona_id` is
// null-safe (the IF guard avoids meta::id(NONE)); name/description/color come
// from the send-time snapshot with a `?? persona.*` fallback for legacy rows
// whose persona still exists (deleted personas keep their frozen snapshot).
//
// W5/H4 plan evidence for the `attachment_mimes` arm: `WHERE meta::id(id) IN
// $array` plans as a media_blob TableScan, correlated PER MESSAGE ROW —
// O(page x |media_blob|) on the hot polled path. The record-pointer form
// (FROM an array of type::record pointers) point-reads instead. `WHERE id IS
// NOT NONE` drops since-vanished blobs (a dangling pointer yields a NONE row
// that would crash meta::id). Verified via EXPLAIN FULL on the SurrealDB
// 3.1.3 server binary (the dev binary): TableScan -> DynamicScan over
// array::map(...).
//
// VERSION-SKEW WARNING (review M-32, unresolved): everything above is
// verified ONLY on the 3.1.3 dev binary. Prod (fenrir) still runs 3.0.4,
// which has NEVER executed this closure-as-FROM-source + correlated `$parent`
// shape — the pre-T10 post-page mime batch was the only form 3.0.4 ever ran —
// and no test can ever cover the skew (the suite runs against the dev
// binary; same blind spot that let the widened `message.kind` ASSERT bug
// reach prod). If 3.0.4 errors on or mis-evaluates this projection, EVERY
// message-page read 500s on prod immediately after deploy. Runbook gate
// before the first deploy of this projection: execute one MSG_PROJECTION
// SELECT against a throwaway namespace on prod's binary over HTTP /sql
// (seeded message + media_blob + whispered reply parent), or upgrade prod to
// the verified 3.1.3 first.
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
        /* record-pointer point-reads — see const doc above */
        (SELECT meta::id(id) AS id, mime
            FROM array::map(($parent.attachments ?? []), |$a| type::record('media_blob', $a))
            WHERE id IS NOT NONE) AS attachment_mimes,
        tier,
        sent_at,
        (IF reply_to != NONE AND reply_to.body != NONE AND reply_to.deleted_at = NONE THEN {
            id: meta::id(reply_to),
            author_display: (reply_to.author.display_name ?: reply_to.author.username),
            /* spoiler-leak guard (W4/T5): a whispered parent's hidden text must
               not surface through the quote snippet — masked with a fixed
               placeholder instead. */
            body_snippet: (IF reply_to.effect = 'whisper' THEN '(whisper)'
                           ELSE string::slice(reply_to.body, 0, 100) END)
         } ELSE NONE END) AS reply_to,
        (type::record('account', $caller) IN (pinged_users ?? [])) AS is_pinged,
        (kind ?? 'user') AS kind,
        effect";

async fn load_messages(
    state: &AppState,
    cid: &str,
    caller: &str,
    cursor: CursorState,
) -> surrealdb::Result<Vec<MessageEnvelope>> {
    // `bound` carries the named (datetime, id) params for the cursor arms.
    // `reverse` is set for the DESC-ordered arms (the newest page and the
    // older-history page): they ORDER BY DESC so LIMIT keeps the rows nearest
    // "now" / nearest the cursor, then we flip the page back to ASC for display.
    //
    // The two cursor arms each run as TWO statements (review M-12): an
    // equality statement covering the cursor's own `sent_at` instant (the tie
    // group, refined by the strict id tie-break) and an OPEN range over
    // everything past it. The planner cannot push the equivalent
    // single-statement OR — `sent_at > $c OR (sent_at = $c AND id > $i)` —
    // into the (channel, sent_at) index: EXPLAIN FULL on the 3.1.3 dev binary
    // planned the OR as an unbounded IndexScan over the WHOLE channel per
    // catch-up call, the split as a `[channel, $c]` point access plus a
    // `MoreThan`/`LessThan` range. Statement order = page order (tie-boundary
    // rows sit nearest the cursor on both arms); the halves are disjoint
    // (`sent_at =` vs `>`/`<`) and each is LIMITed, so concatenating and
    // truncating to the page limit yields exactly the single-statement page.
    // tests/messages.rs pins the tie-group boundary and the nearest-the-cursor
    // truncation in both directions.
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
                      AND sent_at = type::datetime($since)
                      AND meta::id(id) > $after_id
                    ORDER BY sent_at ASC, id_key ASC LIMIT $page_limit;
                 SELECT {MSG_PROJECTION} FROM message
                    WHERE channel = type::record('channel', $cid)
                      AND deleted_at = NONE
                      AND sent_at > type::datetime($since)
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
                      AND sent_at = type::datetime($before)
                      AND meta::id(id) < $before_id
                    ORDER BY sent_at DESC, id_key DESC LIMIT $page_limit;
                 SELECT {MSG_PROJECTION} FROM message
                    WHERE channel = type::record('channel', $cid)
                      AND deleted_at = NONE
                      AND sent_at < type::datetime($before)
                    ORDER BY sent_at DESC, id_key DESC LIMIT $page_limit;"
            ),
            Some(("before", before, "before_id", before_id)),
            true,
        ),
    };

    let split = bound.is_some();
    let mut q = state
        .db
        .query(sql)
        .bind(("cid", cid.to_string()))
        .bind(("caller", caller.to_string()))
        .bind(("page_limit", MESSAGES_PAGE_LIMIT));
    if let Some((k1, v1, k2, v2)) = bound {
        q = q.bind((k1, v1)).bind((k2, v2));
    }
    let mut resp = q.await?.check()?;
    let mut rows: Vec<MessageRow> = resp.take(0)?;
    if split {
        // Cursor arms: statement 0 is the tie-boundary page head, statement 1
        // the open-range tail. Truncate BEFORE the display flip so the kept
        // rows are the ones nearest the cursor — exactly what the
        // single-statement LIMIT kept.
        let tail: Vec<MessageRow> = resp.take(1)?;
        rows.extend(tail);
        rows.truncate(MESSAGES_PAGE_LIMIT as usize);
    }
    let mut out: Vec<MessageEnvelope> = rows.into_iter().map(MessageRow::into_envelope).collect();
    if reverse {
        out.reverse();
    }
    Ok(out)
}
