//! Browser UI (Leptos). Compiles for both ssr and hydrate; all data-fetching
//! lives in `#[cfg(feature = "hydrate")]` blocks so the gloo-net client never
//! enters the ssr graph (the bodies are empty closures under ssr).

use leptos::prelude::*;

use crate::protocol::MeResponse;

pub mod auth;
pub mod markup_view;
pub mod shell;

/// Session state, provided once at the app root and read everywhere.
/// `user` is `None` until resolved; `loading` gates the first paint so we
/// don't flash the app before the `/auth/me` check lands.
#[derive(Clone, Copy)]
pub struct AuthCtx {
    pub user: RwSignal<Option<MeResponse>>,
    pub loading: RwSignal<bool>,
}

impl AuthCtx {
    pub fn is_authed(&self) -> bool {
        self.user.get().is_some()
    }
}
