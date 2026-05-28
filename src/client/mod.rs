//! Browser-side REST client (hydrate-only).
//!
//! Thin gloo-net Fetch wrappers over the server's plain-REST API. Same-origin
//! Fetch sends cookies by default, so the session rides automatically — there
//! is no auth header to attach.

#[cfg(feature = "hydrate")]
pub mod api;
