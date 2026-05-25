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

// ---------------------------------------------------------------------------
// Guilds (servers), channels, membership
// ---------------------------------------------------------------------------

/// Body of `POST /guilds`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateGuildRequest {
    pub name: String,
}

/// One guild as it appears in a list (the caller's guild rail).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuildSummary {
    pub id: String,
    pub name: String,
}

/// Response from `GET /guilds`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListGuildsResponse {
    pub guilds: Vec<GuildSummary>,
}

/// One channel within a guild. `kind` is `"text"` or `"lorebook"`.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub persona_id: Option<String>,
    pub persona_name: Option<String>,
    pub body: String,
    /// AI-visibility tier. Always `"default"` in phase 1.
    pub tier: String,
    pub sent_at: String,
}

/// Successful response from `GET /channels/{cid}/messages`. Up to 100
/// envelopes, ASC by `(sent_at, id)`; iterate with `?since=&after_id=`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListMessagesResponse {
    pub messages: Vec<MessageEnvelope>,
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
}

/// Body of `PATCH /personas/{id}` — partial update.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PatchPersonaRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// One persona in a list (the wardrobe grid).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonaSummary {
    pub id: String,
    pub name: String,
    pub avatar_id: Option<String>,
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

/// Response from `GET /personas/{id}` — the persona plus its gallery.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonaDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub avatar_id: Option<String>,
    pub gallery: Vec<GalleryImage>,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FriendSummary {
    pub account_id: String,
    pub username: String,
}

/// Response from `GET /friends`: accepted friends plus pending requests split
/// by direction (`incoming` = others who requested you, `outgoing` = your
/// unanswered requests).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListFriendsResponse {
    pub friends: Vec<FriendSummary>,
    pub incoming: Vec<FriendSummary>,
    pub outgoing: Vec<FriendSummary>,
}
