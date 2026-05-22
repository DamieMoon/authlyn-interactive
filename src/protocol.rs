//! Wire-format DTOs shared by the server (ssr) and the client (hydrate).
//!
//! Anything in here must compile to `wasm32-unknown-unknown`: no axum,
//! no surrealdb, no tokio. Only `serde` + the always-on crypto helpers
//! that already cross-compile.

use serde::{Deserialize, Serialize};

use crate::crypto::PreKeyBundle;

// ---------------------------------------------------------------------------
// POST /keys/upload
// ---------------------------------------------------------------------------

/// Body of `POST /keys/upload`.
///
/// The device ID isn't here — the auth stub takes it from the `X-Device-Id`
/// header, see plan §"Authentication stub".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadKeysRequest {
    /// Opaque user identifier (ULID in v1, but the server doesn't parse).
    pub user_id: String,
    /// The full pre-key bundle: identity keys, OTK pool, fallback key.
    pub bundle: PreKeyBundle,
}

/// Successful response from `POST /keys/upload`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadKeysResponse {
    /// Echo of the device ID we keyed against (from `X-Device-Id`).
    pub device_id: String,
    /// Size of the freshly-installed OTK pool.
    pub otk_count: usize,
}

// ---------------------------------------------------------------------------
// POST /keys/claim/{user}/{device}
// ---------------------------------------------------------------------------

/// What kind of key the server returned: a one-time key consumed from the
/// pool, or the long-lived fallback because the pool was empty.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClaimKind {
    /// A one-time key was consumed; another call will yield a different key
    /// until the pool is empty.
    Otk,
    /// The fallback key. Multiple callers may receive the same fallback
    /// while the OTK pool stays empty; callers should treat this as a
    /// signal that the device needs replenishment.
    Fallback,
}

/// Response from `POST /keys/claim/{user}/{device}`. Same shape regardless
/// of whether an OTK or the fallback was returned, with `kind` discriminating.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimKeyResponse {
    pub kind: ClaimKind,
    /// Echo of the device we claimed against.
    pub device_id: String,
    /// 32-byte Curve25519 identity key (hex). Peers need this to start
    /// an Olm session.
    pub identity_curve25519: String,
    /// 32-byte Ed25519 identity key (hex). Peers can use this to re-verify
    /// the returned signed key on their own.
    pub identity_ed25519: String,
    /// The signed key the peer should use as the OTK in Olm session setup.
    pub key: crate::crypto::SignedPreKey,
}

// ---------------------------------------------------------------------------
// POST /rooms/{id}/keyshare
// ---------------------------------------------------------------------------

/// Body of `POST /rooms/{id}/keyshare`.
///
/// Sender identity comes from the `X-Device-Id` header, never from this
/// body — preventing a depositor from claiming to be someone else.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyshareDeposit {
    /// Recipient device id (server resolves it to a `device` record).
    pub recipient_device: String,
    /// The Olm wire envelope: `(message_type, ciphertext)`.
    pub envelope: crate::crypto::OlmEnvelope,
}

/// Successful response from `POST /rooms/{id}/keyshare`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyshareDepositResponse {
    /// The new `keyshare_envelope` row's id key. Opaque to callers; useful
    /// for tracing/correlation only.
    pub id: String,
}

// ---------------------------------------------------------------------------
// GET /rooms/{id}/keyshare/inbox
// ---------------------------------------------------------------------------

/// One envelope as returned by `GET /rooms/{id}/keyshare/inbox`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboxEnvelope {
    /// Sender device id (the depositor's `X-Device-Id`).
    pub sender_device: String,
    /// The Olm wire envelope.
    pub envelope: crate::crypto::OlmEnvelope,
    /// RFC 3339 timestamp the envelope was deposited.
    pub created_at: String,
}

/// Successful response from `GET /rooms/{id}/keyshare/inbox`.
///
/// **Delete-on-read.** The envelopes returned here have already been
/// removed from the server. If the HTTP response is lost in transit, those
/// envelopes are gone permanently — the recipient cannot decrypt any
/// messages on the affected sender's Megolm session until that sender's
/// next rotation. Acceptable for v1 (LIVE SELECT is the primary delivery
/// channel; the inbox is for catch-up). See `server::keyshare` for the
/// design rationale and upgrade path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyshareInbox {
    /// Envelopes for the calling device in this room, ordered by
    /// `created_at` ASC. A second GET on the same room/device returns an
    /// empty array.
    pub envelopes: Vec<InboxEnvelope>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Generic typed-error body: `{"error": "<reason>"}`. Used for every 4xx
/// and 5xx the keys endpoints can return.
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
