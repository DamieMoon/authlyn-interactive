//! Persona (wardrobe) actions: create / update / remove / leave / swap /
//! share / avatar + wear / unwear. The optimistic reorder in [`swap_persona`]
//! is invariant-protected: local order is set, every changed position is
//! PATCH'd, then the grid is reloaded to confirm.

use super::super::Shell;
use leptos::prelude::RwSignal;

#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

#[cfg(feature = "hydrate")]
pub fn create_persona(s: Shell, name: String, desc: String) {
    if name.trim().is_empty() {
        return;
    }
    spawn_local(async move {
        match api::create_persona(&name, &desc).await {
            Ok(_) => {
                if let Ok(r) = api::list_personas().await {
                    s.social.personas.set(r.personas);
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Save edits to a persona (name + description + color), then reload the
/// wardrobe grid so the card reflects the change. `done` is set true on
/// success so the caller can close the detail editor.
#[cfg(feature = "hydrate")]
pub fn update_persona(
    s: Shell,
    pid: String,
    name: String,
    description: String,
    color: String,
    done: RwSignal<bool>,
) {
    if name.trim().is_empty() {
        s.composer.status.set("name must not be empty".to_string());
        return;
    }
    spawn_local(async move {
        match api::patch_persona(&pid, Some(name), Some(description), Some(color)).await {
            Ok(()) => {
                if let Ok(r) = api::list_personas().await {
                    s.social.personas.set(r.personas);
                }
                done.set(true);
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn remove_persona(s: Shell, pid: String) {
    spawn_local(async move {
        match api::delete_persona(&pid).await {
            Ok(()) => {
                // If the removed persona was being worn in the open channel,
                // take it off locally (per-channel signal).
                if s.social.active_persona.get_untracked().as_deref() == Some(pid.as_str()) {
                    s.social.active_persona.set(None);
                }
                if let Ok(r) = api::list_personas().await {
                    s.social.personas.set(r.personas);
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Leave a shared persona (editor only): drop it from the caller's list.
/// Mirrors `remove_persona`'s local cleanup, then reloads the grid.
#[cfg(feature = "hydrate")]
pub fn leave_shared_persona(s: Shell, pid: String) {
    spawn_local(async move {
        match api::leave_persona(&pid).await {
            Ok(()) => {
                if s.social.active_persona.get_untracked().as_deref() == Some(pid.as_str()) {
                    s.social.active_persona.set(None);
                }
                if let Ok(r) = api::list_personas().await {
                    s.social.personas.set(r.personas);
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Move a persona up/down in the wardrobe grid and persist the new order.
///
/// `idx` is the card's position in the *currently displayed* (already
/// server-sorted) `s.social.personas` list. We swap it with its neighbor, then
/// renumber the whole list to its array index and PATCH every persona whose
/// position changed. Renumbering (rather than swapping two `position`
/// values) is robust against old rows whose `position` is still NONE: after
/// one reorder the entire list is fully ordered with no gaps. Reorder is
/// only offered when the search filter is empty (see wardrobe.rs), so `idx`
/// always indexes the full list.
#[cfg(feature = "hydrate")]
pub fn swap_persona(s: Shell, idx: usize, up: bool) {
    let mut list = s.social.personas.get_untracked();
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
    // Optimistic local reorder + renumber-and-persist shared with the drag /
    // move-to-bounds helpers.
    persist_persona_order(s, list);
}

/// Move a persona to an absolute `target` index in the wardrobe grid (drag-and-
/// drop drop target). Removes the dragged card from `idx` and re-inserts it at
/// `target`, then renumbers + PATCHes exactly like [`swap_persona`]. No-op when
/// `idx == target` or either is out of range. The server re-checks
/// `can_edit_persona` per PATCH.
#[cfg(feature = "hydrate")]
pub fn move_persona(s: Shell, idx: usize, target: usize) {
    let mut list = s.social.personas.get_untracked();
    if idx >= list.len() || target >= list.len() || idx == target {
        return;
    }
    let item = list.remove(idx);
    list.insert(target, item);
    persist_persona_order(s, list);
}

/// Bring a persona to the very top (`top = true`) or bottom of the grid — the
/// mobile / keyboard fallback for drag. Defers to [`move_persona`].
#[cfg(feature = "hydrate")]
pub fn move_persona_to_bounds(s: Shell, idx: usize, top: bool) {
    let len = s.social.personas.get_untracked().len();
    if len == 0 {
        return;
    }
    let target = if top { 0 } else { len - 1 };
    move_persona(s, idx, target);
}

/// Shared tail of the persona reorders: optimistically set the new local order,
/// PATCH every card whose stored position no longer matches its index, then
/// reload the grid to confirm. Factored out of [`swap_persona`]'s body so the
/// drag / bounds helpers reuse the exact same flow (renumber to array index,
/// robust against NONE positions on legacy rows).
#[cfg(feature = "hydrate")]
fn persist_persona_order(s: Shell, list: Vec<crate::protocol::PersonaSummary>) {
    s.social.personas.set(list.clone());
    let patches: Vec<(String, i64)> = list
        .iter()
        .enumerate()
        .filter(|(i, p)| p.position != Some(*i as i64))
        .map(|(i, p)| (p.id.clone(), i as i64))
        .collect();
    if patches.is_empty() {
        return;
    }
    spawn_local(async move {
        for (pid, pos) in patches {
            if let Err(e) = api::set_persona_position(&pid, pos).await {
                s.composer.status.set(api::humanize(&e));
                break;
            }
        }
        if let Ok(r) = api::list_personas().await {
            s.social.personas.set(r.personas);
        }
    });
}

/// Load the owner-only sharing state for the detail editor's friends
/// checklist: the caller's friends, plus who already has editor access.
#[cfg(feature = "hydrate")]
pub fn load_persona_sharing(
    s: Shell,
    pid: String,
    friends: RwSignal<Vec<crate::protocol::FriendSummary>>,
    editors: RwSignal<Vec<crate::protocol::PersonaEditor>>,
) {
    spawn_local(async move {
        match api::list_friends().await {
            Ok(r) => friends.set(r.friends),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
        if let Ok(r) = api::list_persona_editors(&pid).await {
            editors.set(r.editors);
        }
    });
}

/// Toggle whether a friend may edit/wear this persona (owner only): check =
/// grant, uncheck = revoke. Refreshes the editor set the checklist binds to.
#[cfg(feature = "hydrate")]
pub fn set_persona_share(
    s: Shell,
    pid: String,
    aid: String,
    share: bool,
    editors: RwSignal<Vec<crate::protocol::PersonaEditor>>,
) {
    spawn_local(async move {
        let res = if share {
            api::add_persona_editor(&pid, &aid).await
        } else {
            api::remove_persona_editor(&pid, &aid).await
        };
        match res {
            Ok(()) => {
                if let Ok(r) = api::list_persona_editors(&pid).await {
                    editors.set(r.editors);
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Upload a picture and set it as the persona's avatar: POST the file to
/// `/media`, then PUT the returned id as the avatar, then reload the grid so
/// the new portrait shows. Errors surface via `s.composer.status`.
#[cfg(feature = "hydrate")]
pub fn set_persona_avatar(s: Shell, pid: String, file: web_sys::File) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        let media_id = match api::upload_media(&file).await {
            Ok(id) => id,
            Err(e) => {
                s.composer.status.set(api::humanize(&e));
                return;
            }
        };
        match api::set_persona_avatar(&pid, &media_id).await {
            Ok(()) => {
                if let Ok(r) = api::list_personas().await {
                    s.social.personas.set(r.personas);
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn wear_persona(s: Shell, pid: String) {
    // Per-channel now: wear into the currently-open channel. No open channel
    // → no-op (there's nowhere to wear it).
    let Some(cid) = s.sel.sel_channel.get_untracked().map(|c| c.id) else {
        return;
    };
    s.social.active_persona.set(Some(pid.clone()));
    spawn_local(async move {
        let _ = api::set_channel_active_persona(&cid, Some(pid)).await;
    });
}

#[cfg(feature = "hydrate")]
pub fn unwear(s: Shell) {
    let Some(cid) = s.sel.sel_channel.get_untracked().map(|c| c.id) else {
        return;
    };
    s.social.active_persona.set(None);
    spawn_local(async move {
        let _ = api::set_channel_active_persona(&cid, None).await;
    });
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn create_persona(_s: Shell, _name: String, _desc: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn update_persona(
    _s: Shell,
    _pid: String,
    _name: String,
    _description: String,
    _color: String,
    _done: RwSignal<bool>,
) {
}
#[cfg(not(feature = "hydrate"))]
pub fn remove_persona(_s: Shell, _pid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn leave_shared_persona(_s: Shell, _pid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn swap_persona(_s: Shell, _idx: usize, _up: bool) {}
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn move_persona(_s: Shell, _idx: usize, _target: usize) {}
#[cfg(not(feature = "hydrate"))]
pub fn move_persona_to_bounds(_s: Shell, _idx: usize, _top: bool) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_persona_sharing(
    _s: Shell,
    _pid: String,
    _friends: RwSignal<Vec<crate::protocol::FriendSummary>>,
    _editors: RwSignal<Vec<crate::protocol::PersonaEditor>>,
) {
}
#[cfg(not(feature = "hydrate"))]
pub fn set_persona_share(
    _s: Shell,
    _pid: String,
    _aid: String,
    _share: bool,
    _editors: RwSignal<Vec<crate::protocol::PersonaEditor>>,
) {
}
#[cfg(not(feature = "hydrate"))]
pub fn set_persona_avatar(_s: Shell, _pid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn wear_persona(_s: Shell, _pid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn unwear(_s: Shell) {}
