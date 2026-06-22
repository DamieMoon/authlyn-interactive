//! authlyn-interactive — a self-hosted, server-trusted roleplay chat platform
//! (Discord + SillyTavern: guilds → channels, personas, lorebooks, friends) as a
//! single Rust crate: axum + Leptos 0.8 + SurrealDB.
//!
//! The crate is split into **three disjoint feature graphs**, mutually exclusive
//! at the binary level — never cross-import between them:
//! - **ssr** (`feature = "ssr"`): the server runtime — [`db`], [`server`],
//!   [`storage`]. Never compiled to wasm.
//! - **hydrate** (`feature = "hydrate"`): the browser/WASM front, mounted by the
//!   `hydrate` WASM entrypoint below. Never the server runtime.
//! - **nova** (`feature = "nova"`, binary `src/bin/nova-mcp.rs`): the MCP bridge.
//!   Imports zero ssr/hydrate.
//!
//! Only [`protocol`] (wire DTOs) and [`markup`] are **always-on** and must compile
//! to `wasm32-unknown-unknown` (serde-only — no axum/surrealdb/tokio); they are the
//! shared spine across all three graphs. Per-graph crate membership and each
//! dependency's purpose live in `Cargo.toml [features]` and its `#`-comments.
//!
//! Architecture overview: `docs/architecture/01-overview.md`.

// The hydrate front's deeply-nested view types (AppShell) overflow the default
// type-layout recursion limit when the release profile computes the async
// hydration layout. Raise it crate-wide; harmless for the ssr build.
#![recursion_limit = "512"]

pub mod app;
pub mod client;
pub mod markup;
pub mod protocol;
pub mod ui;

#[cfg(feature = "ssr")]
pub mod db;

#[cfg(feature = "ssr")]
pub mod server;

#[cfg(feature = "ssr")]
pub mod storage;

/// WASM entrypoint (hydrate graph): installs the panic hook and hydrates the
/// server-rendered DOM with the [`app::App`] component. Exported to JS via
/// `wasm_bindgen` and called from the cargo-leptos-generated bootstrap script.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
