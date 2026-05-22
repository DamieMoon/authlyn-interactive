//! Server-side axum bits: shared [`AppState`] and the route table that
//! `main.rs` mounts (plus the test harness consumes via `make_router`).

pub mod keys;
pub mod state;

use axum::routing::post;
use axum::Router;
use tower_http::limit::RequestBodyLimitLayer;

pub use self::state::AppState;

/// Hard cap on the size of any request body the `/keys/*` routes will
/// accept. A normal bundle is ~150 B per OTK + a fixed header overhead,
/// so 64 KiB comfortably covers `MAX_OTKS_PER_PUBLISH = 200` OTKs while
/// still bounding what an adversarial client can push at us.
const KEYS_BODY_LIMIT_BYTES: usize = 64 * 1024;

/// Build the keys subrouter (just the `/keys/*` routes plus the shared
/// body-size limit). Used internally by both [`make_router`] and
/// [`api_router`] so the layer can't drift between the two entry points.
fn keys_routes() -> Router<AppState> {
    Router::new()
        // axum 0.8 uses `{param}` braces, not `:param` colons.
        .route("/keys/upload", post(keys::upload_keys))
        .route("/keys/claim/{user}/{device}", post(keys::claim_key))
        .layer(RequestBodyLimitLayer::new(KEYS_BODY_LIMIT_BYTES))
}

/// Build the application-specific routes (everything outside the Leptos
/// handlers) bound to the given [`AppState`].
///
/// Returns a `Router<()>`: `.with_state(state)` has already been applied,
/// so this is ready to drop into `axum::serve` as-is. Tests rely on this
/// shape so they can call `Router::oneshot` without a separate state
/// argument.
pub fn make_router(state: AppState) -> Router {
    keys_routes().with_state(state)
}

/// Same routes as [`make_router`] but stays `Router<AppState>` so the
/// caller can merge other state-aware routers (e.g. Leptos) on top.
///
/// Used by `main.rs` — tests don't need it.
pub fn api_router() -> Router<AppState> {
    keys_routes()
}
