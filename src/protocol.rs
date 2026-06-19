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
    /// Whether the caller is an app administrator (their username is in the
    /// `AUTHLYN_ADMIN_USERNAMES` set, server-side). Gates admin-only UI such as
    /// the Nova DOT system-broadcast composer. `#[serde(default)]` → `false` for
    /// post-ship wire-compat (older/native clients deserialize cleanly).
    #[serde(default)]
    pub is_admin: bool,
    /// The caller's account-avatar media id (a `/media/{id}` `<img>` source), or
    /// `None` for the monogram fallback. Account identity (display_name + avatar)
    /// is the only LIVE-resolved display data (spec §3, M6). `#[serde(default)]`
    /// for the same post-ship wire-compat reason as `is_admin`.
    #[serde(default)]
    pub avatar_id: Option<String>,
}

/// Body of `POST /admin/system-message` (admin-only): broadcast `body` as a
/// "Nova DOT" system message into every live guild's default channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SendSystemMessageRequest {
    pub body: String,
}

/// Response from `POST /admin/system-message` — how the broadcast fanned out.
/// `guilds_targeted == messages_sent + guilds_skipped`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemBroadcastResult {
    /// Live guilds considered (soft-deleted guilds are excluded entirely).
    pub guilds_targeted: usize,
    /// Guilds that received a message (had a live text channel).
    pub messages_sent: usize,
    /// Guilds skipped for having no live text channel.
    pub guilds_skipped: usize,
}

/// Body of `POST /auth/change-password` (auth-required). The server verifies
/// `current_password` against the stored hash, then re-hashes `new_password`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Body of `PATCH /account` (auth-required) — partial profile update (M6). Every
/// field optional; an absent field is left untouched. PATCH-shaped per convention
/// (derives `Default`, all-`Option<>`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PatchAccountRequest {
    /// New display name (1–32 chars after trim); validated server-side.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Account-avatar media id from a prior `POST /media` (mirrors
    /// [`SetAvatarRequest::media_id`]); validated to exist server-side.
    #[serde(default)]
    pub avatar: Option<String>,
}

/// Body of `POST /auth/admin/reset-password` (admin-only). Sets `username`'s
/// password to `new_password` without needing the target's current password.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdminResetPasswordRequest {
    pub username: String,
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
    /// Per-server accent: a markup-palette name (red…gray) tinting this guild's
    /// chrome, or empty for the default. `#[serde(default)]` for post-ship
    /// wire-compat (older/native clients deserialize cleanly).
    #[serde(default)]
    pub accent_color: String,
    /// The guild's icon media id, used directly as a `/media/{id}` `<img>`
    /// source; `None` renders the monogram fallback. The server derives
    /// `accent_color` from this icon at upload (M6). `#[serde(default)]` for the
    /// same post-ship wire-compat reason as `accent_color`.
    #[serde(default)]
    pub icon_id: Option<String>,
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
    /// Per-server accent (see `GuildSummary::accent_color`).
    #[serde(default)]
    pub accent_color: String,
    /// Guild icon media id (see `GuildSummary::icon_id`).
    #[serde(default)]
    pub icon_id: Option<String>,
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
    /// Markup-palette accent name (red…gray) or empty to clear. Validated
    /// server-side against the same palette as persona.color.
    #[serde(default)]
    pub accent_color: Option<String>,
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
    /// Id of the message this one replies to (Discord-style quote), `None` for a
    /// non-reply (L-3). The server VALIDATES the referenced message exists, is in
    /// the SAME channel, and is not soft-deleted, else 400. Single-level only:
    /// replying to a reply quotes that reply, not its own parent.
    #[serde(default)]
    pub reply_to_id: Option<String>,
    /// Optional delivery effect (W4/T5): `"whisper"` (blurred until tapped),
    /// `"shout"` (shake + warm tint), or `"spell"` (glow + sparks). `None` /
    /// empty = an ordinary message. The server VALIDATES against that exact
    /// set; an unknown value is a 400 (mirroring the body checks). Purely
    /// cosmetic — it gates no behavior.
    #[serde(default)]
    pub effect: Option<String>,
}

/// Body of `PATCH /channels/{cid}/messages/{mid}` — edit a message body.
/// Only the message's author may edit; the server stores the new `body`
/// verbatim (markup rides inside it, as with [`SendMessageRequest`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditMessageRequest {
    pub body: String,
}

/// Body of `POST /channels/{cid}/roll` (W4/T6 Fate Engine). The server parses
/// `expr` against a constrained grammar, rolls with ITS OWN RNG, and persists
/// the formatted result as an immutable `kind='roll'` message — the client
/// never computes (so can never forge) an outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RollRequest {
    /// The roll expression: `NdM`, `NdM+K`, or `NdM-K` (1 ≤ N ≤ 100,
    /// 2 ≤ M ≤ 1000, |K| ≤ 1000; bare `dM` reads as `1dM`; case-insensitive
    /// `d`, no whitespace), or the literals `coin` / `oracle`. Anything else
    /// is a 400.
    pub expr: String,
    /// The persona the caller is wearing — same server-side double-check
    /// semantics as [`SendMessageRequest::persona_id`] (re-validated via
    /// `can_edit_persona`, falling back to the stored per-channel wear, else
    /// the bare account).
    #[serde(default)]
    pub persona: Option<String>,
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
    /// The controlling account's avatar media id (`/media/{id}` source), resolved
    /// LIVE at read like `author_display` — NOT a send-time snapshot (contrast
    /// `persona_avatar_id`, which is frozen). `None` ⇒ monogram fallback. Shown
    /// beside a bare-account message, and carries the account identity behind a
    /// worn persona's subtle "· name" marker (M6/P2). `#[serde(default)]` for the
    /// post-ship wire-compat reason its siblings share.
    #[serde(default)]
    pub author_avatar_id: Option<String>,
    /// Live link to the persona row (None if the author wore none). May dangle
    /// once the persona is deleted — `persona_name`/`persona_description` are
    /// the source of truth for display.
    pub persona_id: Option<String>,
    /// Persona name snapshotted at send time, so it stays put on past messages
    /// even after the persona is renamed or deleted (None if author wore none).
    pub persona_name: Option<String>,
    /// Persona description snapshotted at send time (None if it had no persona).
    /// For the click-the-name info popup. `#[serde(default)]` because it was
    /// added after the DTO shipped — matches the post-ship-field convention its
    /// `persona_avatar_id`/`attachments` siblings follow (review F-D12-3).
    #[serde(default)]
    pub persona_description: Option<String>,
    /// Persona name-tint (markup palette name) snapshotted at send time; the
    /// chat name renders in this color. None/empty = default. `#[serde(default)]`
    /// for the same post-ship wire-compat reason as `persona_description`.
    #[serde(default)]
    pub persona_color: Option<String>,
    /// Media id of the persona's avatar snapshotted at send time, used directly
    /// as a `/media/{id}` `<img>` source. Frozen so a past message keeps the
    /// picture it was sent with even after the persona's avatar changes. None
    /// when the author wore no persona or it had no avatar.
    #[serde(default)]
    pub persona_avatar_id: Option<String>,
    pub body: String,
    /// Inline attachments (images, GIFs, videos), in display order. Empty for a
    /// plain text message. Rendered as a grid; each is a `/media/{id}` blob whose
    /// `mime` decides image-vs-video rendering.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// AI-visibility tier. Always `"default"` in phase 1.
    pub tier: String,
    pub sent_at: String,
    /// Lightweight preview of the message this one replies to (L-3), resolved by
    /// a LIVE null-safe join at read time (not a send-time snapshot). `None` when
    /// this isn't a reply OR the parent was soft-deleted / hard-deleted (the join
    /// degrades gracefully to `None`). `#[serde(default)]` for the same post-ship
    /// wire-compat reason as the persona/attachment siblings above.
    #[serde(default)]
    pub reply_to: Option<ReplyPreview>,
    /// Whether the READING caller is `@`-mentioned (pinged) by this message
    /// (L-4). Per-reader: the server evaluates `caller IN pinged_users` in the
    /// projection, so the same message has `is_pinged = true` for a mentioned
    /// reader and `false` for everyone else. Drives the message highlight and
    /// the sidebar's orange ping glow. `#[serde(default)]` for the same
    /// post-ship wire-compat reason as the siblings above (defaults to `false`).
    #[serde(default)]
    pub is_pinged: bool,
    /// Message kind: `"user"` (a normal send), `"system"` (an app-admin "Nova
    /// DOT" broadcast, authored by the reserved bot account), or `"roll"` (a
    /// W4/T6 Fate Engine dice result — authored, persona-aware, immutable).
    /// Drives distinct rendering and per-kind action gating. `#[serde(default)]`
    /// → `"user"` for the same post-ship wire-compat reason as the siblings above
    /// (and so older/native clients deserialize cleanly).
    #[serde(default = "default_message_kind")]
    pub kind: String,
    /// Delivery effect picked at send time (W4/T5): `"whisper"`, `"shout"`, or
    /// `"spell"`; `None` for an ordinary message (and on every legacy row —
    /// the field is `option<>` in the schema, no backfill). Drives rendering
    /// only (`effect-{name}` class on the message row). `#[serde(default)]`
    /// for the same post-ship wire-compat reason as the siblings above.
    #[serde(default)]
    pub effect: Option<String>,
    /// M7/P2: true when this message was sent by a GUEST (a Guest Cameo — a
    /// `channel_guest`, not a guild member), snapshotted at send time so the
    /// "GÄST" badge survives the cameo being revoked/expired. `#[serde(default)]`
    /// → `false` for the same post-ship wire-compat reason as the siblings above.
    #[serde(default)]
    pub guest_cameo: bool,
}

/// serde default for [`MessageEnvelope::kind`]: a message with no `kind` on the
/// wire is a normal user message.
fn default_message_kind() -> String {
    "user".to_string()
}

/// A lightweight preview of a replied-to (parent) message, rendered as a
/// clickable quote above the reply body (L-3). Carries just enough to show the
/// quote and scroll to the parent: its `id` (the scroll-to-message anchor), the
/// parent author's display name, and a short body snippet. Resolved live at read
/// time, so it reflects the parent's CURRENT body/author and is `None` once the
/// parent is deleted.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ReplyPreview {
    pub id: String,
    pub author_display: String,
    pub body_snippet: String,
}

/// One inline attachment on a message: the media id plus its stored MIME type
/// (e.g. `"image/png"`, `"image/gif"`, `"video/mp4"`). The client uses `mime`
/// to pick `<img>` vs `<video>` rendering; an empty `mime` falls back to image.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub mime: String,
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

/// Body of `POST /channels/{cid}/typing` (W4/T7 Ghost Quill). The ping has
/// been body-less since #19 and MUST stay wire-compatible: a bare POST (no
/// body, no Content-Type) is still a plain "I am typing" stamp. When the
/// SENDER has the Ghost Quill pref ON, the client attaches its current
/// compose text as `draft`; the server stores it in the ephemeral
/// typing-draft map (8s TTL, in-memory only — never the DB, never the SSE
/// bus). Absent or empty `draft` CLEARS any stored entry, so a sender
/// toggling the pref off (or deleting their text) stops ghosting at the very
/// next ping. Drafts over 2000 chars are TRUNCATED on a char boundary, never
/// rejected — a mid-typing ping must not start failing as the composer grows.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TypingPingRequest {
    #[serde(default)]
    pub draft: Option<String>,
    /// The composer's currently ARMED delivery effect (review M-01) — the
    /// same W4/T5 vocabulary as `SendMessageRequest.effect` (`whisper` /
    /// `shout` / `spell`). The server masks a whisper-armed `draft` to the
    /// fixed `(whisper)` placeholder BEFORE storing it, so the spoiler text
    /// of a message that will land hidden-until-tapped never streams live to
    /// the very audience it's veiled from. Absent (today's client) keeps the
    /// plaintext behavior unchanged; non-whisper / unknown values are
    /// IGNORED rather than rejected — a mid-typing ping must never 400.
    #[serde(default)]
    pub effect: Option<String>,
}

/// One live co-writer draft from `GET /channels/{cid}/typing-drafts`
/// (W4/T7 Ghost Quill) — the endpoint returns a bare JSON array of these.
/// Only OTHER members' unexpired drafts appear (your own is excluded, like
/// the typing indicator); `display_name` is resolved exactly like the typing
/// names: the author's worn persona in this channel first, else their
/// account display name / username. Draft text rides ONLY this
/// permission-checked fetch — the receiving client opts in by fetching, the
/// sender opted in by attaching the draft to its ping. A whisper-armed
/// draft arrives as the fixed `(whisper)` placeholder, never the spoiler
/// text (review M-01 — masked server-side at store time).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypingDraftEntry {
    pub account_id: String,
    pub display_name: String,
    pub draft: String,
}

/// Body of `POST /channels/{cid}/mark-read` (L-1) — record the caller's
/// per-channel last-seen high-water mark server-side, so the read/unread state
/// syncs across devices instead of living only in each browser's localStorage.
/// `sent_at`/`id` are the `(sent_at, id)` composite cursor of the latest message
/// the caller has seen in this channel (the same pair the client tracks). The
/// server UPSERTs the `(account, channel)` row keeping the MAX cursor — an older
/// POST never regresses a newer mark.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarkReadRequest {
    /// `sent_at` of the latest seen message (fixed-9-digit RFC 3339, as carried
    /// on [`MessageEnvelope::sent_at`]). Bound server-side via `type::datetime`.
    pub sent_at: String,
    /// Opaque id of the latest seen message — the tie-break half of the cursor.
    pub id: String,
}

/// One channel's persisted read cursor (L-1), as returned by
/// `GET /channels/read-state`. `(sent_at, id)` mirrors the client's per-channel
/// last-seen tuple, so the client can hydrate `notify.last_seen` directly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelReadCursor {
    pub channel_id: String,
    pub sent_at: String,
    pub id: String,
}

/// Response from `GET /channels/read-state` (L-1) — every channel the caller
/// has a stored read cursor for. The client maps these into its per-channel
/// `last_seen` table on shell mount, falling back to localStorage if the fetch
/// fails (offline).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReadStateResponse {
    pub cursors: Vec<ChannelReadCursor>,
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

/// Body of `PUT /guilds/{id}/icon` — set the guild's icon to an already-uploaded
/// media blob (the client POSTs the file to `/media` first, then sends the id
/// here). The server re-derives the per-server `accent_color` from the image
/// (M6, effect G). Mirrors [`SetAvatarRequest`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetGuildIconRequest {
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

/// Body of `POST /personas/{id}/gallery/batch` — add multiple gallery images
/// atomically (paste-many in one request). All inserts succeed or the whole
/// batch fails (single SurrealDB transaction); positions are sequential and
/// contiguous starting from the persona's current max position + 1.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddGalleryImagesBatchRequest {
    pub media_ids: Vec<String>,
}

/// Response from `POST /personas/{id}/gallery/batch`. `ids` are the new
/// `persona_image` row ids in the same order as the input `media_ids`, so the
/// client can correlate each new gallery row with the media id it asked for.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddGalleryImagesBatchResponse {
    pub ids: Vec<String>,
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
// Direct messages (M7/P1 — guild-less DM threads, 1:1 + groups)
// ---------------------------------------------------------------------------

/// Body of `POST /dms` — start a DM thread. `members` are the *other*
/// participants' account ids (the creator is added implicitly); each must be an
/// accepted friend of the creator. One other member = a 1:1 DM (deduped to the
/// existing thread if any); 2+ = a group. `title` is optional and only
/// meaningful for groups.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CreateDmRequest {
    pub members: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// Body of `POST /dms/{tid}/members` — invite one accepted friend into a group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteToDmRequest {
    pub account_id: String,
}

/// One participant of a DM thread, with the live account identity needed to
/// render the thread row (account identity resolves live, like everywhere else).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DmMemberSummary {
    pub account_id: String,
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_id: Option<String>,
}

/// One DM thread the caller belongs to. `id` is the underlying channel id, so
/// messages/read-state/active-persona ride the existing `/channels/{id}/…`
/// routes unchanged. `title` is the optional group name (None / empty for 1:1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DmSummary {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub members: Vec<DmMemberSummary>,
    /// M7/P1 (review M2): true when the thread is read-only — a 1:1 whose two
    /// friends unfriended. History stays readable; posting is server-rejected.
    /// Always false for groups. The server is the source of truth.
    #[serde(default)]
    pub locked: bool,
}

/// Response from `GET /dms` — every DM thread the caller is a member of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListDmsResponse {
    pub dms: Vec<DmSummary>,
}

// ---------------------------------------------------------------------------
// Guest cameos (M7/P2 — scoped ephemeral guest access to one guild text channel)
// ---------------------------------------------------------------------------

/// Body of `POST /channels/{cid}/guests` — invite one accepted friend as a guest
/// in this guild text channel. `account_id` must be an accepted friend of the
/// caller and not already a member of the channel's guild. `expires_at` is an
/// optional RFC3339 instant after which the cameo lapses (None = no expiry); the
/// server enforces it as a lazy-check at every membership query.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InviteGuestRequest {
    pub account_id: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// One active guest of a channel (host-side view, `GET /channels/{cid}/guests`),
/// with the live account identity needed to render the row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuestSummary {
    pub account_id: String,
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_id: Option<String>,
    /// Account id of the member who invited this guest (revoke-authz + display).
    pub invited_by: String,
    /// RFC3339 expiry instant, or None for an open-ended cameo.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// Response from `GET /channels/{cid}/guests` — the channel's active guests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListGuestsResponse {
    pub guests: Vec<GuestSummary>,
}

/// One cameo the caller is a guest in (guest-side view, `GET /cameos`). `channel_id`
/// is the underlying channel id, so messages/read-state/active-persona ride the
/// existing `/channels/{id}/…` routes unchanged. `guild_name` is the host guild's
/// name for context (the guest can't otherwise see the guild).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameoSummary {
    pub channel_id: String,
    pub channel_name: String,
    #[serde(default)]
    pub guild_name: Option<String>,
    /// Account id of the member who invited the caller.
    pub invited_by: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// Response from `GET /cameos` — every cameo the caller is currently a guest in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListCameosResponse {
    pub cameos: Vec<CameoSummary>,
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

// ---------------------------------------------------------------------------
// SSE realtime bus (W1)
// ---------------------------------------------------------------------------

/// W1 realtime: the id-only event vocabulary broadcast over `GET /events`.
/// Deliberately content-free (notify-and-fetch): clients react by refetching
/// through the existing permission-checked endpoints, so this enum never
/// becomes an authorization surface. Shared by ssr (emitter) and hydrate
/// (EventSource consumer); always-on like every other wire DTO here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    /// A message was created in this channel.
    MessageCreated { channel_id: String },
    /// A message was edited in this channel.
    MessageEdited {
        channel_id: String,
        message_id: String,
    },
    /// A message was soft-deleted in this channel.
    MessageDeleted {
        channel_id: String,
        message_id: String,
    },
    /// Someone (not necessarily you) pinged "typing" in this channel.
    Typing { channel_id: String },
    /// Guild/channel/membership metadata changed somewhere visible to you —
    /// refetch lists. Also used as a generic "resync" nudge after broadcast lag.
    ListsChanged,
    /// The caller's read cursor moved in this channel (their OTHER devices
    /// should refresh unread). Account-targeted on the server; never broadcast.
    ReadStateChanged { channel_id: String },
    /// The friends/requests list changed for this account (targeted to the
    /// two accounts of the friendship edge).
    FriendsChanged,
    /// Dev hot-reload: a new build was deployed to the test deck (which runs
    /// the compiled binary, so there is no cargo-leptos live-reload). A global,
    /// content-free nudge telling every connected client to `location.reload()`
    /// onto the new version. Admin-triggered only (`POST /admin/dev/reload`);
    /// delivered to ALL connections as a DISTINCT NAMED `event: reload` frame,
    /// bypassing the per-connection visibility filter (it is not channel-scoped
    /// — `channel_id()` is `None`).
    Reload,
    /// Forward-compat catch-all: an event type this build doesn't know
    /// (a newer server during version skew). Consumers MUST ignore it;
    /// the server never constructs it.
    #[serde(other)]
    Unknown,
}

impl SyncEvent {
    /// The channel this event is VISIBILITY-scoped to, if any. `None`
    /// (ListsChanged) means "deliver to everyone and let the refetch re-derive
    /// visibility".
    ///
    /// `ReadStateChanged` carries a `channel_id` field but reports `None` here:
    /// it (like `FriendsChanged`) is account-targeted on the server, and the
    /// targeted delivery lane bypasses channel-visibility filtering entirely —
    /// this method is never consulted for it. Returning the id would be wrong
    /// anyway: visibility filtering would silently drop the nudge whenever the
    /// recipient's visible-set is momentarily stale.
    pub fn channel_id(&self) -> Option<&str> {
        match self {
            SyncEvent::MessageCreated { channel_id }
            | SyncEvent::MessageEdited { channel_id, .. }
            | SyncEvent::MessageDeleted { channel_id, .. }
            | SyncEvent::Typing { channel_id } => Some(channel_id),
            SyncEvent::ListsChanged
            | SyncEvent::ReadStateChanged { .. }
            | SyncEvent::FriendsChanged
            | SyncEvent::Reload
            | SyncEvent::Unknown => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Batched unread summary (W1 — GET /unread)
// ---------------------------------------------------------------------------

/// W1: one row per visible text channel in `GET /unread`. M7/P1: also DM
/// threads (`kind='dm'`), which have no guild — `guild_id` is then `None`, and
/// the client routes that row to the DM-list badge instead of a guild rail dot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelUnread {
    pub channel_id: String,
    /// M7/P1 + M7/P2: `None` for a DM thread AND for a cameo channel seen as a
    /// guest (both surface standalone, not under a guild rail). The client keys a
    /// guild rail dot ONLY on `Some(guild_id)`, so a guildless row glows just its
    /// own channel in the DM / cameo list — no rail dot for a guild the caller
    /// can't see. Which list the row belongs to is resolved by the separate
    /// `sel.dms` / `sel.cameos` signals (by channel id), not a kind field here.
    #[serde(default)]
    pub guild_id: Option<String>,
    /// Messages newer than the caller's read cursor (capped at 100). 0 when
    /// the channel has no cursor yet — the client baselines instead of glowing.
    pub unread: usize,
    /// True iff any unread message pings the caller.
    pub pinged: bool,
    /// Latest live message's cursor pair, for client-side baselining of
    /// never-visited channels. None when the channel is empty.
    #[serde(default)]
    pub latest_sent_at: Option<String>,
    #[serde(default)]
    pub latest_id: Option<String>,
}

/// W1: response of `GET /unread`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnreadResponse {
    pub channels: Vec<ChannelUnread>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-D12-3: `persona_description` and `persona_color` were added to the
    /// already-shipped `MessageEnvelope` DTO, so a producer that omits them
    /// (a version-skewed server during a rolling deploy, or a hand-rolled
    /// response) must still deserialize — they carry `#[serde(default)]` like
    /// their same-era `persona_avatar_id` / `attachments` siblings. Without it,
    /// the whole message page fails to deserialize on the client.
    #[test]
    fn message_envelope_deserializes_without_persona_description_or_color() {
        let json = r#"{
            "id": "m1",
            "author_id": "a1",
            "author_name": "alice",
            "author_display": "Alice",
            "persona_id": null,
            "persona_name": null,
            "body": "hello",
            "tier": "default",
            "sent_at": "2026-05-30T00:00:00.000000000Z"
        }"#;
        let env: MessageEnvelope = serde_json::from_str(json)
            .expect("envelope omitting persona_description/persona_color must deserialize");
        assert_eq!(env.persona_description, None);
        assert_eq!(env.persona_color, None);
        // The siblings that already had the attribute keep defaulting too.
        assert_eq!(env.persona_avatar_id, None);
        assert!(env.attachments.is_empty());
    }
}
