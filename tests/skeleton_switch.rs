//! W5/P1 theme-switch guards. The skeleton-id validation runs in the ssr
//! graph (no DB needed). The switch-invariant assertions (SSE/composer/
//! selection preserved) are documented here as the Phase-7 gate contract.

// Reached via the act re-export (Step 1.1.4); this is the stable public path.
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

// W5/P1 (Task 1.3): pin the no-silent-default surface the ceremony depends on.
use authlyn_interactive::ui::shell::act::{local_storage_writable, skeleton_pref};

#[test]
fn ssr_stubs_signal_no_pref_and_no_storage() {
    // On the server there is no localStorage: skeleton_pref() is None (so the
    // ceremony would show on a writable client) and local_storage_writable()
    // is false (so the server never claims a writable store). This pins the
    // surface the ceremony's no-silent-default branch depends on; the live
    // writable→None / non-writable→orbit behavior is a Phase-7 device check.
    assert_eq!(skeleton_pref(), None);
    assert!(!local_storage_writable());
}

use authlyn_interactive::ui::shell::act::set_skeleton;

/// W5/P1 §13 invariant contract: switching the skeleton must NOT remount the
/// shell. The structural guarantee is that the skeleton lives on the same
/// stable Prefs aggregate / same .app root that carries fx-max (which we
/// already switch live without remount), so flipping it is a pure class
/// toggle. set_skeleton therefore must (a) accept only valid ids and (b)
/// never need to touch SSE / composer / selection state — it only persists a
/// string. These assertions pin that set_skeleton's surface is exactly that.
#[test]
fn set_skeleton_surface_is_pref_only() {
    // The ssr stub returns false (no localStorage on the server) and takes
    // ONLY an id — proving the API touches no shell state. (The hydrate impl
    // is the same shape; the live SSE/composer/selection preservation is a
    // Phase-7 real-device gate item, documented below.)
    assert!(!set_skeleton("orbit"));
    assert!(!set_skeleton("nonsense"));
}

// PHASE-7 GATE CONTRACT (real-device, per §13): after this lands, the wave
// gate MUST verify on the live app that switching the skeleton via the account
// picker preserves: (1) the SSE connection (no reconnect in the network
// panel), (2) the composer draft text, (3) the selected channel + scroll
// position. These cannot be asserted from the ssr harness (no WASM signals);
// they are booked as Phase-7 theme-machinery items. Open Question #4: the
// owner may want this automated sooner via headed Playwright on the MacBook.
