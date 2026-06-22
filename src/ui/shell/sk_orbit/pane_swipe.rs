//! M7 edge-swipe-back for the six non-channel dispatch panes (Friends, Members,
//! Emoji, Lorebook, DirectMessages, Cameos). These mount full-viewport in the
//! orbit pane-dispatch but, unlike the wardrobe slide-over, dismissed ONLY by
//! the top-left back disc — diverging from the product's swipe-right-to-close
//! paradigm (`modal::SwipeClose`, a-orbit.html:979-997). This engine mirrors
//! that wardrobe gesture: an axis-locked rightward drag past ~28% of the
//! viewport fires `on_close` (wired to `act::show_orbit_map`, the panes' correct
//! dismiss target). The back disc stays the keyboard/a11y fallback.
//!
//! Reuse, not a new gesture engine: the H/V arbitration is `strip::axis_lock`
//! (the SAME slop + dominance the swipe-strip uses) and the commit threshold is
//! `modal::SwipeClose`'s 28%-of-viewport rule, so the feel matches both the
//! strip and the wardrobe. Per-move it writes an inline `transform` + a
//! `.dragging` class (the lightbox/holopanel/Modal discipline — no signal
//! re-render); a hydrate-real impl pairs with an ssr no-op stub so the always-on
//! `<div>` bindings typecheck on the server.

#[cfg(feature = "hydrate")]
use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use super::strip::{axis_lock, Axis};

/// Fraction of the viewport width a rightward drag must cross to commit the
/// back-dismiss — the SAME 28% the wardrobe slide-over uses
/// (`modal::SWIPE_CLOSE_FRACTION`, a-orbit.html:994), so the two gestures feel
/// identical. Kept local (the modal const is private to that module) and pinned
/// to the same value by the unit test below.
#[cfg(any(feature = "hydrate", test))]
const PANE_BACK_FRACTION: f64 = 0.28;

/// Whether a released drag commits to BACK: a RIGHTWARD travel (`dx > 0`) past
/// `PANE_BACK_FRACTION` of the viewport width. A leftward drag never dismisses
/// (the pane fills the viewport; there is nothing to its right to reveal). Pure
/// (no DOM) so it is unit-testable — mirrors `modal::swipe_commits_close`.
#[cfg(any(feature = "hydrate", test))]
pub(crate) fn pane_back_commits(dx: f64, viewport_w: f64) -> bool {
    dx > 0.0 && dx > viewport_w * PANE_BACK_FRACTION
}

/// The swipe-right-to-go-back drag engine for the non-channel panes. Owns the
/// drag bookkeeping and exposes `down`/`moved`/`up` handlers bound ungated on
/// the pane wrapper `<div>` (a hydrate-real impl pairs with an ssr no-op stub,
/// mirroring `modal::SwipeClose` / `drag::StripDrag`). WASM is single-threaded,
/// so the state lives in plain signals.
#[derive(Clone)]
pub struct PaneSwipe {
    /// The pane wrapper node — pointer-capture target + the element we translate.
    #[cfg(feature = "hydrate")]
    pane_ref: NodeRef<leptos::html::Div>,
    /// `(x0, y0)` at pointerdown, else None.
    #[cfg(feature = "hydrate")]
    start: RwSignal<Option<(f64, f64)>>,
    /// Locked axis once past slop, else None.
    #[cfg(feature = "hydrate")]
    axis: RwSignal<Option<Axis>>,
    /// Live horizontal travel (px), mirrored into the inline transform — used at
    /// release to decide whether a spring-back transform must be cleared.
    #[cfg(feature = "hydrate")]
    dx: RwSignal<f64>,
}

#[cfg(feature = "hydrate")]
impl PaneSwipe {
    pub fn new(pane_ref: NodeRef<leptos::html::Div>) -> Self {
        Self {
            pane_ref,
            start: RwSignal::new(None),
            axis: RwSignal::new(None),
            dx: RwSignal::new(0.0),
        }
    }

    /// `pointerdown`: capture the pointer (so moves outside the pane keep feeding
    /// the gesture — proven in `holopanel.rs`/`modal.rs`) and record the start.
    /// A press that STARTS on an interactive control (a button / link / input /
    /// the in-pane "← Friends" links) must reach that control: bail before
    /// pointer-capture, or the captured stream redirects the trailing `click` to
    /// the wrapper and the control never fires on DESKTOP (the M6 regression
    /// pattern; touch dispatches the tap's click before the capture override, so
    /// iOS was unaffected). Mirrors `drag::StripDrag::down` / `modal::SwipeClose`.
    pub fn down(&self, ev: &leptos::ev::PointerEvent) {
        use leptos::wasm_bindgen::JsCast as _;
        if ev
            .target()
            .and_then(|t| t.dyn_into::<leptos::web_sys::Element>().ok())
            .and_then(|e| {
                e.closest("button, a[href], input, textarea, select, label, [role=\"button\"]")
                    .ok()
                    .flatten()
            })
            .is_some()
        {
            return;
        }
        if let Some(el) = self.pane_ref.get_untracked() {
            let el: &leptos::web_sys::Element = (*el).unchecked_ref();
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        self.start
            .set(Some((ev.client_x() as f64, ev.client_y() as f64)));
        self.axis.set(None);
        self.dx.set(0.0);
    }

    /// `pointermove`: lock to an axis past the slop (`strip::axis_lock` — the
    /// same arbitration the swipe-strip uses); while locked horizontal AND
    /// dragging RIGHT, add `.dragging` (kills the spring-back transition) and
    /// translate the pane with the finger via an inline `transform`. A leftward
    /// or vertical-locked drag leaves the transform untouched so the pane scrolls.
    pub fn moved(&self, ev: &leptos::ev::PointerEvent) {
        use leptos::wasm_bindgen::JsCast as _;
        let Some((x0, y0)) = self.start.get_untracked() else {
            return;
        };
        let dx = ev.client_x() as f64 - x0;
        let dy = ev.client_y() as f64 - y0;
        if self.axis.get_untracked().is_none() {
            self.axis.set(axis_lock(dx, dy));
        }
        if self.axis.get_untracked() == Some(Axis::Horizontal) && dx > 0.0 {
            // Horizontal-right lock: keep the page from rubber-banding and track
            // the finger 1:1.
            ev.prevent_default();
            if let Some(el) = self.pane_ref.get_untracked() {
                let el: &leptos::web_sys::Element = (*el).unchecked_ref();
                let _ = el.class_list().add_1("dragging");
                if let Some(html) = el.dyn_ref::<leptos::web_sys::HtmlElement>() {
                    let _ = html
                        .style()
                        .set_property("transform", &format!("translateX({dx}px)"));
                }
            }
            self.dx.set(dx);
        }
    }

    /// `pointerup` / `pointercancel`: drop `.dragging`. RETURNS `true` when the
    /// rightward travel crossed the back threshold — the VIEW then fires the
    /// caller's `on_close` (so the engine stays free of the non-`Send` caller
    /// closure). Otherwise eases the inline transform home and clears it on the
    /// next `transitionend` so the at-rest pane is transform-free again. Mirrors
    /// `modal::SwipeClose::up`.
    pub fn up(&self, ev: &leptos::ev::PointerEvent) -> bool {
        use leptos::wasm_bindgen::JsCast as _;
        let Some((x0, _)) = self.start.get_untracked() else {
            return false;
        };
        self.start.set(None);
        let dx = ev.client_x() as f64 - x0;
        let was_h = self.axis.get_untracked() == Some(Axis::Horizontal);
        self.axis.set(None);
        let was_dragging = self.dx.get_untracked() > 0.0;
        self.dx.set(0.0);
        let pane = self.pane_ref.get_untracked();
        if let Some(el) = pane.as_ref() {
            let el: &leptos::web_sys::Element = el.as_ref();
            let _ = el.class_list().remove_1("dragging");
        }
        let viewport_w = leptos::web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(360.0);
        if was_h && pane_back_commits(dx, viewport_w) {
            // Commit: tell the view to dismiss to the orbit map.
            return true;
        }
        // Spring back home, then clear the inline transform once it lands — but
        // ONLY if we actually pushed the pane (a pure tap / vertical scroll left
        // no transform, so there is nothing to spring; adding `translateX(0)` at
        // rest would needlessly re-anchor any nested fixed descendant). Mirrors
        // `modal::SwipeClose::up`'s one-shot transitionend cleanup.
        if was_dragging {
            if let Some(html) =
                pane.and_then(|el| el.dyn_ref::<leptos::web_sys::HtmlElement>().cloned())
            {
                let _ = html.style().set_property("transform", "translateX(0px)");
                let html_for_cb = html.clone();
                let cleanup =
                    leptos::wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                        let _ = html_for_cb.style().remove_property("transform");
                    });
                let opts = leptos::web_sys::AddEventListenerOptions::new();
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

/// ssr stubs: pointer events only exist in the browser, but the wrapper's
/// handler bindings are always-on and must typecheck on the server.
#[cfg(not(feature = "hydrate"))]
impl PaneSwipe {
    pub fn down(&self, _ev: &leptos::ev::PointerEvent) {}
    pub fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
    pub fn up(&self, _ev: &leptos::ev::PointerEvent) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_back_commits_only_rightward_past_threshold() {
        let vw = 360.0;
        let thresh = vw * PANE_BACK_FRACTION; // 100.8px
                                              // Rightward past 28% of the viewport → back.
        assert!(pane_back_commits(thresh + 1.0, vw));
        // Rightward but short of the threshold → no dismiss (springs back).
        assert!(!pane_back_commits(thresh - 1.0, vw));
        // Leftward never dismisses (the pane fills the viewport, nothing to its
        // right) — matches the wardrobe slide-over (modal::swipe_commits_close).
        assert!(!pane_back_commits(-200.0, vw));
        // Zero travel never dismisses.
        assert!(!pane_back_commits(0.0, vw));
    }

    #[test]
    fn pane_back_fraction_matches_the_wardrobe_slide_over() {
        // The pane back-swipe and the wardrobe Modal swipe-close MUST share the
        // 28% commit threshold so the two gestures feel identical (a-orbit.html:994).
        // The modal const is module-private, so pin the value here.
        assert!((PANE_BACK_FRACTION - 0.28).abs() < 1e-9);
    }
}
