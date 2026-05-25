//! Login + registration pages (public routes).

use leptos::prelude::*;

use crate::ui::AuthCtx;

#[component]
pub fn LoginPage() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    // `auth` is read only inside the hydrate-only submit handler below.
    #[cfg(not(feature = "hydrate"))]
    let _ = auth;
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());

    let submit = move |_| {
        #[cfg(feature = "hydrate")]
        {
            let nav = leptos_router::hooks::use_navigate();
            let body = crate::protocol::LoginRequest {
                username: username.get_untracked(),
                password: password.get_untracked(),
            };
            error.set(String::new());
            leptos::task::spawn_local(async move {
                match crate::client::api::login(&body).await {
                    Ok(_) => {
                        if let Ok(me) = crate::client::api::current_user().await {
                            auth.user.set(Some(me));
                        }
                        nav("/", Default::default());
                    }
                    Err(e) => error.set(crate::client::api::humanize(&e)),
                }
            });
        }
    };

    view! {
        <div class="auth-card">
            <h1>"authlyn"</h1>
            <h2>"Log in"</h2>
            <input
                prop:value=move || username.get()
                on:input=move |ev| username.set(event_target_value(&ev))
                placeholder="username"
            />
            <input
                type="password"
                prop:value=move || password.get()
                on:input=move |ev| password.set(event_target_value(&ev))
                placeholder="password"
            />
            <button on:click=submit>"Log in"</button>
            <p class="error">{move || error.get()}</p>
            <a href="/register">"Create an account"</a>
        </div>
    }
}

#[component]
pub fn RegisterPage() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    // `auth` is read only inside the hydrate-only submit handler below.
    #[cfg(not(feature = "hydrate"))]
    let _ = auth;
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());

    let submit = move |_| {
        #[cfg(feature = "hydrate")]
        {
            let nav = leptos_router::hooks::use_navigate();
            let body = crate::protocol::RegisterRequest {
                username: username.get_untracked(),
                password: password.get_untracked(),
            };
            error.set(String::new());
            leptos::task::spawn_local(async move {
                match crate::client::api::register(&body).await {
                    Ok(_) => {
                        if let Ok(me) = crate::client::api::current_user().await {
                            auth.user.set(Some(me));
                        }
                        nav("/", Default::default());
                    }
                    Err(e) => error.set(crate::client::api::humanize(&e)),
                }
            });
        }
    };

    view! {
        <div class="auth-card">
            <h1>"authlyn"</h1>
            <h2>"Create an account"</h2>
            <input
                prop:value=move || username.get()
                on:input=move |ev| username.set(event_target_value(&ev))
                placeholder="username (3–32 chars)"
            />
            <input
                type="password"
                prop:value=move || password.get()
                on:input=move |ev| password.set(event_target_value(&ev))
                placeholder="password (8+ chars)"
            />
            <button on:click=submit>"Sign up"</button>
            <p class="error">{move || error.get()}</p>
            <a href="/login">"I already have an account"</a>
        </div>
    }
}
