//! Server-side axum bits: the shared [`AppState`] and the route table that
//! `main.rs` mounts (and the test harness consumes via [`make_router`]).
//!
//! Routes split into two body-limit groups: JSON routes under a tight 64 KiB
//! cap, and media upload/download under a 16 MiB cap. The split is required
//! because `RequestBodyLimitLayer` composes with min-limit semantics — a
//! larger inner cap under a smaller outer one still rejects at the smaller
//! one, so the two caps must live on disjoint route groups.

pub mod auth;
pub mod emoji;
pub mod feedback;
pub mod friends;
pub mod guilds;
pub mod lorebook;
pub mod media;
pub mod messages;
pub mod personas;
pub mod push;
pub mod retry;
pub mod state;

// Internal wire-format helper (raw SurrealDB `datetime` -> fixed RFC 3339).
// Used by `messages::load_messages`; kept private to the server module.
mod datetime;

use axum::extract::DefaultBodyLimit;
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
        // Soft-delete trash (#22). `/guilds/trash` is static so it wins over
        // `/guilds/{id}` in axum's router regardless of declaration order.
        .route("/guilds/trash", get(guilds::list_deleted_guilds))
        // Per-user guild rail order (#17/FB2): replace the caller's order.
        .route("/rail/order", put(guilds::set_rail_order))
        .route(
            "/guilds/{id}",
            get(guilds::get_guild)
                .patch(guilds::patch_guild)
                .delete(guilds::delete_guild),
        )
        .route("/guilds/{id}/restore", post(guilds::restore_guild))
        .route(
            "/guilds/{id}/trash/channels",
            get(guilds::list_deleted_channels),
        )
        .route("/guilds/{id}/channels", post(guilds::create_channel))
        .route(
            "/guilds/{id}/channels/{cid}",
            patch(guilds::patch_channel).delete(guilds::delete_channel),
        )
        .route(
            "/guilds/{id}/channels/{cid}/restore",
            post(guilds::restore_channel),
        )
        .route(
            "/guilds/{id}/members",
            get(guilds::list_members).post(guilds::invite_member),
        )
        .route("/guilds/{id}/members/{aid}", delete(guilds::remove_member))
        .route(
            "/guilds/{id}/members/{aid}/role",
            put(guilds::set_member_role),
        )
        .route(
            "/guilds/{id}/emoji",
            get(emoji::list_emoji).post(emoji::create_emoji),
        )
        .route("/guilds/{id}/emoji/{name}", delete(emoji::delete_emoji))
        .route(
            "/guilds/{id}/active-persona",
            put(personas::set_active_persona),
        )
        .route(
            "/channels/{cid}/messages",
            get(messages::list_messages).post(messages::post_message),
        )
        // Static `/messages/trash` wins over `/messages/{mid}` in axum's router.
        .route(
            "/channels/{cid}/messages/trash",
            get(messages::list_deleted_messages),
        )
        .route(
            "/channels/{cid}/messages/{mid}",
            patch(messages::edit_message).delete(messages::delete_message),
        )
        .route(
            "/channels/{cid}/messages/{mid}/restore",
            post(messages::restore_message),
        )
        // Ephemeral "is typing" ping (#19): in-memory, surfaced via the poll.
        .route("/channels/{cid}/typing", post(messages::typing_ping))
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
        // Web Push (#30): public VAPID key fetch + subscribe/unsubscribe.
        .route("/push/vapid-key", get(push::vapid_key))
        .route("/push/subscribe", post(push::subscribe))
        .route("/push/unsubscribe", post(push::unsubscribe))
        // Feedback / bug reports (#31): submit (any authed) + list (admin only).
        .route(
            "/feedback",
            get(feedback::list_feedback).post(feedback::submit_feedback),
        )
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT_BYTES))
        // Dynamic JSON API responses must never be cached (by the service
        // worker or the browser HTTP cache); a cached message list flashed
        // ancient messages on cold open before the live fetch landed.
        .layer(axum::middleware::map_response(
            |mut res: axum::response::Response| async move {
                res.headers_mut().insert(
                    axum::http::header::CACHE_CONTROL,
                    axum::http::HeaderValue::from_static("no-store"),
                );
                res
            },
        ))
}

/// Media upload/download, under the larger body cap.
fn media_routes() -> Router<AppState> {
    Router::new()
        .route("/media", post(media::upload_media))
        .route("/media/{id}", get(media::download_media))
        // axum applies its own ~2 MB DefaultBodyLimit on top of the tower-http
        // layer (min wins), which silently capped uploads well below 16 MiB and
        // failed multi-MB phone photos with "could not read multipart body".
        // Raise axum's default here too so the real cap is MEDIA_BODY_LIMIT_BYTES.
        .layer(DefaultBodyLimit::max(MEDIA_BODY_LIMIT_BYTES))
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

/// Hard-delete soft-deleted rows past their rollback window (#22): message 1h,
/// channel 1d, guild 30d. Cascades a purged channel's messages and a purged
/// guild's channels/members/messages. Idempotent; safe on an interval.
pub async fn purge_soft_deleted(state: &AppState) -> surrealdb::Result<()> {
    state
        .db
        .query(
            r#"
            DELETE message WHERE deleted_at != NONE AND deleted_at < time::now() - 1h;
            DELETE message WHERE channel IN (SELECT VALUE id FROM channel
                WHERE deleted_at != NONE AND deleted_at < time::now() - 1d);
            DELETE channel WHERE deleted_at != NONE AND deleted_at < time::now() - 1d;
            LET $g = (SELECT VALUE id FROM guild
                WHERE deleted_at != NONE AND deleted_at < time::now() - 30d);
            DELETE message WHERE channel IN (SELECT VALUE id FROM channel WHERE guild IN $g);
            DELETE channel WHERE guild IN $g;
            DELETE guild_member WHERE guild IN $g;
            DELETE guild WHERE id IN $g;
            "#,
        )
        .await?
        .check()?;
    Ok(())
}

/// Spawn the purge sweep: runs once shortly after boot, then hourly.
pub fn spawn_purge_sweep(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(e) = purge_soft_deleted(&state).await {
                tracing::error!(error = %e, "purge sweep failed");
            }
        }
    });
}
