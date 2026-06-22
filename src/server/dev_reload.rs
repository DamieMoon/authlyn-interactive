//! `POST /admin/dev/reload` — app-admin dev hot-reload nudge. ssr-only.
//!
//! The test deck runs the COMPILED release binary, so it has no cargo-leptos
//! dev live-reload: when a fresh build is deployed there, already-connected
//! browser/PWA clients keep running the OLD bundle until manually refreshed.
//! This endpoint broadcasts a global, payload-free [`SyncEvent::Reload`] over
//! the existing SSE bus; every connected client's listener
//! (`ui/shell/act/sync.rs`) then calls `location.reload()` onto the new
//! version. The nudge carries no body, so it is fully compatible with the
//! id-only SSE invariant — the SIGNAL is the named frame itself.
//!
//! The endpoint is admin-gated (`is_admin`, fail-closed → 403). The broadcast
//! core ([`broadcast_reload`]) is auth-free and exposed so integration tests
//! can exercise it directly — the admin-ALLOWED path can't be driven through
//! HTTP because `is_admin` reads process env that races the parallel test
//! workers (see `tests/system_messages.rs` / `tests/feedback.rs`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::protocol::SyncEvent;
use crate::server::auth::AuthAccount;
use crate::server::errors::error_response;
use crate::server::permissions::is_admin;
use crate::server::state::AppState;

/// POST /admin/dev/reload — admin-only. Broadcasts a global, payload-free
/// reload nudge to every connected client (test-deck auto-refresh after a
/// deploy). No request body is read.
pub async fn dev_reload(State(state): State<AppState>, account: AuthAccount) -> Response {
    match is_admin(&state, &account.0).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::FORBIDDEN, "forbidden"),
        Err(e) => {
            tracing::error!(error = %e, "admin check failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }

    broadcast_reload(&state);
    StatusCode::NO_CONTENT.into_response()
}

/// Broadcast a global, payload-free reload nudge over the SSE bus. Auth-free
/// core — the HTTP handler gates admin. Best-effort like every bus emission
/// ([`AppState::emit`]): a send with no live subscribers is the idle no-op, not
/// an error. The events handler delivers [`SyncEvent::Reload`] to EVERY
/// connection (bypassing the per-connection visibility filter) as a distinct
/// named `event: reload` frame.
pub fn broadcast_reload(state: &AppState) {
    // `emit` uses the visibility-filtered/global lane (`targets: None`); the
    // events handler special-cases `Reload` to bypass that filter entirely, so
    // the global lane is exactly right here.
    state.emit(SyncEvent::Reload);
}
