//! The account-management modal.
//!
//! First (and so far only) feature: change password. The modal is structured
//! as a list of independent account "sections" so future options — e.g. a
//! notification opt-in, a display-name editor, account deletion — can be added
//! by dropping another `<section class="account-section">…</section>` block
//! below the change-password one. Each section owns its own local form state.

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

                // Future account options go here as further
                // `<section class="account-section">…</section>` blocks
                // (e.g. notification opt-in, display name, delete account).

                <p class="account-status">{move || s.status.get()}</p>
            </div>
        </div>
    }
}
