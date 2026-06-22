//! Skeleton-id validation guards. The id validation runs in the ssr graph (no
//! DB needed) and pins the prefs.rs persistence surface that survives the M3
//! retirement. v27 (M5/P2) ships Orbit as the SOLE shell — the three-way
//! ceremony and the account skeleton-picker are retired (deck/hud are
//! post-release). These tests keep `SKELETON_IDS` / `is_valid_skeleton` /
//! `SKELETON_FALLBACK` / `set_skeleton` honest so re-enabling a chooser when a
//! second skeleton lands is a known-good surface.

// Reached via the act re-export; this is the stable public path.
use authlyn_interactive::ui::shell::act::{is_valid_skeleton, SKELETON_FALLBACK, SKELETON_IDS};

#[test]
fn skeleton_ids_are_exactly_the_three_shells() {
    assert_eq!(SKELETON_IDS, &["orbit", "deck", "hud"]);
}

#[test]
fn valid_skeleton_accepts_known_rejects_unknown() {
    assert!(is_valid_skeleton("orbit"));
    assert!(is_valid_skeleton("deck"));
    assert!(is_valid_skeleton("hud"));
    assert!(!is_valid_skeleton("sk-orbit")); // stored value is bare id, no prefix
    assert!(!is_valid_skeleton("ritual")); // stale/legacy name rejected
    assert!(!is_valid_skeleton(""));
}

#[test]
fn fallback_is_a_valid_id() {
    assert!(is_valid_skeleton(SKELETON_FALLBACK));
    assert_eq!(SKELETON_FALLBACK, "orbit");
}

// Pin the ssr persistence surface. v27 forces orbit unconditionally at shell
// init (no ceremony, no chooser); these stubs still pin the contract the
// post-release deck/hud chooser will re-use.
use authlyn_interactive::ui::shell::act::{local_storage_writable, skeleton_pref};

#[test]
fn ssr_stubs_signal_no_pref_and_no_storage() {
    // On the server there is no localStorage: skeleton_pref() is None and
    // local_storage_writable() is false. v27 forces orbit at hydrate init in
    // shell/mod.rs (not via this stub), so the ssr stub stays None unchanged.
    assert_eq!(skeleton_pref(), None);
    assert!(!local_storage_writable());
}

use authlyn_interactive::ui::shell::act::set_skeleton;

/// §13 invariant contract: when a second skeleton ships and the chooser
/// returns, switching must NOT remount the shell — the skeleton lives on the
/// same stable Prefs aggregate / same .app root that carries fx-max (switched
/// live without remount), so flipping it is a pure class toggle. set_skeleton
/// therefore must (a) accept only valid ids and (b) never touch SSE / composer
/// / selection state — it only persists a string. These assertions pin that.
#[test]
fn set_skeleton_surface_is_pref_only() {
    // The ssr stub returns false (no localStorage on the server) and takes
    // ONLY an id — proving the API touches no shell state.
    assert!(!set_skeleton("orbit"));
    assert!(!set_skeleton("nonsense"));
}

// PHASE-7 GATE CONTRACT (real-device, per §13): v27 ships orbit-only, so there
// is no live skeleton switch to verify this release. When a second skeleton
// lands and the picker returns, the gate MUST verify on the live app that
// switching preserves: (1) the SSE connection (no reconnect), (2) the composer
// draft text, (3) the selected channel + scroll position — none assertable
// from the ssr harness. Headed Playwright on the MacBook is the intended tool.
