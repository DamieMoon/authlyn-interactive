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
    /// True when this gesture's pointerdown landed on a message row — a
    /// small-radius rightward drag then belongs to swipe-to-reply, not a
    /// channel switch (the #14/#5 arbitration; see `strip::row_swipe_wins`).
    #[cfg(feature = "hydrate")]
    started_on_row: RwSignal<bool>,
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
            started_on_row: RwSignal::new(false),
        }
    }

    pub fn down(&self, ev: &leptos::ev::PointerEvent) {
        use leptos::wasm_bindgen::JsCast as _;
        if let Some(el) = self.strip_ref.get_untracked() {
            let el: &leptos::web_sys::Element = (*el).unchecked_ref();
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        // Did this press start on a message row? If so a small-radius rightward
        // drag is a swipe-to-reply, not a channel switch (the #14/#5 arbitration).
        // EXCLUDE system rows: they offer no actions (`message_actions` →
        // reply:false) so they have no reply target, and the radial deliberately
        // skips them too (channel/radial.rs `contains("system")` early-return).
        // Keeping the two row-detection rules consistent makes the strip own a
        // swipe started on a system row — a channel switch, the sensible fallback
        // when there is nothing to reply to (otherwise it would be a dead-zone).
        let on_row = {
            ev.target()
                .and_then(|t| t.dyn_into::<leptos::web_sys::Element>().ok())
                .and_then(|e| e.closest("li[id^='msg-']").ok().flatten())
                .filter(|li| !li.class_list().contains("system"))
                .is_some()
        };
        self.started_on_row.set(on_row);
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
        // #14/#5 arbitration: a small-radius rightward drag that STARTED on a
        // message row is a swipe-to-reply, not a channel switch — bail before any
        // axis lock or offset write so the strip never claims it. The strip only
        // ever wins once the horizontal travel grows past REPLY_POP_PX*1.5.
        if super::strip::row_swipe_wins(self.started_on_row.get_untracked(), dx) {
            // BUT once the finger has drifted past the radial's slop this is a
            // flick/drag, not a stationary hold: the radial's OWN <ul>
            // slop-disarm can't see it (set_pointer_capture in `down` stole the
            // stream), so its armed 450ms timer would fire ~450ms after release
            // and pop a phantom menu with no finger down. Disarm it here once —
            // gated on the drift so a genuine HOLD still blossoms (gesture 4 of
            // the trace) while a row-flick/short-drag cancels it. Idempotent
            // (generation bump), so calling it on each over-slop move is safe.
            if super::strip::moved_past_radial_slop(dx, dy) {
                crate::ui::shell::channel::disarm_radial();
            }
            // Let the row's own swipe-to-reply (Phase-7 follow-up) handle the
            // sub-slop hold; don't move the strip.
            return;
        }
        // Lock the axis once past slop; only track horizontal drags.
        let axis = self.axis.get_untracked().or_else(|| {
            let a = axis_lock(dx, dy);
            if a.is_some() {
                self.axis.set(a);
                // Horizontal lock: StripDrag owns the gesture and (via
                // set_pointer_capture in `down`) will steal the pointer stream
                // from the radial's <ul> listeners, so the radial's own
                // pointermove/up disarm can't fire. Bump the radial generation +
                // close any open menu so a press armed on `down` never blossoms
                // mid-swipe (open_channel_at also disarms on commit, but that's
                // too late for the in-flight drag). pub(super) — reachable from
                // this sibling module under `shell`. Runs ONCE per gesture: only
                // in this or_else arm, which fires only on the transition INTO a
                // lock (subsequent moves take the get_untracked() short-circuit).
                if a == Some(Axis::Horizontal) {
                    crate::ui::shell::channel::disarm_radial();
                }
            }
            a
        });
        if axis != Some(Axis::Horizontal) {
            return;
        }
        // Horizontal: prevent the page from scrolling and track 1:1 + rubber-band.
        ev.prevent_default();
        // Kill the settle transition so the strip tracks the finger 1:1 — a CSS
        // transition would smooth/lag the per-move writes (mirrors the prototype
        // toggling `.snap` OFF during the drag). Re-added in `up()` for the
        // commit settle.
        self.set_dragging(true);
        let width = viewport_width();
        // The live ChannelPane is the MIDDLE of the fixed 3-slot strip, so the
        // resting base is -width; idx/count only decide whether THIS edge can
        // rubber-band (no prev at the first channel, no next at the last).
        let (at_first, at_last) = self.edges();
        let offset = strip_offset(at_first, at_last, width, dx);
        self.write_strip_x(offset);
    }

    pub fn up(&self, ev: &leptos::ev::PointerEvent) {
        let Some((sx, sy, st)) = self.start.get_untracked() else {
            return;
        };
        self.start.set(None);
        let was_h = self.axis.get_untracked() == Some(Axis::Horizontal);
        let on_row = self.started_on_row.get_untracked();
        self.axis.set(None);
        // Reset the per-gesture row flag so the next press starts clean.
        self.started_on_row.set(false);
        let dx = ev.client_x() as f64 - sx;
        let dy = ev.client_y() as f64 - sy;
        if !was_h {
            // Terminal path for every gesture the strip did NOT claim
            // horizontally — including pointercancel (bound to `up`), which the
            // browser fires when it takes over for a vertical scroll started on
            // a row, and a short row-flick release that never locked an axis.
            // For all of these `set_pointer_capture` (in `down`) redirected the
            // pointer stream to the strip div, so the radial's own
            // pointerup/pointercancel disarm on the `.messages <ul>` never ran
            // and its armed 450ms timer would still pop a phantom menu after
            // release. Disarm it once IFF the finger actually moved past the
            // radial's slop — a stationary hold (gesture 4) is preserved so the
            // long-press still blossoms normally.
            if super::strip::moved_past_radial_slop(dx, dy) {
                crate::ui::shell::channel::disarm_radial();
            }
            return;
        }
        // #14/#5 arbitration (release side): if the finger drifted back into the
        // small-radius rightward band, the gesture is a swipe-to-reply, not a
        // channel switch — snap the strip home without committing. (The strip
        // could only reach here by first locking horizontal past REPLY_POP_PX*1.5,
        // so this only fires on a finger that grew then shrank back.)
        if super::strip::row_swipe_wins(on_row, dx) {
            self.set_dragging(false);
            self.write_strip_x(-viewport_width());
            return;
        }
        let dt = ev.time_stamp() - st;
        let width = viewport_width();
        let commit = commit_swipe(dx, dt, width);
        // Re-enable the settle transition for the snap, then snap back to the
        // middle slot (-width). The middle slot is ALWAYS the live pane, so the
        // resting offset is -width regardless of the channel's list index; a
        // committed swipe's on_commit swaps the middle pane's channel in place
        // (it does NOT shift the strip), so the strip stays centered.
        self.set_dragging(false);
        self.write_strip_x(-width);
        if commit != StripCommit::Stay {
            self.on_commit.run(commit);
        }
    }

    /// The TRUE channel-list edges (no prev / no next neighbor), derived from
    /// the live index/count. These gate the rubber-band ONLY; they never feed
    /// the resting base (which is the fixed middle slot, `-width`).
    fn edges(&self) -> (bool, bool) {
        let idx = self.idx.get_value();
        let count = self.count.get_value();
        let at_first = idx == 0;
        let at_last = count == 0 || idx + 1 >= count;
        (at_first, at_last)
    }

    /// Toggle the `--snap` transition class. While dragging it is REMOVED so the
    /// strip tracks the finger 1:1 (a permanent transition would smooth/lag the
    /// per-move `--strip-x` writes); it is re-added for the commit/snap-back
    /// settle. Mirrors the prototype's `.snap` toggle (a-orbit.html).
    fn set_dragging(&self, dragging: bool) {
        if let Some(el) = self.strip_ref.get_untracked() {
            let list = (*el).class_list();
            if dragging {
                let _ = list.remove_1(SNAP_CLASS);
            } else {
                let _ = list.add_1(SNAP_CLASS);
            }
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

/// The class carrying the `transform` settle transition. Present at rest and for
/// the commit snap-back; removed mid-drag so the finger-tracking writes are 1:1.
#[cfg(feature = "hydrate")]
const SNAP_CLASS: &str = "sk-orbit-strip--snap";

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
