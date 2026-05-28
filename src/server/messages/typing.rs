//! Ephemeral "I am typing" indicator (#19). In-memory only; surfaced through
//! the message-list poll. Split from `server/messages.rs` in Wave 3; behavior
//! preserved verbatim.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::server::auth::AuthAccount;
use crate::server::errors::error_response;
use crate::server::state::AppState;

use super::{channel_access, AccessOutcome};

/// How long a typing ping stays "live" (#19). A client re-pings at most every
/// ~2s while typing, so 8s comfortably bridges a few missed pings without
/// leaving a stale indicator hanging after someone stops.
pub(super) const TYPING_TTL: std::time::Duration = std::time::Duration::from_secs(8);

// ---------------------------------------------------------------------------
// POST /channels/{cid}/typing  — ephemeral "I am typing" ping (#19)
// ---------------------------------------------------------------------------

/// Record that the caller is typing in this channel. Membership-gated like the
/// message routes (privacy-404 for non-members / unknown channel). On success
/// it stamps `typing[cid][account] = now` and returns 204; the indicator is
/// surfaced later by `list_messages` (the poll). No body, no DB write — the
/// state is purely in-memory.
#[tracing::instrument(skip_all, fields(account = %account.0, channel = %cid))]
pub async fn typing_ping(
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

    // Tiny critical section: lock → insert → drop. No `.await` is held while the
    // mutex is locked (the membership check above already completed).
    {
        let now = std::time::Instant::now();
        let mut map = state.typing.lock().expect("typing mutex poisoned");
        map.entry(cid.clone()).or_default().insert(account.0, now);
    }

    StatusCode::NO_CONTENT.into_response()
}
