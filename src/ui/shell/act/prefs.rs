//! localStorage-backed user toggle prefs (confirm-delete, compose-preview,
//! dialogue-style, eyecandy). Pure read/write helpers — no Shell interaction.

#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};

// ---- "ask before deleting a message" toggle ----
//
// localStorage key for the "ask before deleting a message" toggle. Absent or
// any value other than "0" means ON (confirm); "0" means the user opted out.
#[cfg(feature = "hydrate")]
const KEY_CONFIRM_DELETE_MSG: &str = "authlyn.confirm_delete_message";

/// Whether message deletes should ask for confirmation (default ON).
#[cfg(feature = "hydrate")]
pub fn confirm_delete_message_enabled() -> bool {
    LocalStorage::get::<String>(KEY_CONFIRM_DELETE_MSG)
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Persist the message-delete confirmation toggle.
#[cfg(feature = "hydrate")]
pub fn set_confirm_delete_message(on: bool) {
    let _ = LocalStorage::set(KEY_CONFIRM_DELETE_MSG, if on { "1" } else { "0" });
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

// ---- ssr stubs (no localStorage on the server) ----

#[cfg(not(feature = "hydrate"))]
pub fn confirm_delete_message_enabled() -> bool {
    true
}

#[cfg(not(feature = "hydrate"))]
pub fn set_confirm_delete_message(_on: bool) {}

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
