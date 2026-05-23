//! Server-side axum bits: shared [`AppState`], shared infrastructure
//! ([`retry`]), and the route table that `main.rs` mounts (plus the test
//! harness consumes via `make_router`).

pub mod keys;
pub mod keyshare;
pub mod media;
pub mod messages;
pub mod retry;
pub mod rooms;
pub mod state;

// Internal: wire-format helper for converting raw SurrealDB `datetime`
// columns into fixed-precision RFC 3339 strings on the way out to JSON.
// Used by `messages::load_messages` + `keyshare::drain` so they share
// one format string (drift here re-introduces the cursor-ordering bug
// they jointly close). Kept private — no out-of-server-module callers.
mod datetime;

use axum::routing::{get, post};
use axum::Router;
use tower_http::limit::RequestBodyLimitLayer;

pub use self::state::AppState;

/// Hard cap on the size of any request body the JSON/keyshare routes
/// will accept. A normal pre-key bundle is ~150 B per OTK + fixed header
/// overhead, so 64 KiB comfortably covers `MAX_OTKS_PER_PUBLISH = 200`
/// OTKs while still bounding what an adversarial client can push at us.
/// Keyshare deposits are a few hundred bytes each (one Olm envelope per
/// POST), so they sit well inside the same cap. Media uploads are NOT
/// under this layer — see [`MEDIA_BODY_LIMIT_BYTES`].
const REQUEST_BODY_LIMIT_BYTES: usize = 64 * 1024;

/// Hard cap on the size of a single `POST /media` body. 16 MiB clears
/// the 1 MiB acceptance-test target with comfortable headroom for a
/// large image while still bounding adversarial upload size. The cap is
/// applied to the media subrouter only; the JSON/keyshare routes keep
/// the much tighter [`REQUEST_BODY_LIMIT_BYTES`].
const MEDIA_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// JSON + keyshare routes, sharing the small body cap.
fn small_body_routes() -> Router<AppState> {
    Router::new()
        // axum 0.8 uses `{param}` braces, not `:param` colons.
        .route("/keys/upload", post(keys::upload_keys))
        .route("/keys/claim/{user}/{device}", post(keys::claim_key))
        .route("/rooms", post(rooms::create_room))
        .route("/rooms/{id}/join", post(rooms::join_room))
        .route("/rooms/{id}/leave", post(rooms::leave_room))
        .route("/rooms/{id}/keyshare", post(keyshare::deposit_keyshare))
        .route("/rooms/{id}/keyshare/inbox", get(keyshare::drain_inbox))
        .route(
            "/rooms/{id}/messages",
            post(messages::post_message).get(messages::list_messages),
        )
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT_BYTES))
}

/// Media upload + download routes, under their own larger body cap.
/// Split out as a separate sub-router because `RequestBodyLimitLayer`
/// composes with min-limit semantics: a 16 MiB inner layer under the
/// 64 KiB outer layer would still reject at 64 KiB because the outer
/// `Limited<Body>` reads first. The two layers therefore have to live
/// on disjoint route groups, merged here.
fn media_routes() -> Router<AppState> {
    Router::new()
        .route("/media", post(media::upload_media))
        .route("/media/{id}", get(media::download_media))
        .layer(RequestBodyLimitLayer::new(MEDIA_BODY_LIMIT_BYTES))
}

/// Build the API subrouter (everything outside the Leptos handlers).
/// Used internally by both [`make_router`] and [`api_router`] so the
/// layer composition can't drift between the two entry points.
fn api_routes() -> Router<AppState> {
    Router::new()
        .merge(small_body_routes())
        .merge(media_routes())
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
///
/// Used by `main.rs` — tests don't need it.
pub fn api_router() -> Router<AppState> {
    api_routes()
}
