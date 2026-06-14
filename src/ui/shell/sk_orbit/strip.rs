//! W5/P2 horizontal swipe-strip physics (Omloppsbana's signature gesture) +
//! the swipe-to-reply axis arbitration. ALL decisions are pure fns so the
//! WASM-only pointer handlers stay thin and the logic is unit-tested (the
//! project has no WASM UI harness). Constants are the prototype's verified
//! values (`a-orbit.html`). No DOM.

/// Axis-lock commit slop: a gesture is uncommitted until it leaves this radius.
pub const AXIS_SLOP_PX: f64 = 10.0;
/// Horizontal dominance ratio: |dx| must beat |dy| by this factor to lock H.
pub const H_DOMINANCE: f64 = 1.15;
/// First/last-channel rubber-band resistance factor.
pub const RUBBER_BAND: f64 = 0.32;
/// Commit-on-release displacement fraction of the pane width.
pub const COMMIT_FRACTION: f64 = 0.32;
/// Commit-on-release velocity (px/ms) regardless of displacement.
pub const COMMIT_VELOCITY_PER_MS: f64 = 0.45;
/// Swipe-to-reply glyph "pop" threshold (px of row displacement).
pub const REPLY_POP_PX: f64 = 64.0;

/// The gesture's axis after the pointer leaves the slop radius. `None` until
/// committed. Horizontal wins only when dx dominates dy by `H_DOMINANCE`;
/// otherwise a vertical move past the slop locks V (a scroll). This is the
/// strip-vs-scroll arbitration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Decide the locked axis from the running (dx, dy). `None` = not yet past slop.
pub fn axis_lock(dx: f64, dy: f64) -> Option<Axis> {
    if dx.abs() > AXIS_SLOP_PX && dx.abs() > dy.abs() * H_DOMINANCE {
        Some(Axis::Horizontal)
    } else if dy.abs() > AXIS_SLOP_PX {
        Some(Axis::Vertical)
    } else {
        None
    }
}

/// Swipe-to-reply wins over the channel strip ONLY when the press started on a
/// message row AND the horizontal travel is still small-radius (a short
/// right-drag on a row), per the #14/#5 arbitration rule. A large-radius
/// horizontal drag is a channel switch even if it began on a row.
pub fn row_swipe_wins(started_on_row: bool, dx: f64) -> bool {
    started_on_row && dx > 0.0 && dx < REPLY_POP_PX * 1.5
}

/// The reply glyph "pops" (and a haptic tick fires) at/after the pop threshold.
pub fn reply_armed(dx: f64) -> bool {
    dx >= REPLY_POP_PX
}

/// The strip's live `translateX` (px) while dragging pane index `idx` of
/// `count` panes in a viewport `width` wide, finger delta `dx`. Edges
/// rubber-band: a drag past the first/last pane is damped by `RUBBER_BAND`.
pub fn strip_offset(idx: usize, count: usize, width: f64, dx: f64) -> f64 {
    let base = -(idx as f64) * width;
    let at_first = idx == 0;
    let at_last = count == 0 || idx + 1 >= count;
    // Dragging right (dx>0) at the first pane, or left (dx<0) at the last, has
    // no neighbor — damp it.
    let extra = if (dx > 0.0 && at_first) || (dx < 0.0 && at_last) {
        dx * RUBBER_BAND
    } else {
        dx
    };
    base + extra
}

/// The committed strip move on pointer release.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StripCommit {
    Prev,
    Next,
    Stay,
}

/// Decide the commit from the release delta `dx`, elapsed `dt_ms`, and pane
/// `width`. Commits when |dx| ≥ `COMMIT_FRACTION`·width OR |velocity| >
/// `COMMIT_VELOCITY_PER_MS`. `dx<0` ⇒ Next (revealed the right neighbor),
/// `dx>0` ⇒ Prev. Edge guards (no prev at first / no next at last) are the
/// caller's job (it knows the neighbor exists); this returns the intent.
pub fn commit_swipe(dx: f64, dt_ms: f64, width: f64) -> StripCommit {
    let dt = dt_ms.max(1.0);
    let velocity = (dx / dt).abs();
    let past_displacement = dx.abs() >= COMMIT_FRACTION * width;
    if !past_displacement && velocity <= COMMIT_VELOCITY_PER_MS {
        return StripCommit::Stay;
    }
    if dx < 0.0 {
        StripCommit::Next
    } else if dx > 0.0 {
        StripCommit::Prev
    } else {
        StripCommit::Stay
    }
}

/// The destination channel INDEX for a committed strip swipe, given the current
/// index and channel count — the picker→switch decision the WASM handler runs
/// (extracted so it's unit-testable without a DOM; the project has no WASM UI
/// harness). `Next` ⇒ `cur+1` if it exists, `Prev` ⇒ `cur-1` if it exists,
/// `Stay`/edge ⇒ `None` (no switch). This is the SAME mapping `on_strip_commit`
/// (Task 5.2.1) and the orbit-map node tap drive, so testing it here covers the
/// roadmap's Phase-2 "picker channel-switch" acceptance at the act-decision
/// layer (the DOM wiring is then a thin pass-through).
pub fn commit_target(commit: StripCommit, cur_idx: usize, count: usize) -> Option<usize> {
    match commit {
        StripCommit::Next => cur_idx.checked_add(1).filter(|&j| j < count),
        StripCommit::Prev => cur_idx.checked_sub(1),
        StripCommit::Stay => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_lock_horizontal_needs_dominance_and_slop() {
        assert_eq!(axis_lock(5.0, 0.0), None, "under slop = uncommitted");
        assert_eq!(axis_lock(20.0, 2.0), Some(Axis::Horizontal));
        // dy too close to dx (within the 1.15 ratio) ⇒ not horizontal; if dy is
        // also past slop it locks vertical.
        assert_eq!(axis_lock(12.0, 12.0), Some(Axis::Vertical));
        assert_eq!(axis_lock(2.0, 20.0), Some(Axis::Vertical));
    }

    #[test]
    fn row_swipe_only_wins_small_radius_rightward_on_a_row() {
        assert!(row_swipe_wins(true, 30.0), "short right-drag on a row");
        assert!(!row_swipe_wins(false, 30.0), "not started on a row");
        assert!(!row_swipe_wins(true, -30.0), "leftward is not a reply");
        assert!(
            !row_swipe_wins(true, 200.0),
            "large-radius is a channel switch"
        );
    }

    #[test]
    fn reply_glyph_pops_at_threshold() {
        assert!(!reply_armed(63.9));
        assert!(reply_armed(64.0));
    }

    #[test]
    fn strip_offset_tracks_one_to_one_in_the_middle() {
        // Middle pane (idx 1 of 3), 360px wide, dragged -100: base -360, extra -100.
        assert!((strip_offset(1, 3, 360.0, -100.0) - (-460.0)).abs() < 1e-9);
    }

    #[test]
    fn strip_offset_rubber_bands_at_edges() {
        // First pane dragged RIGHT (no prev) ⇒ damped by 0.32.
        assert!((strip_offset(0, 3, 360.0, 100.0) - (100.0 * RUBBER_BAND)).abs() < 1e-9);
        // Last pane dragged LEFT (no next) ⇒ base -720 + damped -100*0.32.
        let last = strip_offset(2, 3, 360.0, -100.0);
        assert!((last - (-720.0 + (-100.0 * RUBBER_BAND))).abs() < 1e-9);
        // First pane dragged LEFT (has a next) ⇒ full 1:1, no damping.
        assert!((strip_offset(0, 3, 360.0, -100.0) - (-100.0)).abs() < 1e-9);
    }

    #[test]
    fn commit_swipe_by_displacement_or_velocity() {
        let w = 360.0;
        // 33% displacement, slow ⇒ commit Next.
        assert_eq!(commit_swipe(-120.0, 1000.0, w), StripCommit::Next);
        // small displacement but fast flick ⇒ commit Prev.
        assert_eq!(commit_swipe(30.0, 40.0, w), StripCommit::Prev);
        // small + slow ⇒ Stay.
        assert_eq!(commit_swipe(20.0, 1000.0, w), StripCommit::Stay);
        // zero ⇒ Stay.
        assert_eq!(commit_swipe(0.0, 1000.0, w), StripCommit::Stay);
    }

    #[test]
    fn commit_target_maps_to_neighbor_index_with_edge_guards() {
        // Middle of a 4-channel guild: Next/Prev resolve to the neighbor.
        assert_eq!(commit_target(StripCommit::Next, 1, 4), Some(2));
        assert_eq!(commit_target(StripCommit::Prev, 1, 4), Some(0));
        // Edge guards: no next at the last channel, no prev at the first.
        assert_eq!(
            commit_target(StripCommit::Next, 3, 4),
            None,
            "no next at last"
        );
        assert_eq!(
            commit_target(StripCommit::Prev, 0, 4),
            None,
            "no prev at first"
        );
        // Stay never switches.
        assert_eq!(commit_target(StripCommit::Stay, 1, 4), None);
        // Empty/degenerate guild: nothing to switch to.
        assert_eq!(commit_target(StripCommit::Next, 0, 0), None);
    }
}
