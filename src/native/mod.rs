//! Native desktop/Android client (Freya), feature-gated `freya`.
//!
//! Scope: a standalone Skia-rendered client that talks to the same axum REST
//! backend over `reqwest`. It reuses the always-on wire DTOs in
//! [`crate::protocol`] verbatim and imports ZERO ssr/hydrate crates (no
//! axum/surrealdb/leptos/web-sys/gloo) — the same disjointness rule the `nova`
//! bridge follows. Phase 1 covers the foundations: an authenticated round-trip
//! (login → `/auth/me` → `/guilds`) rendered in a Freya window.

pub mod act;
pub mod api;
pub mod emoji_manager;
pub mod friends;
pub mod image;
pub mod lorebook;
pub mod markup_view;
pub mod members;
pub mod modal;
pub mod state;
pub mod theme;
pub mod trash;
pub mod ui;
pub mod wardrobe;
