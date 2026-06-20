//! M5/P2 effect-blossom long-hold detector. Mirrors `radial::LongPress`: an
//! always-on struct with hydrate-only fields, a generation-counter timer
//! (480ms), move-slop disarm (so a jittery thumb tapping Send never blossoms),
//! and an ssr no-op stub. On fire it opens the effect blossom + fires a Tick
//! haptic; the trailing click is guarded so it doesn't dismiss the blossom.

// Prelude items (`StoredValue`/`RwSignal`) are only touched on hydrate (the
// fields + real impl); the ssr stub uses fully-qualified `leptos::ev` types and
// `new()` builds an empty struct, so the import is hydrate-only to stay warning-
// clean under `clippy --features ssr -D warnings`.
#[cfg(feature = "hydrate")]
use leptos::prelude::*;

/// Long-hold move slop (px): a press that drifts past this disarms (it's a drag,
/// not a hold) — the same discipline as the radial, load-bearing for #47.
/// Hydrate-only (used solely by the timer/pointer plumbing), mirroring
/// `radial::MOVE_SLOP_PX`'s gating so it isn't dead-code on the ssr graph.
#[cfg(feature = "hydrate")]
const HOLD_SLOP_PX: f64 = 10.0;
/// Long-hold duration (ms) before the blossom opens. INTENTIONALLY distinct from
/// (and slightly longer than) the radial's `LONG_PRESS_MS = 450`
/// (`channel/radial.rs`) — these are two separate detectors, and the orb is a
/// SEND affordance, so a longer hold reduces an accidental blossom on a
/// deliberate send tap (the #47 mis-send concern). NOT a typo to "fix" to 450;
/// the difference is the design. Hydrate-only like the radial's const.
#[cfg(feature = "hydrate")]
const HOLD_MS: u32 = 480;

#[derive(Clone, Copy)]
pub(super) struct BlossomHold {
    #[cfg(feature = "hydrate")]
    gen: StoredValue<u64>,
    #[cfg(feature = "hydrate")]
    origin: StoredValue<Option<(f64, f64)>>,
    /// Set true when the hold fires; the click handler reads + clears it to
    /// guard the trailing click (so the hold doesn't also send).
    #[cfg(feature = "hydrate")]
    fired: StoredValue<bool>,
}

impl BlossomHold {
    pub(super) fn new() -> Self {
        Self {
            #[cfg(feature = "hydrate")]
            gen: StoredValue::new(0),
            #[cfg(feature = "hydrate")]
            origin: StoredValue::new(None),
            #[cfg(feature = "hydrate")]
            fired: StoredValue::new(false),
        }
    }
}

#[cfg(feature = "hydrate")]
impl BlossomHold {
    /// pointerdown: arm the hold; on expiry (if not disarmed) open the blossom.
    pub(super) fn down(
        &self,
        ev: &leptos::ev::PointerEvent,
        open: RwSignal<bool>,
        orb: leptos::web_sys::Element,
    ) {
        use leptos::task::spawn_local;
        self.origin
            .set_value(Some((ev.client_x() as f64, ev.client_y() as f64)));
        self.fired.set_value(false);
        let g = self.gen.get_value() + 1;
        self.gen.set_value(g);
        let gen = self.gen;
        let fired = self.fired;
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(HOLD_MS).await;
            if gen.try_get_value() == Some(g) {
                fired.set_value(true);
                open.set(true);
                crate::ui::shell::act::haptics::vh(&orb, crate::ui::shell::act::haptics::Vh::Tick);
            }
        });
    }

    /// pointermove: disarm if the press drifts past the slop (a drag, not a hold).
    pub(super) fn moved(&self, ev: &leptos::ev::PointerEvent) {
        if let Some((ox, oy)) = self.origin.get_value() {
            let dx = ev.client_x() as f64 - ox;
            let dy = ev.client_y() as f64 - oy;
            if (dx * dx + dy * dy).sqrt() > HOLD_SLOP_PX {
                self.cancel();
            }
        }
    }

    /// pointerup/cancel: disarm the pending timer (a completed/aborted press).
    pub(super) fn cancel(&self) {
        self.gen.set_value(self.gen.get_value() + 1);
        self.origin.set_value(None);
    }

    /// True if the hold fired (consume to guard the trailing click).
    pub(super) fn take_fired(&self) -> bool {
        let f = self.fired.get_value();
        self.fired.set_value(false);
        f
    }
}

/// ssr stubs: never run; the orb bindings are always-on and must typecheck.
#[cfg(not(feature = "hydrate"))]
impl BlossomHold {
    pub(super) fn moved(&self, _ev: &leptos::ev::PointerEvent) {}
    pub(super) fn cancel(&self) {}
    pub(super) fn take_fired(&self) -> bool {
        false
    }
}
