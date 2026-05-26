//! The account-management modal.
//!
//! Sections: change password, preferences, send feedback / report a bug.
//! Each section owns its own local form state.

use leptos::prelude::*;

use super::{act, Shell};

/// The account-management window. Renders a `.modal-backdrop`/`.modal`
/// (classes shared with the persona-info popup) over the shell. `open` is the
/// caller's visibility signal; the ✕ and the backdrop both flip it to `false`.
#[component]
pub(crate) fn AccountModal(s: Shell, open: RwSignal<bool>) -> impl IntoView {
    // ---- change-password section: local form state ----
    let current = RwSignal::new(String::new());
    let new_pw = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());

    // ---- preferences section: message-delete confirmation toggle ----
    // Seeded from localStorage (default ON); each change persists immediately.
    let confirm_delete_msg = RwSignal::new(act::confirm_delete_message_enabled());

    // ---- feedback section: local form state ----
    let feedback_open = RwSignal::new(false);
    let fb_kind = RwSignal::new("other".to_string());
    let fb_body = RwSignal::new(String::new());

    // ---- feedback INBOX (admin only): None until loaded; stays None for
    // non-admins (the server 403s GET /feedback), so the section never renders.
    // Loaded when the modal opens. ----
    let inbox = RwSignal::new(None::<Vec<crate::protocol::FeedbackItem>>);
    Effect::new(move |_| {
        let is_open = open.get();
        #[cfg(feature = "hydrate")]
        if is_open && inbox.get_untracked().is_none() {
            leptos::task::spawn_local(async move {
                if let Ok(r) = crate::client::api::list_feedback().await {
                    inbox.set(Some(r.items));
                }
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = is_open;
    });

    let save = move |_| {
        let cur = current.get_untracked();
        let new = new_pw.get_untracked();
        let conf = confirm.get_untracked();
        // Client-side guard before hitting the server; the server re-checks.
        if new != conf {
            s.status.set("new passwords do not match".to_string());
            return;
        }
        act::change_password(s, cur, new);
        // Clear the inputs; the status line reports success/failure.
        current.set(String::new());
        new_pw.set(String::new());
        confirm.set(String::new());
    };

    let send_feedback = move |_| {
        let kind = fb_kind.get_untracked();
        let body = fb_body.get_untracked();
        if body.trim().is_empty() {
            s.status.set("feedback body must not be empty".to_string());
            return;
        }
        // Build context JSON — hydrate-gated via act so web_sys never runs on ssr.
        let context = act::build_feedback_context(s);
        act::submit_feedback(s, kind, body, context, feedback_open);
        fb_body.set(String::new());
    };

    view! {
        // Backdrop click closes; stop propagation on the panel so inner clicks
        // don't bubble up and close it.
        <div class="modal-backdrop" on:click=move |_| open.set(false)>
            <div class="modal account-modal" on:click=|ev| ev.stop_propagation()>
                <header class="account-head">
                    <h2>"Account"</h2>
                    <button class="row-edit" title="Close"
                        on:click=move |_| open.set(false)>"✕"</button>
                </header>

                // ---- Change password ----
                <section class="account-section">
                    <h3>"Change password"</h3>
                    <input type="password" placeholder="current password"
                        prop:value=move || current.get()
                        on:input=move |ev| current.set(event_target_value(&ev))/>
                    <input type="password" placeholder="new password"
                        prop:value=move || new_pw.get()
                        on:input=move |ev| new_pw.set(event_target_value(&ev))/>
                    <input type="password" placeholder="confirm new password"
                        prop:value=move || confirm.get()
                        on:input=move |ev| confirm.set(event_target_value(&ev))/>
                    <button class="account-save" on:click=save>"Save"</button>
                </section>

                // ---- Preferences ----
                <section class="account-section">
                    <h3>"Preferences"</h3>
                    <label class="pref-row">
                        <input type="checkbox" prop:checked=move || confirm_delete_msg.get()
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                confirm_delete_msg.set(on);
                                act::set_confirm_delete_message(on);
                            }/>
                        <span>"Ask before deleting a message"</span>
                    </label>
                    <label class="pref-row">
                        <input type="checkbox" prop:checked=move || s.dialogue_style.get()
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                s.dialogue_style.set(on);
                                act::set_rp_dialogue_style(on);
                            }/>
                        <span>"Style roleplay dialogue"</span>
                    </label>
                </section>

                // ---- Feedback / bug report ----
                <section class="account-section">
                    <h3>"Send feedback / Report a bug"</h3>
                    {move || if feedback_open.get() {
                        view! {
                            <div class="feedback-form">
                                <div class="feedback-kind-row">
                                    <label class="pref-row">
                                        <input type="radio" name="fb-kind" value="bug"
                                            prop:checked=move || fb_kind.get() == "bug"
                                            on:change=move |_| fb_kind.set("bug".to_string())/>
                                        <span>"Bug"</span>
                                    </label>
                                    <label class="pref-row">
                                        <input type="radio" name="fb-kind" value="idea"
                                            prop:checked=move || fb_kind.get() == "idea"
                                            on:change=move |_| fb_kind.set("idea".to_string())/>
                                        <span>"Idea"</span>
                                    </label>
                                    <label class="pref-row">
                                        <input type="radio" name="fb-kind" value="other"
                                            prop:checked=move || fb_kind.get() == "other"
                                            on:change=move |_| fb_kind.set("other".to_string())/>
                                        <span>"Other"</span>
                                    </label>
                                </div>
                                <textarea class="feedback-body" rows="5"
                                    placeholder="Describe the bug or your idea…"
                                    prop:value=move || fb_body.get()
                                    on:input=move |ev| fb_body.set(event_target_value(&ev))/>
                                <div class="feedback-actions">
                                    <button class="account-save" on:click=send_feedback>"Send"</button>
                                    <button on:click=move |_| {
                                        feedback_open.set(false);
                                        fb_body.set(String::new());
                                        fb_kind.set("other".to_string());
                                    }>"Cancel"</button>
                                </div>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <button class="account-save"
                                on:click=move |_| feedback_open.set(true)>
                                "Open feedback form"
                            </button>
                        }.into_any()
                    }}
                </section>

                // ---- Feedback inbox (admin only; renders only once GET /feedback
                // succeeds, i.e. the caller is in AUTHLYN_ADMIN_USERNAMES) ----
                {move || inbox.get().map(|items| {
                    let n = items.len();
                    view! {
                        <section class="account-section feedback-inbox">
                            <h3>{format!("Feedback inbox ({n})")}</h3>
                            {if items.is_empty() {
                                view! { <p class="muted">"No feedback submitted yet."</p> }.into_any()
                            } else {
                                view! {
                                    <ul class="fb-list">
                                        {items.into_iter().map(|it| {
                                            let crate::protocol::FeedbackItem {
                                                author_username, kind, body, context, created_at, ..
                                            } = it;
                                            let kind_class = format!("fb-kind fb-{kind}");
                                            view! {
                                                <li class="fb-item">
                                                    <div class="fb-meta">
                                                        <span class=kind_class>{kind}</span>
                                                        <span class="fb-who">{author_username}</span>
                                                        <time class="fb-when">{created_at}</time>
                                                    </div>
                                                    <p class="fb-body">{body}</p>
                                                    {context.map(|c| view! {
                                                        <p class="fb-ctx muted">{c}</p>
                                                    })}
                                                </li>
                                            }
                                        }).collect_view()}
                                    </ul>
                                }.into_any()
                            }}
                        </section>
                    }
                })}

                <p class="account-status">{move || s.status.get()}</p>
            </div>
        </div>
    }
}
