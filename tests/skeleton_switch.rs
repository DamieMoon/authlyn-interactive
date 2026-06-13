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
