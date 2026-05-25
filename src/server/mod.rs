//! Server-side axum bits: the shared [`AppState`] and the route table that
//! `main.rs` mounts (and the test harness consumes via [`make_router`]).
//!
//! Phase-1 rebuild in progress — handler modules land per build step. The
//! kept infrastructure (`retry`, `datetime`) is re-wired as the first handler
//! that needs it arrives. See `~/.claude/plans/synthetic-zooming-cookie.md`.

pub mod auth;
pub mod guilds;
pub mod retry;
pub mod state;

use axum::routing::{delete, get, patch, post};
use axum::Router;
use tower_http::limit::RequestBodyLimitLayer;

pub use self::state::AppState;

/// Hard cap on JSON request bodies. Auth payloads are a username + a
/// password; 64 KiB is generous headroom while bounding adversarial input.
/// Media uploads (added later) get their own larger cap on a separate group.
const REQUEST_BODY_LIMIT_BYTES: usize = 64 * 1024;

/// Build the API subrouter (everything outside the Leptos handlers).
///
/// `register`/`login` are public; `logout`/`me` self-gate via the
/// [`auth::AuthAccount`] extractor (or, for logout, by reading the cookie
/// directly), so no global auth middleware is needed.
fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route(
            "/guilds",
            get(guilds::list_guilds).post(guilds::create_guild),
        )
        .route(
            "/guilds/{id}",
            get(guilds::get_guild)
                .patch(guilds::patch_guild)
                .delete(guilds::delete_guild),
        )
        .route("/guilds/{id}/channels", post(guilds::create_channel))
        .route(
            "/guilds/{id}/channels/{cid}",
            patch(guilds::patch_channel).delete(guilds::delete_channel),
        )
        .route("/guilds/{id}/members", post(guilds::invite_member))
        .route("/guilds/{id}/members/{aid}", delete(guilds::remove_member))
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT_BYTES))
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
