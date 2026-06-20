//! M5/P0 #49 HoloPanel: one drag-summoned panel engine. Pointer drag maps to
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
/// of absolute progress (Concept C's threshold).
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
/// at 1.0). The card-deck skeleton's sheet uses two; single-detent panels pass one.
#[derive(Clone, Copy, Debug)]
pub struct Detent {
    pub at: f64,
    pub key: &'static str,
}

/// The mount-time `open` target: the fully-open detent's `at`. Detents are
/// sorted ascending, so the open one is last; an empty slice (defensive — a
/// HoloPanel always has ≥1 detent) falls back to fully-open `1.0`. Extracted
/// from the component's `on_load` so the selection is unit-testable without a
/// DOM (mirrors `nearest_detent`).
pub fn open_target_at(detents: &[Detent]) -> f64 {
    detents.last().map(|d| d.at).unwrap_or(1.0)
}

/// Where a TAP-release (sub-slop travel) leaves the panel's progress, by mode.
///
/// The pointer handlers are bound on the panel root and child controls do NOT
/// stop propagation, so a tap on a persona card / toggle bubbles a full
/// pointerdown→pointerup pair to the panel — `PanelDrag::up` sees `is_tap`.
///
/// - Legacy DRAG-summoned panels (`on_close == None`, so `open_at == None`
///   here): a tap "passes through" — the panel snaps to `--p=0` (the parent
///   toggle / off-screen rest), the historical behaviour.
/// - BUTTON-summoned / Modal-parity panels (`on_close` wired ⇒ `open_at =
///   Some(open detent)`): the panel RESTS open (`--p=1`); a tap on a child
///   control must NOT slide it off-screen (which would strand it mounted —
///   `on_close` only fires on Esc / swipe-to-close, never on a tap — behind an
///   invisible scrim). It re-asserts the open detent so the child click acts
///   normally and the station stays open for further toggles. Extracted as a
///   pure fn so the tap-vs-open decision is unit-testable without a DOM.
pub fn tap_release_progress(open_at: Option<f64>) -> f64 {
    open_at.unwrap_or(0.0)
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
    /// `(along, cross, time_ms)` at pointerdown — the coord along the open axis,
    /// the coord on the CROSS axis (feeds the scroll-vs-drag lock), and the start
    /// time. `None` between gestures.
    #[cfg(feature = "hydrate")]
    start: RwSignal<Option<(f64, f64, f64)>>,
    /// Axis lock for the in-flight gesture (the Modal swipe-close pattern, reused
    /// via `modal::swipe_close_lock`): `Some('h')` = a drag ALONG the open axis
    /// (drives `--p`, can close); `Some('v')` = a CROSS-axis scroll → leave `--p`
    /// untouched so the panel CONTENT scrolls and never dismisses (the
    /// station-vanishes-on-scroll fix); `None` until the first move past the slop.
    #[cfg(feature = "hydrate")]
    locked: RwSignal<Option<char>>,
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
        // Capture the pointer so a drag whose finger leaves the panel keeps
        // feeding the gesture — but NOT when the press starts on an interactive
        // control. On DESKTOP a captured pointer redirects the trailing `click`
        // to the capture target, so a tapped Station button's on:click never
        // fired (the M6 deck regression; touch dispatches the tap before the
        // capture override, so iOS was unaffected). Same trap sk_orbit/drag.rs
        // dodges for the composer. A tap never drags, so skipping it costs nothing;
        // a real drag from blank panel area still captures.
        let on_control = ev
            .target()
            .and_then(|t| t.dyn_into::<leptos::web_sys::Element>().ok())
            .and_then(|e| {
                e.closest("button, a[href], input, textarea, select, label, [role=\"button\"]")
                    .ok()
                    .flatten()
            })
            .is_some();
        if !on_control {
            if let Some(el) = self.panel_ref.get_untracked() {
                // Open Question #6: `set_pointer_capture` binding name confirmed at
                // execution (web-sys `Element::set_pointer_capture`, as in
                // lightbox.rs:530).
                let el: &leptos::web_sys::Element = (*el).unchecked_ref();
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
        self.start.set(Some((
            self.axis_coord(ev),
            self.cross_coord(ev),
            ev.time_stamp(),
        )));
        self.locked.set(None);
    }

    /// `pointermove`: lock to an axis on the first move past the slop, then map
    /// the signed open-axis delta into `--p` — but ONLY while locked to a drag
    /// along the open axis. A cross-axis scroll lock leaves `--p` alone so the
    /// panel content scrolls instead of the panel dismissing.
    fn moved(&self, ev: &leptos::ev::PointerEvent) {
        if let Some((start_along, start_cross, _)) = self.start.get_untracked() {
            let d_along = self.axis_coord(ev) - start_along;
            let d_cross = self.cross_coord(ev) - start_cross;
            // Scroll-vs-drag lock (the Modal swipe-close pattern, reused): pass
            // the open-axis delta as `dx` and the cross-axis delta as `dy`. `'h'`
            // ⇒ a drag along the open axis (drive the panel); `'v'` ⇒ a cross-axis
            // scroll (leave `--p` untouched — the station-vanishes-on-scroll fix).
            if self.locked.get_untracked().is_none() {
                self.locked
                    .set(crate::ui::modal::swipe_close_lock(d_along, d_cross));
            }
            if self.locked.get_untracked() == Some('h') {
                // Sign the delta toward "open" per edge (right/bottom open by
                // moving toward the negative direction; left/top by positive).
                let signed = match self.edge {
                    Edge::Left | Edge::Top => d_along,
                    Edge::Right | Edge::Bottom => -d_along,
                };
                self.progress
                    .set(progress_from_delta(signed, self.extent()));
            }
        }
    }

    /// `pointerup` / `pointercancel`: tap passes through; a real drag either
    /// commits (snap to nearest detent + fire `on_commit`) or snaps back to 0.
    fn up(&self, ev: &leptos::ev::PointerEvent) {
        let Some((start_along, _start_cross, start_t)) = self.start.get_untracked() else {
            return;
        };
        self.start.set(None);
        // A cross-axis scroll ('v') or an undecided gesture (None — a tap that
        // never passed the slop) is NOT a close drag: re-assert the open detent
        // (button-summoned) / pass through (legacy) and NEVER dismiss. This is
        // what stops the station vanishing when you scroll its content.
        if self.locked.get_untracked() != Some('h') {
            let open_at = self
                .on_close
                .map(|_| self.detents.with_value(|d| open_target_at(d)));
            self.progress.set(tap_release_progress(open_at));
            return;
        }
        let raw = self.axis_coord(ev) - start_along;
        // Tap: no commit. The pointer handlers live on the panel root and child
        // controls don't stop propagation, so a tap on a persona card / toggle
        // bubbles a full pointerdown→pointerup here. For a button-summoned panel
        // (on_close wired) the panel RESTS open, so a tap must RE-ASSERT the open
        // detent — zeroing progress would slide it off-screen yet leave it
        // mounted (on_close never fires on a tap), stranding the user behind an
        // invisible scrim. Legacy drag-summoned panels (no on_close) keep the old
        // "pass through to --p=0" behaviour. Decision is the pure
        // `tap_release_progress` (unit-tested without a DOM).
        if is_tap(raw.abs()) {
            let open_at = self
                .on_close
                .map(|_| self.detents.with_value(|d| open_target_at(d)));
            self.progress.set(tap_release_progress(open_at));
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

    /// The pointer coordinate on the CROSS axis (perpendicular to the open axis):
    /// vertical for a side panel, horizontal for top/bottom. Feeds the
    /// scroll-vs-drag lock so a content scroll doesn't read as a close drag.
    fn cross_coord(&self, ev: &leptos::ev::PointerEvent) -> f64 {
        match self.edge {
            Edge::Left | Edge::Right => ev.client_y() as f64,
            Edge::Top | Edge::Bottom => ev.client_x() as f64,
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
    /// Optional accessible name for the dialog (WCAG 4.1.2 / Modal-parity §13):
    /// bound to `aria-label` on the `role="dialog"` root so a screen reader
    /// announces a NAMED dialog, not a bare "dialog". `None` for legacy
    /// drag-summoned panels; explicit-affordance (Modal-parity) consumers name it.
    #[prop(optional, into)]
    label: Option<&'static str>,
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
    // open target. Computed (via the unit-tested `open_target_at`) before
    // `detents` moves into the gesture state.
    let detents_open_at = open_target_at(&detents);
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
        locked: RwSignal::new(None),
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
    // Modal scrim: a full-viewport backdrop sibling, rendered ONLY when the
    // parent wired `on_close` (the explicit-affordance / Modal-parity case —
    // legacy drag-summoned panels pass no on_close and stay scrimless as
    // before). It makes the slide-over a TRUE modal: `var(--scrim)` at z:59 (just
    // under the panel) blocks pointer/touch interaction with the chrome behind
    // it (pill, composer orb, swipe strip) — without it `aria-modal` is a lie,
    // the Tab-trap only contains KEYBOARD focus — and gives click-outside-to-
    // close via the same `on_close` (so it fires the parent's un-mount + focus-
    // restore, exactly like Esc/swipe-to-close). Opacity tracks --p through
    // `scrim_opacity` (0 closed → 0.85 open), so it fades in as the panel slides.
    let scrim = on_close.map(|cb| {
        view! {
            <button
                class="holopanel-scrim"
                aria-label="Close"
                tabindex="-1"
                style:opacity=move || scrim_opacity(progress.get()).to_string()
                on:click=move |_| cb.run(())
            ></button>
        }
    });
    view! {
        {scrim}
        <div
            node_ref=panel_ref
            class=format!("holopanel {edge_class}")
            class:holopanel--desktop-chrome=desktop_chrome
            role="dialog"
            aria-modal="true"
            aria-label=label
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
    fn tap_release_stays_open_for_button_summoned_panels() {
        // Legacy drag-summoned panel (no on_close): a tap passes through to the
        // off-screen rest (--p=0), the historical behaviour.
        assert_eq!(tap_release_progress(None), 0.0);
        // Button-summoned / Modal-parity panel (on_close wired ⇒ Some(open
        // detent)): a tap on a child control must re-assert the OPEN detent, not
        // zero progress (which would strand the panel mounted behind an invisible
        // scrim — on_close fires only on Esc / swipe-to-close, never on a tap).
        assert_eq!(tap_release_progress(Some(1.0)), 1.0);
        // A non-trivial open detent (e.g. a half-open sheet) is preserved too.
        assert_eq!(tap_release_progress(Some(0.5)), 0.5);
    }

    #[test]
    fn open_target_is_the_last_ascending_detent() {
        let detents = [Detent { at: 0.5, key: "d1" }, Detent { at: 1.0, key: "d2" }];
        // Mount-time `open` raises progress to the fully-open (last) detent.
        assert_eq!(open_target_at(&detents), 1.0);
        // Single-detent panel (orbit's case) opens to that one detent.
        let single = [Detent {
            at: 1.0,
            key: "open",
        }];
        assert_eq!(open_target_at(&single), 1.0);
        // Empty slice (defensive — a HoloPanel always has ≥1 detent) falls back
        // to fully-open 1.0, the only non-trivial branch the inline test missed.
        assert_eq!(open_target_at(&[]), 1.0);
    }
}
