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
