//! W5/P2 horizontal swipe-strip physics (Orbit's signature gesture) +
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
/// Radial-disarm drift slop (px). Mirrors `radial::MOVE_SLOP_PX` (also 10px):
/// once a press drifts past this radius it is a flick / drag / scroll, not a
/// long-press hold. The StripDrag terminal/bail paths reuse it to disarm the
/// inherited radial when `set_pointer_capture` has starved the radial's OWN
/// `<ul>` slop-disarm (the phantom-menu fix; see `drag.rs`).
pub const RADIAL_DISARM_SLOP_PX: f64 = 10.0;

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
// TODO(Phase-7 9.4.3-a): forward-stub — first consumed by the swipe-to-reply
// row VISUAL (the ↩ glyph offset + haptic tick + act::start_reply trigger),
// booked as a follow-up. The arbitration (strip yields, drag.rs) shipped ahead
// of the visual on purpose, so this predicate has no caller yet; not orphaned.
pub fn reply_armed(dx: f64) -> bool {
    dx >= REPLY_POP_PX
}

/// Has the press drifted past the radial's disarm slop (a flick / drag / scroll
/// rather than a stationary long-press hold)? A pure mirror of the radial's own
/// `MOVE_SLOP_PX` test (`dx²+dy² > slop²`, strict). The StripDrag terminal/bail
/// paths gate `disarm_radial()` on this so a genuine hold still blossoms the
/// menu while a moved gesture (whose disarm the stolen pointer capture would
/// otherwise eat) cancels the armed 450ms timer.
pub fn moved_past_radial_slop(dx: f64, dy: f64) -> bool {
    dx * dx + dy * dy > RADIAL_DISARM_SLOP_PX * RADIAL_DISARM_SLOP_PX
}

/// The strip's live `translateX` (px) while dragging, given a viewport `width`
/// wide and finger delta `dx`. The DOM is a FIXED 3-slot strip whose live
/// ChannelPane is ALWAYS the middle slot, so the resting base is `-width`
/// regardless of the current channel's position in the sidebar list — `idx`/
/// `count` MUST NOT feed this base (the bug that translated the strip off-screen
/// for any channel not at list-index 1). `at_first`/`at_last` are the TRUE
/// channel-list edges (no prev / no next neighbor), used ONLY to rubber-band a
/// drag that has no neighbor to reveal: dragging right (`dx>0`) at the first
/// channel, or left (`dx<0`) at the last, is damped by `RUBBER_BAND`.
pub fn strip_offset(at_first: bool, at_last: bool, width: f64, dx: f64) -> f64 {
    // The middle slot of the 3-slot strip is the resting position.
    let base = -width;
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
    // A non-positive width (an unmeasured / zero-laid-out pane) has no
    // displacement threshold to clear — otherwise `COMMIT_FRACTION * 0 == 0`
    // would let ANY nonzero dx commit. Fall through to the velocity branch.
    let past_displacement = width > 0.0 && dx.abs() >= COMMIT_FRACTION * width;
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

/// Whether the orbit swipe-strip collapses to its single, un-swipeable pane
/// (`--single`: one pane filling the viewport, no neighbor peeks). TRUE only for
/// a GENUINELY single-channel guild (`== 1`). `count == 0` is the TRANSIENT
/// far-dive load window — `act::open_server` clears `s.sel.channels` then
/// async-fetches them — and MUST keep the multi geometry, or the strip flashes a
/// lone pane on every far-server dive before the channels land (the shipped
/// `<= 1` conflated load-0 with a real single).
pub fn collapses_to_single(channel_count: usize) -> bool {
    channel_count == 1
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
    fn moved_past_radial_slop_preserves_a_stationary_hold() {
        // A pure mirror of the radial's own MOVE_SLOP_PX drift test: under the
        // slop radius the finger is "stationary" (a genuine long-press hold the
        // radial must still open), past it the finger has drifted (a flick /
        // drag / scroll the StripDrag terminal paths must disarm because the
        // stolen pointer capture starves the radial's own <ul> slop-disarm).
        assert!(
            !moved_past_radial_slop(0.0, 0.0),
            "no motion = a hold; radial must survive"
        );
        assert!(
            !moved_past_radial_slop(7.0, 7.0),
            "7,7 (r≈9.9) under the 10px slop = still a hold"
        );
        assert!(
            moved_past_radial_slop(11.0, 0.0),
            "11px horizontal drift past the slop"
        );
        assert!(
            moved_past_radial_slop(0.0, 11.0),
            "11px vertical drift (scroll) past the slop"
        );
        // Exactly the radial's boundary (> not ≥): RADIAL_DISARM_SLOP_PX² is the
        // threshold, so a point on the circle does NOT count as moved.
        assert!(
            !moved_past_radial_slop(RADIAL_DISARM_SLOP_PX, 0.0),
            "on the slop circle is not past it (mirrors radial's strict >)"
        );
    }

    #[test]
    fn strip_offset_tracks_one_to_one_from_the_middle_slot() {
        // The live ChannelPane is ALWAYS the middle of the fixed 3-slot DOM, so
        // the resting base is -width regardless of the channel's list index.
        // Interior channel (not at either edge), 360px wide, dragged -100:
        // base -360, extra -100 ⇒ -460.
        assert!((strip_offset(false, false, 360.0, -100.0) - (-460.0)).abs() < 1e-9);
    }

    #[test]
    fn strip_offset_resting_base_is_one_slot_regardless_of_list_index() {
        // The 3-slot invariant the broken wiring violated: the no-drag offset is
        // -width for a current channel at list-index 0, in the middle, AND at
        // the last index — the math only ever sees slot geometry now. dx=0 ⇒
        // exactly -width in every edge configuration.
        for (at_first, at_last) in [(true, false), (false, false), (false, true)] {
            assert!(
                (strip_offset(at_first, at_last, 360.0, 0.0) - (-360.0)).abs() < 1e-9,
                "resting offset must be -width (the middle slot) for \
                 at_first={at_first} at_last={at_last}",
            );
        }
    }

    #[test]
    fn strip_offset_rubber_bands_only_at_true_edges() {
        // First channel dragged RIGHT (no prev neighbor) ⇒ damped by 0.32 off
        // the -width resting base.
        assert!(
            (strip_offset(true, false, 360.0, 100.0) - (-360.0 + 100.0 * RUBBER_BAND)).abs() < 1e-9
        );
        // Last channel dragged LEFT (no next neighbor) ⇒ damped -100*0.32 off
        // the same -width base.
        assert!(
            (strip_offset(false, true, 360.0, -100.0) - (-360.0 + (-100.0 * RUBBER_BAND))).abs()
                < 1e-9
        );
        // First channel dragged LEFT (a next neighbor EXISTS) ⇒ full 1:1.
        assert!((strip_offset(true, false, 360.0, -100.0) - (-360.0 - 100.0)).abs() < 1e-9);
        // Last channel dragged RIGHT (a prev neighbor EXISTS) ⇒ full 1:1.
        assert!((strip_offset(false, true, 360.0, 100.0) - (-360.0 + 100.0)).abs() < 1e-9);
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
        // zero width (unmeasured pane): displacement threshold is 0, so the
        // guard must NOT let a slow drag commit on width alone — falls through
        // to velocity, which here is too slow ⇒ Stay.
        assert_eq!(
            commit_swipe(50.0, 1000.0, 0.0),
            StripCommit::Stay,
            "zero width must not auto-commit by displacement"
        );
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

    #[test]
    fn strip_collapses_only_for_a_genuinely_single_channel_guild() {
        // count 0 is the TRANSIENT far-dive load window (open_server clears
        // s.sel.channels then async-fetches) — it must NOT collapse, or the strip
        // flashes a lone pane on every far-server dive. The shipped `<= 1`
        // returned true here (the regression this test pins); `== 1` is correct.
        assert!(
            !collapses_to_single(0),
            "count 0 = transient load, not a single-channel guild; keep multi geometry"
        );
        assert!(
            collapses_to_single(1),
            "exactly one channel = a lone un-swipeable pane"
        );
        assert!(
            !collapses_to_single(2),
            "two channels = a swipeable multi strip"
        );
        assert!(!collapses_to_single(9));
    }
}
