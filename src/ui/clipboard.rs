//! Shared clipboard helpers for paste-driven flows.
//!
//! Hydrate-only — `web_sys::ClipboardEvent` and friends live in the browser.
//! Gated at the `mod` declaration in [`crate::ui`], matching the local
//! convention used for `db`/`server`/`storage` in `lib.rs`.
//!
//! Today this hosts a single helper, [`read_pasted_images`], shared by the
//! composer's paste-to-upload handler (`shell/channel/mod.rs`) and reused by
//! the persona-gallery paste handler (`shell/wardrobe.rs`, B4) once that
//! lands. Extracted in W7/B2 from the inlined composer loop so the gallery
//! doesn't duplicate the MIME-filter + `File`-collection logic.

use web_sys::{ClipboardEvent, File};

/// Pull every `image/*` entry out of a `ClipboardEvent` as a `Vec<File>`.
///
/// Order is preserved as the browser reports it (item index ascending).
/// Returns an empty vec if the event has no clipboard data or no image
/// items — callers should decide whether to `prevent_default()` based on
/// emptiness so non-image pastes (e.g. plain text) still flow through.
pub fn read_pasted_images(ev: &ClipboardEvent) -> Vec<File> {
    let Some(dt) = ev.clipboard_data() else {
        return Vec::new();
    };
    let items = dt.items();
    let mut out = Vec::new();
    for i in 0..items.length() {
        let Some(item) = items.get(i) else { continue };
        if item.type_().starts_with("image/") {
            if let Ok(Some(file)) = item.get_as_file() {
                out.push(file);
            }
        }
    }
    out
}
