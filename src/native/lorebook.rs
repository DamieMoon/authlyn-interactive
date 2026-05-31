//! The lorebook editor pane (native mirror of `src/ui/shell/lorebook.rs`):
//! list, add, enable-toggle, reorder, inline-edit, and delete lore entries.
//!
//! Rendered by `ui::channel_pane` in place of the message reader when the open
//! channel is `kind == "lorebook"`. Owns its write actions (like `wardrobe.rs`
//! owns persona writes). A plain vertical column (not a `ScrollView`, so the
//! per-entry buttons fire); the add-row sits at the TOP — like the wardrobe
//! create-row — so a growing list never pushes it off-screen (Freya `fill`
//! won't reserve room for a fixed footer). Inline edit uses a single
//! `lore_editing` id + the shared `lore_edit_*` buffers (one entry at a time),
//! since the flat `NativeState` can't hold the web's per-row signals.

use freya::prelude::*;

use crate::native::api::client;
use crate::native::state::NativeState;
use crate::native::theme;
use crate::protocol::{LorebookEntry, PatchLorebookEntryRequest};

// ---------------------------------------------------------------------------
// Data load (called from act.rs) + write actions.
// ---------------------------------------------------------------------------

/// Enter a lorebook channel: clear inline-edit + add-row state and load the
/// entries, epoch-guarded against a mid-fetch channel switch. Awaited inline
/// from `act::open_channel_inner` (the epoch was already bumped there).
pub(crate) async fn enter_channel(state: NativeState, cid: &str) {
    *state.lore.write_unchecked() = Vec::new();
    *state.lore_editing.write_unchecked() = None;
    *state.lore_edit_title.write_unchecked() = String::new();
    *state.lore_edit_keys.write_unchecked() = String::new();
    *state.lore_edit_content.write_unchecked() = String::new();
    *state.lore_new_keys.write_unchecked() = String::new();
    *state.lore_new_content.write_unchecked() = String::new();
    let epoch = *state.epoch.peek();
    match client().list_lore(cid).await {
        Ok(r) if *state.epoch.peek() == epoch => *state.lore.write_unchecked() = r.entries,
        Ok(_) => {} // a newer channel switch happened — drop this load
        Err(e) => *state.status.write_unchecked() = format!("lorebook: {e}"),
    }
}

/// Reload the open channel's entries after a mutation (same channel, no guard).
async fn reload(state: NativeState, cid: &str) {
    match client().list_lore(cid).await {
        Ok(r) => *state.lore.write_unchecked() = r.entries,
        Err(e) => *state.status.write_unchecked() = format!("lorebook: {e}"),
    }
}

/// The open channel id (every action guards on it).
fn open_cid(state: NativeState) -> Option<String> {
    state.sel_channel.peek().as_ref().map(|c| c.id.clone())
}

/// Split a comma-separated keyword string into trimmed, non-empty keys.
fn parse_keys(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect()
}

/// Create an entry from the add-row buffers; clear them and reload.
fn create(state: NativeState) {
    let Some(cid) = open_cid(state) else {
        return;
    };
    let keys = parse_keys(&state.lore_new_keys.peek());
    let content = state.lore_new_content.peek().trim().to_string();
    if content.is_empty() {
        return;
    }
    *state.lore_new_keys.write_unchecked() = String::new();
    *state.lore_new_content.write_unchecked() = String::new();
    spawn(async move {
        match client().create_lore(&cid, keys, &content).await {
            Ok(_) => reload(state, &cid).await,
            Err(e) => *state.status.write_unchecked() = format!("add entry failed: {e}"),
        }
    });
}

/// Flip an entry's enabled flag, then reload.
fn toggle(state: NativeState, eid: String, enabled: bool) {
    let Some(cid) = open_cid(state) else {
        return;
    };
    let body = PatchLorebookEntryRequest {
        enabled: Some(!enabled),
        ..Default::default()
    };
    spawn(async move {
        match client().patch_lore(&cid, &eid, &body).await {
            Ok(()) => reload(state, &cid).await,
            Err(e) => *state.status.write_unchecked() = format!("toggle failed: {e}"),
        }
    });
}

/// Open inline edit for an entry: seed the buffers + set the editing id.
fn open_edit(state: NativeState, e: &LorebookEntry) {
    *state.lore_edit_title.write_unchecked() = e.title.clone();
    *state.lore_edit_keys.write_unchecked() = e.keys.join(", ");
    *state.lore_edit_content.write_unchecked() = e.content.clone();
    *state.lore_editing.write_unchecked() = Some(e.id.clone());
}

/// Cancel inline edit (discard the buffers' pending changes).
fn cancel_edit(state: NativeState) {
    *state.lore_editing.write_unchecked() = None;
}

/// Save the inline-edit buffers to the entry, exit edit, then reload.
fn save(state: NativeState, eid: String) {
    let Some(cid) = open_cid(state) else {
        return;
    };
    let body = PatchLorebookEntryRequest {
        title: Some(state.lore_edit_title.peek().clone()),
        keys: Some(parse_keys(&state.lore_edit_keys.peek())),
        content: Some(state.lore_edit_content.peek().clone()),
        enabled: None,
        position: None,
    };
    *state.lore_editing.write_unchecked() = None;
    spawn(async move {
        match client().patch_lore(&cid, &eid, &body).await {
            Ok(()) => reload(state, &cid).await,
            Err(e) => *state.status.write_unchecked() = format!("save failed: {e}"),
        }
    });
}

/// Delete an entry (web parity: no confirm), then reload.
fn delete(state: NativeState, eid: String) {
    let Some(cid) = open_cid(state) else {
        return;
    };
    spawn(async move {
        match client().delete_lore(&cid, &eid).await {
            Ok(()) => reload(state, &cid).await,
            Err(e) => *state.status.write_unchecked() = format!("delete failed: {e}"),
        }
    });
}

/// Move an entry up/down by swapping its `position` with the visual neighbor
/// (two PATCHes, then reload — mirrors the web `swap_lore`). The list is
/// position-sorted, so the neighbor at `idx ± 1` is the adjacent entry.
fn reorder(state: NativeState, idx: usize, up: bool) {
    let entries = state.lore.peek().clone();
    let Some(cid) = open_cid(state) else {
        return;
    };
    let other = if up {
        idx.checked_sub(1)
    } else {
        idx.checked_add(1).filter(|&j| j < entries.len())
    };
    let Some(j) = other else {
        return;
    };
    let a = entries[idx].clone();
    let b = entries[j].clone();
    let body_a = PatchLorebookEntryRequest {
        position: Some(b.position),
        ..Default::default()
    };
    let body_b = PatchLorebookEntryRequest {
        position: Some(a.position),
        ..Default::default()
    };
    spawn(async move {
        let r1 = client().patch_lore(&cid, &a.id, &body_a).await;
        let r2 = client().patch_lore(&cid, &b.id, &body_b).await;
        if r1.is_err() || r2.is_err() {
            *state.status.write_unchecked() = "reorder failed".to_string();
        }
        reload(state, &cid).await;
    });
}

// ---------------------------------------------------------------------------
// View.
// ---------------------------------------------------------------------------

/// The lorebook pane: the add-entry row (top), then one card per entry.
pub fn pane(state: NativeState) -> Element {
    let entries = state.lore.read().clone();
    let editing = state.lore_editing.read().clone();
    let len = entries.len();

    let mut col = rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .color(theme::INK)
        .padding(16.)
        .spacing(10.)
        .child(add_row(state));

    if entries.is_empty() {
        col = col.child(label().color(theme::INK_MUTED).text("No lore entries yet."));
    }
    for (idx, e) in entries.into_iter().enumerate() {
        let is_editing = editing.as_deref() == Some(e.id.as_str());
        col = col.child(entry_card(state, e, idx, len, is_editing));
    }
    col.into()
}

/// The add-entry row: keys + content inputs and an "Add entry" button.
fn add_row(state: NativeState) -> Element {
    let inputs = rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(6.)
        .child(
            Input::new(state.lore_new_keys)
                .placeholder("trigger keywords (comma-separated)")
                .width(Size::px(240.0)),
        )
        .child(
            Input::new(state.lore_new_content)
                .placeholder("entry content")
                .width(Size::px(300.0)),
        );
    rect()
        .vertical()
        .width(Size::fill())
        .spacing(6.)
        .padding(8.)
        .background(theme::VELLUM)
        .corner_radius(theme::RADIUS)
        .child(inputs)
        .child(
            Button::new()
                .on_press(move |_| create(state))
                .child("Add entry"),
        )
        .into()
}

/// One lore entry card: head row (toggle / reorder / title / edit controls)
/// over the content (or the keys + content inputs while editing).
fn entry_card(
    state: NativeState,
    e: LorebookEntry,
    idx: usize,
    len: usize,
    is_editing: bool,
) -> Element {
    let eid = e.id.clone();
    let enabled = e.enabled;
    let display_title = if e.title.trim().is_empty() {
        e.keys.join(", ")
    } else {
        e.title.clone()
    };

    let toggle_eid = eid.clone();
    let toggle_btn: Element = rect()
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM_2)
        .color(if enabled {
            theme::TINT_GREEN
        } else {
            theme::INK_MUTED
        })
        .padding((4., 8.))
        .on_press(move |_| toggle(state, toggle_eid.clone(), enabled))
        .child(label().text(if enabled { "\u{2713}" } else { "\u{25cb}" }))
        .into();

    // Bounded (not `fill`) title: `fill` starves the trailing edit/delete
    // controls, squashing them to vertical text at the head row's right edge.
    let title_el: Element = if is_editing {
        rect()
            .width(Size::px(280.0))
            .child(
                Input::new(state.lore_edit_title)
                    .placeholder("title (optional)")
                    .width(Size::px(280.0)),
            )
            .into()
    } else {
        rect()
            .width(Size::px(280.0))
            .child(
                label()
                    .color(theme::INK)
                    .font_weight(FontWeight::BOLD)
                    .text(display_title),
            )
            .into()
    };

    let mut head = rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(6.)
        .child(toggle_btn)
        .child(reorder_arrow(state, idx, true, idx > 0))
        .child(reorder_arrow(state, idx, false, idx + 1 < len))
        .child(title_el);

    if is_editing {
        let save_eid = eid.clone();
        head = head
            .child(head_btn("Save", theme::INK, move || {
                save(state, save_eid.clone())
            }))
            .child(head_btn("Cancel", theme::INK_MUTED, move || {
                cancel_edit(state)
            }));
    } else {
        let edit_e = e.clone();
        let del_eid = eid.clone();
        head = head
            .child(head_btn("Edit", theme::INK, move || {
                open_edit(state, &edit_e)
            }))
            .child(head_btn("\u{2715}", theme::INK_DANGER, move || {
                delete(state, del_eid.clone())
            }));
    }

    let body: Element = if is_editing {
        rect()
            .vertical()
            .width(Size::fill())
            .spacing(6.)
            .child(
                Input::new(state.lore_edit_keys)
                    .placeholder("trigger keywords (comma-separated)")
                    .width(Size::fill()),
            )
            .child(
                Input::new(state.lore_edit_content)
                    .placeholder("entry content")
                    .width(Size::fill()),
            )
            .into()
    } else {
        rect()
            .width(Size::fill())
            .child(label().color(theme::INK_SOFT).text(e.content.clone()))
            .into()
    };

    rect()
        .vertical()
        .width(Size::fill())
        .spacing(4.)
        .padding(10.)
        .background(theme::VELLUM)
        .corner_radius(theme::RADIUS)
        .child(head)
        .child(body)
        .into()
}

/// A small head-row text button (Save / Cancel / Edit / delete).
fn head_btn(text: &str, color: theme::Rgb, on_press: impl Fn() + 'static) -> Element {
    rect()
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM_2)
        .color(color)
        .padding((4., 8.))
        .on_press(move |_| on_press())
        .child(label().font_size(theme::FS_META).text(text.to_string()))
        .into()
}

/// A reorder ↑/↓ control (disabled at the list edge), mirroring `wardrobe`.
fn reorder_arrow(state: NativeState, idx: usize, up: bool, enabled: bool) -> Element {
    let glyph = if up { "\u{2191}" } else { "\u{2193}" };
    if enabled {
        rect()
            .corner_radius(theme::RADIUS_SM)
            .background(theme::VELLUM_2)
            .color(theme::INK_SOFT)
            .padding((4., 8.))
            .on_press(move |_| reorder(state, idx, up))
            .child(label().font_size(theme::FS_META).text(glyph))
            .into()
    } else {
        rect()
            .corner_radius(theme::RADIUS_SM)
            .background(theme::VELLUM)
            .color(theme::INK_MUTED)
            .padding((4., 8.))
            .child(label().font_size(theme::FS_META).text(glyph))
            .into()
    }
}
