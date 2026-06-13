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
