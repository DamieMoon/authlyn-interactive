//! L-5: the unified channel-management window — a single [`Modal`] that lets a
//! guild owner/admin create, rename, delete, and **reorder** channels in one
//! place (opened from the sidebar's "⚙ Manage" button).
//!
//! Reorder offers two cooperating paths so it works everywhere:
//! - **Drag-and-drop** (HTML5) on a coarse-free desktop pointer: each row is
//!   `draggable`; `dragstart` records the row index in a signal, `dragover`
//!   `prevent_default`s to mark a valid drop target, and `drop` calls
//!   [`act::move_channel`] with the (from, to) indices. We carry the index in a
//!   local signal rather than the `DataTransfer` payload so no extra web-sys
//!   feature / clipboard plumbing is needed.
//! - **Buttons** for touch / keyboard: ↑ / ↓ (swap a neighbour) plus
//!   ⤒ bring-to-top / ⤓ bring-to-bottom ([`act::move_channel_to_bounds`]).
//!
//! Every reorder/rename/delete drives the existing owner-gated server routes
//! (`PATCH`/`DELETE /guilds/{id}/channels/{cid}`); the server re-validates
//! `require_manager` (which also rejects a soft-deleted guild) on each call, so
//! this view never trusts its own gating.
//!
//! The rename row reuses [`InlineRename`]; the modal reuses the shared
//! [`Modal`] (Esc / backdrop / ✕ close).

use leptos::prelude::*;

use super::super::{act, PendingDelete, Shell};
use crate::protocol::ChannelSummary;
use crate::ui::inline_rename::InlineRename;
use crate::ui::modal::Modal;

#[cfg(feature = "hydrate")]
use leptos::ev::DragEvent;

/// The channel-management modal. `open` is the caller-owned visibility signal;
/// the modal clears it on backdrop/Esc/✕. Channels are read live from
/// `s.sel.channels` (already position-sorted by the server).
#[component]
pub fn ChannelManagerModal(s: Shell, open: RwSignal<bool>) -> impl IntoView {
    // Inline-rename target (which channel id, if any) and the new-channel
    // creator buffers — scoped to this modal so they reset when it closes.
    let editing = RwSignal::new(None::<String>);
    let new_name = RwSignal::new(String::new());
    let new_kind = RwSignal::new("text".to_string());
    // The index of the row currently being dragged (HTML5 DnD). `None` between
    // drags. Set on `dragstart`, read on `drop`, cleared on `dragend`/`drop`.
    let drag_from = RwSignal::new(None::<usize>);

    view! {
        <Modal class="channel-manager" close=move || open.set(false)>
            <div class="manager-head">
                <h3>"Manage channels"</h3>
                <button class="row-edit" title="close"
                    on:click=move |_| open.set(false)>"✕"</button>
            </div>

            <ul class="manager-list">
                {move || {
                    let chans = s.sel.channels.get();
                    let len = chans.len();
                    chans.into_iter().enumerate().map(|(idx, c)| {
                        view! {
                            <ManagerRow s=s ch=c idx=idx len=len
                                editing=editing drag_from=drag_from/>
                        }
                    }).collect_view()
                }}
            </ul>

            // New-channel creator: kind picker + name + Create (mirrors the
            // standalone creator dialog, kept here so management is one window).
            <div class="manager-create">
                <div class="creator-kind">
                    <label class="pref-row">
                        <input type="radio" name="mgr-ch-kind" value="text"
                            prop:checked=move || new_kind.get() == "text"
                            on:change=move |_| new_kind.set("text".to_string())/>
                        <span>"# Text"</span>
                    </label>
                    <label class="pref-row">
                        <input type="radio" name="mgr-ch-kind" value="lorebook"
                            prop:checked=move || new_kind.get() == "lorebook"
                            on:change=move |_| new_kind.set("lorebook".to_string())/>
                        <span>"📖 Lorebook"</span>
                    </label>
                </div>
                <div class="manager-create-row">
                    <input class="creator-name" prop:value=move || new_name.get()
                        on:input=move |ev| new_name.set(event_target_value(&ev))
                        placeholder="new channel name"/>
                    <button class="account-save" on:click=move |_| {
                        let name = new_name.get_untracked();
                        if name.trim().is_empty() {
                            return;
                        }
                        let kind = new_kind.get_untracked();
                        new_name.set(String::new());
                        new_kind.set("text".to_string());
                        act::create_channel(s, name, kind);
                    }>"Create"</button>
                </div>
            </div>
        </Modal>
    }
}

/// One row in the manager list: a drag handle + name (or inline-rename input),
/// the reorder buttons (↑ ↓ ⤒ ⤓), rename (✎) and delete (🗑). The whole row is
/// `draggable`; the drop target is the row the pointer is over.
#[component]
fn ManagerRow(
    s: Shell,
    ch: ChannelSummary,
    idx: usize,
    len: usize,
    editing: RwSignal<Option<String>>,
    drag_from: RwSignal<Option<usize>>,
) -> impl IntoView {
    // `idx`/`len`/`drag_from` feed handlers/`disabled` closures the view! macro
    // strips on ssr — silence the unused warnings (mirrors ChannelRow).
    let _ = (idx, len, drag_from);
    let cid = ch.id.clone();
    let name0 = ch.name.clone();
    let sigil = if ch.kind == "lorebook" { "📖 " } else { "# " };

    // Drag handlers (hydrate-only — DnD is a no-op on ssr). `dragstart` records
    // this row's index; `dragover` allows the drop; `drop` performs the move.
    #[cfg(feature = "hydrate")]
    let on_dragstart = move |_ev: DragEvent| drag_from.set(Some(idx));
    #[cfg(feature = "hydrate")]
    let on_dragover = move |ev: DragEvent| ev.prevent_default();
    #[cfg(feature = "hydrate")]
    let on_drop = move |ev: DragEvent| {
        ev.prevent_default();
        if let Some(from) = drag_from.get_untracked() {
            act::move_channel(s, from, idx);
        }
        drag_from.set(None);
    };
    #[cfg(feature = "hydrate")]
    let on_dragend = move |_ev: DragEvent| drag_from.set(None);

    view! {
        <li class="manager-row" draggable="true"
            on:dragstart=move |_ev| {
                #[cfg(feature = "hydrate")] on_dragstart(_ev);
            }
            on:dragover=move |_ev| {
                #[cfg(feature = "hydrate")] on_dragover(_ev);
            }
            on:drop=move |_ev| {
                #[cfg(feature = "hydrate")] on_drop(_ev);
            }
            on:dragend=move |_ev| {
                #[cfg(feature = "hydrate")] on_dragend(_ev);
            }>
            <span class="manager-grip" title="Drag to reorder" aria-hidden="true">"⠿"</span>
            {
                let cid = cid.clone();
                let name0 = name0.clone();
                move || {
                    let cid = cid.clone();
                    let name0 = name0.clone();
                    if editing.get().as_deref() == Some(cid.as_str()) {
                        let save_cid = cid.clone();
                        view! {
                            <InlineRename
                                value=name0.clone()
                                on_save=move |v| {
                                    if let Some(gid) = s.sel.sel_server.get_untracked() {
                                        act::rename_channel(s, gid, save_cid.clone(), v);
                                    }
                                    editing.set(None);
                                }
                                on_cancel=move || editing.set(None)
                            />
                        }.into_any()
                    } else {
                        view! {
                            <span class="manager-name">{sigil}{name0.clone()}</span>
                        }.into_any()
                    }
                }
            }
            <div class="manager-row-actions">
                <button class="channel-reorder" title="Move up"
                    disabled=move || idx == 0
                    on:click=move |_| act::swap_channel(s, idx, true)>"↑"</button>
                <button class="channel-reorder" title="Move down"
                    disabled=move || idx == len.saturating_sub(1)
                    on:click=move |_| act::swap_channel(s, idx, false)>"↓"</button>
                <button class="channel-reorder" title="Bring to top"
                    disabled=move || idx == 0
                    on:click=move |_| act::move_channel_to_bounds(s, idx, true)>"⤒"</button>
                <button class="channel-reorder" title="Bring to bottom"
                    disabled=move || idx == len.saturating_sub(1)
                    on:click=move |_| act::move_channel_to_bounds(s, idx, false)>"⤓"</button>
                <button class="row-edit" title="rename channel" on:click={
                    let cid = cid.clone();
                    move |_| editing.set(Some(cid.clone()))
                }>"✎"</button>
                <button class="row-edit danger" title="delete channel" on:click={
                    let del_cid = cid.clone();
                    let del_name = name0.clone();
                    move |_| {
                        if let Some(gid) = s.sel.sel_server.get_untracked() {
                            act::ask_delete(
                                s,
                                format!(
                                    "Delete the channel “{del_name}” and all its \
                                     messages? This cannot be undone."
                                ),
                                PendingDelete::Channel { gid, cid: del_cid.clone() },
                            );
                        }
                    }
                }>"🗑"</button>
            </div>
        </li>
    }
}
