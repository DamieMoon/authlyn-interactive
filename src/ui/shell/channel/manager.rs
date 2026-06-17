//! L-5: the unified channel-management window — a single [`Modal`] that lets a
//! guild owner/admin create, rename, delete, and **reorder** channels in one
//! place (opened from the orbit station's "Server settings" window and, until
//! the W3 shell retires, the W3 sidebar's "⚙ Manage" button).
//!
//! Reorder is **finger-drag on the grip** (`⠿`), no ↑/↓/⤒/⤓ buttons (owner
//! directive 2026-06-17): press the grip and drag the row to its new slot. It is
//! built on Pointer Events so the one gesture covers touch, mouse, and pen:
//! - `pointerdown` on the grip records this row's index and `set_pointer_capture`s
//!   the pointer (so moves that drift off the grip keep feeding the gesture —
//!   the same capture trick `lightbox.rs` / the orbit `StripDrag` use), and
//!   `prevent_default`s so the touch reorders instead of scrolling the list (the
//!   grip also carries `touch-action: none`).
//! - `pointermove` hit-tests the row under the finger via
//!   `document.elementFromPoint` → the `.manager-row[data-idx]` it lands in →
//!   marks it the live drop target (`drag_over`).
//! - `pointerup`/`pointercancel` commits the move via [`act::move_channel`] when
//!   the target differs, then clears the drag state.
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
use leptos::ev::PointerEvent;

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
    // Drag-to-reorder state, shared across rows: `drag_from` is the index of the
    // row being dragged (set on the grip's `pointerdown`); `drag_over` is the
    // index the finger is currently over (the live drop target). `None` between
    // drags. The commit reads both on `pointerup`.
    let drag_from = RwSignal::new(None::<usize>);
    let drag_over = RwSignal::new(None::<usize>);

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
                    chans.into_iter().enumerate().map(|(idx, c)| {
                        view! {
                            <ManagerRow s=s ch=c idx=idx
                                editing=editing drag_from=drag_from drag_over=drag_over/>
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

/// One row in the manager list: a finger-drag grip (`⠿`) + name (or inline-rename
/// input), then rename (✎) and delete (🗑). Reorder is a press-drag on the grip
/// (see the module header); `data-idx` lets a drag's `elementFromPoint` hit-test
/// resolve which row the finger is over.
#[component]
fn ManagerRow(
    s: Shell,
    ch: ChannelSummary,
    idx: usize,
    editing: RwSignal<Option<String>>,
    drag_from: RwSignal<Option<usize>>,
    drag_over: RwSignal<Option<usize>>,
) -> impl IntoView {
    let cid = ch.id.clone();
    let name0 = ch.name.clone();
    let sigil = if ch.kind == "lorebook" { "📖 " } else { "# " };

    // Pointer-drag reorder (hydrate-only — the drag is a no-op on ssr). The grip
    // captures the pointer on `down`, hit-tests the row under the finger on
    // `move`, and commits the reorder on `up`. See the module header.
    #[cfg(feature = "hydrate")]
    let on_grip_down = move |ev: PointerEvent| {
        use leptos::wasm_bindgen::JsCast as _;
        if let Some(el) = ev
            .current_target()
            .and_then(|t| t.dyn_into::<leptos::web_sys::Element>().ok())
        {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        drag_from.set(Some(idx));
        drag_over.set(Some(idx));
        ev.prevent_default();
    };
    #[cfg(feature = "hydrate")]
    let on_grip_move = move |ev: PointerEvent| {
        use leptos::wasm_bindgen::JsCast as _;
        if drag_from.get_untracked().is_none() {
            return;
        }
        // Keep the touch on the reorder gesture instead of scrolling the list.
        ev.prevent_default();
        // Hit-test the row under the finger by its bounding box (the NodeList
        // order is the channel order, so the row's position IS the target index
        // — no DOM attribute round-trip).
        let y = ev.client_y() as f64;
        let Some(rows) = leptos::web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector_all(".channel-manager .manager-row").ok())
        else {
            return;
        };
        for i in 0..rows.length() {
            let Some(el) = rows
                .item(i)
                .and_then(|n| n.dyn_into::<leptos::web_sys::Element>().ok())
            else {
                continue;
            };
            let r = el.get_bounding_client_rect();
            if y >= r.top() && y <= r.bottom() {
                drag_over.set(Some(i as usize));
                break;
            }
        }
    };
    #[cfg(feature = "hydrate")]
    let on_grip_up = move |_ev: PointerEvent| {
        if let (Some(from), Some(to)) = (drag_from.get_untracked(), drag_over.get_untracked()) {
            if from != to {
                act::move_channel(s, from, to);
            }
        }
        drag_from.set(None);
        drag_over.set(None);
    };

    view! {
        <li class="manager-row"
            attr:data-idx=move || idx.to_string()
            class:dragging=move || drag_from.get() == Some(idx)
            class:drag-over=move || drag_over.get() == Some(idx) && drag_from.get() != Some(idx)>
            <span class="manager-grip" title="Drag to reorder"
                on:pointerdown=move |_ev| {
                    #[cfg(feature = "hydrate")] on_grip_down(_ev);
                }
                on:pointermove=move |_ev| {
                    #[cfg(feature = "hydrate")] on_grip_move(_ev);
                }
                on:pointerup=move |_ev| {
                    #[cfg(feature = "hydrate")] on_grip_up(_ev);
                }
                on:pointercancel=move |_ev| {
                    #[cfg(feature = "hydrate")] on_grip_up(_ev);
                }>"⠿"</span>
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
