//! Channel actions: open (incl. deep link + session restore), create/rename/
//! delete/swap/restore. Cross-calls: `open_channel_at` dispatches into
//! [`super::message`] for the per-channel sync setup; channel reorders defer to
//! [`super::guild::open_server`] for the post-reorder reload.

use super::super::Shell;
use crate::protocol::ChannelSummary;

#[cfg(feature = "hydrate")]
use super::guild::{KEY_CHANNEL, KEY_DRAFTS, KEY_SERVER};
#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

#[cfg(feature = "hydrate")]
pub fn open_channel(s: Shell, ch: ChannelSummary) {
    open_channel_at(s, ch, None);
}

/// Load the persisted per-channel drafts (channel id -> text) from
/// localStorage. Called once when the [`super::super::Composer`] is built so
/// drafts survive a reload / PWA close.
#[cfg(feature = "hydrate")]
pub fn load_drafts() -> std::collections::HashMap<String, String> {
    LocalStorage::get(KEY_DRAFTS).unwrap_or_default()
}

/// Save the open channel's in-progress composer `text` to the in-memory map and
/// persist the whole map to localStorage. Empty text removes the entry (so a
/// cleared composer or sent message drops the draft). No-op if no channel is
/// open.
#[cfg(feature = "hydrate")]
pub fn save_draft(s: Shell, text: &str) {
    // While editing an existing message in the composer, the compose box holds
    // the edit text, not a draft — don't persist it over the channel's real
    // draft (which is restored when the edit is saved or cancelled).
    if s.composer.editing.get_untracked().is_some() {
        return;
    }
    let Some(cid) = s.sel.sel_channel.get_untracked().map(|c| c.id) else {
        return;
    };
    s.composer.drafts.update(|m| {
        if text.is_empty() {
            m.remove(&cid);
        } else {
            m.insert(cid, text.to_string());
        }
    });
    let map = s.composer.drafts.get_untracked();
    let _ = LocalStorage::set(KEY_DRAFTS, &map);
}

/// Like [`open_channel`] but, after the first page loads, asks the scroll
/// Effect to bring message `anchor` into view — for the notification
/// deep-link. The jump only lands if the target is on the newest page.
#[cfg(feature = "hydrate")]
pub fn open_channel_at(s: Shell, ch: ChannelSummary, anchor: Option<String>) {
    use super::super::Pane;
    let cid = ch.id.clone();
    let kind = ch.kind.clone();
    // Re-opening the channel you're already on (e.g. returning from the
    // Wardrobe pane) must NOT reset the worn persona from the server — a
    // just-worn value could be clobbered by a stale read before its write
    // commits. Only adopt the server's remembered persona when SWITCHING
    // to a different channel.
    let same_channel = s.sel.sel_channel.get_untracked().map(|c| c.id) == Some(cid.clone());
    // Per-channel draft scoping: when actually switching channels, restore the
    // incoming channel's saved draft (feedback fvffwu / fkqdtp). The outgoing
    // channel's text is already in `drafts` — `save_draft` keeps the map current
    // on every keystroke — so no stash is needed here. Client-only.
    if !same_channel {
        let restored = s
            .composer
            .drafts
            .get_untracked()
            .get(&cid)
            .cloned()
            .unwrap_or_default();
        s.composer.compose.set(restored);
        // The reply target is channel-scoped (the parent must be in THIS
        // channel); drop it when actually switching so a reply doesn't carry
        // over to a channel where its parent doesn't live (L-3).
        s.composer.replying_to.set(None);
        // Abandon any in-progress message edit: it targets a message in the
        // outgoing channel, and `compose` was just overwritten with the
        // incoming channel's draft. The outgoing draft is untouched (edits
        // never persist), so nothing is lost.
        s.composer.editing.set(None);
    }
    // Opening a channel auto-dismisses the wardrobe popup (F-2): navigating to
    // a channel should leave nothing overlaying it.
    s.sync.wardrobe_open.set(false);
    let _ = LocalStorage::set(KEY_CHANNEL, &cid);
    s.sel.sel_channel.set(Some(ch));
    if kind == "lorebook" {
        s.sync.pane.set(Pane::Lorebook);
        super::message::load_lore(s, cid);
    } else {
        s.sync.pane.set(Pane::Channel);
        s.msg.messages.set(Vec::new());
        s.msg.cursor.set(None);
        s.msg.oldest.set(None);
        s.msg.loading_older.set(false);
        s.msg.more_history.set(true);
        s.msg.anchor_to.set(None);
        s.msg.seen.update(|h| h.clear());
        // First page is now in flight: show loading skeletons until it lands
        // (the spawned task below clears this on every exit path) (F-7).
        s.msg.loading_initial.set(true);
        // Drop the previous channel's typing indicator at once; the poll
        // repopulates it from the new channel's response.
        s.msg.typing.set(Vec::new());
        // Opening clears the unread glow, the ping glow, and the count badge at
        // once (L-4); the high-water mark advances once messages load below.
        s.notify.unread.update(|u| {
            u.remove(&cid);
        });
        s.notify.pinged.update(|p| {
            p.remove(&cid);
        });
        s.notify.unread_count.update(|c| {
            c.remove(&cid);
        });
        // Ask the SW to close any tray notifications for this channel so a
        // burst of stacked notifs disappears once the user lands on the
        // channel that produced them (feedback row kx24k2cwftdppidhmh0e).
        super::notify::clear_notifs_for_channel(&cid);
        // Capture the prior last-seen mark BEFORE the page load advances it, so
        // we can jump to the OLDEST unread message on open (L-4). An explicit
        // `anchor` (a notification deep-link) always wins over the unread jump.
        let prior_seen = s.notify.last_seen.with_untracked(|m| m.get(&cid).cloned());
        super::message::start_poll(s);
        let seen_cid = cid.clone();
        spawn_local(async move {
            if let Ok(l) = api::list_messages(&cid, None).await {
                // Stale-guard: if the user switched channels while this initial
                // page was in flight, drop it so we don't paint the previous
                // channel's messages under the new header (feedback gwiif7xy).
                // The newer switch owns `loading_initial` now, so leave it.
                if s.sel.sel_channel.get_untracked().map(|c| c.id) != Some(cid.clone()) {
                    return;
                }
                // First page landed: drop the loading skeletons (F-7). Cleared
                // before `ingest` so the skeleton predicate and the real rows
                // never both render for a frame.
                s.msg.loading_initial.set(false);
                // The initial page is the NEWEST messages (ASC); remember the
                // oldest of it as the scroll-up cursor, and whether a full page
                // came back (i.e. older history may exist).
                let oldest = l
                    .messages
                    .first()
                    .map(|m| (m.sent_at.clone(), m.id.clone()));
                let full_page = l.messages.len() == super::message::MESSAGES_PAGE_LIMIT;
                // The worn persona is per-channel: restore this channel's
                // remembered value (or None = speak as account) when SWITCHING
                // channels; preserve a just-worn value on same-channel re-open.
                if !same_channel {
                    s.social.active_persona.set(l.active_persona);
                }
                super::message::ingest(s, l.messages);
                s.msg.oldest.set(oldest);
                s.msg.more_history.set(full_page);
                // Deep-link: now the page is in the DOM, ask the scroll
                // Effect to bring the notified message into view. An explicit
                // deep-link anchor wins; otherwise jump to the OLDEST unread
                // message — the first one strictly past the prior last-seen
                // composite cursor (L-4). String-tuple compare matches the
                // cursor's strict (sent_at, id) tie-break, same as `hydrate_last_seen`.
                let jump = anchor.or_else(|| {
                    let prior = prior_seen.as_ref()?;
                    s.msg.messages.with_untracked(|msgs| {
                        msgs.iter()
                            .find(|m| (m.sent_at.clone(), m.id.clone()) > *prior)
                            .map(|m| m.id.clone())
                    })
                });
                if let Some(mid) = jump {
                    s.msg.anchor_to.set(Some(mid));
                }
                if let Some(cur) = s.msg.cursor.get_untracked() {
                    super::message::set_last_seen(s, &seen_cid, cur);
                }
            } else if s.sel.sel_channel.get_untracked().map(|c| c.id) == Some(cid.clone()) {
                // First-page fetch failed and we're still on this channel: drop
                // the skeletons so the pane doesn't shimmer forever (F-7). A
                // newer switch already owns the flag, so only clear it for ours.
                s.msg.loading_initial.set(false);
            }
        });
    }
}

/// Open a channel from a notification deep-link: fetch its guild, select
/// it, then open the channel and (via `open_channel_at`) jump to the
/// notified message. Mirrors `restore_session`'s guild→channel resolution.
#[cfg(feature = "hydrate")]
pub fn open_deep_link(s: Shell, gid: String, cid: String, message: Option<String>) {
    spawn_local(async move {
        let Ok(d) = api::get_guild(&gid).await else {
            return;
        };
        let _ = LocalStorage::set(KEY_SERVER, &gid);
        s.sel.sel_server.set(Some(gid.clone()));
        s.sel.sel_owner.set(Some(d.owner_id.clone()));
        s.sel.channels.set(d.channels.clone());
        super::emoji::refresh_guild_emoji(s, gid.clone());
        if let Some(ch) = d.channels.iter().find(|c| c.id == cid).cloned() {
            open_channel_at(s, ch, message);
        }
    });
}

/// Restore the last server / channel / worn persona from localStorage.
///
/// Runs after `refresh_guilds` on mount. Returns `true` if a server was
/// restored, so the caller can leave the Friends pane as the default only
/// when there was nothing to restore. The whole restore is one spawned task
/// so it never races the default `open_server` path (it doesn't call it):
/// it fetches the guild itself, sets `sel_owner` + `channels`, then opens
/// the *specific* stored channel (falling back to the first text channel,
/// then any channel) via the existing `open_channel`.
#[cfg(feature = "hydrate")]
pub fn restore_session(s: Shell) -> bool {
    let Ok(gid) = LocalStorage::get::<String>(KEY_SERVER) else {
        return false;
    };
    let stored_channel = LocalStorage::get::<String>(KEY_CHANNEL).ok();

    spawn_local(async move {
        let Ok(d) = api::get_guild(&gid).await else {
            // The stored server is gone — drop the stale keys and bail.
            LocalStorage::delete(KEY_SERVER);
            LocalStorage::delete(KEY_CHANNEL);
            return;
        };
        s.sel.sel_server.set(Some(gid.clone()));
        s.sel.sel_owner.set(Some(d.owner_id.clone()));
        s.sel.channels.set(d.channels.clone());
        super::emoji::refresh_guild_emoji(s, gid.clone());

        // Prefer the stored channel; fall back to the first text channel,
        // then to the first channel of any kind (matches `open_server`).
        let target = stored_channel
            .as_deref()
            .and_then(|cid| d.channels.iter().find(|c| c.id == cid))
            .or_else(|| d.channels.iter().find(|c| c.kind == "text"))
            .or_else(|| d.channels.first())
            .cloned();
        // The worn persona is per-channel now — `open_channel` restores it
        // from the channel's `list_messages` response, so no global restore.
        if let Some(ch) = target {
            open_channel(s, ch);
        }
    });
    true
}

#[cfg(feature = "hydrate")]
pub fn create_channel(s: Shell, name: String, kind: String) {
    let Some(gid) = s.sel.sel_server.get_untracked() else {
        return;
    };
    if name.trim().is_empty() {
        return;
    }
    spawn_local(async move {
        match api::create_channel(&gid, &name, &kind).await {
            Ok(_) => super::guild::open_server(s, gid),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn rename_channel(s: Shell, gid: String, cid: String, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    spawn_local(async move {
        match api::patch_channel(&gid, &cid, &name).await {
            Ok(()) => {
                s.sel.channels.update(|cs| {
                    if let Some(c) = cs.iter_mut().find(|c| c.id == cid) {
                        c.name = name.clone();
                    }
                });
                s.sel.sel_channel.update(|sc| {
                    if let Some(c) = sc {
                        if c.id == cid {
                            c.name = name.clone();
                        }
                    }
                });
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Delete a channel (owner only). On success, clear the selection if it was
/// the open channel and reload the server so the sidebar drops the dead row.
#[cfg(feature = "hydrate")]
pub fn delete_channel(s: Shell, gid: String, cid: String) {
    use super::super::Pane;
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::delete_channel(&gid, &cid).await {
            Ok(()) => {
                if s.sel.sel_channel.get_untracked().map(|c| c.id).as_deref() == Some(cid.as_str())
                {
                    s.sel.sel_channel.set(None);
                    s.sync.pane.set(Pane::Friends);
                }
                super::guild::open_server(s, gid);
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Reorder a channel within the open guild's sidebar list. `idx` indexes
/// `s.sel.channels` (already position-sorted from the server). We swap it with
/// its neighbor, renumber the whole list to its array index, and PATCH every
/// channel whose `position` changed. Renumbering (rather than swapping two
/// values) keeps the list gap-free and stable even though existing channels
/// all start at position 0. Mirrors `swap_persona`. Owner-gated in the UI.
#[cfg(feature = "hydrate")]
pub fn swap_channel(s: Shell, idx: usize, up: bool) {
    let Some(gid) = s.sel.sel_server.get_untracked() else {
        return;
    };
    let mut list = s.sel.channels.get_untracked();
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
    persist_channel_order(s, gid, list);
}

/// Move a channel to an absolute `target` index in the open guild's sidebar
/// list (drag-and-drop drop target). Removes the dragged channel from `idx` and
/// re-inserts it at `target`, then renumbers + PATCHes exactly like
/// [`swap_channel`]. No-op when `idx == target` or either is out of range.
/// Owner-gated in the UI; the server re-checks `require_manager` per PATCH.
#[cfg(feature = "hydrate")]
pub fn move_channel(s: Shell, idx: usize, target: usize) {
    let Some(gid) = s.sel.sel_server.get_untracked() else {
        return;
    };
    let mut list = s.sel.channels.get_untracked();
    if idx >= list.len() || target >= list.len() || idx == target {
        return;
    }
    let item = list.remove(idx);
    list.insert(target, item);
    persist_channel_order(s, gid, list);
}

/// Bring a channel to the very top (`top = true`) or bottom of the sidebar
/// list — the mobile / keyboard fallback for drag. Defers to [`move_channel`].
#[cfg(feature = "hydrate")]
pub fn move_channel_to_bounds(s: Shell, idx: usize, top: bool) {
    let len = s.sel.channels.get_untracked().len();
    if len == 0 {
        return;
    }
    let target = if top { 0 } else { len - 1 };
    move_channel(s, idx, target);
}

/// Shared tail of the channel reorders: optimistically set the new local order,
/// PATCH every channel whose stored position no longer matches its index, then
/// reload the server to confirm. Factored out of [`swap_channel`]'s body so the
/// drag / bounds helpers reuse the exact same persist flow (invariant: renumber
/// to array index, never swap raw position values).
#[cfg(feature = "hydrate")]
fn persist_channel_order(s: Shell, gid: String, list: Vec<ChannelSummary>) {
    s.sel.channels.set(list.clone());
    let patches: Vec<(String, i64)> = list
        .iter()
        .enumerate()
        .filter(|(i, c)| c.position != *i as i64)
        .map(|(i, c)| (c.id.clone(), i as i64))
        .collect();
    if patches.is_empty() {
        return;
    }
    spawn_local(async move {
        for (cid, pos) in patches {
            if let Err(e) = api::set_channel_position(&gid, &cid, pos).await {
                s.composer.status.set(api::humanize(&e));
                break;
            }
        }
        super::guild::open_server(s, gid);
    });
}

/// Restore a soft-deleted channel (owner/admin). On success, reload the
/// server so the channel reappears in the sidebar, and refresh the deleted list.
#[cfg(feature = "hydrate")]
pub fn restore_channel(s: Shell, gid: String, cid: String) {
    spawn_local(async move {
        match api::restore_channel(&gid, &cid).await {
            Ok(()) => {
                super::message::load_deleted_channels(s, gid.clone());
                super::guild::open_server(s, gid);
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

// ---- ssr stubs (allow dead_code: some are only reached via hydrate-gated view
// branches; the unconditional `pub use` in act/mod.rs needs them to exist) ----

#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn open_channel(_s: Shell, _ch: ChannelSummary) {}
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn load_drafts() -> std::collections::HashMap<String, String> {
    Default::default()
}
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn save_draft(_s: Shell, _text: &str) {}
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn open_channel_at(_s: Shell, _ch: ChannelSummary, _anchor: Option<String>) {}
#[cfg(not(feature = "hydrate"))]
pub fn open_deep_link(_s: Shell, _gid: String, _cid: String, _message: Option<String>) {}
#[cfg(not(feature = "hydrate"))]
pub fn restore_session(_s: Shell) -> bool {
    false
}
#[cfg(not(feature = "hydrate"))]
pub fn create_channel(_s: Shell, _name: String, _kind: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn rename_channel(_s: Shell, _gid: String, _cid: String, _name: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn delete_channel(_s: Shell, _gid: String, _cid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn swap_channel(_s: Shell, _idx: usize, _up: bool) {}
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn move_channel(_s: Shell, _idx: usize, _target: usize) {}
#[cfg(not(feature = "hydrate"))]
pub fn move_channel_to_bounds(_s: Shell, _idx: usize, _top: bool) {}
#[cfg(not(feature = "hydrate"))]
pub fn restore_channel(_s: Shell, _gid: String, _cid: String) {}
