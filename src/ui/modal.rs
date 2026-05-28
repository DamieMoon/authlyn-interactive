//! Shared modal component: `.modal-backdrop` + `.modal $extra-class` wrapper
//! with the a11y the audit asked for — `role="dialog"`, `aria-modal="true"`,
//! Esc-to-close, focus-trap (Tab/Shift+Tab cycle within the dialog), and
//! initial focus on the first focusable child. Replaces 6 hand-rolled
//! backdrops across the shell (PendingDelete confirm, the account modal,
//! persona-info popups in channel + wardrobe, persona-detail editor,
//! persona-remove confirm).
//!
//! The attachment lightbox stays bespoke — it uses a `.lightbox` parent
//! rather than the `.modal-backdrop` pattern, and styling it through this
//! component would change visuals.
//!
//! Behavior:
//! - Backdrop click → invokes `close` (the closure the caller passes).
//! - Dialog click → `stop_propagation`, so a click inside doesn't bubble up
//!   and dismiss.
//! - Esc keydown on the dialog → invokes `close`. The dialog's
//!   `stop_propagation` keeps Esc from competing with the channel-view
//!   popover Esc handler (channel.rs:1186) when both are on screen — a
//!   Modal on top wins.
//! - Tab / Shift+Tab on the dialog → wrap focus within the dialog's
//!   focusables, so keyboard users can't tab out into the background page.
//! - On mount the first focusable inside the dialog (or the dialog itself,
//!   via `tabindex="-1"`) is given focus so keystrokes land in scope
//!   immediately.
//!
//! Focus restoration to the trigger element on unmount is intentionally
//! deferred: capturing `web_sys::HtmlElement` in a Leptos `on_cleanup`
//! closure requires Send+Sync wrapping (the wasm types aren't), which is
//! more machinery than is justified for the audit's a11y win.

use leptos::ev::KeyboardEvent;
use leptos::html::Div;
use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use wasm_bindgen::JsCast;
#[cfg(feature = "hydrate")]
use web_sys::HtmlElement;

/// Render a centered modal dialog backed by a click-to-dismiss backdrop.
///
/// `class` — extra CSS class on the dialog element, e.g. `"confirm-modal"` or
/// `"persona-detail"`. Applied alongside the base `modal` class so existing
/// per-site styling continues to work without edits to `main.scss`.
///
/// `close` — invoked when the user clicks the backdrop, hits Esc, or
/// activates whatever explicit close affordance the children render. The
/// caller owns the open/close signal; the modal never mutates state itself.
///
/// `children` — the dialog's contents. Caller is responsible for any explicit
/// close button inside.
#[component]
pub fn Modal<F>(
    /// Extra CSS class(es) applied to the `.modal` dialog element.
    #[prop(into)]
    class: String,
    /// Caller-owned close handler; fired on backdrop click and Esc.
    close: F,
    children: Children,
) -> impl IntoView
where
    F: Fn() + Copy + 'static,
{
    let dialog_class = format!("modal {class}");
    let dialog_ref = NodeRef::<Div>::new();

    // Initial focus on mount: the first focusable inside the dialog, or the
    // dialog itself (tabindex=-1) so keystrokes land in scope immediately.
    // No captured wasm state, so no Send+Sync grief.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(dialog) = dialog_ref.get() {
            let target: Option<HtmlElement> = first_focusable(dialog.as_ref())
                .or_else(|| dialog.dyn_ref::<HtmlElement>().cloned());
            if let Some(el) = target {
                let _ = el.focus();
            }
        }
    });

    let on_keydown = move |ev: KeyboardEvent| {
        #[cfg(feature = "hydrate")]
        {
            match ev.key().as_str() {
                "Escape" => {
                    ev.prevent_default();
                    ev.stop_propagation();
                    close();
                }
                "Tab" => {
                    if let Some(dialog) = dialog_ref.get() {
                        let focusables = collect_focusables(dialog.as_ref());
                        if focusables.is_empty() {
                            return;
                        }
                        let active = web_sys::window()
                            .and_then(|w| w.document())
                            .and_then(|d| d.active_element())
                            .and_then(|el| el.dyn_into::<HtmlElement>().ok());
                        let idx = active
                            .as_ref()
                            .and_then(|a| focusables.iter().position(|el| el == a));
                        let last = focusables.len() - 1;
                        let (wrap, target) = if ev.shift_key() {
                            (idx == Some(0) || idx.is_none(), last)
                        } else {
                            (idx == Some(last), 0)
                        };
                        if wrap {
                            ev.prevent_default();
                            let _ = focusables[target].focus();
                        }
                    }
                }
                _ => {}
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = &ev;
    };

    view! {
        <div class="modal-backdrop" on:click=move |_| close()>
            <div node_ref=dialog_ref class=dialog_class
                role="dialog" aria-modal="true" tabindex="-1"
                on:click=move |_ev| {
                    #[cfg(feature = "hydrate")]
                    _ev.stop_propagation();
                }
                on:keydown=on_keydown>
                {children()}
            </div>
        </div>
    }
}

/// Selector for the focusable-children query.
#[cfg(feature = "hydrate")]
const FOCUSABLE_SEL: &str = "a[href], button:not([disabled]), input:not([disabled]), \
                             textarea:not([disabled]), select:not([disabled]), \
                             [tabindex]:not([tabindex=\"-1\"])";

#[cfg(feature = "hydrate")]
fn collect_focusables(dialog: &web_sys::Element) -> Vec<HtmlElement> {
    let Ok(list) = dialog.query_selector_all(FOCUSABLE_SEL) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(list.length() as usize);
    for i in 0..list.length() {
        if let Some(node) = list.item(i) {
            if let Ok(el) = node.dyn_into::<HtmlElement>() {
                out.push(el);
            }
        }
    }
    out
}

#[cfg(feature = "hydrate")]
fn first_focusable(dialog: &web_sys::Element) -> Option<HtmlElement> {
    collect_focusables(dialog).into_iter().next()
}
