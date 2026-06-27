//! Personas + gallery, and the per-guild / per-channel "worn" persona.
//!
//! Wave-3 split of the original `server/personas.rs` into focused submodules.
//! Personas are account-global: a user builds a library (image + name +
//! description + gallery) and "wears" one per channel via
//! `PUT /channels/{cid}/active-persona` (a per-channel current path) or the
//! legacy `PUT /guilds/{id}/active-persona`. `server::messages` stamps the
//! resolved persona onto each post. All persona routes are owner-scoped:
//! another account's persona reads/writes as a privacy-404. Images reuse
//! `server::media` — endpoints here take an already-uploaded `media_id`.
//!
//! ## Layout
//! - `core` — list/create/get/patch/delete + redeem/leave + share-key.
//! - `editors` — list/add/remove editor + the persona-roster helper.
//! - `gallery` — avatar + gallery image add/remove + media-id validation.
//! - `wear` — per-guild + per-channel "worn" persona endpoints.

mod core;
mod editors;
mod gallery;
mod wear;

use crate::server::state::AppState;

/// Persona realtime (review C3, bug hunt 019ef87b): nudge the affected accounts
/// over the SSE bus when a persona's editor set changes (share / revoke /
/// redeem / leave), so an already-mounted recipient/owner session refetches
/// `GET /personas` instead of showing a stale wardrobe + orbit-station grid.
/// Account-targeted (never broadcast), id-only like every `SyncEvent` — the
/// persona twin of `friends::emit_friends_changed`. `accounts` is the affected
/// set (owner + editor for a grant/revoke; just the caller for a self-leave).
pub(super) fn emit_personas_changed(state: &AppState, accounts: Vec<String>) {
    state.emit_for(accounts, crate::protocol::SyncEvent::PersonasChanged);
}

/// Content propagation (review A/B): resolve every account whose `GET /personas`
/// shows persona `pid` — the owner PLUS every redeemed editor — and nudge them
/// all. Used when a SHARED persona's grid-projected CONTENT changes
/// (`patch_persona` name/description/color, `set_avatar`), so a co-viewer's
/// mounted wardrobe + orbit grid refetches instead of showing a stale card.
/// Best-effort: the mutation already committed, so a lookup failure is logged,
/// not surfaced. For a solo persona the set is just the owner — a harmless
/// self-nudge that keeps their other devices consistent.
pub(super) async fn emit_personas_changed_for_persona(state: &AppState, pid: &str) {
    let lookup = state
        .db
        .query(
            "SELECT VALUE meta::id(owner) FROM persona WHERE id = type::record('persona', $pid);
             SELECT VALUE meta::id(account) FROM persona_editor
                WHERE persona = type::record('persona', $pid);",
        )
        .bind(("pid", pid.to_string()))
        .await;
    let mut accounts: Vec<String> = Vec::new();
    match lookup {
        Ok(mut r) => {
            if let Ok(owner) = r.take::<Vec<String>>(0) {
                accounts.extend(owner);
            }
            if let Ok(editors) = r.take::<Vec<String>>(1) {
                accounts.extend(editors);
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "emit_personas_changed_for_persona lookup failed");
            return;
        }
    }
    if !accounts.is_empty() {
        emit_personas_changed(state, accounts);
    }
}

// Route-table handlers keep their `crate::server::personas::<fn>` paths via
// these re-exports.
pub use self::core::{
    create_persona, delete_persona, get_persona, leave_persona, list_personas, patch_persona,
    redeem_persona_key,
};
pub use self::editors::{add_editor, list_editors, remove_editor};
pub use self::gallery::{
    add_gallery_image, add_gallery_images_batch, remove_gallery_image, set_avatar,
};
pub use self::wear::{set_active_persona, set_channel_active_persona};
