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
            <a href="/reset">"Forgot password?"</a>
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
                placeholder="username (3-32 chars)"
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

/// Public self-service password reset: enter a username, answer the security
/// question the account set, and choose a new password. The question lookup
/// returns `None` for both unknown users and accounts with no question set, so
/// this page can't be used to probe which usernames exist.
#[component]
pub fn ResetPage() -> impl IntoView {
    let username = RwSignal::new(String::new());
    let answer = RwSignal::new(String::new());
    let new_pw = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    // `None` = not looked up yet; `Some(None)` = unknown user / no question;
    // `Some(Some(q))` = show the question and the reset form.
    let question = RwSignal::new(None::<Option<String>>);
    let error = RwSignal::new(String::new());
    let done = RwSignal::new(false);

    view! {
        <div class="auth-card">
            <h1>"authlyn"</h1>
            <h2>"Reset password"</h2>
            {move || if done.get() {
                view! {
                    <p>"Your password has been reset. You can now log in."</p>
                    <a href="/login">"Back to log in"</a>
                }.into_any()
            } else {
                match question.get() {
                    // Stage 1: ask for the username, then look up its question.
                    None => view! {
                        <input
                            prop:value=move || username.get()
                            on:input=move |ev| username.set(event_target_value(&ev))
                            placeholder="username"
                        />
                        <button on:click=move |_| {
                            #[cfg(feature = "hydrate")]
                            {
                                let u = username.get_untracked();
                                if u.trim().is_empty() {
                                    error.set("enter your username".to_string());
                                    return;
                                }
                                error.set(String::new());
                                leptos::task::spawn_local(async move {
                                    match crate::client::api::reset_question(&u).await {
                                        Ok(r) => question.set(Some(r.question)),
                                        Err(e) => error.set(crate::client::api::humanize(&e)),
                                    }
                                });
                            }
                        }>"Continue"</button>
                        <a href="/login">"Back to log in"</a>
                    }.into_any(),
                    // Looked up, but no question set (or the user doesn't exist).
                    Some(None) => view! {
                        <p class="muted">
                            "No security question is set for this account. Ask an admin to reset your password."
                        </p>
                        <a href="/login">"Back to log in"</a>
                    }.into_any(),
                    // Stage 2: show the question and collect answer + new password.
                    Some(Some(q)) => view! {
                        <p class="reset-question">{q}</p>
                        <input
                            prop:value=move || answer.get()
                            on:input=move |ev| answer.set(event_target_value(&ev))
                            placeholder="your answer"
                        />
                        <input type="password"
                            prop:value=move || new_pw.get()
                            on:input=move |ev| new_pw.set(event_target_value(&ev))
                            placeholder="new password (8+ chars)"
                        />
                        <input type="password"
                            prop:value=move || confirm.get()
                            on:input=move |ev| confirm.set(event_target_value(&ev))
                            placeholder="confirm new password"
                        />
                        <button on:click=move |_| {
                            #[cfg(feature = "hydrate")]
                            {
                                let u = username.get_untracked();
                                let a = answer.get_untracked();
                                let p = new_pw.get_untracked();
                                let c = confirm.get_untracked();
                                if p != c {
                                    error.set("new passwords do not match".to_string());
                                    return;
                                }
                                error.set(String::new());
                                leptos::task::spawn_local(async move {
                                    match crate::client::api::confirm_reset(&u, &a, &p).await {
                                        Ok(()) => done.set(true),
                                        Err(e) => error.set(crate::client::api::humanize(&e)),
                                    }
                                });
                            }
                        }>"Reset password"</button>
                        <a href="/login">"Back to log in"</a>
                    }.into_any(),
                }
            }}
            <p class="error">{move || error.get()}</p>
        </div>
    }
}
