//! Message + chat-related actions. This is the largest of the act submodules:
//!
//! - Compose: [`send_message`], [`add_compose_attachment`],
//!   [`remove_compose_attachment`].
//! - Edits + deletes: [`edit_message`], [`delete_message`],
//!   [`restore_deleted_message`], [`load_deleted_messages`].
//! - Background sync: [`start_poll`] + [`start_sync`] (idempotent wrapper),
//!   [`refresh_lists`], [`refresh_unread`], [`sync_messages`], [`ingest`],
//!   [`unseen`].
//! - Three-cursor pagination: [`load_older`] using `cursor` / `oldest` /
//!   `last_seen` with the `seen` HashSet dedupe across all paths.
//! - Mute + last-seen marks: [`load_muted`], [`toggle_mute`],
//!   [`load_last_seen`], [`set_last_seen`].
//! - The destructive-action queue: [`ask_delete`], [`cancel_delete`],
//!   [`confirm_delete`] — `PendingDelete` is data, not a closure, dispatched
//!   by match here.
//! - Pane switchers + friends + lorebook + invites + deleted-channel loads —
//!   parked here rather than spawning a dedicated submodule.
//!
//! Cross-submodule calls: [`confirm_delete`] dispatches into
//! [`super::guild::delete_server`], [`super::channel::delete_channel`],
//! [`super::persona::remove_persona`], and `delete_message` here; the poll loop
//! calls [`super::notify::notify_messages`].

use super::super::{PendingDelete, Shell};

#[cfg(feature = "hydrate")]
use super::super::Pane;
#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use crate::protocol::MessageEnvelope;
#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Server page size for messages (mirrors `MESSAGES_PAGE_LIMIT` on the
/// server). Below this, the whole channel is loaded in one page and can be
/// reconciled wholesale; at/above it we only append.
#[cfg(feature = "hydrate")]
pub(super) const MESSAGES_PAGE_LIMIT: usize = 100;

// ---- compose ----

#[cfg(feature = "hydrate")]
pub fn send_message(s: Shell) {
    let Some(ch) = s.sel.sel_channel.get_untracked() else {
        return;
    };
    let body = s.composer.compose.get_untracked();
    // The wire SEND request is ids-only; map the staged attachments down,
    // keeping only the ones whose upload has finished (`Ready`) — in-flight or
    // failed slots carry a placeholder id, not a real media id (F-8).
    let attachments: Vec<String> = s
        .composer
        .compose_attachments
        .get_untracked()
        .into_iter()
        .filter(|a| a.status == super::super::state::UploadStatus::Ready)
        .map(|a| a.att.id)
        .collect();
    // A message needs text OR at least one attachment.
    if body.trim().is_empty() && attachments.is_empty() {
        return;
    }
    s.composer.compose.set(String::new());
    // Drop the now-sent channel's persisted draft (removes the key + persists).
    super::channel::save_draft(s, "");
    s.composer.compose_attachments.set(Vec::new());
    s.composer.status.set(String::new());
    // Sending is a user gesture — a reliable point to request notification
    // permission so background channels can notify later.
    super::notify::request_notify_permission();
    // Carry the persona worn in THIS channel so attribution is decided at
    // send time (race-proof) rather than depending on a separately-written
    // per-channel row having committed.
    let persona = s.social.active_persona.get_untracked();
    spawn_local(async move {
        match api::post_message(&ch.id, &body, attachments, persona).await {
            Ok(_) => {
                let cur = s.msg.cursor.get_untracked();
                if let Ok(l) = api::list_messages(&ch.id, cur.as_ref()).await {
                    ingest(s, l.messages);
                    // FB10a: advance this channel's last-seen to the new
                    // cursor so `refresh_unread` doesn't glow the channel
                    // for the user's OWN just-sent message.
                    if let Some(cur) = s.msg.cursor.get_untracked() {
                        set_last_seen(s, &ch.id, cur);
                    }
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Upload a picked/pasted image or video and stage it as a pending composer
/// attachment (its media id is sent with the next message). The browser's
/// reported MIME (`file.type_()`) is kept locally so the pending thumbnail
/// renders image-vs-video correctly before the message round-trips.
///
/// F-8: the slot is inserted immediately in `Uploading` state and its progress
/// bar is driven from `upload_media_with_progress`; on success it flips to
/// `Ready` (real media id), on failure to `Failed` with a retry button.
#[cfg(feature = "hydrate")]
pub fn add_compose_attachment(s: Shell, file: web_sys::File) {
    s.composer.status.set(String::new());
    let key = next_stage_key();
    let mime = file.type_();
    stash_retry_file(key, file.clone());
    s.composer.compose_attachments.update(|v| {
        v.push(super::super::state::StagedAttachment {
            key,
            att: crate::protocol::Attachment {
                id: format!("pending-{key}"),
                mime,
            },
            status: super::super::state::UploadStatus::Uploading(Some(0.0)),
        });
    });
    spawn_local(async move { upload_staged(s, key, file).await });
}

/// Upload a batch of picked files and stage them **in pick order**. The slots
/// are inserted up front in `files` order so the row layout is stable; each
/// upload then drives its own slot's progress independently (F-8). Concurrency
/// is left to the browser's connection pool. Replaces the prior `join_all`
/// reorder fix (feedback mnjs2ljw…) — order is now guaranteed by inserting the
/// slots before any upload completes, not by awaiting in order.
#[cfg(feature = "hydrate")]
pub fn add_compose_attachments(s: Shell, files: Vec<web_sys::File>) {
    if files.is_empty() {
        return;
    }
    s.composer.status.set(String::new());
    // Insert all slots first (pick order), then kick off the uploads.
    let staged: Vec<(u64, web_sys::File)> = files
        .into_iter()
        .map(|f| {
            let key = next_stage_key();
            let mime = f.type_();
            stash_retry_file(key, f.clone());
            s.composer.compose_attachments.update(|v| {
                v.push(super::super::state::StagedAttachment {
                    key,
                    att: crate::protocol::Attachment {
                        id: format!("pending-{key}"),
                        mime,
                    },
                    status: super::super::state::UploadStatus::Uploading(Some(0.0)),
                });
            });
            (key, f)
        })
        .collect();
    for (key, f) in staged {
        spawn_local(async move { upload_staged(s, key, f).await });
    }
}

/// Run one staged attachment's upload, writing progress into its slot and
/// flipping it to `Ready`/`Failed` on completion (F-8). Shared by the single,
/// batch, and retry entry points.
#[cfg(feature = "hydrate")]
async fn upload_staged(s: Shell, key: u64, file: web_sys::File) {
    let result = api::upload_media_with_progress(&file, move |frac| {
        set_stage_status(
            s,
            key,
            super::super::state::UploadStatus::Uploading(Some(frac)),
        );
    })
    .await;
    match result {
        Ok(id) => {
            forget_retry_file(key);
            s.composer.compose_attachments.update(|v| {
                if let Some(it) = v.iter_mut().find(|it| it.key == key) {
                    it.att.id = id;
                    it.status = super::super::state::UploadStatus::Ready;
                }
            });
        }
        Err(e) => set_stage_status(
            s,
            key,
            super::super::state::UploadStatus::Failed(api::humanize(&e)),
        ),
    }
}

/// Re-attempt a failed staged upload using the file stashed at stage time
/// (F-8). No-op if the slot or file is gone (already removed).
#[cfg(feature = "hydrate")]
pub fn retry_compose_attachment(s: Shell, key: u64) {
    let Some(file) = peek_retry_file(key) else {
        return;
    };
    set_stage_status(
        s,
        key,
        super::super::state::UploadStatus::Uploading(Some(0.0)),
    );
    spawn_local(async move { upload_staged(s, key, file).await });
}

/// Write a staged attachment's status by key, if it still exists.
#[cfg(feature = "hydrate")]
fn set_stage_status(s: Shell, key: u64, status: super::super::state::UploadStatus) {
    s.composer.compose_attachments.update(|v| {
        if let Some(it) = v.iter_mut().find(|it| it.key == key) {
            it.status = status;
        }
    });
}

/// Drop one staged attachment before sending, addressed by its stage key
/// (stable across the upload lifecycle, unlike the late-arriving media id).
#[cfg(feature = "hydrate")]
pub fn remove_compose_attachment(s: Shell, key: u64) {
    forget_retry_file(key);
    s.composer
        .compose_attachments
        .update(|v| v.retain(|a| a.key != key));
}

// ---- staged-upload bookkeeping (hydrate-only, F-8) ----

#[cfg(feature = "hydrate")]
thread_local! {
    /// Monotonic counter minting unique stage keys per staged attachment.
    static STAGE_KEY: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Files held for retry, keyed by stage key. `web_sys::File` is `!Send` and
    /// hydrate-only, so it lives here rather than in the shared `Composer`
    /// state. Entries are dropped on success or removal.
    static RETRY_FILES: std::cell::RefCell<std::collections::HashMap<u64, web_sys::File>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

#[cfg(feature = "hydrate")]
fn next_stage_key() -> u64 {
    STAGE_KEY.with(|c| {
        let k = c.get().wrapping_add(1);
        c.set(k);
        k
    })
}

#[cfg(feature = "hydrate")]
fn stash_retry_file(key: u64, file: web_sys::File) {
    RETRY_FILES.with(|m| {
        m.borrow_mut().insert(key, file);
    });
}

#[cfg(feature = "hydrate")]
fn peek_retry_file(key: u64) -> Option<web_sys::File> {
    RETRY_FILES.with(|m| m.borrow().get(&key).cloned())
}

#[cfg(feature = "hydrate")]
fn forget_retry_file(key: u64) {
    RETRY_FILES.with(|m| {
        m.borrow_mut().remove(&key);
    });
}

/// Copy a message body to the clipboard as raw markup, **stripping color
/// tokens** so the receiver can re-paste it under their own persona without
/// dragging the original speaker's palette along. Foxtrot feedback row
/// 3szov1qgatobhhrc3mf2 / ctx 019e6f23-fcfc.
///
/// `navigator.clipboard.writeText` is async and reached via reflection so we
/// don't pull the `Clipboard` web-sys feature flag just for this. On failure
/// (no clipboard permission, navigator missing) the status pane surfaces a
/// short toast; on success it shows "Copied" briefly.
#[cfg(feature = "hydrate")]
pub fn copy_message_body(s: Shell, body: String) {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    let stripped = crate::markup::strip_color_tokens(&body);
    s.composer.status.set(String::new());
    spawn_local(async move {
        let promise = (|| -> Option<js_sys::Promise> {
            let win = leptos::web_sys::window()?;
            let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
            let clip = js_sys::Reflect::get(&nav, &JsValue::from_str("clipboard")).ok()?;
            let write_fn = js_sys::Reflect::get(&clip, &JsValue::from_str("writeText")).ok()?;
            let func: js_sys::Function = write_fn.dyn_into().ok()?;
            let arg = JsValue::from_str(&stripped);
            func.call1(&clip, &arg).ok()?.dyn_into().ok()
        })();
        let toast = match promise {
            Some(p) => match JsFuture::from(p).await {
                Ok(_) => "Copied",
                Err(_) => "Couldn't copy — check clipboard permission",
            },
            None => "Clipboard unavailable",
        };
        s.composer.status.set(toast.to_string());
    });
}

// ---- edit / delete / restore ----

/// Edit one of the caller's own messages, then patch `s.msg.messages` in
/// place. `ingest` only appends (dedupes by id), so an edit needs a direct
/// in-place body update — the row's id and cursor position don't change.
#[cfg(feature = "hydrate")]
pub fn edit_message(s: Shell, cid: String, mid: String, body: String) {
    let body = body.trim_end().to_string();
    if body.trim().is_empty() {
        return;
    }
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::edit_message(&cid, &mid, &body).await {
            Ok(()) => s.msg.messages.update(|v| {
                if let Some(m) = v.iter_mut().find(|m| m.id == mid) {
                    m.body = body.clone();
                }
            }),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Delete one of the caller's own messages, then drop it from `s.msg.messages`
/// and `s.msg.seen` so a subsequent catch-up poll doesn't treat it as already
/// seen (it won't reappear regardless — the server row is gone — but
/// clearing `seen` keeps the dedupe set tidy). `s.msg.cursor` is left as-is:
/// it still marks the high-water mark for the poll, so deleting a row never
/// rewinds the catch-up window.
#[cfg(feature = "hydrate")]
pub fn delete_message(s: Shell, cid: String, mid: String) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::delete_message(&cid, &mid).await {
            Ok(()) => {
                s.msg.messages.update(|v| v.retain(|m| m.id != mid));
                s.msg.seen.update(|h| {
                    h.remove(&mid);
                });
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Load soft-deleted messages for the given channel into `s.trash.deleted_messages`.
#[cfg(feature = "hydrate")]
pub fn load_deleted_messages(s: Shell, cid: String) {
    spawn_local(async move {
        match api::list_deleted_messages(&cid).await {
            Ok(r) => s.trash.deleted_messages.set(r.messages),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Restore one of the caller's own deleted messages. On success, remove it
/// from the trash list and reload the channel messages.
#[cfg(feature = "hydrate")]
pub fn restore_deleted_message(s: Shell, cid: String, mid: String) {
    spawn_local(async move {
        match api::restore_message(&cid, &mid).await {
            Ok(()) => {
                // Drop from the trash list immediately (no re-load needed).
                s.trash
                    .deleted_messages
                    .update(|v| v.retain(|m| m.id != mid));
                // Reload channel messages so the restored one reappears.
                if let Ok(l) = api::list_messages(&cid, None).await {
                    s.msg.messages.set(l.messages.clone());
                    s.msg.seen.update(|h| {
                        h.clear();
                        for m in &l.messages {
                            h.insert(m.id.clone());
                        }
                    });
                    s.msg
                        .cursor
                        .set(l.messages.last().map(|m| (m.sent_at.clone(), m.id.clone())));
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Load soft-deleted channels for the given guild into `s.trash.deleted_channels`.
#[cfg(feature = "hydrate")]
pub fn load_deleted_channels(s: Shell, gid: String) {
    spawn_local(async move {
        match api::list_deleted_channels(&gid).await {
            Ok(r) => s.trash.deleted_channels.set(r.channels),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

// ---- destructive-action confirmation ----

/// Queue a destructive action behind the top-level confirm modal: stash the
/// action plus its human prompt. The modal dispatches it via `confirm_delete`.
#[cfg(feature = "hydrate")]
pub fn ask_delete(s: Shell, prompt: String, pending: PendingDelete) {
    s.modals.confirm_prompt.set(Some(prompt));
    s.modals.pending_delete.set(Some(pending));
}

/// Clear a pending confirm without acting (Cancel / backdrop).
#[cfg(feature = "hydrate")]
pub fn cancel_delete(s: Shell) {
    s.modals.pending_delete.set(None);
    s.modals.confirm_prompt.set(None);
}

/// Run the pending destructive action (the modal's "Delete"), then clear it.
#[cfg(feature = "hydrate")]
pub fn confirm_delete(s: Shell) {
    let pending = s.modals.pending_delete.get_untracked();
    cancel_delete(s);
    match pending {
        Some(PendingDelete::Message { cid, mid }) => delete_message(s, cid, mid),
        Some(PendingDelete::Channel { gid, cid }) => super::channel::delete_channel(s, gid, cid),
        Some(PendingDelete::Server { gid }) => super::guild::delete_server(s, gid),
        Some(PendingDelete::Persona { pid }) => super::persona::remove_persona(s, pid),
        None => {}
    }
}

// ---- pane switchers ----

#[cfg(feature = "hydrate")]
pub fn show_friends(s: Shell) {
    s.sync.pane.set(Pane::Friends);
    reload_friends(s);
}

#[cfg(feature = "hydrate")]
pub fn show_wardrobe(s: Shell) {
    s.sync.pane.set(Pane::Wardrobe);
    spawn_local(async move {
        if let Ok(r) = api::list_personas().await {
            s.social.personas.set(r.personas);
        }
    });
}

/// Open the per-guild custom-emoji manager. The list is already kept fresh
/// in `s.sel.guild_emoji` (loaded when the guild opens, refreshed on each
/// create/delete), so this only flips the pane.
#[cfg(feature = "hydrate")]
pub fn show_emoji_manager(s: Shell) {
    s.sync.pane.set(Pane::Emoji);
}

/// Open the member-management pane. The roster is local to the pane and
/// fetched there on mount (an Effect keyed on the selected guild), so this
/// only flips the pane.
#[cfg(feature = "hydrate")]
pub fn show_members(s: Shell) {
    s.sync.pane.set(Pane::Members);
}

// ---- friends + member ops ----

#[cfg(feature = "hydrate")]
pub fn add_friend(s: Shell, username: String) {
    if username.trim().is_empty() {
        return;
    }
    spawn_local(async move {
        match api::add_friend(&username).await {
            Ok(()) => reload_friends(s),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn invite_member(s: Shell, gid: String, username: String) {
    let username = username.trim().to_string();
    if username.is_empty() {
        return;
    }
    spawn_local(async move {
        match api::invite_member(&gid, &username).await {
            Ok(()) => s.composer.status.set(format!("invited {username}")),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn accept_friend(s: Shell, aid: String) {
    spawn_local(async move {
        let _ = api::accept_friend(&aid).await;
        reload_friends(s);
    });
}

#[cfg(feature = "hydrate")]
pub fn remove_friend(s: Shell, aid: String) {
    spawn_local(async move {
        let _ = api::remove_friend(&aid).await;
        reload_friends(s);
    });
}

// ---- lorebook ----

#[cfg(feature = "hydrate")]
pub fn create_lore(s: Shell, cid: String, keys: Vec<String>, content: String) {
    if cid.is_empty() || content.trim().is_empty() {
        return;
    }
    spawn_local(async move {
        match api::create_lore(&cid, keys, &content).await {
            Ok(_) => load_lore(s, cid),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(feature = "hydrate")]
#[allow(clippy::too_many_arguments)]
pub fn patch_lore(
    s: Shell,
    cid: String,
    eid: String,
    title: Option<String>,
    keys: Option<Vec<String>>,
    content: Option<String>,
    enabled: Option<bool>,
    position: Option<i64>,
) {
    use crate::protocol::PatchLorebookEntryRequest;
    spawn_local(async move {
        let req = PatchLorebookEntryRequest {
            title,
            keys,
            content,
            enabled,
            position,
        };
        match api::patch_lore(&cid, &eid, &req).await {
            Ok(()) => load_lore(s, cid),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

/// Swap `eid` with the neighbor above (`up = true`) or below (`up = false`)
/// by exchanging their `position` values, then reload the list.
#[cfg(feature = "hydrate")]
pub fn swap_lore(s: Shell, cid: String, eid: String, position: i64, up: bool) {
    use crate::protocol::PatchLorebookEntryRequest;
    let entries = s.social.lore.get_untracked();
    let neighbor = if up {
        entries
            .iter()
            .filter(|e| e.position < position)
            .max_by_key(|e| e.position)
            .cloned()
    } else {
        entries
            .iter()
            .filter(|e| e.position > position)
            .min_by_key(|e| e.position)
            .cloned()
    };
    let Some(nbr) = neighbor else { return };
    let nbr_pos = nbr.position;
    let nbr_id = nbr.id.clone();
    let cid2 = cid.clone();
    spawn_local(async move {
        let r1 = api::patch_lore(
            &cid,
            &eid,
            &PatchLorebookEntryRequest {
                position: Some(nbr_pos),
                ..Default::default()
            },
        )
        .await;
        let r2 = api::patch_lore(
            &cid2,
            &nbr_id,
            &PatchLorebookEntryRequest {
                position: Some(position),
                ..Default::default()
            },
        )
        .await;
        match (r1, r2) {
            (Ok(()), Ok(())) => load_lore(s, cid),
            (Err(e), _) | (_, Err(e)) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn delete_lore(s: Shell, cid: String, eid: String) {
    spawn_local(async move {
        let _ = api::delete_lore(&cid, &eid).await;
        load_lore(s, cid);
    });
}

// ---- internal: friend + lore reloads, message dedupe ingest ----

#[cfg(feature = "hydrate")]
fn reload_friends(s: Shell) {
    spawn_local(async move {
        if let Ok(f) = api::list_friends().await {
            s.social.friends.set(f);
        }
    });
}

#[cfg(feature = "hydrate")]
pub(super) fn load_lore(s: Shell, cid: String) {
    spawn_local(async move {
        if let Ok(l) = api::list_lore(&cid).await {
            s.social.lore.set(l.entries);
        }
    });
}

/// Append messages new since the last call, deduped via `s.msg.seen`, and advance
/// `s.msg.cursor` to the latest of them. Used by the initial channel open + the
/// poll's catch-up branch + the post-send fetch. Public to siblings.
#[cfg(feature = "hydrate")]
pub(super) fn ingest(s: Shell, incoming: Vec<MessageEnvelope>) {
    for m in incoming {
        if s.msg.seen.with_untracked(|h| h.contains(&m.id)) {
            continue;
        }
        s.msg.seen.update(|h| {
            h.insert(m.id.clone());
        });
        s.msg.cursor.set(Some((m.sent_at.clone(), m.id.clone())));
        s.msg.messages.update(|v| v.push(m));
    }
}

// ---- muted channels + per-channel last-seen marks ----

// localStorage key for the muted-channel id list.
#[cfg(feature = "hydrate")]
const KEY_MUTED: &str = "authlyn.muted_channels";

/// Load muted channels from localStorage into the reactive set (on mount).
#[cfg(feature = "hydrate")]
pub fn load_muted(s: Shell) {
    let ids = LocalStorage::get::<Vec<String>>(KEY_MUTED).unwrap_or_default();
    s.notify.muted.set(ids.into_iter().collect());
}

/// localStorage key for the per-channel last-seen high-water marks (#23).
#[cfg(feature = "hydrate")]
const KEY_LAST_SEEN: &str = "authlyn.last_seen";

/// Load last-seen marks from localStorage into the reactive map (on mount).
#[cfg(feature = "hydrate")]
pub fn load_last_seen(s: Shell) {
    s.notify
        .last_seen
        .set(LocalStorage::get(KEY_LAST_SEEN).unwrap_or_default());
}

/// Record `cur = (sent_at, id)` as the last message seen in `cid`, and
/// persist the whole map. Idempotent. Public to siblings.
#[cfg(feature = "hydrate")]
pub(super) fn set_last_seen(s: Shell, cid: &str, cur: (String, String)) {
    s.notify.last_seen.update(|m| {
        m.insert(cid.to_string(), cur);
    });
    let _ = LocalStorage::set(KEY_LAST_SEEN, s.notify.last_seen.get_untracked());
}

/// Recompute the unread set across EVERY guild the caller belongs to (#23).
/// The open channel is always considered seen (advance its mark to the live
/// cursor); every other text channel is "unread" iff it has any message past
/// its last-seen mark. A never-seen channel is baselined to its current
/// latest (no retroactive glow on first sight). Runs on the ~6s list tick.
///
/// Iterating across all guilds (not only the open one) is what lets the rail
/// glow a guild button whose unread is in a non-open guild — feedback row
/// grt9ohmw8pj2fi4eqb6h.
#[cfg(feature = "hydrate")]
fn refresh_unread(s: Shell) {
    let open = s.sel.sel_channel.get_untracked().map(|c| c.id);
    if let Some(ref oc) = open {
        if let Some(cur) = s.msg.cursor.get_untracked() {
            set_last_seen(s, oc, cur);
        }
        s.notify.unread.update(|u| {
            u.remove(oc);
        });
    }
    // Flatten the per-guild channel cache into a single list (cross-guild).
    // Falls back to the open guild's `s.sel.channels` while the cache is
    // still warming so the very-first poll tick post-mount still glows the
    // open server's rows.
    let channels: Vec<crate::protocol::ChannelSummary> = {
        let cached: Vec<_> = s
            .sel
            .guild_channels
            .with_untracked(|m| m.values().flat_map(|v| v.iter().cloned()).collect());
        if cached.is_empty() {
            s.sel.channels.get_untracked()
        } else {
            cached
        }
    };
    spawn_local(async move {
        for ch in channels {
            if ch.kind != "text" || Some(&ch.id) == open.as_ref() {
                continue;
            }
            match s
                .notify
                .last_seen
                .with_untracked(|m| m.get(&ch.id).cloned())
            {
                Some(cur) => {
                    let Ok(l) = api::list_messages(&ch.id, Some(&cur)).await else {
                        continue;
                    };
                    let has_new = !l.messages.is_empty();
                    let marked = s.notify.unread.with_untracked(|u| u.contains(&ch.id));
                    if has_new != marked {
                        s.notify.unread.update(|u| {
                            if has_new {
                                u.insert(ch.id.clone());
                            } else {
                                u.remove(&ch.id);
                            }
                        });
                    }
                }
                // First sight: baseline to the current latest, don't glow.
                None => {
                    if let Ok(l) = api::list_messages(&ch.id, None).await {
                        if let Some(last) = l.messages.last() {
                            set_last_seen(s, &ch.id, (last.sent_at.clone(), last.id.clone()));
                        }
                    }
                }
            }
        }
    });
}

/// True iff any text channel in the given guild is in `notify.unread`. Pure
/// projection over `guild_channels` + `notify.unread`; called per rail button
/// from a reactive closure so the badge tracks both signals.
#[cfg(feature = "hydrate")]
pub fn guild_has_unread(s: Shell, gid: &str) -> bool {
    let channels = s.sel.guild_channels.with(|m| m.get(gid).cloned());
    let Some(channels) = channels else {
        return false;
    };
    s.notify.unread.with(|u| {
        channels
            .iter()
            .any(|c| c.kind == "text" && u.contains(&c.id))
    })
}

/// Toggle mute for a channel: flip the reactive set + persist. A click is a
/// user gesture, so it's also a good moment to ask for notification permission.
#[cfg(feature = "hydrate")]
pub fn toggle_mute(s: Shell, cid: String) {
    s.notify.muted.update(|m| {
        if !m.remove(&cid) {
            m.insert(cid.clone());
        }
    });
    let ids: Vec<String> = s
        .notify
        .muted
        .with_untracked(|m| m.iter().cloned().collect());
    let _ = LocalStorage::set(KEY_MUTED, &ids);
    super::notify::request_notify_permission();
}

// ---- background sync loop + page reconciler ----

/// The subset of `msgs` not yet in `s.msg.seen` — genuinely new this tick.
#[cfg(feature = "hydrate")]
fn unseen(s: Shell, msgs: &[MessageEnvelope]) -> Vec<MessageEnvelope> {
    msgs.iter()
        .filter(|m| !s.msg.seen.with_untracked(|h| h.contains(&m.id)))
        .cloned()
        .collect()
}

/// Full-set reconcile for a channel that fits in one page: reflects new,
/// edited, and deleted messages (including from other people), writing the
/// signal only when something actually changed so an idle poll causes no
/// re-render or scroll jump.
#[cfg(feature = "hydrate")]
fn sync_messages(s: Shell, fresh: Vec<MessageEnvelope>) {
    let changed = s.msg.messages.with_untracked(|cur| {
        cur.len() != fresh.len()
            || cur
                .iter()
                .zip(fresh.iter())
                .any(|(a, b)| a.id != b.id || a.body != b.body || a.persona_name != b.persona_name)
    });
    if !changed {
        return;
    }
    s.msg.seen.update(|h| {
        h.clear();
        for m in &fresh {
            h.insert(m.id.clone());
        }
    });
    s.msg
        .cursor
        .set(fresh.last().map(|m| (m.sent_at.clone(), m.id.clone())));
    s.msg.messages.set(fresh);
}

/// In-place refresh of the guild rail, every guild's channel list, and the
/// friends list — each written only when it changed, so things created or
/// removed elsewhere appear/disappear without a manual reload.
///
/// Fetches every guild's channels in parallel (cross-guild unread badges in
/// the rail need to know which channels live where — feedback row
/// grt9ohmw8pj2fi4eqb6h). The open guild's channels mirror into `s.sel.channels`
/// so the sidebar/channel-list selector keeps its single-source-of-truth.
/// Vanished guilds (left/deleted) are pruned from the per-guild cache.
#[cfg(feature = "hydrate")]
fn refresh_lists(s: Shell) {
    let sel = s.sel.sel_server.get_untracked();
    spawn_local(async move {
        if let Ok(r) = api::list_guilds().await {
            if s.sel.guilds.with_untracked(|g| *g != r.guilds) {
                s.sel.guilds.set(r.guilds);
            }
        }
        if let Ok(f) = api::list_friends().await {
            if s.social.friends.with_untracked(|cur| *cur != f) {
                s.social.friends.set(f);
            }
        }
        // Cross-guild channel cache: pull every guild's channels in parallel.
        // Cost is bounded by guild count (single-deployment, single-user) and
        // amortizes against the existing 4-tick poll cadence (~6s).
        let gids: Vec<String> = s
            .sel
            .guilds
            .with_untracked(|g| g.iter().map(|g| g.id.clone()).collect());
        let details = futures_util::future::join_all(
            gids.iter()
                .map(|gid| api::get_guild(gid))
                .collect::<Vec<_>>(),
        )
        .await;
        let mut next: std::collections::HashMap<String, Vec<crate::protocol::ChannelSummary>> =
            std::collections::HashMap::with_capacity(gids.len());
        for (gid, r) in gids.iter().zip(details) {
            if let Ok(d) = r {
                next.insert(gid.clone(), d.channels);
            }
        }
        // Only rewrite if something actually changed (avoid a needless
        // re-render of every rail badge each tick).
        if s.sel.guild_channels.with_untracked(|cur| *cur != next) {
            s.sel.guild_channels.set(next.clone());
        }
        if let Some(gid) = sel {
            if let Some(channels) = next.get(&gid) {
                if s.sel.channels.with_untracked(|c| *c != *channels) {
                    s.sel.channels.set(channels.clone());
                }
            }
        }
    });
}

/// Backfill older history when the user scrolls near the top: fetch the
/// page immediately before `oldest`, prepend it, and ask the channel pane
/// to re-anchor so the viewport stays put. Guarded against overlap and
/// against running past the start of history.
#[cfg(feature = "hydrate")]
pub fn load_older(s: Shell) {
    if s.msg.loading_older.get_untracked() || !s.msg.more_history.get_untracked() {
        return;
    }
    let Some(oldest) = s.msg.oldest.get_untracked() else {
        return;
    };
    let Some(ch) = s.sel.sel_channel.get_untracked() else {
        return;
    };
    s.msg.loading_older.set(true);
    spawn_local(async move {
        if let Ok(l) = api::list_messages_before(&ch.id, &oldest).await {
            if l.messages.len() < MESSAGES_PAGE_LIMIT {
                s.msg.more_history.set(false);
            }
            let fresh: Vec<_> = l
                .messages
                .into_iter()
                .filter(|m| !s.msg.seen.with_untracked(|h| h.contains(&m.id)))
                .collect();
            if !fresh.is_empty() {
                s.msg
                    .oldest
                    .set(fresh.first().map(|m| (m.sent_at.clone(), m.id.clone())));
                s.msg.seen.update(|h| {
                    for m in &fresh {
                        h.insert(m.id.clone());
                    }
                });
                // Anchor to the row currently at the top before everything
                // shifts down, so the viewport doesn't jump.
                let anchor = s
                    .msg
                    .messages
                    .with_untracked(|v| v.first().map(|m| m.id.clone()));
                s.msg.anchor_to.set(anchor);
                s.msg.messages.update(|v| {
                    let mut nw = fresh;
                    nw.append(v);
                    *v = nw;
                });
            }
        }
        s.msg.loading_older.set(false);
    });
}

/// The background sync loop (single instance, guarded by `s.sync.polling`).
/// Every tick it refreshes the open channel's messages; every ~6s it also
/// refreshes the lists. Started on shell mount via [`start_sync`] so the
/// lists stay live even on the Friends pane. SEAM: replace with SSE.
#[cfg(feature = "hydrate")]
pub(super) fn start_poll(s: Shell) {
    if s.sync.polling.get_untracked() {
        return;
    }
    s.sync.polling.set(true);
    spawn_local(async move {
        let mut tick: u32 = 0;
        loop {
            gloo_timers::future::TimeoutFuture::new(1500).await;
            tick = tick.wrapping_add(1);
            if tick.is_multiple_of(4) {
                refresh_lists(s);
                refresh_unread(s);
            }
            if s.sync.pane.get_untracked() != Pane::Channel {
                continue;
            }
            let Some(ch) = s.sel.sel_channel.get_untracked() else {
                continue;
            };
            match api::list_messages(&ch.id, None).await {
                Ok(l) if l.messages.len() < MESSAGES_PAGE_LIMIT => {
                    // Stale-guard: drop this tick's data if the channel changed
                    // while the fetch was in flight (feedback gwiif7xy).
                    if s.sel.sel_channel.get_untracked().map(|c| c.id) != Some(ch.id.clone()) {
                        continue;
                    }
                    let fresh = unseen(s, &l.messages);
                    s.msg.typing.set(l.typing);
                    sync_messages(s, l.messages);
                    super::notify::notify_messages(s, &ch, &fresh);
                }
                Ok(_) => {
                    // Long history: page 1 isn't the whole channel, so only
                    // append new messages past the cursor.
                    let cur = s.msg.cursor.get_untracked();
                    if let Ok(l) = api::list_messages(&ch.id, cur.as_ref()).await {
                        // Stale-guard: drop this tick's data if the channel
                        // changed while the fetch was in flight (feedback
                        // gwiif7xy).
                        if s.sel.sel_channel.get_untracked().map(|c| c.id) != Some(ch.id.clone()) {
                            continue;
                        }
                        let fresh = unseen(s, &l.messages);
                        s.msg.typing.set(l.typing);
                        ingest(s, l.messages);
                        super::notify::notify_messages(s, &ch, &fresh);
                    }
                }
                Err(_) => {}
            }
        }
    });
}

/// Start the background sync loop (idempotent). Called on shell mount so the
/// rail/sidebar/friends stay live before any channel is opened.
#[cfg(feature = "hydrate")]
pub fn start_sync(s: Shell) {
    start_poll(s);
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn copy_message_body(_s: Shell, _body: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn guild_has_unread(_s: Shell, _gid: &str) -> bool {
    false
}
#[cfg(not(feature = "hydrate"))]
pub fn send_message(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn add_compose_attachment(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn remove_compose_attachment(_s: Shell, _key: u64) {}
#[cfg(not(feature = "hydrate"))]
pub fn retry_compose_attachment(_s: Shell, _key: u64) {}
#[cfg(not(feature = "hydrate"))]
pub fn edit_message(_s: Shell, _cid: String, _mid: String, _body: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn delete_message(_s: Shell, _cid: String, _mid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_deleted_messages(_s: Shell, _cid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn restore_deleted_message(_s: Shell, _cid: String, _mid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_deleted_channels(_s: Shell, _gid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn ask_delete(_s: Shell, _prompt: String, _pending: PendingDelete) {}
#[cfg(not(feature = "hydrate"))]
pub fn cancel_delete(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn confirm_delete(s: Shell) {
    // Mirrors the hydrate dispatch shape so the per-action stubs stay "used".
    match None::<PendingDelete> {
        Some(PendingDelete::Message { cid, mid }) => delete_message(s, cid, mid),
        Some(PendingDelete::Channel { gid, cid }) => super::channel::delete_channel(s, gid, cid),
        Some(PendingDelete::Server { gid }) => super::guild::delete_server(s, gid),
        Some(PendingDelete::Persona { pid }) => super::persona::remove_persona(s, pid),
        None => {}
    }
}
#[cfg(not(feature = "hydrate"))]
pub fn show_friends(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn show_wardrobe(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn show_emoji_manager(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn show_members(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn add_friend(_s: Shell, _username: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn invite_member(_s: Shell, _gid: String, _username: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn accept_friend(_s: Shell, _aid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn remove_friend(_s: Shell, _aid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn create_lore(_s: Shell, _cid: String, _keys: Vec<String>, _content: String) {}
#[cfg(not(feature = "hydrate"))]
#[allow(clippy::too_many_arguments)]
pub fn patch_lore(
    _s: Shell,
    _cid: String,
    _eid: String,
    _title: Option<String>,
    _keys: Option<Vec<String>>,
    _content: Option<String>,
    _enabled: Option<bool>,
    _position: Option<i64>,
) {
}
#[cfg(not(feature = "hydrate"))]
pub fn swap_lore(_s: Shell, _cid: String, _eid: String, _position: i64, _up: bool) {}
#[cfg(not(feature = "hydrate"))]
pub fn delete_lore(_s: Shell, _cid: String, _eid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_muted(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_last_seen(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn toggle_mute(_s: Shell, _cid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn start_sync(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn load_older(_s: Shell) {}
