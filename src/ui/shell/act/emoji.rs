//! Per-guild custom-emoji actions: refresh / create / delete, plus the image
//! upload helper used by the emoji-manager pane to stage a media id before
//! naming it.

use super::super::Shell;
use leptos::prelude::RwSignal;

#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use crate::protocol::CreateEmojiRequest;
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Load the open guild's custom emoji into `guild_emoji` (drives the picker,
/// `:`-autocomplete, and `:name:` render resolution). The resolver `Memo`
/// recomputes from this signal, so an upload is usable without a reload.
#[cfg(feature = "hydrate")]
pub fn refresh_guild_emoji(s: Shell, gid: String) {
    spawn_local(async move {
        if let Ok(r) = api::list_emoji(&gid).await {
            s.sel.guild_emoji.set(r.emoji);
        }
    });
}

/// Create a named custom emoji from an already-uploaded media id, then
/// reload `s.sel.guild_emoji` so the new emoji is immediately usable.
#[cfg(feature = "hydrate")]
pub fn create_guild_emoji(s: Shell, gid: String, name: String, media_id: String) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::create_emoji(&gid, &CreateEmojiRequest { name, media_id }).await {
            Ok(_) => refresh_guild_emoji(s, gid),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Delete a custom emoji by name (owner/admin only — backend enforces),
/// then reload `s.sel.guild_emoji`.
#[cfg(feature = "hydrate")]
pub fn delete_guild_emoji(s: Shell, gid: String, name: String) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::delete_emoji(&gid, &name).await {
            Ok(()) => refresh_guild_emoji(s, gid),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Upload a picked image and stage its media id in `into` (the emoji
/// manager's pending-upload signal); "Add" then names it. Mirrors
/// `add_compose_attachment`'s upload, staging into a caller-owned signal
/// rather than the composer's attachment list.
#[cfg(feature = "hydrate")]
pub fn upload_emoji_image(s: Shell, file: web_sys::File, into: RwSignal<Option<String>>) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::upload_media(&file).await {
            Ok(id) => into.set(Some(id)),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn refresh_guild_emoji(_s: Shell, _gid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn create_guild_emoji(_s: Shell, _gid: String, _name: String, _media_id: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn delete_guild_emoji(_s: Shell, _gid: String, _name: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn upload_emoji_image(_s: Shell, _into: RwSignal<Option<String>>) {}
