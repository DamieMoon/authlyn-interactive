//! Freya UI for the native client (feature `freya`).
//!
//! Phase 1 is deliberately minimal: a single column that auto-runs the
//! authenticated round-trip on mount (login/register → `/auth/me` → `/guilds`)
//! and renders the logged-in profile + guild names. The rail/sidebar/channel
//! layout and an interactive login form are later-phase work — auto-login keeps
//! the headless screenshot verification free of simulated input. Builder API
//! per Freya 0.4-rc (de-risked in Phase 0).

use freya::prelude::*;
use std::sync::Arc;

use crate::native::api::ApiClient;

/// Root component: drives the auth round-trip and renders the result.
pub fn app() -> impl IntoElement {
    let mut status = use_state(|| "connecting…".to_string());
    let mut profile = use_state(String::new);
    let mut guilds = use_state(Vec::<String>::new);

    // Fire the round-trip exactly once, on mount. Freya's `spawn` runs on its
    // own executor (tokio context is entered in `main`) and may update state.
    use_hook(move || {
        let client = Arc::new(ApiClient::new());
        spawn(async move {
            let user =
                std::env::var("AUTHLYN_NATIVE_USER").unwrap_or_else(|_| "native-dev".to_string());
            let pass = std::env::var("AUTHLYN_NATIVE_PASS")
                .unwrap_or_else(|_| "native-dev-password".to_string());

            if let Err(e) = client.ensure_session(&user, &pass).await {
                status.set(format!("auth failed — {e}"));
                return;
            }
            match client.current_user().await {
                Ok(me) => profile.set(format!("{} (@{})", me.display_name, me.username)),
                Err(e) => {
                    status.set(format!("/auth/me failed — {e}"));
                    return;
                }
            }
            let mut list = match client.list_guilds().await {
                Ok(r) => r.guilds,
                Err(e) => {
                    status.set(format!("/guilds failed — {e}"));
                    return;
                }
            };
            // The dev DB starts empty; create one guild so the render is non-trivial.
            if list.is_empty() {
                if let Ok(g) = client.create_guild("Native Test Guild").await {
                    list.push(g);
                }
            }
            status.set(format!("authenticated · {} guild(s)", list.len()));
            guilds.set(list.into_iter().map(|g| g.name).collect());
        });
    });

    let mut col = rect()
        .width(Size::fill())
        .height(Size::fill())
        .background((20, 22, 30))
        .color(Color::WHITE)
        .padding(Gaps::new_all(22.))
        .child(label().text("authlyn-native · Phase 1 — login + guilds"))
        .child(label().text(format!("status: {}", status.read())))
        .child(label().text(format!("user:   {}", profile.read())))
        .child(label().text("guilds:"));

    for name in guilds.read().iter() {
        col = col.child(label().text(format!("   • {name}")));
    }
    col
}
