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
//! - [`core`] — list/create/get/patch/delete + redeem/leave + share-key.
//! - [`editors`] — list/add/remove editor + the persona-roster helper.
//! - [`gallery`] — avatar + gallery image add/remove + media-id validation.
//! - [`wear`] — per-guild + per-channel "worn" persona endpoints.

mod core;
mod editors;
mod gallery;
mod wear;

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
