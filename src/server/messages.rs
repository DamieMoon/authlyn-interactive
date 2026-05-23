//! `POST /rooms/{id}/messages` and `GET /rooms/{id}/messages`
//! (routing-plan step 8).
//!
//! Persist ciphertext + routing metadata in the `message` table and serve
//! catch-up reads via a composite `(sent_at, id)` cursor. The server is a
//! **dumb relay**: it stores ciphertext bytes, never decrypts, never enforces
//! uniqueness on the message row (see the explicit decision documented at
//! `src/storage/schema.surql:70-79` — only a non-unique `(room, sent_at)`
//! lookup index exists).
//!
//! Live delivery to currently-connected clients rides SurrealDB
//! `LIVE SELECT` queries which subscribers issue directly against the
//! shared `AppState::db` handle — there is no axum LIVE proxy in v1. This
//! handler module is server-only; nothing here is invoked from the wasm
//! bundle.
//!
//! ## Stance
//!
//! - **Defensive at the HTTP boundary.** Every wire-shape check (header,
//!   JSON body, base64 ciphertext, RFC 3339 cursor, partial-cursor 400)
//!   fires *before* the DB pre-check, and the DB pre-check fires *before*
//!   any `CREATE` — `validate-before-side-effect`. Caller identity comes
//!   from `X-Device-Id` only; the body deliberately omits
//!   `sender_device`/`sent_at`/`tier` (forgeable inputs). The 404
//!   privacy-ordering for unknown-room vs caller-not-member matches
//!   step 7's `server::rooms` body verbatim.
//! - **Adversarial for the forward-exclusion test.** Step 8 ships the
//!   cryptographic invariant test the membership state machine deferred:
//!   removed members must not decrypt new ciphertexts, even with raw
//!   `arena.db` access (i.e. bypassing the privacy-404). See
//!   `tests/messages.rs::forward_exclusion_three_user_rotation`.
//!
//! ## No transaction wrapper on POST
//!
//! The POST is a single-statement `CREATE message …`. There is no UNIQUE
//! constraint on the `message` table (dumb-relay decision), no multi-row
//! atomicity requirement, and no read-then-write pattern — so the
//! [`with_write_conflict_retry`](crate::server::retry::with_write_conflict_retry)
//! wrapper that protects `server::keyshare` / `server::rooms` is not in
//! scope here. If a future schema gains a `(room, sender_device,
//! megolm_session_id, message_index)` UNIQUE for exactly-once delivery the
//! retry wrapper goes back in, but step 8 deliberately ships without it.
//!
//! ## Composite cursor empirical findings (SurrealDB 3.1.0-beta.3)
//!
//! Three SurrealQL invariants are load-bearing and were verified
//! empirically against the dev DB before this code shipped (the original
//! probes lived in an ephemeral test binary that was deleted after the
//! findings were folded in):
//!
//! 1. **`type::datetime($since)` is mandatory.** Binding `$since` as a
//!    plain `String` and comparing `sent_at > $since` does NOT compare as
//!    a datetime — SurrealDB silently does a string compare that matches
//!    too many rows (specifically, a strict `>` against a string equal to
//!    `m1.sent_at` returns both m1 AND m2 instead of m2 only). Wrapping in
//!    `type::datetime($since)` restores datetime semantics. Without this
//!    cast the cursor would silently re-deliver the boundary row on every
//!    page.
//!
//! 2. **`<string>sent_at AS sent_at` cast is mandatory.** `#[derive(SurrealValue)]`
//!    on a struct with `sent_at: String` does NOT accept a raw `datetime`
//!    column; the take surfaces `"Expected string, got datetime"`.
//!    Projecting `<string>sent_at AS sent_at` in SQL casts at the DB layer.
//!    This mirrors `server::keyshare::drain`'s `<string>created_at AS
//!    created_at` (`keyshare.rs:344`).
//!
//! 3. **ORDER BY must reference projected idioms.** SurrealDB 3.1.0-beta.3
//!    rejects `ORDER BY meta::id(id) ASC` (or `ORDER BY id ASC` when `id`
//!    is not in the SELECT) with `"Parse error: Missing order idiom … in
//!    statement selection"`. The fix is to alias the ordered idiom in the
//!    SELECT (`<string>sent_at AS sent_at`, `meta::id(id) AS id_key`) and
//!    `ORDER BY <alias> ASC` against the alias. This applies to every
//!    composite-cursor SELECT in this module.
//!
//! ## Privacy 404s
//!
//! Unknown-room and caller-not-member both surface as `404 "room not
//! found"` — same body, same status. Returning `403` would confirm the
//! room exists. Same shape as `server::rooms` (`rooms.rs:32-37`).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    ErrorBody, ListMessagesResponse, MessageEnvelope, SendMessageRequest, SendMessageResponse,
};
use crate::server::keys::extract_device_id;
use crate::server::state::AppState;

/// Hard cap on how many messages a single `GET /rooms/:id/messages`
/// returns. Callers iterate with the composite cursor for more. 100 is
/// the same cap step 8b/9 will inherit; no `?limit=` override in v1.
///
/// Typed as `i64` to match SurrealQL's `int` directly at the bind site
/// (the single call site in `load_messages`) — avoids a lossy-cast lint
/// at the boundary.
const MESSAGES_PAGE_LIMIT: i64 = 100;

// ---------------------------------------------------------------------------
// POST /rooms/{id}/messages
// ---------------------------------------------------------------------------

/// Persist one Megolm ciphertext into the `message` table.
///
/// Wire contract:
///
/// - Auth: `X-Device-Id` → caller's `device` row → caller's `user`. Caller
///   must be a member of `room_id`.
/// - Body: [`SendMessageRequest`]. Server cross-fills `sender_device`
///   (from header), `sent_at` (`DEFAULT time::now()`), and `tier` (`'default'`).
/// - Success: `201 Created` + [`SendMessageResponse`]. The returned `id`
///   is the **client's dedup key** under dumb-relay semantics — a retried
///   POST produces a second row with a different id, and clients are
///   expected to dedup in-memory keyed on this id.
///
/// Validation table (privacy-consistent with step 7):
///
/// | Failure | Status | Body |
/// |---|---|---|
/// | Missing X-Device-Id | 401 | `missing X-Device-Id header` |
/// | Unknown caller device | 401 | `unknown caller device` |
/// | JSON parse failure | 400 | (typed via `json_rejection_response`) |
/// | Empty megolm_session_id | 400 | `megolm_session_id must not be empty` |
/// | Empty ciphertext | 400 | `ciphertext must not be empty` |
/// | Non-base64 ciphertext | 400 | `ciphertext must be base64` |
/// | Unknown room OR caller not a member | 404 | `room not found` (privacy) |
#[tracing::instrument(skip_all, fields(caller_device, caller_user, room = %room_id))]
pub async fn post_message(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<SendMessageRequest>, JsonRejection>,
) -> Response {
    let device_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("caller_device", &tracing::field::display(&device_id));

    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => {
            tracing::warn!(rejection = %rej, "JSON extraction failed");
            return json_rejection_response(rej);
        }
    };

    // Wire-shape validation, all before any DB round-trip.
    let session_id = req.megolm_session_id.trim();
    if session_id.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "megolm_session_id must not be empty",
        );
    }
    let ciphertext = req.ciphertext.trim();
    if ciphertext.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "ciphertext must not be empty");
    }
    // The server has no key material to decrypt with, so base64
    // well-formedness is the cheapest defense against schema corruption
    // (and against an admin SELECT that surfaces invalid utf8/binary).
    {
        use base64::engine::general_purpose::STANDARD as B64;
        use base64::Engine;
        if B64.decode(ciphertext).is_err() {
            return error_response(StatusCode::BAD_REQUEST, "ciphertext must be base64");
        }
    }

    // Resolve caller user. `extract_device_id` doesn't talk to the DB, so
    // unknown-device is caught here.
    let caller_user = match load_caller_user(&state, &device_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "unknown caller device"),
        Err(e) => {
            tracing::error!(error = %e, "load_caller_user failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    tracing::Span::current().record("caller_user", &tracing::field::display(&caller_user));

    // Pre-checks (read-only, separate round-trip). validate-before-side-effect:
    // unknown-room + non-member-caller both 404 BEFORE any CREATE fires.
    let pre = match message_prechecks(&state, &room_id, &caller_user).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "message_prechecks failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    match pre {
        MessagePrecheckOutcome::Ok => {}
        MessagePrecheckOutcome::RoomNotFound | MessagePrecheckOutcome::CallerNotMember => {
            return error_response(StatusCode::NOT_FOUND, "room not found");
        }
    }

    match persist_message(
        &state,
        &device_id,
        &room_id,
        session_id,
        req.message_index,
        ciphertext,
    )
    .await
    {
        Ok(id) => (StatusCode::CREATED, Json(SendMessageResponse { id })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "persist_message failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// Single-statement CREATE — no transaction wrapper, no retry loop. See the
/// module doc's "No transaction wrapper on POST" section for the rationale.
///
/// `tier` is omitted from the SET clause so SurrealDB's `DEFAULT 'default'`
/// fires (`src/storage/schema.surql:77`). `sent_at` likewise relies on the
/// schema's `DEFAULT time::now()` — making `sent_at` server-controlled is
/// what lets the (sent_at, id) cursor be a totally-ordered key.
async fn persist_message(
    state: &AppState,
    sender_device_id: &str,
    room_id: &str,
    megolm_session_id: &str,
    message_index: u32,
    ciphertext: &str,
) -> surrealdb::Result<String> {
    #[derive(SurrealValue)]
    struct Created {
        id_key: String,
    }
    let mut resp = state
        .db
        .query(
            r#"
            CREATE message SET
                room              = type::record("room", $room_id),
                sender_device     = type::record("device", $sender_id),
                megolm_session_id = $megolm_session_id,
                message_index     = $message_index,
                ciphertext        = $ciphertext
                RETURN meta::id(id) AS id_key;
            "#,
        )
        .bind(("room_id", room_id.to_string()))
        .bind(("sender_id", sender_device_id.to_string()))
        .bind(("megolm_session_id", megolm_session_id.to_string()))
        // SurrealQL `int` is i64; `u32::into` widens losslessly.
        .bind(("message_index", i64::from(message_index)))
        .bind(("ciphertext", ciphertext.to_string()))
        .await?
        .check()?;
    let rows: Vec<Created> = resp.take(0)?;
    rows.into_iter()
        .next()
        .map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("CREATE message returned no rows".to_string()))
}

// ---------------------------------------------------------------------------
// GET /rooms/{id}/messages
// ---------------------------------------------------------------------------

/// Query string for `GET /rooms/{id}/messages`. Cursor is composite —
/// `since` and `after_id` are present together or both absent. Either one
/// alone is a `400`.
#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub since: Option<String>,
    pub after_id: Option<String>,
}

/// Catch-up read for messages in `room_id`. Composite-cursor pagination.
///
/// - Auth: same as POST.
/// - Query (optional, both-or-neither): `?since=<RFC3339>&after_id=<id>`.
/// - Success: `200 OK` + [`ListMessagesResponse`]. Up to 100 envelopes,
///   ordered ASC by `(sent_at, id)`.
///
/// Validation table:
///
/// | Failure | Status | Body |
/// |---|---|---|
/// | Missing X-Device-Id | 401 | `missing X-Device-Id header` |
/// | Unknown caller device | 401 | `unknown caller device` |
/// | Malformed `?since=` | 400 | `since must be RFC3339 datetime` |
/// | Only one of `?since=`/`?after_id=` | 400 | `since and after_id must be provided together` |
/// | Unknown room OR caller not a member | 404 | `room not found` (privacy) |
///
/// The RFC 3339 check is a client-side fail-fast (`chrono` is not in scope
/// and we don't need calendar arithmetic). Instead, we hand the binding to
/// SurrealDB's `type::datetime($since)`; SurrealDB returns a parse error on
/// a malformed string. We pre-screen with a lightweight format probe
/// described in [`is_rfc3339`] so the HTTP path returns the typed 400 the
/// validation table promises rather than bubbling a Surreal parse error
/// out as a 500.
///
/// The empirical SurrealQL workarounds the SELECT depends on
/// (`type::datetime($since)` cast, `<string>sent_at AS sent_at` projection,
/// ORDER BY alias requirement) are documented in the module doc's
/// "Composite cursor empirical findings" section (`messages.rs:44-73`) and
/// applied in [`load_messages`].
#[tracing::instrument(skip_all, fields(caller_device, caller_user, room = %room_id))]
pub async fn list_messages(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Query(cursor): Query<ListMessagesQuery>,
    headers: HeaderMap,
) -> Response {
    let device_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("caller_device", &tracing::field::display(&device_id));

    let parsed_cursor = match parse_cursor(&cursor) {
        Ok(c) => c,
        Err(msg) => return error_response(StatusCode::BAD_REQUEST, msg),
    };

    let caller_user = match load_caller_user(&state, &device_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "unknown caller device"),
        Err(e) => {
            tracing::error!(error = %e, "load_caller_user failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    tracing::Span::current().record("caller_user", &tracing::field::display(&caller_user));

    let pre = match message_prechecks(&state, &room_id, &caller_user).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "message_prechecks failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    match pre {
        MessagePrecheckOutcome::Ok => {}
        MessagePrecheckOutcome::RoomNotFound | MessagePrecheckOutcome::CallerNotMember => {
            return error_response(StatusCode::NOT_FOUND, "room not found");
        }
    }

    match load_messages(&state, &room_id, parsed_cursor).await {
        Ok(messages) => (StatusCode::OK, Json(ListMessagesResponse { messages })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "load_messages failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// Parsed cursor: either both fields are present, or neither.
enum CursorState {
    None,
    Both { since: String, after_id: String },
}

fn parse_cursor(q: &ListMessagesQuery) -> Result<CursorState, &'static str> {
    match (&q.since, &q.after_id) {
        (None, None) => Ok(CursorState::None),
        (Some(since), Some(after_id)) => {
            // Trim both halves, then validate. An empty-after-trim `since`
            // falls through to the RFC3339 shape probe (which rejects on
            // `len < 20`); an empty-after-trim `after_id` is rejected
            // explicitly so the cursor never reaches SurrealDB with
            // `meta::id(id) > ""` (degenerate — matches every id at the
            // boundary). Mirrors `post_message`'s trim-and-reject on
            // `megolm_session_id` / `ciphertext` (see `:154-163`).
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

/// Lightweight RFC 3339 shape probe.
///
/// SurrealDB's `type::datetime($s)` does the authoritative parse; this
/// function exists to map a malformed input to a typed HTTP 400 instead of
/// letting it bubble up as a SurrealDB error (which would become a 500 via
/// the `load_messages` storage-error branch).
///
/// The check is *necessary-condition* only — it accepts everything Surreal
/// will accept and rejects only the obviously-not-RFC3339 inputs. False
/// positives that Surreal then rejects surface as a 500, which is the
/// pre-existing failure mode and acceptable for v1. Strictness can be
/// tightened later without touching the wire contract.
fn is_rfc3339(s: &str) -> bool {
    // RFC 3339 minimum shape: "YYYY-MM-DDTHH:MM:SSZ" = 20 chars. Anything
    // shorter is definitely not a datetime; anything with no `T` and no
    // `Z`/`+`/`-` after position 10 is definitely not either.
    if s.len() < 20 {
        return false;
    }
    let bytes = s.as_bytes();
    // YYYY-MM-DDT...
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
}

#[derive(SurrealValue)]
struct MessageRow {
    id_key: String,
    sender_device_key: String,
    megolm_session_id: String,
    message_index: i64,
    ciphertext: String,
    tier: String,
    sent_at: String,
}

impl MessageRow {
    fn into_envelope(self) -> Result<MessageEnvelope, surrealdb::Error> {
        let message_index = u32::try_from(self.message_index).map_err(|_| {
            surrealdb::Error::thrown(format!(
                "message row has out-of-range message_index: {}",
                self.message_index
            ))
        })?;
        Ok(MessageEnvelope {
            id: self.id_key,
            sender_device: self.sender_device_key,
            megolm_session_id: self.megolm_session_id,
            message_index,
            ciphertext: self.ciphertext,
            tier: self.tier,
            sent_at: self.sent_at,
        })
    }
}

/// Load up to [`MESSAGES_PAGE_LIMIT`] messages from `room_id` past the
/// optional composite cursor. ORDER BY uses the projected aliases
/// (`sent_at` and `id_key`) — see the empirical finding #3 in the module
/// doc — and ties on equal `sent_at` are broken by `meta::id(id) >
/// $after_id`. `type::datetime($since)` is mandatory (finding #1).
async fn load_messages(
    state: &AppState,
    room_id: &str,
    cursor: CursorState,
) -> surrealdb::Result<Vec<MessageEnvelope>> {
    let sql = match cursor {
        CursorState::None => {
            r#"
            SELECT
                meta::id(id)              AS id_key,
                meta::id(sender_device)   AS sender_device_key,
                megolm_session_id,
                message_index,
                ciphertext,
                tier,
                <string>sent_at           AS sent_at
            FROM message
            WHERE room = type::record("room", $room_id)
            ORDER BY sent_at ASC, id_key ASC
            LIMIT $page_limit;
            "#
        }
        CursorState::Both { .. } => {
            r#"
            SELECT
                meta::id(id)              AS id_key,
                meta::id(sender_device)   AS sender_device_key,
                megolm_session_id,
                message_index,
                ciphertext,
                tier,
                <string>sent_at           AS sent_at
            FROM message
            WHERE room = type::record("room", $room_id)
              AND (
                sent_at > type::datetime($since)
                OR (sent_at = type::datetime($since)
                    AND meta::id(id) > $after_id)
              )
            ORDER BY sent_at ASC, id_key ASC
            LIMIT $page_limit;
            "#
        }
    };

    let mut q = state
        .db
        .query(sql)
        .bind(("room_id", room_id.to_string()))
        .bind(("page_limit", MESSAGES_PAGE_LIMIT));
    if let CursorState::Both { since, after_id } = cursor {
        q = q.bind(("since", since)).bind(("after_id", after_id));
    }

    let mut resp = q.await?.check()?;
    let rows: Vec<MessageRow> = resp.take(0)?;
    rows.into_iter()
        .map(MessageRow::into_envelope)
        .collect::<Result<Vec<_>, _>>()
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

enum MessagePrecheckOutcome {
    Ok,
    RoomNotFound,
    CallerNotMember,
}

/// Read-only pre-check: room exists AND caller is a member. Both failure
/// modes collapse to the same `404 "room not found"` body in the handler
/// (see the "Privacy 404s" module-doc section). Runs outside any
/// transaction; the dumb-relay design has no TOCTOU-sensitive write that
/// would need an in-tx re-check.
async fn message_prechecks(
    state: &AppState,
    room_id: &str,
    caller_user: &str,
) -> surrealdb::Result<MessagePrecheckOutcome> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let sql = r#"
        SELECT meta::id(id) AS id_key FROM type::record("room", $room_id);
        SELECT meta::id(id) AS id_key
            FROM room_member
            WHERE room = type::record("room", $room_id)
              AND user = type::record("user", $caller_user_key);
    "#;
    let mut resp = state
        .db
        .query(sql)
        .bind(("room_id", room_id.to_string()))
        .bind(("caller_user_key", caller_user.to_string()))
        .await?
        .check()?;
    let room: Option<IdRow> = resp.take(0)?;
    if room.is_none() {
        return Ok(MessagePrecheckOutcome::RoomNotFound);
    }
    let membership: Option<IdRow> = resp.take(1)?;
    if membership.is_none() {
        return Ok(MessagePrecheckOutcome::CallerNotMember);
    }
    Ok(MessagePrecheckOutcome::Ok)
}

/// Resolve a device id (from `X-Device-Id`) to its owning user id.
/// Returns `Ok(None)` when the device row doesn't exist — handlers map
/// that to `401 "unknown caller device"`. Same shape as
/// `rooms::load_caller_user` (`rooms.rs:600-613`).
async fn load_caller_user(state: &AppState, device_id: &str) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        user_key: String,
    }
    let mut resp = state
        .db
        .query("SELECT meta::id(user) AS user_key FROM type::record('device', $device_id);")
        .bind(("device_id", device_id.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| r.user_key))
}

// ---------------------------------------------------------------------------
// HTTP response shaping (identical to keys.rs / keyshare.rs / rooms.rs)
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
        // `JsonRejection` is `#[non_exhaustive]`; catch-all required.
        _ => "invalid JSON request",
    };
    error_response(StatusCode::BAD_REQUEST, reason)
}
