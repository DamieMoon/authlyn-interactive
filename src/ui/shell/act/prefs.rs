//! localStorage-backed user toggle prefs (compose-preview, dialogue-style,
//! eyecandy, ghost-quill). Pure read/write helpers — no Shell interaction.
//!
//! (The old "ask before deleting a message" toggle and its
//! `authlyn.confirm_delete_message` key were retired by the undo-toast
//! deletion evolution — UX evolution #11: message deletes are instant with a
//! 6s undo now, so there is no confirm modal to gate. A stale stored key is
//! simply ignored.)

#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};

// ---- W5/P1 structural UI skeleton pref ----
//
// W5/P1: the three structural UI skeletons (spec §1). `sk-`-prefixed in code
// (.app.sk-*, _sk_*.scss, sk_*/mod.rs); the stored pref value is the bare id
// WITHOUT the `sk-` prefix (orbit/deck/hud). NO silent default — a pref-less
// device gets the onboarding ceremony, except the localStorage-unavailable
// fallback which boots orbit for the session.
pub const SKELETON_IDS: &[&str] = &["orbit", "deck", "hud"];
/// The session fallback when localStorage cannot persist (private mode etc.).
pub const SKELETON_FALLBACK: &str = "orbit";

/// Validate a stored/selected skeleton id; unknown ids are rejected so a
/// stale or corrupt localStorage value can never apply a bogus root class.
pub fn is_valid_skeleton(id: &str) -> bool {
    SKELETON_IDS.contains(&id)
}

#[cfg(feature = "hydrate")]
const KEY_SKELETON: &str = "authlyn.skeleton";

/// The persisted skeleton id, if any AND valid. `None` means pref-less →
/// the caller runs the onboarding ceremony (no silent default).
#[cfg(feature = "hydrate")]
pub fn skeleton_pref() -> Option<String> {
    LocalStorage::get::<String>(KEY_SKELETON)
        .ok()
        .filter(|id| is_valid_skeleton(id))
}

/// Persist the chosen skeleton id. Returns false if the write failed
/// (localStorage unavailable) so the caller can fall back to a session-only
/// `orbit` without claiming it was saved.
#[cfg(feature = "hydrate")]
pub fn set_skeleton(id: &str) -> bool {
    if !is_valid_skeleton(id) {
        return false;
    }
    LocalStorage::set(KEY_SKELETON, id).is_ok()
}

/// Remove the stored skeleton pref (used by the ceremony's writability probe
/// to leave no committed value — see Task 1.3, "no silent default").
#[cfg(feature = "hydrate")]
pub fn clear_skeleton() {
    LocalStorage::delete(KEY_SKELETON);
}

/// The throwaway probe key the ceremony uses to detect localStorage
/// writability WITHOUT touching authlyn.skeleton (Open Question #3).
#[cfg(feature = "hydrate")]
const KEY_PREF_PROBE: &str = "_authlyn_pref_test";

/// True if localStorage can be written. Sets then deletes a throwaway key so
/// it never leaves a side effect on the real skeleton pref. A failed write
/// (private mode / quota / disabled) returns false → session fallback.
#[cfg(feature = "hydrate")]
pub fn local_storage_writable() -> bool {
    let ok = LocalStorage::set(KEY_PREF_PROBE, "1").is_ok();
    LocalStorage::delete(KEY_PREF_PROBE);
    ok
}

// ---- composer live-preview toggle ----
//
// localStorage key for the composer's live formatting-preview toggle. "1" =
// on; absent or anything else = off (the preview is opt-in).
#[cfg(feature = "hydrate")]
const KEY_COMPOSE_PREVIEW: &str = "authlyn.compose_preview";

/// Whether the composer shows a live rendered preview (default OFF).
#[cfg(feature = "hydrate")]
pub fn compose_preview_enabled() -> bool {
    LocalStorage::get::<String>(KEY_COMPOSE_PREVIEW)
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Persist the composer preview toggle.
#[cfg(feature = "hydrate")]
pub fn set_compose_preview(on: bool) {
    let _ = LocalStorage::set(KEY_COMPOSE_PREVIEW, if on { "1" } else { "0" });
}

// ---- per-user "style RP dialogue" toggle ----
//
// localStorage key for the per-user "style RP dialogue" toggle. "1" = on;
// absent or anything else = off (the styling is opt-in).
#[cfg(feature = "hydrate")]
const KEY_DIALOGUE_STYLE: &str = "authlyn.dialogue_style";

/// Whether `"…"` dialogue should be visually styled at render (default OFF).
#[cfg(feature = "hydrate")]
pub fn rp_dialogue_style_enabled() -> bool {
    LocalStorage::get::<String>(KEY_DIALOGUE_STYLE)
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Persist the dialogue-styling toggle.
#[cfg(feature = "hydrate")]
pub fn set_rp_dialogue_style(on: bool) {
    let _ = LocalStorage::set(KEY_DIALOGUE_STYLE, if on { "1" } else { "0" });
}

// ---- Eye-candy appearance tier toggle ----
//
// localStorage key for the Eye-candy appearance tier (`.fx-max`) toggle. "1"
// = on; absent or anything else = off (Standard is the default).
#[cfg(feature = "hydrate")]
const KEY_EYECANDY: &str = "authlyn.eyecandy";

/// Eye-candy appearance tier (`.fx-max`). Default OFF (Standard).
#[cfg(feature = "hydrate")]
pub fn eyecandy_enabled() -> bool {
    LocalStorage::get::<String>(KEY_EYECANDY)
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Persist the Eye-candy appearance-tier toggle.
#[cfg(feature = "hydrate")]
pub fn set_eyecandy(on: bool) {
    let _ = LocalStorage::set(KEY_EYECANDY, if on { "1" } else { "0" });
}

// ---- Ghost Quill live co-writer toggle (W4/T7) ----
//
// localStorage key for the Ghost Quill toggle. "1" = on; absent or anything
// else = off. NOTE: gloo-storage JSON-encodes values, so the stored string is
// `"1"` WITH quotes — always read it back through `LocalStorage::get`, never
// raw. Privacy-respecting opt-in BOTH ways, default OFF: when on, this
// client SENDS its compose text with the typing ping AND fetches/renders
// other members' ghost drafts; when off it does neither.
#[cfg(feature = "hydrate")]
const KEY_GHOST_QUILL: &str = "authlyn.ghost_quill";

/// Ghost Quill live co-writer draft preview (W4/T7). Default OFF.
#[cfg(feature = "hydrate")]
pub fn ghost_quill_enabled() -> bool {
    LocalStorage::get::<String>(KEY_GHOST_QUILL)
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Persist the Ghost Quill toggle.
#[cfg(feature = "hydrate")]
pub fn set_ghost_quill(on: bool) {
    let _ = LocalStorage::set(KEY_GHOST_QUILL, if on { "1" } else { "0" });
}

// ---- ssr stubs (no localStorage on the server) ----

#[cfg(not(feature = "hydrate"))]
pub fn compose_preview_enabled() -> bool {
    false
}

#[cfg(not(feature = "hydrate"))]
pub fn set_compose_preview(_on: bool) {}

#[cfg(not(feature = "hydrate"))]
pub fn rp_dialogue_style_enabled() -> bool {
    false
}

#[cfg(not(feature = "hydrate"))]
pub fn set_rp_dialogue_style(_on: bool) {}

#[cfg(not(feature = "hydrate"))]
pub fn eyecandy_enabled() -> bool {
    false
}

#[cfg(not(feature = "hydrate"))]
pub fn set_eyecandy(_on: bool) {}

#[cfg(not(feature = "hydrate"))]
pub fn ghost_quill_enabled() -> bool {
    false
}

#[cfg(not(feature = "hydrate"))]
pub fn set_ghost_quill(_on: bool) {}

#[cfg(not(feature = "hydrate"))]
pub fn skeleton_pref() -> Option<String> {
    None
}
#[cfg(not(feature = "hydrate"))]
pub fn set_skeleton(_id: &str) -> bool {
    false
}
#[cfg(not(feature = "hydrate"))]
pub fn clear_skeleton() {}
#[cfg(not(feature = "hydrate"))]
pub fn local_storage_writable() -> bool {
    false
}
