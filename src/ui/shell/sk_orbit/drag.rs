//! W5/P2 swipe-strip drag engine. Mirrors `holopanel::PanelDrag` /
//! `radial::LongPress`: an always-on struct (the `<div>` binds its methods
//! ungated) with hydrate-only fields + a real impl paired to an ssr no-op stub.
//! Per-move it writes `--strip-x` (no signal re-render — the lightbox/holopanel
//! discipline); on release it runs the pure `strip` math and fires `on_commit`.
//! `pointercancel` shares the release path with `pointerup` (lightbox M-35).

// `prelude::*` (signals/Callback/NodeRef) + the strip math + the
// `StripCommit`/`Axis` types are referenced ONLY from the hydrate-real fields
// and impl; on the ssr graph the struct is fieldless and the stubs are pure
// no-ops using fully-qualified `leptos::ev::*`, so both imports are
// hydrate-gated (the disjoint-graph discipline — an ssr-unused import trips
// `-D warnings`).
#[cfg(feature = "hydrate")]
use super::strip::{axis_lock, commit_swipe, strip_offset, Axis, StripCommit};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;

/// The strip drag engine. `on_commit(StripCommit)` is called once on a real
/// release that commits (Prev/Next); a Stay snaps back with no callback. The
/// caller owns the pane index/count + the actual channel switch.
#[derive(Clone)]
pub struct StripDrag {
    #[cfg(feature = "hydrate")]
    idx: StoredValue<usize>,
    #[cfg(feature = "hydrate")]
    count: StoredValue<usize>,
    #[cfg(feature = "hydrate")]
    on_commit: Callback<StripCommit>,
    #[cfg(feature = "hydrate")]
    strip_ref: NodeRef<leptos::html::Div>,
    /// (start_x, start_y, start_t) at pointerdown, else None.
    #[cfg(feature = "hydrate")]
    start: RwSignal<Option<(f64, f64, f64)>>,
    /// Locked axis once past slop, else None.
    #[cfg(feature = "hydrate")]
    axis: RwSignal<Option<Axis>>,
}

#[cfg(feature = "hydrate")]
impl StripDrag {
    pub fn new(
        idx: StoredValue<usize>,
        count: StoredValue<usize>,
        on_commit: Callback<StripCommit>,
        strip_ref: NodeRef<leptos::html::Div>,
    ) -> Self {
        Self {
            idx,
            count,
            on_commit,
            strip_ref,
            start: RwSignal::new(None),
            axis: RwSignal::new(None),
        }
    }

    pub fn down(&self, ev: &leptos::ev::PointerEvent) {
        use leptos::wasm_bindgen::JsCast as _;
        if let Some(el) = self.strip_ref.get_untracked() {
            let el: &leptos::web_sys::Element = (*el).unchecked_ref();
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        self.start.set(Some((
            ev.client_x() as f64,
            ev.client_y() as f64,
            ev.time_stamp(),
        )));
        self.axis.set(None);
    }

    pub fn moved(&self, ev: &leptos::ev::PointerEvent) {
        let Some((sx, sy, _)) = self.start.get_untracked() else {
            return;
        };
        let dx = ev.client_x() as f64 - sx;
        let dy = ev.client_y() as f64 - sy;
        // Lock the axis once past slop; only track horizontal drags.
        let axis = self.axis.get_untracked().or_else(|| {
            let a = axis_lock(dx, dy);
            if a.is_some() {
                self.axis.set(a);
            }
            a
        });
        if axis != Some(Axis::Horizontal) {
            return;
        }
        // Horizontal: prevent the page from scrolling and track 1:1 + rubber-band.
        ev.prevent_default();
        let width = viewport_width();
        let offset = strip_offset(self.idx.get_value(), self.count.get_value(), width, dx);
        self.write_strip_x(offset);
    }

    pub fn up(&self, ev: &leptos::ev::PointerEvent) {
        let Some((sx, _, st)) = self.start.get_untracked() else {
            return;
        };
        self.start.set(None);
        let was_h = self.axis.get_untracked() == Some(Axis::Horizontal);
        self.axis.set(None);
        if !was_h {
            return;
        }
        let dx = ev.client_x() as f64 - sx;
        let dt = ev.time_stamp() - st;
        let width = viewport_width();
        let commit = commit_swipe(dx, dt, width);
        // Snap back to the resting offset for the (possibly new) index; the
        // caller's on_commit advances the channel, which re-renders the strip.
        self.write_strip_x(-(self.idx.get_value() as f64) * width);
        if commit != StripCommit::Stay {
            self.on_commit.run(commit);
        }
    }

    fn write_strip_x(&self, px: f64) {
        use leptos::wasm_bindgen::JsCast as _;
        if let Some(el) = self.strip_ref.get_untracked() {
            if let Some(html) = (*el).dyn_ref::<leptos::web_sys::HtmlElement>() {
                let _ = html.style().set_property("--strip-x", &format!("{px}px"));
            }
        }
    }
}

#[cfg(feature = "hydrate")]
fn viewport_width() -> f64 {
    leptos::web_sys::window()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(360.0)
}

/// ssr stubs: never run (pointer events are browser-only) but the `<div>`
/// bindings are always-on and must typecheck on the server.
#[cfg(not(feature = "hydrate"))]
impl StripDrag {
    pub fn down(&self, _ev: &leptos::ev::PointerEvent) {}
    pub fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
    pub fn up(&self, _ev: &leptos::ev::PointerEvent) {}
}
