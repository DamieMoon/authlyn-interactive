//! Guild-rail actions: refresh + reorder + open + create/rename/delete +
//! restore. Cross-calls: `open_server` → [`super::emoji::refresh_guild_emoji`]
//! and [`super::channel::open_channel`]; `create_server`/`delete_server`/
//! `restore_deleted_guild` → `refresh_guilds`.

use super::super::Shell;

#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

// localStorage keys for the last-used selection, restored on reload. Shared
// with `super::channel` (which restores them) — kept here because guild is the
// first thing reset on a restore round-trip.
#[cfg(feature = "hydrate")]
pub(super) const KEY_SERVER: &str = "authlyn.last_server";
#[cfg(feature = "hydrate")]
pub(super) const KEY_CHANNEL: &str = "authlyn.last_channel";

#[cfg(feature = "hydrate")]
pub fn refresh_guilds(s: Shell) {
    spawn_local(async move {
        if let Ok(r) = api::list_guilds().await {
            s.sel.guilds.set(r.guilds);
        }
    });
}

/// Reorder the personal guild rail (#17/FB2). `idx` indexes `s.sel.guilds` (the
/// caller's persisted order from `list_guilds`). We swap with the neighbor,
/// optimistically update the rail, then PUT the full new id order and reload
/// to confirm. The server replaces the caller's `user_guild_order` rows.
#[cfg(feature = "hydrate")]
pub fn swap_guild(s: Shell, idx: usize, up: bool) {
    let mut list = s.sel.guilds.get_untracked();
    let other = if up {
        if idx == 0 {
            return;
        }
        idx - 1
    } else {
        if idx + 1 >= list.len() {
            return;
        }
        idx + 1
    };
    list.swap(idx, other);
    s.sel.guilds.set(list.clone());
    let order: Vec<String> = list.iter().map(|g| g.id.clone()).collect();
    spawn_local(async move {
        if let Err(e) = api::set_rail_order(order).await {
            s.composer.status.set(api::humanize(&e));
        }
        if let Ok(r) = api::list_guilds().await {
            s.sel.guilds.set(r.guilds);
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn open_server(s: Shell, gid: String) {
    let _ = LocalStorage::set(KEY_SERVER, &gid);
    s.sel.sel_server.set(Some(gid.clone()));
    s.sel.sel_owner.set(None);
    s.sel.channels.set(Vec::new());
    s.sel.guild_emoji.set(Vec::new());
    super::emoji::refresh_guild_emoji(s, gid.clone());
    spawn_local(async move {
        if let Ok(d) = api::get_guild(&gid).await {
            s.sel.sel_owner.set(Some(d.owner_id.clone()));
            s.sel.channels.set(d.channels.clone());
            if let Some(first) = d
                .channels
                .iter()
                .find(|c| c.kind == "text")
                .or_else(|| d.channels.first())
            {
                super::channel::open_channel(s, first.clone());
            }
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn create_server(s: Shell, name: String) {
    if name.trim().is_empty() {
        return;
    }
    spawn_local(async move {
        match api::create_guild(&name).await {
            Ok(g) => {
                refresh_guilds(s);
                open_server(s, g.id);
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn rename_server(s: Shell, gid: String, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    spawn_local(async move {
        match api::patch_guild(&gid, &name).await {
            // Patch the rail list in place; the sidebar title derives from it.
            Ok(()) => s.sel.guilds.update(|gs| {
                if let Some(g) = gs.iter_mut().find(|g| g.id == gid) {
                    g.name = name.clone();
                }
            }),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Delete a guild (owner only). On success, clear the server selection and
/// refresh the rail so it no longer points at a dead id.
#[cfg(feature = "hydrate")]
pub fn delete_server(s: Shell, gid: String) {
    use super::super::Pane;
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::delete_guild(&gid).await {
            Ok(()) => {
                if s.sel.sel_server.get_untracked().as_deref() == Some(gid.as_str()) {
                    s.sel.sel_server.set(None);
                    s.sel.sel_owner.set(None);
                    s.sel.channels.set(Vec::new());
                    s.sel.sel_channel.set(None);
                    s.sync.pane.set(Pane::Friends);
                    LocalStorage::delete(KEY_SERVER);
                    LocalStorage::delete(KEY_CHANNEL);
                }
                refresh_guilds(s);
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Load the caller's own soft-deleted guilds into `s.trash.deleted_guilds`.
#[cfg(feature = "hydrate")]
pub fn load_deleted_guilds(s: Shell) {
    spawn_local(async move {
        match api::list_deleted_guilds().await {
            Ok(r) => s.trash.deleted_guilds.set(r.guilds),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Restore a soft-deleted guild (owner). On success, refresh the rail and
/// the deleted-guilds list so the restored server reappears and leaves trash.
#[cfg(feature = "hydrate")]
pub fn restore_deleted_guild(s: Shell, gid: String) {
    spawn_local(async move {
        match api::restore_guild(&gid).await {
            Ok(()) => {
                refresh_guilds(s);
                load_deleted_guilds(s);
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn refresh_guilds(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn swap_guild(_s: Shell, _idx: usize, _up: bool) {}
#[cfg(not(feature = "hydrate"))]
pub fn open_server(_s: Shell, _gid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn create_server(_s: Shell, _name: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn rename_server(_s: Shell, _gid: String, _name: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn delete_server(_s: Shell, _gid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_deleted_guilds(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn restore_deleted_guild(_s: Shell, _gid: String) {}
