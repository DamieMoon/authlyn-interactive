//! Server-side axum bits: the shared [`AppState`] and the route table that
//! `main.rs` mounts (and the test harness consumes via [`make_router`]).
//!
//! Routes split into two body-limit groups: JSON routes under a tight 512 KiB
//! cap, and media upload/download under a 64 MiB cap. The split is required
//! because `RequestBodyLimitLayer` composes with min-limit semantics — a
//! larger inner cap under a smaller outer one still rejects at the smaller
//! one, so the two caps must live on disjoint route groups.

pub mod auth;
pub mod emoji;
pub mod events;
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
pub mod system_messages;

// Internal wire-format helper (raw SurrealDB `datetime` -> fixed RFC 3339).
// Used by `messages::load_messages`; kept private to the server module.
mod datetime;

// Shared HTTP error-response helpers (`error_response`, `json_rejection_response`),
// used by every JSON handler module. Crate-internal.
mod errors;

// Shared SurrealDB row-projection helpers (e.g. `IdRow`). Crate-internal.
mod db_helpers;

// Shared input-validation helpers (`validate_name`, `validate_emoji_name`).
// Crate-internal.
mod validate;

// Shared authorization helpers (guild role gates, persona edit-access, admin
// guard). Crate-internal.
mod permissions;

// Shared channel-membership resolution (the core behind messages'
// channel_access, lorebook's check_access, personas' is_channel_member).
// Crate-internal.
mod access;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::limit::RequestBodyLimitLayer;

pub use self::state::AppState;

/// Tight cap for JSON request bodies (auth, guilds, messages, personas, …).
const REQUEST_BODY_LIMIT_BYTES: usize = 512 * 1024;

/// Larger cap for `POST /media` image and video uploads.
const MEDIA_BODY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// JSON API routes, under the small body cap. Mutations self-gate via the
/// [`auth::AuthAccount`] extractor; `register`/`login` are public.
fn small_body_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/change-password", post(auth::change_password))
        .route("/auth/me", get(auth::me))
        // Password recovery: admin reset (admin-only), set the self-service
        // security question (authed), and the public reset flow.
        .route(
            "/auth/admin/reset-password",
            post(auth::admin_reset_password),
        )
        .route("/auth/security-question", post(auth::set_security_question))
        .route("/auth/reset/question", get(auth::get_reset_question))
        .route("/auth/reset/confirm", post(auth::confirm_password_reset))
        // W1 realtime: long-lived SSE stream of id-only sync events, filtered
        // per-connection to channels the caller may see. The group's no-store
        // Cache-Control layer is correct for SSE.
        .route("/events", get(events::events))
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
            "/channels/{cid}/active-persona",
            put(personas::set_channel_active_persona),
        )
        // Cross-device read state (L-1): the caller's per-channel read cursors.
        // Static `/channels/read-state` is declared before the `/channels/{cid}/…`
        // family; axum's router prefers a static segment over a `{cid}` capture
        // regardless of order, so there's no shadowing either way.
        .route("/channels/read-state", get(messages::read_state))
        .route("/channels/{cid}/mark-read", post(messages::mark_read))
        // W1: batched unread/ping summary for every visible text channel —
        // one request instead of a poll per channel.
        .route("/unread", get(messages::unread))
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
        // Fate Engine (W4/T6): server-rolled dice persisted as an immutable
        // kind='roll' message.
        .route("/channels/{cid}/roll", post(messages::roll_message))
        // Ephemeral "is typing" ping (#19): in-memory, surfaced via the poll.
        // W4/T7 Ghost Quill: the ping's optional `draft` body + the
        // permission-checked drafts read (the ONLY way draft text leaves the
        // server — the SSE bus stays id-only).
        .route("/channels/{cid}/typing", post(messages::typing_ping))
        .route(
            "/channels/{cid}/typing-drafts",
            get(messages::typing_drafts),
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
            "/personas/{id}/gallery/batch",
            post(personas::add_gallery_images_batch),
        )
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
        .route("/feedback/{id}", delete(feedback::delete_feedback))
        // App-admin broadcast (#system): post a "Nova DOT" system message into
        // every live guild's default channel. Admin-gated (is_admin → 403).
        .route(
            "/admin/system-message",
            post(system_messages::send_system_message),
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

/// `GET /sw.js` — the service worker, served from the embedded `public/sw.js`
/// with its `__BUILD_REV__` placeholder replaced by the compile-time git short
/// rev (`build.rs` sets `BUILD_REV`). A unique `CACHE_VERSION` per build makes
/// the browser see a new SW each release, which drives `register-sw.js`'s
/// "new version available" refresh banner. `no-cache` so the SW update check
/// always reads fresh bytes.
async fn serve_service_worker() -> impl axum::response::IntoResponse {
    const SW: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/public/sw.js"));
    let body = SW.replace("__BUILD_REV__", env!("BUILD_REV"));
    (
        [
            (
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/javascript; charset=utf-8"),
            ),
            (
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-cache"),
            ),
            (
                axum::http::HeaderName::from_static("service-worker-allowed"),
                axum::http::HeaderValue::from_static("/"),
            ),
        ],
        body,
    )
}

fn api_routes() -> Router<AppState> {
    Router::new()
        // The service worker, served dynamically so its CACHE_VERSION carries the
        // per-build git rev (BUILD_REV via build.rs) — every release is a new SW,
        // which drives register-sw.js's "new version available" prompt.
        .route("/sw.js", get(serve_service_worker))
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
        // The guild_member delete below uses an inline guild-subquery, NOT the
        // `$g` LET var its sibling deletes use: SurrealDB 3.1.0-beta.3 mis-plans
        // DELETE on a composite-index leading column (guild_member_pair =
        // (guild, account)) + IN + a LET var, silently matching zero rows.
        // Guard: tests/soft_delete.rs::purge_should_cascade_guild_member_rows.
        .query(
            r#"
            DELETE message WHERE deleted_at != NONE AND deleted_at < time::now() - 1h;
            -- Channel 1d purge: cascade the channel's children before the channel.
            DELETE message WHERE channel IN (SELECT VALUE id FROM channel
                WHERE deleted_at != NONE AND deleted_at < time::now() - 1d);
            DELETE lorebook_entry WHERE channel IN (SELECT VALUE id FROM channel
                WHERE deleted_at != NONE AND deleted_at < time::now() - 1d);
            DELETE channel_active_persona WHERE channel IN (SELECT VALUE id FROM channel
                WHERE deleted_at != NONE AND deleted_at < time::now() - 1d);
            DELETE channel_read_state WHERE channel IN (SELECT VALUE id FROM channel
                WHERE deleted_at != NONE AND deleted_at < time::now() - 1d);
            DELETE channel WHERE deleted_at != NONE AND deleted_at < time::now() - 1d;
            -- Guild 30d purge: cascade channels + their children + guild children.
            LET $g = (SELECT VALUE id FROM guild
                WHERE deleted_at != NONE AND deleted_at < time::now() - 30d);
            DELETE message WHERE channel IN (SELECT VALUE id FROM channel WHERE guild IN $g);
            DELETE lorebook_entry WHERE channel IN (SELECT VALUE id FROM channel WHERE guild IN $g);
            DELETE channel_active_persona WHERE channel IN (SELECT VALUE id FROM channel WHERE guild IN $g);
            DELETE channel_read_state WHERE channel IN (SELECT VALUE id FROM channel WHERE guild IN $g);
            DELETE channel WHERE guild IN $g;
            DELETE guild_member WHERE guild IN (SELECT VALUE id FROM guild
                WHERE deleted_at != NONE AND deleted_at < time::now() - 30d);
            -- custom_emoji.(guild,name) is a composite index with guild leading,
            -- so use an inline guild-subquery (not $g) to dodge the same mis-plan
            -- noted above; user_guild_order matched the same way for consistency
            -- (review F-D7-1).
            DELETE custom_emoji WHERE guild IN (SELECT VALUE id FROM guild
                WHERE deleted_at != NONE AND deleted_at < time::now() - 30d);
            DELETE user_guild_order WHERE guild IN (SELECT VALUE id FROM guild
                WHERE deleted_at != NONE AND deleted_at < time::now() - 30d);
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
