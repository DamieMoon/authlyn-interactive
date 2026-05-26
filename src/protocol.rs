//! Wire-format DTOs shared by the server (ssr) and the client (hydrate).
//!
//! Anything in here must compile to `wasm32-unknown-unknown`: no axum,
//! no surrealdb, no tokio. Only `serde`.
//!
//! The phase-1 rebuild adds DTOs per build step (auth, guilds, channels,
//! messages, personas, lorebook, friends); only the shared error body lives
//! here from the start.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Generic typed-error body: `{"error": "<reason>"}`. Used for every 4xx
/// and 5xx the API can return.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

impl ErrorBody {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            error: reason.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Auth — POST /auth/register, /auth/login, /auth/logout; GET /auth/me
// ---------------------------------------------------------------------------

/// Body of `POST /auth/register` and (same shape) `POST /auth/login`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
}

/// Body of `POST /auth/login`. Identical fields to [`RegisterRequest`]; kept
/// a distinct type so the two endpoints can diverge (e.g. add 2FA) later.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Response from a successful `register`/`login`. The session cookie rides
/// in a `Set-Cookie` header alongside this body.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthResponse {
    /// Opaque id of the caller's `account` row.
    pub account_id: String,
    /// The stored display form of the username (not the lowercased key).
    pub username: String,
}

/// Response from `GET /auth/me` — the authenticated caller's profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeResponse {
    pub account_id: String,
    pub username: String,
    pub display_name: String,
}

/// Body of `POST /auth/change-password` (auth-required). The server verifies
/// `current_password` against the stored hash, then re-hashes `new_password`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

// ---------------------------------------------------------------------------
// Guilds (servers), channels, membership
// ---------------------------------------------------------------------------

/// Body of `POST /guilds`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateGuildRequest {
    pub name: String,
}

/// One guild as it appears in a list (the caller's guild rail).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuildSummary {
    pub id: String,
    pub name: String,
}

/// Response from `GET /guilds`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListGuildsResponse {
    pub guilds: Vec<GuildSummary>,
}

/// Body of `PUT /rail/order` (#17/FB2) — the caller's personal guild-rail order.
/// `guild_ids` is the full rail in the desired top-to-bottom order; the server
/// replaces the caller's `user_guild_order` rows with one row per id (index =
/// position). Ids the caller isn't a member of are rejected.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RailOrderRequest {
    pub guild_ids: Vec<String>,
}

/// One channel within a guild. `kind` is `"text"` or `"lorebook"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub position: i64,
}

/// Response from `GET /guilds/{id}` — the guild plus its channel list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuildDetail {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub channels: Vec<ChannelSummary>,
}

/// Body of `POST /guilds/{id}/channels`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    /// `"text"` or `"lorebook"`.
    pub kind: String,
}

/// Body of `PATCH /guilds/{id}` — every field optional (partial update).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PatchGuildRequest {
    pub name: Option<String>,
}

/// Body of `PATCH /guilds/{id}/channels/{cid}` — partial update.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PatchChannelRequest {
    pub name: Option<String>,
    pub position: Option<i64>,
}

/// Body of `POST /guilds/{id}/members` — invite a user by username.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteMemberRequest {
    pub username: String,
}

/// Body of `PUT /guilds/{id}/members/{aid}/role` — grant/revoke admin.
/// `role` is `"admin"` or `"member"` (the owner's role is fixed).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetMemberRoleRequest {
    pub role: String,
}

/// One member of a guild, as returned by `GET /guilds/{id}/members`.
/// `role` is `"owner"`, `"admin"`, or `"member"`. `avatar_id` is the account's
/// avatar media id (used directly as a `/media/{id}` `<img>` source), `None`
/// when the account has no avatar set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberSummary {
    pub account_id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    #[serde(default)]
    pub avatar_id: Option<String>,
}

/// Response from `GET /guilds/{id}/members` — the guild's full member roster.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListMembersResponse {
    pub members: Vec<MemberSummary>,
}

// ---------------------------------------------------------------------------
// Messages (channel-scoped, plaintext body; markup rides inside `body`)
// ---------------------------------------------------------------------------

/// Body of `POST /channels/{cid}/messages`. `body` is the raw message text,
/// which may contain markup (see [`crate::markup`]); the server stores it
/// verbatim. The author and "speaking-as" persona are resolved server-side
/// (from the session + the caller's active persona in that guild), never
/// trusted from the request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SendMessageRequest {
    pub body: String,
    /// Media ids (from `POST /media`) to attach as inline images, in display
    /// order. May be empty. A message with attachments may have an empty body.
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    /// The persona the author is "wearing" in THIS channel, sent by the client
    /// so attribution is decided at send time and never races a separate
    /// per-channel write. The server VALIDATES the caller may use it
    /// (`can_edit_persona`); if absent/invalid it falls back to the stored
    /// per-channel persona (`channel_active_persona`), else speaks as the account.
    #[serde(default)]
    pub persona_id: Option<String>,
}

/// Body of `PATCH /channels/{cid}/messages/{mid}` — edit a message body.
/// Only the message's author may edit; the server stores the new `body`
/// verbatim (markup rides inside it, as with [`SendMessageRequest`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditMessageRequest {
    pub body: String,
}

/// Successful response from `POST /channels/{cid}/messages`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SendMessageResponse {
    /// Opaque id of the new `message` row. Clients dedup catch-up reads on it.
    pub id: String,
}

/// One message as returned by `GET /channels/{cid}/messages`.
///
/// `persona_id`/`persona_name` are the persona the author was "wearing" when
/// they sent it (both `None` if they had none). `sent_at` is the fixed-9-digit
/// RFC 3339 cursor key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub id: String,
    pub author_id: String,
    /// Username of the account that controlled the persona for this message
    /// (the "who was in control" — fixed at send time via `author_id`).
    pub author_name: String,
    /// The controlling account's nickname (display_name, or username when
    /// unset). Shown as the message author when no persona was worn — the
    /// "default" identity — instead of a raw id.
    pub author_display: String,
    /// Live link to the persona row (None if the author wore none). May dangle
    /// once the persona is deleted — `persona_name`/`persona_description` are
    /// the source of truth for display.
    pub persona_id: Option<String>,
    /// Persona name snapshotted at send time, so it stays put on past messages
    /// even after the persona is renamed or deleted (None if author wore none).
    pub persona_name: Option<String>,
    /// Persona description snapshotted at send time (None if it had no persona).
    /// For the click-the-name info popup.
    pub persona_description: Option<String>,
    /// Persona name-tint (markup palette name) snapshotted at send time; the
    /// chat name renders in this color. None/empty = default.
    pub persona_color: Option<String>,
    /// Media id of the persona's avatar snapshotted at send time, used directly
    /// as a `/media/{id}` `<img>` source. Frozen so a past message keeps the
    /// picture it was sent with even after the persona's avatar changes. None
    /// when the author wore no persona or it had no avatar.
    #[serde(default)]
    pub persona_avatar_id: Option<String>,
    pub body: String,
    /// Inline image attachments: media ids, in display order. Empty for a
    /// plain text message. Rendered as a grid; each is a `/media/{id}` image.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// AI-visibility tier. Always `"default"` in phase 1.
    pub tier: String,
    pub sent_at: String,
}

/// Successful response from `GET /channels/{cid}/messages`. Up to 100
/// envelopes, ASC by `(sent_at, id)`; iterate with `?since=&after_id=`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListMessagesResponse {
    pub messages: Vec<MessageEnvelope>,
    /// Display names of OTHER channel members currently typing (#19), surfaced
    /// by piggybacking on the message poll. Ephemeral server state with an ~8s
    /// TTL; empty when nobody else is typing (and on the trash list, which never
    /// populates it). `#[serde(default)]` keeps older clients / the trash
    /// response wire-compatible.
    #[serde(default)]
    pub typing: Vec<String>,
    /// The caller's worn persona id for THIS channel (per-channel, #persona),
    /// or `None` when speaking as the account. Lets the client restore the
    /// "speaking as" state on channel open. `#[serde(default)]` keeps older
    /// clients / the trash response wire-compatible.
    #[serde(default)]
    pub active_persona: Option<String>,
}

/// A flat list of channels — used by the soft-delete trash view
/// (`GET /guilds/{id}/trash/channels`, #22).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelListResponse {
    pub channels: Vec<ChannelSummary>,
}

// ---------------------------------------------------------------------------
// Media (server-visible images: avatars, persona art, gallery)
// ---------------------------------------------------------------------------

/// Successful response from `POST /media` (multipart upload). The `id` is
/// used directly in a `/media/{id}` URL as an `<img>` source.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaUploadResponse {
    pub id: String,
}

// ---------------------------------------------------------------------------
// Personas (account-global; "worn" per-guild)
// ---------------------------------------------------------------------------

/// Body of `POST /personas`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatePersonaRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Markup palette name (red…gray) tinting the persona's chat name, or empty
    /// for the default color.
    #[serde(default)]
    pub color: Option<String>,
}

/// Body of `PATCH /personas/{id}` — partial update.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PatchPersonaRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Markup palette name (red…gray) or empty string to clear the tint.
    pub color: Option<String>,
    /// Wardrobe display order (0-based). Set when reordering cards.
    pub position: Option<i64>,
}

/// One persona in a list (the wardrobe grid). Carries `description` so the
/// character cards can show a blurb without a per-card detail fetch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonaSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub avatar_id: Option<String>,
    /// Markup palette name tinting the persona's chat name (empty for default).
    #[serde(default)]
    pub color: String,
    /// True when the caller owns this persona; false when it was redeemed via a
    /// share key (editor access — can wear + edit, but not delete/share).
    #[serde(default)]
    pub owned: bool,
    /// Wardrobe display order. `None` for rows predating the field (sorted last
    /// by the server, which orders the list before sending it).
    #[serde(default)]
    pub position: Option<i64>,
}

/// Response from `GET /personas`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListPersonasResponse {
    pub personas: Vec<PersonaSummary>,
}

/// One gallery image of a persona.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GalleryImage {
    pub id: String,
    pub media_id: String,
    pub position: i64,
}

/// One editor of a persona (owner-only view).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonaEditor {
    pub account_id: String,
    pub username: String,
}

/// Response from `GET /personas/{id}` — the persona plus its gallery.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonaDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub avatar_id: Option<String>,
    pub gallery: Vec<GalleryImage>,
    /// The shareable redeem key — populated only for the owner; `None` for
    /// editors (so editors can't re-share a persona they don't own).
    #[serde(default)]
    pub share_key: Option<String>,
    /// Accounts granted editor access via the share key — owner-only view
    /// (empty for editors).
    #[serde(default)]
    pub editors: Vec<PersonaEditor>,
}

/// Response from `GET /personas/{id}/editors` — owner-only.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListPersonaEditorsResponse {
    pub editors: Vec<PersonaEditor>,
}

/// Body of `POST /personas/redeem` — gain editor access via a share key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RedeemPersonaKeyRequest {
    pub key: String,
}

/// Body of `PUT /personas/{id}/avatar` — set the primary image.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetAvatarRequest {
    pub media_id: String,
}

/// Body of `POST /personas/{id}/gallery` — add a gallery image.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddGalleryImageRequest {
    pub media_id: String,
}

/// Response from `POST /personas/{id}/gallery`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddGalleryImageResponse {
    pub id: String,
}

/// Body of `PUT /guilds/{id}/active-persona` — wear a persona in this guild,
/// or `null` to take it off.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetActivePersonaRequest {
    pub persona_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Lorebook entries (SillyTavern-style world info; on a kind='lorebook' channel)
// ---------------------------------------------------------------------------

/// One lorebook entry. `keys` are the trigger keywords; `content` is the
/// text a future AI layer would inject; `enabled`/`position` gate + order it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LorebookEntry {
    pub id: String,
    pub title: String,
    pub keys: Vec<String>,
    pub content: String,
    pub enabled: bool,
    pub position: i64,
}

/// Response from `GET /channels/{cid}/lorebook` — entries ordered by position.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListLorebookResponse {
    pub entries: Vec<LorebookEntry>,
}

/// Body of `POST /channels/{cid}/lorebook`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateLorebookEntryRequest {
    #[serde(default)]
    pub title: Option<String>,
    pub keys: Vec<String>,
    pub content: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub position: Option<i64>,
}

/// Body of `PATCH /channels/{cid}/lorebook/{eid}` — partial update.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PatchLorebookEntryRequest {
    pub title: Option<String>,
    pub keys: Option<Vec<String>>,
    pub content: Option<String>,
    pub enabled: Option<bool>,
    pub position: Option<i64>,
}

/// Response from `POST /channels/{cid}/lorebook`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateLorebookEntryResponse {
    pub id: String,
}

// ---------------------------------------------------------------------------
// Friends (account-to-account; global, independent of guilds)
// ---------------------------------------------------------------------------

/// Body of `POST /friends` — send a friend request by username (or, if the
/// target already requested you, accept it).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FriendRequest {
    pub username: String,
}

/// One account in a friends list / pending list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FriendSummary {
    pub account_id: String,
    pub username: String,
}

/// Response from `GET /friends`: accepted friends plus pending requests split
/// by direction (`incoming` = others who requested you, `outgoing` = your
/// unanswered requests).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListFriendsResponse {
    pub friends: Vec<FriendSummary>,
    pub incoming: Vec<FriendSummary>,
    pub outgoing: Vec<FriendSummary>,
}

// ---------------------------------------------------------------------------
// Web Push (#30 background notifications)
// ---------------------------------------------------------------------------

/// Response from `GET /push/vapid-key` — the server's VAPID public key
/// (base64url, unpadded), which the browser decodes into the `Uint8Array`
/// `applicationServerKey` for `pushManager.subscribe`. The endpoint 404s when
/// push isn't configured server-side, so the client can skip subscription.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VapidKeyResponse {
    pub key: String,
}

/// Body of `POST /push/subscribe` — a browser `PushSubscription` shaped like
/// its `.toJSON()` output. `endpoint` is the push-service URL; `keys.p256dh`
/// and `keys.auth` are the receiver keys the server needs to encrypt payloads
/// (RFC 8291). Re-subscribing the same browser upserts on `endpoint`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushSubscribeRequest {
    pub endpoint: String,
    pub keys: PushSubscriptionKeys,
}

/// The `keys` object of a browser `PushSubscription` (both base64url strings).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushSubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

/// Body of `POST /push/unsubscribe` — drop a stored subscription by its
/// endpoint (e.g. when the browser reports the subscription changed/stale).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushUnsubscribeRequest {
    pub endpoint: String,
}

// ---------------------------------------------------------------------------
// Custom emoji (Discord-style shortcodes, per-guild)
// ---------------------------------------------------------------------------

/// Body of `POST /guilds/{id}/emoji`. `media_id` is the id returned by a prior
/// `POST /media` upload — the client uploads the image first, then registers
/// the emoji shortcode against it. `name` must match `^[a-z0-9_]{2,32}$`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateEmojiRequest {
    pub name: String,
    pub media_id: String,
}

/// One custom emoji as stored in the guild. `media_id` is a `/media/{id}` key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomEmoji {
    pub id: String,
    pub name: String,
    pub media_id: String,
    pub creator_id: String,
    pub created_at: String,
}

/// Response from `GET /guilds/{id}/emoji`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListEmojiResponse {
    pub emoji: Vec<CustomEmoji>,
}

// ---------------------------------------------------------------------------
// Feedback / bug reports (#31 — submit side only; admin inbox is out of scope)
// ---------------------------------------------------------------------------

/// Body of `POST /feedback` — a user-submitted feedback item. `kind` is
/// `"bug"`, `"idea"`, or `"other"` (the server coerces anything else to
/// `"other"`). `context` is an optional JSON string from the client
/// (channel id, app version, user agent).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitFeedbackRequest {
    pub kind: String,
    pub body: String,
    #[serde(default)]
    pub context: Option<String>,
}

/// One feedback item as returned by `GET /feedback` (admin only). `id` is the
/// opaque row id; `author_username` is the submitting account's display name;
/// `created_at` is a fixed-9-digit RFC 3339 string (lex-monotonic cursor key).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackItem {
    pub id: String,
    pub author_username: String,
    pub kind: String,
    pub body: String,
    pub context: Option<String>,
    pub status: String,
    pub created_at: String,
}

/// Response from `GET /feedback` (admin only) — newest-first list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListFeedbackResponse {
    pub items: Vec<FeedbackItem>,
}
