//! The authed app frame. `Home` is the `/` route: it bounces to `/login`
//! when the session resolves unauthenticated, otherwise renders the shell.
//! The shell is a placeholder until the server-rail / channel / wardrobe /
//! lorebook / friends panes land in later slices.

use leptos::prelude::*;

use crate::ui::AuthCtx;

#[component]
pub fn Home() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");

    // Once the session check resolves, send unauthenticated visitors to login.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if !auth.loading.get() && auth.user.get().is_none() {
            leptos_router::hooks::use_navigate()("/login", Default::default());
        }
    });

    view! {
        <Show
            when=move || auth.is_authed()
            fallback=|| view! { <p class="loading">"Loading…"</p> }
        >
            <AppShell/>
        </Show>
    }
}

#[component]
fn AppShell() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    let username = move || auth.user.get().map(|u| u.username).unwrap_or_default();

    let logout = move |_| {
        #[cfg(feature = "hydrate")]
        {
            let nav = leptos_router::hooks::use_navigate();
            leptos::task::spawn_local(async move {
                let _ = crate::client::api::logout().await;
                auth.user.set(None);
                nav("/login", Default::default());
            });
        }
    };

    view! {
        <div class="app-shell">
            <header class="topbar">
                <strong>"authlyn"</strong>
                <span class="spacer"></span>
                <span class="muted">"Signed in as " {username}</span>
                <button on:click=logout>"Log out"</button>
            </header>
            <p class="muted placeholder">
                "Servers, channels, the wardrobe, lorebook, and friends arrive in the next slices."
            </p>
        </div>
    }
}
