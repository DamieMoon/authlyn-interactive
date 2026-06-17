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
//!
//! Swipe-to-close (opt-in, `swipe_close`): under the Omloppsbana (`.app.sk-orbit`)
//! skeleton the dialog is presented as a full-screen slide-over (SCSS in
//! `_modal.scss`), and the prototype closes it with a rightward drag inside the
//! panel (a-orbit.html:979-997). When `swipe_close` is set, the dialog binds
//! pointer handlers that translate the panel with the finger (inline
//! `transform` + a `.dragging` class that kills the spring-back transition) and
//! invoke `close` once the drag crosses ~28% of the viewport width — reusing
//! the SAME `close` the backdrop/Esc fire, so focus-restore is unchanged. This
//! is purely ADDITIVE presentation behaviour; the a11y machinery above is
//! untouched. The default (no `swipe_close`) leaves desktop/deck/hud modals
//! exactly as before. Drag bookkeeping lives in a hydrate-real / ssr-stub
//! engine (mirroring `holopanel::PanelDrag`), so the always-on `view!` binds
//! the handlers and only the browser build touches `web_sys`.

use leptos::ev::KeyboardEvent;
#[cfg(feature = "hydrate")]
use leptos::ev::PointerEvent;
use leptos::html::Div;
use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use send_wrapper::SendWrapper;
#[cfg(feature = "hydrate")]
use wasm_bindgen::JsCast;
#[cfg(feature = "hydrate")]
use web_sys::HtmlElement;

// The swipe-close geometry consts + the two pure decision fns are exercised by
// the hydrate pointer handlers and by the unit tests (which run under the ssr
// harness). Gating them to `hydrate` OR `test` keeps the ssr-NON-test build
// (`cargo clippy --features ssr`, `-D warnings`) free of a dead-code warning
// without sprinkling per-item `#[allow]`s.
/// Tap-vs-drag slop in CSS px before a gesture locks to an axis — matches the
/// prototype's `12` (a-orbit.html:983) and the HoloPanel slop family.
#[cfg(any(feature = "hydrate", test))]
const SWIPE_LOCK_SLOP_PX: f64 = 12.0;
/// Fraction of the viewport width a rightward drag must cross to commit to
/// close (a-orbit.html:994 `innerWidth*.28`).
#[cfg(any(feature = "hydrate", test))]
const SWIPE_CLOSE_FRACTION: f64 = 0.28;

/// Lock decision for the swipe-close drag, given the signed deltas from the
/// pointerdown. Returns `Some('h')` once a horizontal intent is clear,
/// `Some('v')` for a vertical scroll (so the drag bows out and lets the panel
/// scroll), or `None` while still inside the slop. Horizontal needs `|dx|`
/// past the slop AND dominant over `|dy|` by the prototype's 1.2 ratio
/// (a-orbit.html:982-985). Pure (no DOM) so it is unit-testable.
#[cfg(any(feature = "hydrate", test))]
pub(crate) fn swipe_close_lock(dx: f64, dy: f64) -> Option<char> {
    if dx.abs() > SWIPE_LOCK_SLOP_PX && dx.abs() > dy.abs() * 1.2 {
        Some('h')
    } else if dy.abs() > SWIPE_LOCK_SLOP_PX {
        Some('v')
    } else {
        None
    }
}

/// Whether a released drag commits to CLOSE: a RIGHTWARD travel (`dx > 0`)
/// past `SWIPE_CLOSE_FRACTION` of the viewport width (a-orbit.html:994). A
/// leftward drag never closes (the panel sits at the right edge). Pure so it
/// is unit-testable without a DOM.
#[cfg(any(feature = "hydrate", test))]
pub(crate) fn swipe_commits_close(dx: f64, viewport_w: f64) -> bool {
    dx > 0.0 && dx > viewport_w * SWIPE_CLOSE_FRACTION
}

/// The swipe-right-to-close drag engine for a slide-over Modal. Owns the
/// drag-start bookkeeping and exposes `down`/`moved`/`up` handlers bound
/// ungated in the view (a hydrate-real impl pairs with an ssr no-op stub,
/// mirroring `holopanel::PanelDrag`). Only fires when the dialog carries
/// `swipe_close`. WASM is single-threaded, so the state lives in plain signals.
#[cfg(feature = "hydrate")]
#[derive(Clone)]
struct SwipeClose {
    /// Opt-in flag (the `swipe_close` prop): when `false` every handler is an
    /// immediate no-op, so a desktop/deck/hud centered modal — bound with the
    /// same always-on handlers — never drags. Always-on field (cheap bool).
    enabled: bool,
    /// The dialog node — pointer-capture target + the element we translate.
    #[cfg(feature = "hydrate")]
    dialog_ref: NodeRef<Div>,
    /// `(x0, y0)` at pointerdown, plus the locked axis once decided.
    #[cfg(feature = "hydrate")]
    start: RwSignal<Option<(f64, f64)>>,
    #[cfg(feature = "hydrate")]
    locked: RwSignal<Option<char>>,
    /// Live horizontal travel (px), mirrored into the inline transform.
    #[cfg(feature = "hydrate")]
    dx: RwSignal<f64>,
}

#[cfg(feature = "hydrate")]
impl SwipeClose {
    /// `pointerdown`: capture the pointer (so moves outside the panel keep
    /// feeding the gesture — proven in `holopanel.rs`/`lightbox.rs`) and record
    /// the start coordinate. Controls inside the head/body don't stop
    /// propagation, so a tap on a button bubbles a full down→up pair here and
    /// `up` sees a sub-slop travel (no close).
    fn down(&self, ev: &PointerEvent) {
        if !self.enabled {
            return;
        }
        // Don't capture when the press starts on an interactive control — on
        // DESKTOP a captured pointer redirects the trailing `click` to the capture
        // target, so a tapped button/input inside a swipe-close modal never fires
        // its on:click (the M6 desktop regression; mirrors the holopanel.rs /
        // sk_orbit drag fix; touch was unaffected). A tap never drags, so it costs
        // nothing; a real swipe from blank dialog area still captures.
        let on_control = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|e| {
                e.closest("button, a[href], input, textarea, select, label, [role=\"button\"]")
                    .ok()
                    .flatten()
            })
            .is_some();
        if !on_control {
            if let Some(el) = self.dialog_ref.get_untracked() {
                let el: &web_sys::Element = el.as_ref();
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
        self.start
            .set(Some((ev.client_x() as f64, ev.client_y() as f64)));
        self.locked.set(None);
        self.dx.set(0.0);
    }

    /// `pointermove`: lock to an axis past the slop; while locked horizontal and
    /// dragging RIGHT, add `.dragging` (kills the transition) and translate the
    /// panel with the finger via an inline `transform`. A leftward drag or a
    /// vertical lock leaves the transform untouched so the panel scrolls.
    fn moved(&self, ev: &PointerEvent) {
        let Some((x0, y0)) = self.start.get_untracked() else {
            return;
        };
        let dx = ev.client_x() as f64 - x0;
        let dy = ev.client_y() as f64 - y0;
        if self.locked.get_untracked().is_none() {
            self.locked.set(swipe_close_lock(dx, dy));
        }
        if self.locked.get_untracked() == Some('h') && dx > 0.0 {
            if let Some(el) = self.dialog_ref.get_untracked() {
                let el: &web_sys::Element = el.as_ref();
                let _ = el.class_list().add_1("dragging");
                if let Some(html) = el.dyn_ref::<HtmlElement>() {
                    let _ = html
                        .style()
                        .set_property("transform", &format!("translateX({dx}px)"));
                }
            }
            self.dx.set(dx);
        }
    }

    /// `pointerup` / `pointercancel`: drop `.dragging` (re-enabling the
    /// spring-back transition). RETURNS `true` when the rightward travel crossed
    /// the close threshold — the VIEW then fires the caller's `close` (so the
    /// engine stays free of the non-`Send` caller closure; the dismiss path is
    /// the SAME backdrop/Esc one → un-mount → on_cleanup → focus restore).
    /// Otherwise eases the inline transform to `translateX(0)` and clears it on
    /// the next `transitionend` so the at-rest panel is transform-free again (a
    /// transform at rest would re-anchor a nested confirm's fixed backdrop), and
    /// returns `false`.
    fn up(&self, ev: &PointerEvent) -> bool {
        let Some((x0, _)) = self.start.get_untracked() else {
            return false;
        };
        self.start.set(None);
        let dx = ev.client_x() as f64 - x0;
        let viewport_w = web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(360.0);
        let was_dragging = self.dx.get_untracked() > 0.0;
        self.dx.set(0.0);
        let dialog = self.dialog_ref.get_untracked();
        if let Some(el) = dialog.as_ref() {
            let el: &web_sys::Element = el.as_ref();
            let _ = el.class_list().remove_1("dragging");
        }
        if self.locked.get_untracked() == Some('h') && swipe_commits_close(dx, viewport_w) {
            // Commit: tell the view to dismiss (un-mount + focus restore).
            return true;
        }
        // Spring back home, then clear the inline transform once it lands — but
        // ONLY if we actually pushed the panel (an inline `translateX(>0)` is
        // live). A pure tap / vertical scroll left no transform, so there is
        // nothing to spring and we must NOT add a `translateX(0)` (a transform
        // at rest would become the containing block for a nested confirm's fixed
        // backdrop). The `Npx → 0px` change is real, so `transitionend` fires
        // and the once-listener clears the inline transform back to none.
        if was_dragging {
            if let Some(html) = dialog.and_then(|el| el.dyn_ref::<HtmlElement>().cloned()) {
                let _ = html.style().set_property("transform", "translateX(0px)");
                let html_for_cb = html.clone();
                let cleanup =
                    leptos::wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                        let _ = html_for_cb.style().remove_property("transform");
                    });
                // web-sys 0.3.85: `AddEventListenerOptions::new()` (value) +
                // `set_once(true)` (returns ()) + pass `&opts` — the proven
                // binding (act/haptics.rs:66-72).
                let opts = web_sys::AddEventListenerOptions::new();
                opts.set_once(true);
                let _ = html.add_event_listener_with_callback_and_add_event_listener_options(
                    "transitionend",
                    cleanup.as_ref().unchecked_ref(),
                    &opts,
                );
                cleanup.forget(); // one-shot listener owns itself; fires once then GCs
            }
        }
        false
    }
}

/// ssr stubs: pointer events only exist in the browser, but the dialog's
/// handler bindings are always-on and must typecheck on the server. The
/// `enabled` field is carried so the construction site is feature-agnostic
/// (the value is dead on the server — the methods are no-ops regardless).
#[cfg(not(feature = "hydrate"))]
#[derive(Clone)]
struct SwipeClose {
    // Dead on the server (the methods are no-ops) — carried only so the
    // construction site stays feature-agnostic.
    #[allow(dead_code)]
    enabled: bool,
}

#[cfg(not(feature = "hydrate"))]
impl SwipeClose {
    fn down(&self, _ev: &leptos::ev::PointerEvent) {}
    fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
    fn up(&self, _ev: &leptos::ev::PointerEvent) -> bool {
        false
    }
}

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
    /// Opt IN to swipe-right-to-close (the Omloppsbana full-screen slide-over
    /// gesture, a-orbit.html:979-997). Default `false` keeps desktop/deck/hud
    /// centered modals — and the nested confirm sub-dialogs — untouched. When
    /// set, the dialog gains the `.modal--swipe-close` class (SCSS hook) and
    /// pointer handlers that drag the panel with the finger and fire `close`
    /// past the threshold; the existing Esc/backdrop/focus-restore a11y is
    /// unchanged (the gesture reuses the SAME `close`).
    #[prop(optional)]
    swipe_close: bool,
    children: Children,
) -> impl IntoView
where
    F: Fn() + Copy + 'static,
{
    let dialog_class = format!(
        "modal {class}{}",
        if swipe_close {
            " modal--swipe-close"
        } else {
            ""
        }
    );
    let dialog_ref = NodeRef::<Div>::new();

    // Swipe-to-close drag engine (opt-in via `swipe_close`). Hydrate-real /
    // ssr-stub; the handlers are bound ungated on the dialog below. `up` RETURNS
    // whether the gesture committed to close — the view fires the caller's
    // `close` (the SAME dismiss the backdrop/Esc do → un-mount → on_cleanup →
    // focus restore), keeping the engine free of the non-`Send` caller closure.
    let swipe = SwipeClose {
        enabled: swipe_close,
        #[cfg(feature = "hydrate")]
        dialog_ref,
        #[cfg(feature = "hydrate")]
        start: RwSignal::new(None),
        #[cfg(feature = "hydrate")]
        locked: RwSignal::new(None),
        #[cfg(feature = "hydrate")]
        dx: RwSignal::new(0.0),
    };
    let (sw_down, sw_move, sw_up, sw_cancel) = (swipe.clone(), swipe.clone(), swipe.clone(), swipe);

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
                on:keydown=on_keydown
                // Swipe-right-to-close (no-op unless `swipe_close`; the engine's
                // `enabled` flag gates it). Bound ungated — the ssr stub is a
                // no-op so the always-on view typechecks on the server. `up`
                // returns whether to dismiss; the view fires the caller's `close`
                // (same path as backdrop/Esc → on_cleanup focus restore).
                on:pointerdown=move |ev| sw_down.down(&ev)
                on:pointermove=move |ev| sw_move.moved(&ev)
                on:pointerup=move |ev| { if sw_up.up(&ev) { close() } }
                on:pointercancel=move |ev| { if sw_cancel.up(&ev) { close() } }>
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

#[cfg(test)]
mod tests {
    use super::*;

    // The pointer-handler bodies are WASM-only (web_sys), but the swipe-close
    // DECISION logic is extracted into pure fns so the axis-lock + commit
    // threshold are unit-testable without a DOM (mirrors holopanel's pattern).

    #[test]
    fn swipe_lock_needs_dominant_horizontal_past_slop() {
        // Inside the slop in both axes → undecided.
        assert_eq!(swipe_close_lock(5.0, 4.0), None);
        // Horizontal past slop AND dominant over vertical by 1.2x → locks 'h'.
        assert_eq!(swipe_close_lock(20.0, 5.0), Some('h'));
        // Leftward also locks horizontal (sign is handled at commit time).
        assert_eq!(swipe_close_lock(-20.0, 5.0), Some('h'));
        // Mostly-vertical past slop → locks 'v' so the panel scrolls instead.
        assert_eq!(swipe_close_lock(8.0, 30.0), Some('v'));
        // Horizontal past slop but NOT dominant (dy too large) → vertical wins.
        assert_eq!(swipe_close_lock(14.0, 30.0), Some('v'));
    }

    #[test]
    fn swipe_commits_only_rightward_past_threshold() {
        let vw = 360.0;
        let thresh = vw * SWIPE_CLOSE_FRACTION; // 100.8px
                                                // Rightward past 28% of the viewport → close.
        assert!(swipe_commits_close(thresh + 1.0, vw));
        // Rightward but short of the threshold → no close (springs back).
        assert!(!swipe_commits_close(thresh - 1.0, vw));
        // Leftward never closes (the panel sits at the right edge).
        assert!(!swipe_commits_close(-200.0, vw));
        // Zero travel never closes.
        assert!(!swipe_commits_close(0.0, vw));
    }
}
