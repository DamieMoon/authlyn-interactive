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

use crate::protocol::{ErrorBody, PushSubscribeRequest, PushUnsubscribeRequest, VapidKeyResponse};
use crate::server::auth::AuthAccount;
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
    state
        .db
        .query(
            "BEGIN TRANSACTION;
             DELETE push_subscription WHERE endpoint = $endpoint;
             CREATE push_subscription SET
                account  = type::record('account', $account),
                endpoint = $endpoint,
                p256dh   = $p256dh,
                `auth`   = $auth;
             COMMIT TRANSACTION;",
        )
        .bind(("account", account.to_string()))
        .bind(("endpoint", req.endpoint.clone()))
        .bind(("p256dh", req.keys.p256dh.clone()))
        .bind(("auth", req.keys.auth.clone()))
        .await?
        .check()?;
    Ok(())
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

async fn notify_inner(state: &AppState, mid: &str, author: &str) -> surrealdb::Result<()> {
    let Some(sender) = state.push.clone() else {
        return Ok(());
    };

    // One query resolves everything the notification needs from the fresh row:
    // its channel (id + name), guild, sender display (persona snapshot, else the
    // author's nickname), and body.
    #[derive(SurrealValue)]
    struct MsgInfo {
        channel_key: String,
        channel_name: String,
        guild_key: String,
        sender_name: String,
        body: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT
                meta::id(channel)        AS channel_key,
                channel.name             AS channel_name,
                meta::id(channel.guild)  AS guild_key,
                (persona_name ?? (author.display_name ?: author.username)) AS sender_name,
                body
             FROM type::record('message', $mid);",
        )
        .bind(("mid", mid.to_string()))
        .await?
        .check()?;
    let Some(info) = resp.take::<Option<MsgInfo>>(0)? else {
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
    }
    let mut resp = state
        .db
        .query(
            "SELECT endpoint, p256dh, `auth` FROM push_subscription
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

    // Same payload for every recipient. Title = "<who> in #<channel>".
    let payload = serde_json::json!({
        "title": format!("{} in #{}", info.sender_name, info.channel_name),
        "body": notification_body(&info.body),
        "channel": info.channel_key,
    })
    .to_string()
    .into_bytes();

    let mut dead: Vec<String> = Vec::new();
    for sub in &subs {
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
/// message is image-only (empty body).
fn notification_body(body: &str) -> String {
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

// ---------------------------------------------------------------------------
// Shaping (local copies, matching the per-module style in messages.rs/auth.rs)
// ---------------------------------------------------------------------------

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(msg))).into_response()
}

fn json_rejection_response(rej: JsonRejection) -> Response {
    let reason: &'static str = match rej {
        JsonRejection::JsonDataError(_) => "invalid JSON body shape",
        JsonRejection::JsonSyntaxError(_) => "malformed JSON",
        JsonRejection::MissingJsonContentType(_) => "missing Content-Type: application/json",
        JsonRejection::BytesRejection(_) => "could not read request body",
        _ => "invalid JSON request",
    };
    error_response(StatusCode::BAD_REQUEST, reason)
}
