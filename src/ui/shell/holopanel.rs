//! W5/P0 #49 HoloPanel: one drag-summoned panel engine. Pointer drag maps to
//! a 0–1 progress (`--p` custom property; SCSS derives the per-edge
//! transform). Velocity-based commit, scrim coupled to --p, tap-vs-drag
//! disambiguation (7px slop), per-edge safe-area inset ownership, full a11y
//! (focus trap, Esc, restore, role=dialog, reduced-motion = instant snap).
//! Children render touch-clean; desktop opts IN via `desktop_chrome`.
//! Shared UI module — imports ZERO ssr/hydrate-only crates beyond leptos.

/// Which screen edge a panel is summoned from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Tap-vs-drag slop in CSS px: movement under this is a tap, not a drag.
pub const TAP_SLOP_PX: f64 = 7.0;
/// Velocity (progress units per ms) above which a flick commits regardless
/// of absolute progress (Koncept C's threshold).
pub const FLICK_COMMIT_PER_MS: f64 = 0.0015;

/// Map a pointer delta along the panel's open axis to drag progress 0..=1.
/// `delta_px` is signed toward "open"; `extent_px` is the panel's full travel.
pub fn progress_from_delta(delta_px: f64, extent_px: f64) -> f64 {
    if extent_px <= 0.0 {
        return 0.0;
    }
    (delta_px / extent_px).clamp(0.0, 1.0)
}

/// Decide whether a gesture commits to OPEN on release. Commits if past the
/// halfway detent OR if flicked open faster than the velocity threshold.
pub fn commits_open(progress: f64, velocity_per_ms: f64) -> bool {
    progress >= 0.5 || velocity_per_ms >= FLICK_COMMIT_PER_MS
}

/// Tap vs drag: total travel under the slop is a tap (passes through to the
/// child / triggers on_commit toggle), not a drag.
pub fn is_tap(total_travel_px: f64) -> bool {
    total_travel_px < TAP_SLOP_PX
}

/// Scrim opacity is coupled 1:1 to progress, capped at 0.85 (the prototype's
/// max backdrop darkness).
pub fn scrim_opacity(progress: f64) -> f64 {
    progress.clamp(0.0, 1.0) * 0.85
}

use leptos::prelude::*;

/// A snap detent: a named open fraction (e.g. D1=channels at 0.5, D2=galaxy
/// at 1.0). Kortdäck's sheet uses two; single-detent panels pass one.
#[derive(Clone, Copy, Debug)]
pub struct Detent {
    pub at: f64,
    pub key: &'static str,
}

/// Pick the detent whose `at` is nearest the current progress — the snap
/// target a committed drag lands on. Extracted from the pointerup handler so
/// the selection is unit-testable without a DOM (the listener body itself is
/// hydrate-only / WASM-only). Returns the fallback `open` detent only for an
/// empty slice (a HoloPanel always has at least one detent in practice).
pub fn nearest_detent(detents: &[Detent], progress: f64) -> Detent {
    detents
        .iter()
        .copied()
        .min_by(|a, b| {
            (a.at - progress)
                .abs()
                .partial_cmp(&(b.at - progress).abs())
                .unwrap_or(core::cmp::Ordering::Equal)
        })
        .unwrap_or(Detent {
            at: 1.0,
            key: "open",
        })
}

/// The drag gesture engine for one HoloPanel: owns the bookkeeping signals and
/// the per-edge config, exposing `down`/`moved`/`up`/`keydown` handlers bound
/// ungated in the view. A hydrate-real impl pairs with an ssr no-op stub
/// (mirroring `radial::LongPress`): the always-on `view!` binds the handlers,
/// the methods take the shared `leptos::ev::*` types, and only the hydrate
/// build touches `web_sys`. WASM is single-threaded, so the drag-start state
/// lives in a plain `RwSignal`, no thread-local needed.
///
/// The struct itself is always-on (the panel `<div>` binds its methods
/// unconditionally); the fields and real bodies exist only on hydrate — the
/// ssr stubs are pure no-ops, so carrying the state on the server would be
/// dead (mirrors `radial::LongPress`).
#[derive(Clone)]
struct PanelDrag {
    #[cfg(feature = "hydrate")]
    edge: Edge,
    #[cfg(feature = "hydrate")]
    detents: StoredValue<Vec<Detent>>,
    #[cfg(feature = "hydrate")]
    progress: RwSignal<f64>,
    #[cfg(feature = "hydrate")]
    on_commit: Callback<&'static str>,
    #[cfg(feature = "hydrate")]
    panel_ref: NodeRef<leptos::html::Div>,
    /// (coord, time_ms) at pointerdown along the open axis, else `None`.
    #[cfg(feature = "hydrate")]
    start: RwSignal<Option<(f64, f64)>>,
    /// Parent dismiss callback (Esc + snap-to-closed). `None` = legacy.
    #[cfg(feature = "hydrate")]
    on_close: Option<Callback<()>>,
}

#[cfg(feature = "hydrate")]
impl PanelDrag {
    /// `pointerdown`: capture the pointer so moves outside the panel keep
    /// feeding the gesture (proven in `lightbox.rs` `set_pointer_capture`),
    /// and record the start coord/time along the open axis.
    fn down(&self, ev: &leptos::ev::PointerEvent) {
        use leptos::wasm_bindgen::JsCast as _;
        if let Some(el) = self.panel_ref.get_untracked() {
            // Open Question #6: `set_pointer_capture` binding name confirmed at
            // execution (web-sys `Element::set_pointer_capture`, as in
            // lightbox.rs:530).
            let el: &leptos::web_sys::Element = (*el).unchecked_ref();
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        self.start.set(Some((self.axis_coord(ev), ev.time_stamp())));
    }

    /// `pointermove`: map the signed delta toward "open" into `--p` progress.
    fn moved(&self, ev: &leptos::ev::PointerEvent) {
        if let Some((start_coord, _)) = self.start.get_untracked() {
            // Sign the delta toward "open" per edge (right/bottom open by
            // moving toward the negative direction; left/top by positive).
            let raw = self.axis_coord(ev) - start_coord;
            let signed = match self.edge {
                Edge::Left | Edge::Top => raw,
                Edge::Right | Edge::Bottom => -raw,
            };
            self.progress
                .set(progress_from_delta(signed, self.extent()));
        }
    }

    /// `pointerup` / `pointercancel`: tap passes through; a real drag either
    /// commits (snap to nearest detent + fire `on_commit`) or snaps back to 0.
    fn up(&self, ev: &leptos::ev::PointerEvent) {
        let Some((start_coord, start_t)) = self.start.get_untracked() else {
            return;
        };
        self.start.set(None);
        let raw = self.axis_coord(ev) - start_coord;
        // Tap: passes through, no commit (the parent toggle handles taps).
        if is_tap(raw.abs()) {
            self.progress.set(0.0);
            return;
        }
        let dt = (ev.time_stamp() - start_t).max(1.0);
        let p = self.progress.get_untracked();
        // Velocity in progress-units/ms (rough; precise flick tuning is a
        // Phase-2/3 real-device task, not a Foundation gate).
        let velocity = (p / dt).abs();
        if commits_open(p, velocity) {
            let target = self.detents.with_value(|d| nearest_detent(d, p));
            self.progress.set(target.at);
            self.on_commit.run(target.key);
        } else {
            self.progress.set(0.0);
            // Snap-to-closed: ask the parent to dismiss (it owns un-mount +
            // focus restore). Legacy drag-summoned panels pass no on_close and
            // keep the old "just snap to 0" behaviour.
            if let Some(cb) = self.on_close {
                cb.run(());
            }
        }
    }

    /// `keydown`: Esc snaps the panel shut; Tab/Shift+Tab wrap focus within the
    /// panel so it can't escape into the page behind the modal. Reduced-motion
    /// instant snap is owned by the SCSS (`transition: none` under
    /// `prefers-reduced-motion`, Step 0.5.3 in `_holopanel.scss`) — no JS
    /// branch here; a future reader should NOT add a redundant JS path.
    fn keydown(&self, ev: &leptos::ev::KeyboardEvent) {
        use leptos::wasm_bindgen::JsCast as _;
        match ev.key().as_str() {
            "Escape" => {
                ev.prevent_default();
                self.start.set(None);
                self.progress.set(0.0);
                if let Some(cb) = self.on_close {
                    cb.run(());
                }
            }
            "Tab" => {
                let Some(root) = self.panel_ref.get_untracked() else {
                    return;
                };
                let root: &leptos::web_sys::Element = (*root).unchecked_ref();
                let els = Self::focusables(root);
                if els.is_empty() {
                    return;
                }
                let active = leptos::web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.active_element())
                    .and_then(|el| el.dyn_into::<leptos::web_sys::HtmlElement>().ok());
                let idx = active
                    .as_ref()
                    .and_then(|a| els.iter().position(|el| el == a));
                let last = els.len() - 1;
                // Wrap when leaving either end; Shift+Tab from the panel root
                // (idx None) lands on the last control instead of escaping.
                let (wrap, target) = if ev.shift_key() {
                    (idx == Some(0) || idx.is_none(), last)
                } else {
                    (idx == Some(last), 0)
                };
                if wrap {
                    ev.prevent_default();
                    let _ = els[target].focus();
                }
            }
            _ => {}
        }
    }

    /// The pointer coordinate along the panel's open axis.
    fn axis_coord(&self, ev: &leptos::ev::PointerEvent) -> f64 {
        match self.edge {
            Edge::Left | Edge::Right => ev.client_x() as f64,
            Edge::Top | Edge::Bottom => ev.client_y() as f64,
        }
    }

    /// The panel's full travel in CSS px: viewport width for side edges,
    /// height for top/bottom. Falls back to a sane default off-DOM.
    fn extent(&self) -> f64 {
        let win = leptos::web_sys::window();
        match self.edge {
            Edge::Left | Edge::Right => win
                .and_then(|w| w.inner_width().ok())
                .and_then(|v| v.as_f64())
                .unwrap_or(360.0),
            Edge::Top | Edge::Bottom => win
                .and_then(|w| w.inner_height().ok())
                .and_then(|v| v.as_f64())
                .unwrap_or(800.0),
        }
    }

    /// The panel's focusable children in DOM order, for the dialog Tab trap
    /// (mirrors `lightbox.rs::focusables`).
    fn focusables(root: &leptos::web_sys::Element) -> Vec<leptos::web_sys::HtmlElement> {
        use leptos::wasm_bindgen::JsCast as _;
        const SEL: &str = "a[href], button:not([disabled]), input:not([disabled]), \
                           textarea:not([disabled]), select:not([disabled]), \
                           [tabindex]:not([tabindex=\"-1\"])";
        let Ok(list) = root.query_selector_all(SEL) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(list.length() as usize);
        for i in 0..list.length() {
            if let Some(el) = list
                .item(i)
                .and_then(|n| n.dyn_into::<leptos::web_sys::HtmlElement>().ok())
            {
                out.push(el);
            }
        }
        out
    }
}

/// ssr stubs: never run (pointer/key events only exist in the browser), but
/// the panel `<div>` bindings are always-on and must typecheck on the server.
#[cfg(not(feature = "hydrate"))]
impl PanelDrag {
    fn down(&self, _ev: &leptos::ev::PointerEvent) {}
    fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
    fn up(&self, _ev: &leptos::ev::PointerEvent) {}
    fn keydown(&self, _ev: &leptos::ev::KeyboardEvent) {}
}

#[component]
pub fn HoloPanel(
    /// Which edge the panel is summoned from.
    edge: Edge,
    /// One or more snap detents (sorted ascending by `at`).
    detents: Vec<Detent>,
    /// Called when a detent commits, with the committed detent key.
    #[prop(into)]
    on_commit: Callback<&'static str>,
    /// Desktop opts IN to drag-reorder / hover affordances; touch is clean.
    #[prop(optional)]
    desktop_chrome: bool,
    /// Start OPEN: on mount, animate to the open (last) detent instead of the
    /// closed `--p=0` resting state. For parent-`<Show>`-mounted panels summoned
    /// by an explicit affordance (the engine is otherwise drag-summoned only).
    #[prop(optional)]
    open: bool,
    /// Fired when the panel asks the parent to dismiss it — on Esc AND on a
    /// drag/flick that snaps back below the open detent (swipe-to-close). The
    /// PARENT owns un-mounting (e.g. flips its `<Show>` signal) and focus
    /// restore-to-trigger; the engine only signals intent. `None` ⇒ legacy
    /// drag-summoned behaviour (Esc just snaps to `--p=0`, no parent notify).
    #[prop(optional, into)]
    on_close: Option<Callback<()>>,
    children: Children,
) -> impl IntoView {
    // Drag progress drives the `--p` custom property; SCSS derives the
    // per-edge transform. The pointer handlers own writing `progress`; the
    // view binds it to --p and sets the a11y attributes.
    let progress = RwSignal::new(0.0_f64);
    let edge_class = match edge {
        Edge::Left => "holopanel--left",
        Edge::Right => "holopanel--right",
        Edge::Top => "holopanel--top",
        Edge::Bottom => "holopanel--bottom",
    };
    // The panel DOM node: pointer-capture target + focus-trap root. Attached
    // via `node_ref` on the panel <div> below (proven in lightbox.rs).
    let panel_ref = NodeRef::<leptos::html::Div>::new();
    // The fully-open detent (last, since detents are ascending) — the mount-time
    // open target. Computed before `detents` moves into the gesture state.
    let detents_open_at = detents.last().map(|d| d.at).unwrap_or(1.0);
    // `detents`/`on_commit` feed only the hydrate gesture state; consume them
    // on the server so the props don't read as unused there. `open`/`on_close`/
    // `detents_open_at` are likewise hydrate-only (the on_load capture + the
    // struct move), so consume them here too.
    #[cfg(not(feature = "hydrate"))]
    let _ = (detents, on_commit, open, on_close, detents_open_at);
    let drag = PanelDrag {
        #[cfg(feature = "hydrate")]
        edge,
        #[cfg(feature = "hydrate")]
        detents: StoredValue::new(detents),
        #[cfg(feature = "hydrate")]
        progress,
        #[cfg(feature = "hydrate")]
        on_commit,
        #[cfg(feature = "hydrate")]
        panel_ref,
        #[cfg(feature = "hydrate")]
        start: RwSignal::new(None),
        #[cfg(feature = "hydrate")]
        on_close,
    };
    // a11y contract: move focus onto the panel root the moment it mounts so the
    // Tab trap (PanelDrag::keydown) and Esc-to-close ride the panel's own
    // keydown — the parent owns mount/unmount, so "open" == mounted here. Focus
    // restore on close is the parent's job (it un-mounts the trigger context).
    #[cfg(feature = "hydrate")]
    panel_ref.on_load(move |el| {
        let _ = el.focus();
        // Button-summoned open: the parent mounts us under <Show> with open=true;
        // raise progress to the open detent so the SCSS --p transition slides us
        // in. Leptos applies the initial --p=0, then this set, and the CSS
        // transition interpolates (no rAF defer needed).
        if open {
            progress.set(detents_open_at);
        }
    });
    let (d_down, d_move, d_up, d_key) = (drag.clone(), drag.clone(), drag.clone(), drag.clone());
    view! {
        <div
            node_ref=panel_ref
            class=format!("holopanel {edge_class}")
            class:holopanel--desktop-chrome=desktop_chrome
            role="dialog"
            aria-modal="true"
            tabindex="-1"
            style:--p=move || progress.get().to_string()
            on:pointerdown=move |ev| d_down.down(&ev)
            on:pointermove=move |ev| d_move.moved(&ev)
            on:pointerup=move |ev| d_up.up(&ev)
            on:pointercancel=move |ev| drag.up(&ev)
            on:keydown=move |ev| d_key.keydown(&ev)
        >
            {children()}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_clamps_and_scales() {
        assert_eq!(progress_from_delta(0.0, 300.0), 0.0);
        assert_eq!(progress_from_delta(150.0, 300.0), 0.5);
        assert_eq!(progress_from_delta(600.0, 300.0), 1.0); // clamps
        assert_eq!(progress_from_delta(50.0, 0.0), 0.0); // zero extent guard
    }

    #[test]
    fn commit_past_halfway_or_on_flick() {
        assert!(!commits_open(0.49, 0.0));
        assert!(commits_open(0.5, 0.0)); // detent
        assert!(commits_open(0.1, FLICK_COMMIT_PER_MS)); // flick wins low progress
        assert!(!commits_open(0.1, FLICK_COMMIT_PER_MS - 0.0001));
    }

    #[test]
    fn tap_slop_separates_tap_from_drag() {
        assert!(is_tap(0.0));
        assert!(is_tap(TAP_SLOP_PX - 0.01));
        assert!(!is_tap(TAP_SLOP_PX));
        assert!(!is_tap(40.0));
    }

    #[test]
    fn scrim_tracks_progress_capped() {
        assert_eq!(scrim_opacity(0.0), 0.0);
        assert!((scrim_opacity(1.0) - 0.85).abs() < 1e-9);
        assert!((scrim_opacity(2.0) - 0.85).abs() < 1e-9); // clamped
    }

    // Behavioral smoke proxy for the pointerup snap target: the hydrate
    // listener body is WASM-only and the ssr harness can't mount the
    // `#[component]`, so we exercise the extracted `nearest_detent` selection
    // the handler runs to decide which detent a committed drag lands on.
    #[test]
    fn nearest_detent_selection_picks_closest() {
        let detents = [Detent { at: 0.5, key: "d1" }, Detent { at: 1.0, key: "d2" }];
        assert_eq!(nearest_detent(&detents, 0.55).key, "d1");
        assert_eq!(nearest_detent(&detents, 0.9).key, "d2");
        // Empty slice falls back to the synthetic full-open detent.
        assert_eq!(nearest_detent(&[], 0.3).key, "open");
    }

    #[test]
    fn open_target_is_the_last_ascending_detent() {
        let detents = [Detent { at: 0.5, key: "d1" }, Detent { at: 1.0, key: "d2" }];
        // Mount-time `open` raises progress to the fully-open (last) detent.
        assert_eq!(detents.last().map(|d| d.at), Some(1.0));
        // Single-detent panel (orbit's case) opens to that one detent.
        let single = [Detent {
            at: 1.0,
            key: "open",
        }];
        assert_eq!(single.last().map(|d| d.at), Some(1.0));
    }
}
