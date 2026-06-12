//! Web Push (#30): VAPID-signed background notifications.
//!
//! The poll-based local notifications shipped earlier could never fire while a
//! mobile PWA was backgrounded — the page's timers are frozen, so the only
//! state where a notification was *allowed* to show (`document.hidden`) was
//! exactly the state where the code that showed it couldn't run. Background
//! delivery needs server-sent Web Push: the OS wakes the service worker via a
//! `push` event even when the page is dead.
//!
//! Flow: the client fetches the VAPID public key (`GET /push/vapid-key`),
//! subscribes via the Push API, and POSTs the subscription
//! (`POST /push/subscribe`). The server stores it in `push_subscription` and,
//! on every new message, sends an encrypted (aes128gcm) push to each guild
//! member's subscriptions — except the author's — via [`notify_new_message`].
//!
//! Configured entirely from env (`VAPID_PRIVATE_KEY` / `VAPID_PUBLIC_KEY` /
//! `VAPID_SUBJECT`). When the keys are unset, [`PushSender::from_env`] returns
//! `None` and the whole feature degrades to a no-op — the app runs fine without
//! push, and the client's `GET /push/vapid-key` 404s so it skips subscribing.

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushError, WebPushMessageBuilder,
};

use crate::protocol::{PushSubscribeRequest, PushUnsubscribeRequest, VapidKeyResponse};
use crate::server::auth::AuthAccount;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::state::AppState;

/// How long the push service should hold an undelivered message. A chat ping
/// stale by more than an hour is noise, so we don't ask it to keep trying.
const PUSH_TTL_SECS: u32 = 60 * 60;

/// Title/body payload is tiny, but the encrypted aes128gcm body must stay under
/// the push services' 4 KiB cap (Apple enforces it). Truncating the body well
/// below that keeps us clear with room for the title + channel id + overhead.
const MAX_BODY_CHARS: usize = 120;

// ---------------------------------------------------------------------------
// Sender — built once at startup, held in AppState
// ---------------------------------------------------------------------------

/// Reusable push-send context, built once from env and held in [`AppState`].
/// The isahc client owns a connection pool and is "expensive to create, cheap
/// to reuse," so there is exactly one.
pub struct PushSender {
    client: IsahcWebPushClient,
    /// VAPID private key, base64url-unpadded — the raw 32-byte P-256 scalar
    /// (the form `npx web-push generate-vapid-keys` emits and `from_base64`
    /// consumes).
    vapid_private: String,
    /// VAPID public key, base64url-unpadded (the 65-byte uncompressed point).
    /// Served to clients verbatim; the browser decodes it into the
    /// `applicationServerKey` Uint8Array.
    vapid_public: String,
    /// VAPID `sub` claim — must be a `mailto:` or `https:` URL (Apple's push
    /// endpoint rejects anything else with 403).
    subject: String,
}

impl PushSender {
    /// Build from env. Returns `None` (push disabled — every path a no-op)
    /// unless both VAPID keys are present and non-empty. `VAPID_SUBJECT`
    /// defaults to a mailto for this deployment.
    pub fn from_env() -> Option<Arc<Self>> {
        let vapid_private = std::env::var("VAPID_PRIVATE_KEY").ok()?;
        let vapid_public = std::env::var("VAPID_PUBLIC_KEY").ok()?;
        if vapid_private.trim().is_empty() || vapid_public.trim().is_empty() {
            return None;
        }
        let subject = std::env::var("VAPID_SUBJECT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "mailto:admin@authlyn.tplinkdns.com".to_string());
        let client = match IsahcWebPushClient::new() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "web-push client init failed; push disabled");
                return None;
            }
        };
        Some(Arc::new(Self {
            client,
            vapid_private,
            vapid_public,
            subject,
        }))
    }

    /// Encrypt + VAPID-sign + send one notification to a single subscription.
    /// `Ok(true)` = delivered; `Ok(false)` = the subscription is dead (HTTP
    /// 404/410 — the caller should prune it); `Err` = a transient/other failure.
    async fn send_one(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        payload: &[u8],
    ) -> Result<bool, WebPushError> {
        let sub = SubscriptionInfo::new(endpoint, p256dh, auth);

        let mut sig = VapidSignatureBuilder::from_base64(&self.vapid_private, &sub)?;
        sig.add_claim("sub", self.subject.as_str());
        let signature = sig.build()?;

        let mut builder = WebPushMessageBuilder::new(&sub);
        builder.set_payload(ContentEncoding::Aes128Gcm, payload);
        builder.set_ttl(PUSH_TTL_SECS);
        builder.set_vapid_signature(signature);
        let message = builder.build()?;

        match self.client.send(message).await {
            Ok(()) => Ok(true),
            Err(WebPushError::EndpointNotValid(_)) | Err(WebPushError::EndpointNotFound(_)) => {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
}

// ---------------------------------------------------------------------------
// GET /push/vapid-key  (public — the public key is, by definition, public)
// ---------------------------------------------------------------------------

/// The server's VAPID public key, or 404 when push isn't configured (so the
/// client knows to skip the whole subscription dance).
pub async fn vapid_key(State(state): State<AppState>) -> Response {
    match &state.push {
        Some(sender) => (
            StatusCode::OK,
            Json(VapidKeyResponse {
                key: sender.vapid_public.clone(),
            }),
        )
            .into_response(),
        None => error_response(StatusCode::NOT_FOUND, "push not configured"),
    }
}

// ---------------------------------------------------------------------------
// POST /push/subscribe  (auth) — store/refresh this browser's subscription
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn subscribe(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<PushSubscribeRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    if req.endpoint.trim().is_empty()
        || req.keys.p256dh.trim().is_empty()
        || req.keys.auth.trim().is_empty()
    {
        return error_response(StatusCode::BAD_REQUEST, "incomplete subscription");
    }

    match store_subscription(&state, &account.0, &req).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "store_subscription failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

/// Upsert by the unique `endpoint`: a browser re-subscribing (same endpoint)
/// replaces its row rather than duplicating. DELETE-then-CREATE in one
/// transaction mirrors the codebase's defensive-upsert idiom (`personas.rs`).
async fn store_subscription(
    state: &AppState,
    account: &str,
    req: &PushSubscribeRequest,
) -> surrealdb::Result<()> {
    // Idempotent DELETE-then-CREATE on the unique `endpoint`, wrapped in the
    // write-conflict retry so two concurrent re-subscribes (a service worker
    // firing twice) converge on one row and return the documented idempotent 204
    // rather than intermittently 500ing on the MVCC loser (inv13).
    crate::server::retry::with_write_conflict_retry(|| async {
        state
            .db
            .query(
                "BEGIN TRANSACTION;
                 DELETE push_subscription WHERE endpoint = $endpoint;
                 CREATE push_subscription SET
                    account  = type::record('account', $account),
                    endpoint = $endpoint,
                    p256dh   = $p256dh,
                    `auth`   = $auth_key;
                 COMMIT TRANSACTION;",
            )
            .bind(("account", account.to_string()))
            .bind(("endpoint", req.endpoint.clone()))
            .bind(("p256dh", req.keys.p256dh.clone()))
            .bind(("auth_key", req.keys.auth.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// POST /push/unsubscribe  (auth) — drop a subscription by endpoint
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(account = %account.0))]
pub async fn unsubscribe(
    State(state): State<AppState>,
    account: AuthAccount,
    payload: Result<Json<PushUnsubscribeRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    // Scoped to the caller's own rows so one account can't delete another's.
    let result = state
        .db
        .query(
            "DELETE push_subscription
                WHERE endpoint = $endpoint
                  AND account = type::record('account', $account);",
        )
        .bind(("endpoint", req.endpoint.clone()))
        .bind(("account", account.0.clone()))
        .await
        .and_then(|r| r.check());
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "unsubscribe failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

// ---------------------------------------------------------------------------
// Trigger: notify a guild's members of a new message (fire-and-forget)
// ---------------------------------------------------------------------------

/// Spawn a best-effort Web Push to every member of the message's guild except
/// the author. Never blocks or fails the send — `post_message` calls this and
/// returns immediately. A no-op when push is disabled or nobody is subscribed.
pub fn notify_new_message(state: AppState, message_id: String, author: String) {
    // Cheap early-out before spawning when push is off (tests, no VAPID env).
    if state.push.is_none() {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = notify_inner(&state, &message_id, &author).await {
            tracing::warn!(error = %e, "push notify failed");
        }
    });
}

/// Everything a new-message notification needs from the fresh message row:
/// its channel (id + name), guild, sender display (persona snapshot, else the
/// author's nickname), body, delivery effect, avatar, and ping targets.
/// `pub` so the integration suite can pin the effect-column plumbing from a
/// REAL DB row through to the masked body (tests/push.rs, review M-42) —
/// `notify_inner` itself is fire-and-forget behind a live push service and
/// has no end-to-end test.
#[derive(SurrealValue)]
pub struct NotificationInfo {
    pub channel_key: String,
    pub channel_name: String,
    pub guild_key: String,
    pub sender_name: String,
    pub body: String,
    /// The message author's persona avatar media id (snapshot ?? live
    /// fallback, same null-safe pattern as `reading.rs` MSG_PROJECTION); the
    /// SW maps it to `/media/{id}` as the notification's large image. `None`
    /// when the persona has no avatar — the SW then omits the image.
    pub sender_avatar_id: Option<String>,
    /// Delivery effect (W4/T5): `whisper`/`shout`/`spell`, or `None`. Read
    /// so a whispered body can be masked before it rides the push payload
    /// (see [`Self::notification_body`]) — the same spoiler-leak guard as the
    /// reply-quote mask in `reading.rs` MSG_PROJECTION.
    pub effect: Option<String>,
    /// Account-id keys this message `@`-mentions (L-4) — used to set a
    /// per-recipient `pinged` flag on the push payload so a future SW can
    /// style a ping differently. Empty when the message pings nobody.
    pub pinged_keys: Vec<String>,
}

impl NotificationInfo {
    /// The payload body for THIS row: the whisper mask keyed on the row's own
    /// `effect` column, then the snippet rules — the exact composition
    /// [`notify_inner`] sends, exposed as a method so the integration pin
    /// (review M-42) exercises the same effect→body thread-through.
    pub fn notification_body(&self) -> String {
        notification_body(&self.body, self.effect.as_deref())
    }
}

/// Resolve the notification payload fields for one message row, or `None`
/// when the message vanished (e.g. deleted between persist and notify). One
/// parameterized query; this is [`notify_inner`]'s row read, split out so the
/// `effect` projection → decode → mask seam is integration-testable without a
/// live push service (review M-42).
pub async fn load_notification_info(
    state: &AppState,
    mid: &str,
) -> surrealdb::Result<Option<NotificationInfo>> {
    let mut resp = state
        .db
        .query(
            "SELECT
                meta::id(channel)        AS channel_key,
                channel.name             AS channel_name,
                meta::id(channel.guild)  AS guild_key,
                (persona_name ?? (author.display_name ?: author.username)) AS sender_name,
                (IF persona_avatar != NONE THEN meta::id(persona_avatar)
                 ELSE (IF persona.avatar != NONE THEN meta::id(persona.avatar) ELSE NONE END) END)
                    AS sender_avatar_id,
                body,
                effect,
                (pinged_users ?? []).map(|$u| meta::id($u)) AS pinged_keys
             FROM type::record('message', $mid);",
        )
        .bind(("mid", mid.to_string()))
        .await?
        .check()?;
    resp.take::<Option<NotificationInfo>>(0)
}

async fn notify_inner(state: &AppState, mid: &str, author: &str) -> surrealdb::Result<()> {
    let Some(sender) = state.push.clone() else {
        return Ok(());
    };

    let Some(info) = load_notification_info(state, mid).await? else {
        // Message vanished (e.g. deleted between persist and here) — nothing to do.
        return Ok(());
    };

    // Recipients: every push_subscription owned by a guild member who isn't the
    // author. (Mutes are client-side only, so the server can't honour them; the
    // payload carries the channel id so the client/SW could filter later.)
    #[derive(SurrealValue)]
    struct Sub {
        endpoint: String,
        p256dh: String,
        auth: String,
        /// The subscription owner's account-id key — matched against the
        /// message's `pinged_keys` to set the per-recipient `pinged` flag (L-4).
        account_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT endpoint, p256dh, `auth`, meta::id(account) AS account_key
                FROM push_subscription
                WHERE account != type::record('account', $author)
                  AND account IN (SELECT VALUE account FROM guild_member
                      WHERE guild = type::record('guild', $gid));",
        )
        .bind(("author", author.to_string()))
        .bind(("gid", info.guild_key.clone()))
        .await?
        .check()?;
    let subs: Vec<Sub> = resp.take(0)?;
    if subs.is_empty() {
        return Ok(());
    }

    // Title = "<who> in #<channel>". Per-channel notification `tag`: the service
    // worker forwards it to showNotification, so a burst of messages in the SAME
    // channel collapses into ONE notification window (replaces + re-alerts)
    // instead of stacking — less spammy. `channel`/`guild`/`message` carry the
    // ids the click handler deep-links to. `pinged` is PER-RECIPIENT (L-4): true
    // only for a subscription whose owner this message @-mentions, so a future SW
    // can style a ping differently — hence the payload is built per recipient.
    // The persona avatar `image` (when present) is the same for everyone but is
    // added to each per-recipient payload below.
    let title = format!("{} in #{}", info.sender_name, info.channel_name);
    let body = info.notification_body();

    let mut dead: Vec<String> = Vec::new();
    for sub in &subs {
        let pinged = info.pinged_keys.contains(&sub.account_key);
        let mut payload_obj = serde_json::json!({
            "title": title,
            "body": body,
            "channel": info.channel_key,
            "guild": info.guild_key,
            "message": mid,
            "tag": info.channel_key,
            "pinged": pinged,
        });
        // Persona avatar → the SW's large `image` (`/media/{id}`), only when
        // present so a personaless/avatarless message omits it (no white-square
        // placeholder) and the payload stays well under the 4 KiB push cap.
        if let Some(avatar_id) = &info.sender_avatar_id {
            payload_obj["image"] = serde_json::Value::String(avatar_id.clone());
        }
        let payload = payload_obj.to_string().into_bytes();
        match sender
            .send_one(&sub.endpoint, &sub.p256dh, &sub.auth, &payload)
            .await
        {
            Ok(true) => {}
            Ok(false) => dead.push(sub.endpoint.clone()),
            Err(e) => tracing::warn!(error = %e, "push send failed for one endpoint"),
        }
    }

    // Prune subscriptions the push service reported gone (404/410).
    if !dead.is_empty() {
        let n = dead.len();
        let _ = state
            .db
            .query("DELETE push_subscription WHERE endpoint IN $eps;")
            .bind(("eps", dead))
            .await
            .and_then(|r| r.check());
        tracing::debug!(pruned = n, "removed dead push subscriptions");
    }
    Ok(())
}

/// Notification body: a trimmed snippet of the message, or a stand-in when the
/// message is image-only (empty body). A whispered message (W4/T5
/// hidden-until-tapped spoiler) is masked FIRST — its secret must never appear
/// in plaintext on a lock screen — using the same fixed `(whisper)`
/// placeholder as the reply-quote guard in `reading.rs` MSG_PROJECTION, so
/// every leak vector shows one consistent stand-in.
fn notification_body(body: &str, effect: Option<&str>) -> String {
    if effect == Some("whisper") {
        return "(whisper)".to_string();
    }
    let t = body.trim();
    if t.is_empty() {
        return "\u{1F4F7} sent an image".to_string();
    }
    if t.chars().count() > MAX_BODY_CHARS {
        let snippet: String = t.chars().take(MAX_BODY_CHARS).collect();
        format!("{snippet}\u{2026}")
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{notification_body, MAX_BODY_CHARS};

    /// Spoiler-leak guard (W4/T5 review): a whispered message's hidden text
    /// must never ride the push payload onto the OS lock screen — the body is
    /// masked with the SAME fixed `(whisper)` placeholder as the reply-quote
    /// guard (`reading.rs` MSG_PROJECTION, pinned by
    /// `reply_preview_masks_whispered_parent_snippet` in tests/messages.rs).
    #[test]
    fn whisper_effect_masks_push_notification_body_with_fixed_placeholder() {
        let masked = notification_body("the hidden secret", Some("whisper"));
        assert!(
            !masked.contains("hidden secret"),
            "whispered text must not leak into the push payload, got {masked:?}"
        );
        assert_eq!(masked, "(whisper)", "masked with the fixed placeholder");
        // An image-only whisper (empty body) is masked too — the placeholder,
        // not the "sent an image" stand-in.
        assert_eq!(notification_body("  ", Some("whisper")), "(whisper)");
    }

    /// Non-whisper effects keep the normal snippet behavior: pass-through,
    /// image-only stand-in, and the [`MAX_BODY_CHARS`] truncation.
    #[test]
    fn non_whisper_effects_keep_the_normal_snippet_behavior() {
        for effect in [None, Some("shout"), Some("spell")] {
            assert_eq!(notification_body("hello there", effect), "hello there");
        }
        assert_eq!(notification_body("", None), "\u{1F4F7} sent an image");
        let long = "x".repeat(MAX_BODY_CHARS + 80);
        let out = notification_body(&long, Some("shout"));
        assert_eq!(
            out.chars().count(),
            MAX_BODY_CHARS + 1,
            "truncated snippet plus the ellipsis"
        );
        assert!(out.ends_with('\u{2026}'));
    }
}
