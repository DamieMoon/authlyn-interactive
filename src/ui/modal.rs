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
//! Focus restoration: the element that had focus at mount is captured via
//! `send_wrapper::SendWrapper<HtmlElement>` (the wasm types aren't `Send`,
//! so the wrapper is what crosses the `StoredValue` / `on_cleanup` boundary)
//! and re-focused on cleanup. This matches WCAG 2.4.3 (Focus Order) when the
//! modal is dismissed via Esc, backdrop click, or its own close button — the
//! keyboard user lands back on the trigger they pressed to open it.

use leptos::ev::KeyboardEvent;
use leptos::html::Div;
use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use send_wrapper::SendWrapper;
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

    // Trigger element to restore focus to on unmount — the thing that had focus
    // at mount, typically the button the user pressed to open the modal.
    // `StoredValue` is `'static` so the cleanup closure can `take` from it; the
    // `SendWrapper` carries the non-`Send` wasm `HtmlElement` across.
    #[cfg(feature = "hydrate")]
    let trigger: StoredValue<Option<SendWrapper<HtmlElement>>> = StoredValue::new(None);

    // Initial focus on mount: the first focusable inside the dialog, or the
    // dialog itself (tabindex=-1) so keystrokes land in scope immediately.
    // Before moving focus, capture the previously-focused element so cleanup
    // can return focus there (WCAG 2.4.3).
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(dialog) = dialog_ref.get() {
            let dialog_el: &web_sys::Element = dialog.as_ref();
            // Snapshot the current active element *before* we steal focus.
            // Skip if it's the dialog itself or one of its descendants — that
            // would mean focus had already moved into the modal, and restoring
            // to the about-to-be-removed dialog is a no-op (or worse).
            let prev = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.active_element())
                .and_then(|el| el.dyn_into::<HtmlElement>().ok())
                .filter(|el| !dialog_el.contains(Some(el.as_ref())));
            if let Some(el) = prev {
                trigger.set_value(Some(SendWrapper::new(el)));
            }

            let target: Option<HtmlElement> =
                first_focusable(dialog_el).or_else(|| dialog.dyn_ref::<HtmlElement>().cloned());
            if let Some(el) = target {
                let _ = el.focus();
            }
        }
    });

    // Restore focus to the trigger on unmount (Esc, backdrop click, explicit
    // close — any path that drops the component). `None` (no element had focus
    // at mount) falls through silently; the document body keeps focus, which
    // is the correct fallback for the initial-keyboard-nav case.
    #[cfg(feature = "hydrate")]
    on_cleanup(move || {
        if let Some(wrap) = trigger.try_get_value().flatten() {
            let _ = wrap.focus();
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
