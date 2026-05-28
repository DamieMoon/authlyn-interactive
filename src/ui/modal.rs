//! Shared modal component: `.modal-backdrop` + `.modal $extra-class` wrapper
//! with the a11y the audit asked for — `role="dialog"`, `aria-modal="true"`,
//! and backdrop click-to-close. Replaces 6 hand-rolled backdrops across the
//! shell (PendingDelete confirm, the account modal, persona-info popups in
//! channel + wardrobe, persona-detail editor, persona-remove confirm).
//!
//! The attachment lightbox stays bespoke — it uses a `.lightbox` parent
//! rather than the `.modal-backdrop` pattern, and styling it through this
//! component would change visuals (W6 redesign concern).
//!
//! Behavior:
//! - Backdrop click → invokes `close` (the closure the caller passes).
//! - Dialog click → `stop_propagation`, so a click inside doesn't bubble up
//!   and dismiss.
//!
//! Esc-to-close + focus-move-on-open are DEFERRED to W6 with the SCSS work —
//! both need a Closure/on_cleanup dance to avoid leaking a window listener
//! per modal open, and the focus-trap pattern interacts with the redesign's
//! z-index/stacking decisions. The audit's role/aria-modal items, which are
//! the screen-reader-visible half, ship here.

use leptos::prelude::*;

/// Render a centered modal dialog backed by a click-to-dismiss backdrop.
///
/// `class` — extra CSS class on the dialog element, e.g. `"confirm-modal"` or
/// `"persona-detail"`. Applied alongside the base `modal` class so existing
/// per-site styling continues to work without edits to `main.scss`.
///
/// `close` — invoked when the user clicks the backdrop. The caller owns the
/// open/close signal; the modal never mutates state itself.
///
/// `children` — the dialog's contents. Caller is responsible for any explicit
/// close button inside.
#[component]
pub fn Modal<F>(
    /// Extra CSS class(es) applied to the `.modal` dialog element.
    #[prop(into)]
    class: String,
    /// Caller-owned close handler; fired on backdrop click.
    close: F,
    children: Children,
) -> impl IntoView
where
    F: Fn() + 'static,
{
    let dialog_class = format!("modal {class}");
    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div class=dialog_class
                role="dialog" aria-modal="true"
                on:click=move |_ev| {
                    #[cfg(feature = "hydrate")]
                    _ev.stop_propagation();
                }>
                {children()}
            </div>
        </div>
    }
}
