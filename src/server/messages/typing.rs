//! Ephemeral "I am typing" indicator (#19) + the Ghost Quill live-draft store
//! (W4/T7). In-memory only; the indicator is surfaced through the message-list
//! poll, the drafts through `GET /channels/{cid}/typing-drafts`. Split from
//! `server/messages.rs` in Wave 3; behavior preserved verbatim.
//!
//! ## Ghost Quill design constraints (W4/T7)
//! - **The SSE bus stays id-only.** Draft TEXT never rides a `SyncEvent`; the
//!   existing `Typing` event only NUDGES clients to fetch the drafts endpoint,
//!   which re-checks channel membership on every call.
//! - **Opt-in both ways.** The SENDER's client attaches `draft` to its ping
//!   only when their own pref is on; the RECEIVER fetches/renders only when
//!   theirs is. The server just stores what it's given, briefly.
//! - **Absent or empty `draft` CLEARS the stored entry** — a sender toggling
//!   the pref off (or deleting their text) stops ghosting at the very next
//!   ping, and the pre-W4/T7 bare ping keeps working unchanged.
//! - **Over-cap drafts are TRUNCATED on a char boundary** (never rejected):
//!   a ping firing mid-typing must not start failing because the composer
//!   grew past [`MAX_DRAFT_CHARS`].
//! - Same mutex discipline as `AppState.typing`: tiny critical sections,
//!   never held across an `.await`; pruned opportunistically on write and
//!   read against the injectable `AppState.draft_ttl` (8s in production).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::{TypingDraftEntry, TypingPingRequest};
use crate::server::auth::AuthAccount;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::state::AppState;

use super::{channel_access, AccessOutcome};

/// How long a typing ping stays "live" (#19). A client re-pings at most every
/// ~2s while typing, so 8s comfortably bridges a few missed pings without
/// leaving a stale indicator hanging after someone stops.
pub(super) const TYPING_TTL: std::time::Duration = std::time::Duration::from_secs(8);

/// Max characters of draft text stored per `(channel, account)` (W4/T7).
/// Anything longer is truncated — see the module header for why truncation
/// beats rejection here.
const MAX_DRAFT_CHARS: usize = 2000;

// ---------------------------------------------------------------------------
// POST /channels/{cid}/typing  — ephemeral "I am typing" ping (#19, W4/T7)
// ---------------------------------------------------------------------------

/// Record that the caller is typing in this channel. Membership-gated like the
/// message routes (privacy-404 for non-members / unknown channel). On success
/// it stamps `typing[cid][account] = now` and returns 204; the indicator is
/// surfaced later by `list_messages` (the poll). No DB write — the state is
/// purely in-memory.
///
/// The body is OPTIONAL (W4/T7 Ghost Quill): a bare POST is the classic ping;
/// a JSON body may carry `draft` — the sender's current compose text — which
/// is stored (truncated to [`MAX_DRAFT_CHARS`]) for other members to fetch
/// via [`typing_drafts`]. Absent/empty `draft` clears the stored entry. A
/// malformed JSON body (when one IS sent) is the usual typed 400.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn typing_ping(
    State(state): State<AppState>,
    Path(cid): Path<String>,
    account: AuthAccount,
    payload: Result<Option<Json<TypingPingRequest>>, JsonRejection>,
) -> Response {
    // `Ok(None)` = no body / no Content-Type: the wire-compatible bare ping.
    let draft = match payload {
        Ok(body) => body.and_then(|Json(req)| req.draft),
        Err(rej) => return json_rejection_response(rej),
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

    // Tiny critical section: lock → insert → drop. No `.await` is held while the
    // mutex is locked (the membership check above already completed).
    let now = std::time::Instant::now();
    {
        let mut map = state.typing.lock().expect("typing mutex poisoned");
        map.entry(cid.clone())
            .or_default()
            .insert(account.0.clone(), now);
    }

    // Ghost Quill store (W4/T7): same lock discipline, separate mutex. Stale
    // entries are pruned on every write so the map can't accumulate dead
    // drafts between reads.
    {
        let ttl = state.draft_ttl;
        let mut drafts = state
            .typing_drafts
            .lock()
            .expect("typing_drafts mutex poisoned");
        drafts.retain(|_, (_, stamped)| now.duration_since(*stamped) < ttl);
        let key = (cid.clone(), account.0.clone());
        match draft {
            Some(text) if !text.is_empty() => {
                drafts.insert(key, (truncate_chars(text, MAX_DRAFT_CHARS), now));
            }
            // Absent or empty: the sender's pref is off / nothing typed —
            // clear immediately so toggling off stops ghosting at once.
            _ => {
                drafts.remove(&key);
            }
        }
    }

    state.emit(crate::protocol::SyncEvent::Typing { channel_id: cid });

    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// GET /channels/{cid}/typing-drafts — Ghost Quill live drafts (W4/T7)
// ---------------------------------------------------------------------------

/// List OTHER members' live (unexpired) typing drafts in this channel, as a
/// bare JSON array of [`TypingDraftEntry`]. Membership-gated with the same
/// privacy-404 as every channel route — this fetch is the ONLY way draft text
/// leaves the server, so the permission check here carries the whole design.
/// The caller's own draft is excluded (you never see your own ghost); names
/// resolve persona-first like the typing indicator. Entries are sorted by
/// `account_id` for a stable render order. Prunes stale entries on read.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn typing_drafts(
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

    // Tiny critical section: lock → prune + collect → drop. The name
    // resolution below is a DB read and happens AFTER the lock is released,
    // so the mutex is never held across an `.await`.
    let live: Vec<(String, String)> = {
        let now = std::time::Instant::now();
        let ttl = state.draft_ttl;
        let mut drafts = state
            .typing_drafts
            .lock()
            .expect("typing_drafts mutex poisoned");
        drafts.retain(|_, (_, stamped)| now.duration_since(*stamped) < ttl);
        drafts
            .iter()
            .filter(|((chan, acct), _)| *chan == cid && *acct != account.0)
            .map(|((_, acct), (text, _))| (acct.clone(), text.clone()))
            .collect()
    };

    let accounts: Vec<String> = live.iter().map(|(acct, _)| acct.clone()).collect();
    let names = match super::reading::resolve_display_names(&state, &cid, &accounts).await {
        Ok(pairs) => pairs
            .into_iter()
            .collect::<std::collections::HashMap<String, String>>(),
        Err(e) => {
            tracing::error!(error = %e, "resolve_display_names failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    let mut entries: Vec<TypingDraftEntry> = live
        .into_iter()
        // A vanished account has no name row — drop its draft (same
        // degradation as the typing indicator).
        .filter_map(|(acct, draft)| {
            names.get(&acct).map(|name| TypingDraftEntry {
                account_id: acct.clone(),
                display_name: name.clone(),
                draft,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.account_id.cmp(&b.account_id));

    (StatusCode::OK, Json(entries)).into_response()
}

/// Drop the author's stored draft for this channel (W4/T7 clear-on-send).
/// Called from the success paths of `post_message` and `roll_message` —
/// without it a ghost row would linger beside the just-landed real message
/// for up to the TTL. Lock discipline as everywhere: lock → remove → drop,
/// no `.await` in scope.
pub(super) fn clear_draft(state: &AppState, cid: &str, account: &str) {
    let mut drafts = state
        .typing_drafts
        .lock()
        .expect("typing_drafts mutex poisoned");
    drafts.remove(&(cid.to_string(), account.to_string()));
}

/// Truncate `text` to at most `max` CHARS, always on a char boundary (a byte
/// index would split a multi-byte char and panic in `String::truncate`).
/// Returns the original string untouched when it's already within the cap.
fn truncate_chars(mut text: String, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((byte_idx, _)) => {
            text.truncate(byte_idx);
            text
        }
        None => text,
    }
}
