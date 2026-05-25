//! Browser-side REST client (hydrate-only).
//!
//! Thin gloo-net Fetch wrappers over the server's plain-REST API. Same-origin
//! Fetch sends cookies by default, so the session rides automatically — there
//! is no auth header to attach (unlike the retired E2EE `X-Device-Id` client).

#[cfg(feature = "hydrate")]
pub mod api;
