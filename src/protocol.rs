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
