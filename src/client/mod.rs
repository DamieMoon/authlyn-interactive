//! Browser-side client: the E2EE state machine + the REST/storage glue that
//! drives the routing protocol from the WASM bundle.
//!
//! [`session`] (the [`DeviceClient`] crypto state machine) compiles for both
//! targets — all of `crate::crypto` is target-agnostic, and `app.rs` (which is
//! shared) names the type. The networking ([`api`]) and persistence ([`store`])
//! layers are wasm-only and gated to the `hydrate` feature, so their browser
//! crates (`gloo-*`) never enter the ssr dependency graph.

pub mod session;

pub use session::{DeviceClient, Snapshot};

#[cfg(feature = "hydrate")]
pub mod api;
#[cfg(feature = "hydrate")]
pub mod store;
