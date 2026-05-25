//! Server-side axum bits: the shared [`AppState`] and the route table that
//! `main.rs` mounts (and the test harness consumes via [`make_router`]).
//!
//! Routes split into two body-limit groups: JSON routes under a tight 64 KiB
//! cap, and media upload/download under a 16 MiB cap. The split is required
//! because `RequestBodyLimitLayer` composes with min-limit semantics — a
//! larger inner cap under a smaller outer one still rejects at the smaller
//! one, so the two caps must live on disjoint route groups.

pub mod auth;
pub mod friends;
pub mod guilds;
pub mod lorebook;
pub mod media;
pub mod messages;
pub mod personas;
pub mod retry;
pub mod state;

// Internal wire-format helper (raw SurrealDB `datetime` -> fixed RFC 3339).
// Used by `messages::load_messages`; kept private to the server module.
mod datetime;

use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::limit::RequestBodyLimitLayer;

pub use self::state::AppState;

/// Tight cap for JSON request bodies (auth, guilds, messages, personas, …).
const REQUEST_BODY_LIMIT_BYTES: usize = 512 * 1024;

/// Larger cap for `POST /media` image uploads.
const MEDIA_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// JSON API routes, under the small body cap. Mutations self-gate via the
/// [`auth::AuthAccount`] extractor; `register`/`login` are public.
fn small_body_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/change-password", post(auth::change_password))
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
        .route(
            "/guilds/{id}/members/{aid}/role",
            put(guilds::set_member_role),
        )
        .route(
            "/guilds/{id}/active-persona",
            put(personas::set_active_persona),
        )
        .route(
            "/channels/{cid}/messages",
            get(messages::list_messages).post(messages::post_message),
        )
        .route(
            "/channels/{cid}/messages/{mid}",
            patch(messages::edit_message).delete(messages::delete_message),
        )
        .route(
            "/channels/{cid}/lorebook",
            get(lorebook::list_entries).post(lorebook::create_entry),
        )
        .route(
            "/channels/{cid}/lorebook/{eid}",
            patch(lorebook::patch_entry).delete(lorebook::delete_entry),
        )
        .route(
            "/personas",
            get(personas::list_personas).post(personas::create_persona),
        )
        .route("/personas/redeem", post(personas::redeem_persona_key))
        .route(
            "/personas/{id}",
            get(personas::get_persona)
                .patch(personas::patch_persona)
                .delete(personas::delete_persona),
        )
        .route("/personas/{id}/leave", delete(personas::leave_persona))
        .route("/personas/{id}/editors", get(personas::list_editors))
        .route(
            "/personas/{id}/editors/{aid}",
            put(personas::add_editor).delete(personas::remove_editor),
        )
        .route("/personas/{id}/avatar", put(personas::set_avatar))
        .route("/personas/{id}/gallery", post(personas::add_gallery_image))
        .route(
            "/personas/{id}/gallery/{img}",
            delete(personas::remove_gallery_image),
        )
        .route(
            "/friends",
            get(friends::list_friends).post(friends::add_friend),
        )
        .route("/friends/{aid}/accept", post(friends::accept_friend))
        .route("/friends/{aid}", delete(friends::remove_friend))
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT_BYTES))
}

/// Media upload/download, under the larger body cap.
fn media_routes() -> Router<AppState> {
    Router::new()
        .route("/media", post(media::upload_media))
        .route("/media/{id}", get(media::download_media))
        .layer(RequestBodyLimitLayer::new(MEDIA_BODY_LIMIT_BYTES))
}

fn api_routes() -> Router<AppState> {
    Router::new()
        .merge(small_body_routes())
        .merge(media_routes())
}

/// Build the application-specific routes bound to the given [`AppState`].
/// Returns a `Router<()>` (state already applied) so tests can `oneshot`.
pub fn make_router(state: AppState) -> Router {
    api_routes().with_state(state)
}

/// Same routes as [`make_router`] but stays `Router<AppState>` so `main.rs`
/// can merge the Leptos handlers on top.
pub fn api_router() -> Router<AppState> {
    api_routes()
}
