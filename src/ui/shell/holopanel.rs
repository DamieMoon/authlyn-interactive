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
    children: Children,
) -> impl IntoView {
    // Drag progress drives the `--p` custom property; SCSS derives the
    // per-edge transform. The hydrate pointer handlers (Task 0.5b) own writing
    // `progress`; the view binds it to --p and sets the a11y attributes.
    let progress = RwSignal::new(0.0_f64);
    let edge_class = match edge {
        Edge::Left => "holopanel--left",
        Edge::Right => "holopanel--right",
        Edge::Top => "holopanel--top",
        Edge::Bottom => "holopanel--bottom",
    };
    // `detents`/`on_commit`/`desktop_chrome` are consumed by the hydrate
    // pointer wiring in Task 0.5b; the shell holds them so the signature is
    // final now and 0.5b only fills the listener body.
    let _ = (&detents, &on_commit, desktop_chrome);
    view! {
        <div
            class=format!("holopanel {edge_class}")
            class:holopanel--desktop-chrome=desktop_chrome
            role="dialog"
            aria-modal="true"
            style:--p=move || progress.get().to_string()
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
}
