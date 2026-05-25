//! Server-side axum bits: the shared [`AppState`] and the route table that
//! `main.rs` mounts (and the test harness consumes via [`make_router`]).
//!
//! Phase-1 rebuild: the route table is empty for now. Handler modules land
//! per build step (auth, guilds, messages, personas, lorebook, friends), and
//! the kept infrastructure (`retry`, `datetime`) is re-wired as the first
//! handler that needs it arrives. See
//! `~/.claude/plans/synthetic-zooming-cookie.md`.

pub mod state;

use axum::Router;

pub use self::state::AppState;

/// Build the API subrouter (everything outside the Leptos handlers).
/// Empty during the rebuild; routes are merged in as handlers land.
fn api_routes() -> Router<AppState> {
    Router::new()
}

/// Build the application-specific routes bound to the given [`AppState`].
///
/// Returns a `Router<()>`: `.with_state(state)` has already been applied,
/// so this is ready to drop into `axum::serve` as-is. Tests rely on this
/// shape so they can call `Router::oneshot` without a separate state arg.
pub fn make_router(state: AppState) -> Router {
    api_routes().with_state(state)
}

/// Same routes as [`make_router`] but stays `Router<AppState>` so the
/// caller can merge other state-aware routers (e.g. Leptos) on top.
/// Used by `main.rs` — tests don't need it.
pub fn api_router() -> Router<AppState> {
    api_routes()
}
