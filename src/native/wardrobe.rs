//! The wardrobe pane (Phase 4b leaf) — the native mirror of
//! `src/ui/shell/wardrobe.rs` plus `src/ui/shell/act/persona.rs`.
//!
//! A vertical list of persona *character cards* (portrait + name + description +
//! a "worn" badge when active), each with Wear/Worn, Edit (opens the
//! `PersonaEditor` modal), and Remove (owner) / Leave (shared-as-editor), plus
//! reorder up/down. A create row at the top makes a fresh persona. The Edit
//! modal — built by [`editor_modal`] and dispatched from `ui.rs`'s `modal_view`
//! `PersonaEditor` arm — carries the avatar slot plus upload, the gallery grid
//! (click = set avatar, x = confirm-remove), an add-to-gallery picker, name plus
//! description inputs, a color-swatch picker, an owner-only sharing checklist,
//! and Save / Cancel.
//!
//! All persona write actions this UI needs live in THIS file (ports of
//! `act/persona.rs`): create / update / leave / reorder / set-avatar / gallery
//! add+batch / set-share / open-editor. (Delete-persona and gallery-image remove
//! go through the shared confirm dialogs wired in `ui.rs` + `act.rs`.) They
//! follow the native `act.rs` conventions: read with `.peek()` inside async
//! closures, write with `*sig.write_unchecked() = v`, and never nest a `spawn`
//! inside a task. State buffers (`pe_*`, `modal`) live on [`NativeState`]; the
//! api client is `client()`.
//!
//! Layout note: the card list and the gallery grid are PLAIN rect columns/rows,
//! NOT `ScrollView`s — a `ScrollView` swallows child `on_press` under the bare-
//! rect press path (see `ui.rs` `persona_menu`). Every clickable helper builds
//! the full `rect()…on_press(…)` chain inline and returns an `Element` (via
//! `.into()`); none returns a bare builder type.

use freya::prelude::*;

use crate::markup::Color;
use crate::native::api::client;
use crate::native::image::RemoteImage;
use crate::native::markup_view::render_body;
use crate::native::state::{NativeModal, NativeState};
use crate::native::{act, modal, theme};

/// Max gallery images per batch upload — matches the web's `GALLERY_BATCH_MAX`
/// and the composer cap; the server also re-validates. Files past this are
/// dropped client-side with a status toast.
const GALLERY_BATCH_MAX: usize = 100;

/// Portrait edge in the wardrobe card list.
const CARD_PORTRAIT: f32 = 56.0;
/// Portrait edge in the detail editor.
const EDITOR_PORTRAIT: f32 = 96.0;
/// Gallery thumbnail edge in the detail editor.
const GALLERY_THUMB: f32 = 64.0;
/// Gallery thumbnails per row (the grid is a stack of plain rows).
const GALLERY_COLS: usize = 4;

// ---------------------------------------------------------------------------
// Persona write actions (ports of `src/ui/shell/act/persona.rs`). Errors
// surface via `state.status`, mirroring the web's `s.composer.status`.
// ---------------------------------------------------------------------------

/// Create a persona from the add-row, then reload the wardrobe list and clear
/// the buffers the add-row reused. No-op on an empty name (web parity).
pub fn create_persona(state: NativeState, name: String, desc: String) {
    if name.trim().is_empty() {
        return;
    }
    spawn(async move {
        match client().create_persona(name.trim(), &desc).await {
            Ok(_) => {
                *state.pe_name.write_unchecked() = String::new();
                *state.pe_description.write_unchecked() = String::new();
                refresh_personas(state).await;
            }
            Err(e) => *state.status.write_unchecked() = format!("create failed: {e}"),
        }
    });
}

/// Save edits to a persona (name + description + color), reload the grid so the
/// card reflects the change, and close the editor. A blank name is rejected
/// (web parity) without touching the server.
pub fn update_persona(state: NativeState, pid: String, name: String, desc: String, color: String) {
    if name.trim().is_empty() {
        *state.status.write_unchecked() = "name must not be empty".to_string();
        return;
    }
    spawn(async move {
        match client()
            .patch_persona(&pid, Some(name), Some(desc), Some(color), None)
            .await
        {
            Ok(()) => {
                refresh_personas(state).await;
                // Clear the shared create-row buffers so the saved values don't
                // bleed back into the wardrobe create row after the editor closes.
                *state.pe_name.write_unchecked() = String::new();
                *state.pe_description.write_unchecked() = String::new();
                *state.modal.write_unchecked() = None;
            }
            Err(e) => *state.status.write_unchecked() = format!("save failed: {e}"),
        }
    });
}

/// Leave a shared persona (editor only): drop it from the caller's wardrobe.
/// Takes it off locally first if it was worn in the open channel (web parity),
/// then reloads the grid.
pub fn leave_persona(state: NativeState, pid: String) {
    if state.active_persona.peek().as_deref() == Some(pid.as_str()) {
        *state.active_persona.write_unchecked() = None;
    }
    spawn(async move {
        match client().leave_persona(&pid).await {
            Ok(()) => refresh_personas(state).await,
            Err(e) => *state.status.write_unchecked() = format!("leave failed: {e}"),
        }
    });
}

/// Move the persona at `idx` up (`up=true`) or down by one in the wardrobe list,
/// then durably persist the new order (web parity with `act::swap_persona`).
///
/// Optimistic: swap the two rows locally first so the list updates instantly,
/// then PATCH each moved persona's `position` to its new 0-based index. The
/// server stores `position`; a subsequent `refresh_personas` re-sorts by it, so
/// the chosen order survives a reload.
pub fn reorder_persona(state: NativeState, idx: usize, up: bool) {
    let mut list = state.personas.peek().clone();
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
    // After the swap, `list[idx]`/`list[other]` are the two moved rows; assign
    // each its new visible index as `position`, then persist both.
    let a = list[idx].clone();
    let b = list[other].clone();
    *state.personas.write_unchecked() = list;
    spawn(async move {
        let _ = client()
            .patch_persona(&a.id, None, None, None, Some(idx as i64))
            .await;
        let _ = client()
            .patch_persona(&b.id, None, None, None, Some(other as i64))
            .await;
    });
}

/// Set a gallery image's media as the persona's primary avatar, then update the
/// editor's avatar pointer + reload the wardrobe grid so the portrait updates.
pub fn set_avatar_from_gallery(state: NativeState, pid: String, media_id: String) {
    *state.status.write_unchecked() = String::new();
    spawn(async move {
        match client().set_persona_avatar(&pid, &media_id).await {
            Ok(()) => {
                *state.pe_avatar_id.write_unchecked() = Some(media_id);
                refresh_personas(state).await;
            }
            Err(e) => *state.status.write_unchecked() = format!("set avatar failed: {e}"),
        }
    });
}

/// Open the OS image picker, upload the chosen file, set it as the persona's
/// avatar, then update the editor pointer + reload the grid. Async picker so the
/// winit/Skia event loop never blocks (the rfd `AsyncFileDialog` pattern from
/// `act.rs`).
pub fn pick_and_set_avatar(state: NativeState, pid: String) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
            .set_title("Choose a portrait")
            .pick_file()
            .await
        else {
            return; // cancelled
        };
        let name = file.file_name();
        let mime = mime_from_name(&name);
        let bytes = file.read().await;
        if bytes.is_empty() {
            return;
        }
        let media_id = match client().upload_media(bytes, name, mime).await {
            Ok(id) => id,
            Err(e) => {
                *state.status.write_unchecked() = format!("upload failed: {e}");
                return;
            }
        };
        match client().set_persona_avatar(&pid, &media_id).await {
            Ok(()) => {
                *state.pe_avatar_id.write_unchecked() = Some(media_id);
                refresh_personas(state).await;
            }
            Err(e) => *state.status.write_unchecked() = format!("set avatar failed: {e}"),
        }
    });
}

/// Open the OS image picker (multi-select), upload each file, then commit them
/// to the persona's gallery in one batch (capped at [`GALLERY_BATCH_MAX`]).
/// Reloads the editor's gallery buffer on success. Per-file upload errors are
/// toasted and skipped; the rest of the batch still ships (web parity).
pub fn pick_and_add_gallery(state: NativeState, pid: String) {
    spawn(async move {
        let Some(files) = rfd::AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
            .set_title("Add images to the gallery")
            .pick_files()
            .await
        else {
            return; // cancelled
        };
        if files.len() > GALLERY_BATCH_MAX {
            *state.status.write_unchecked() =
                format!("Gallery batch limit ({GALLERY_BATCH_MAX}) reached");
        }
        let mut media_ids: Vec<String> = Vec::new();
        for f in files.into_iter().take(GALLERY_BATCH_MAX) {
            let name = f.file_name();
            let mime = mime_from_name(&name);
            let bytes = f.read().await;
            if bytes.is_empty() {
                continue;
            }
            match client().upload_media(bytes, name.clone(), mime).await {
                Ok(id) => media_ids.push(id),
                Err(e) => *state.status.write_unchecked() = format!("upload failed: {name} — {e}"),
            }
        }
        if media_ids.is_empty() {
            return;
        }
        match client().add_gallery_images_batch(&pid, &media_ids).await {
            Ok(_) => reload_gallery(state, pid).await,
            Err(e) => *state.status.write_unchecked() = format!("gallery add failed: {e}"),
        }
    });
}

/// Toggle whether `aid` (a friend) may edit/wear persona `pid` (owner only):
/// `share=true` grants, `false` revokes. Refreshes the editor roster the
/// checklist binds to.
pub fn set_persona_share(state: NativeState, pid: String, aid: String, share: bool) {
    spawn(async move {
        let res = if share {
            client().set_persona_editor(&pid, &aid).await
        } else {
            client().remove_persona_editor(&pid, &aid).await
        };
        match res {
            Ok(()) => {
                if let Ok(r) = client().list_persona_editors(&pid).await {
                    *state.pe_editors.write_unchecked() = r.editors;
                }
            }
            Err(e) => *state.status.write_unchecked() = format!("share failed: {e}"),
        }
    });
}

/// Open the persona detail editor: seed the `pe_*` buffers from the grid entry,
/// load the gallery + (owner-only) sharing state, then show the modal. Called
/// from the card's Edit control.
pub fn open_editor(state: NativeState, pid: String, owned: bool) {
    // Seed name/description/color/avatar from the already-loaded grid entry.
    if let Some(p) = state.personas.peek().iter().find(|p| p.id == pid).cloned() {
        *state.pe_name.write_unchecked() = p.name;
        *state.pe_description.write_unchecked() = p.description;
        *state.pe_color.write_unchecked() = p.color;
        *state.pe_avatar_id.write_unchecked() = p.avatar_id;
    }
    *state.pe_gallery.write_unchecked() = Vec::new();
    *state.pe_editors.write_unchecked() = Vec::new();
    *state.pe_friends.write_unchecked() = Vec::new();
    *state.modal.write_unchecked() = Some(NativeModal::PersonaEditor { pid: pid.clone() });
    spawn(async move {
        if let Ok(d) = client().get_persona(&pid).await {
            *state.pe_gallery.write_unchecked() = d.gallery;
            // Trust the detail's avatar pointer over the (possibly stale) seed.
            *state.pe_avatar_id.write_unchecked() = d.avatar_id;
        }
        if owned {
            if let Ok(r) = client().list_friends().await {
                *state.pe_friends.write_unchecked() = r.friends;
            }
            if let Ok(r) = client().list_persona_editors(&pid).await {
                *state.pe_editors.write_unchecked() = r.editors;
            }
        }
    });
}

/// Reload the caller's persona list into `state.personas`. Inline (no nested
/// `spawn`) so it can be awaited from another task.
async fn refresh_personas(state: NativeState) {
    if let Ok(p) = client().list_personas().await {
        *state.personas.write_unchecked() = p.personas;
    }
}

/// Reload the open editor's gallery buffer after an add/remove. Inline.
async fn reload_gallery(state: NativeState, pid: String) {
    if let Ok(d) = client().get_persona(&pid).await {
        *state.pe_gallery.write_unchecked() = d.gallery;
    }
}

/// Infer an upload MIME from a filename extension (the server re-validates
/// against its image allowlist). Mirrors `act.rs::mime_from_name`.
fn mime_from_name(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// The wardrobe pane: a create row at the top, then a plain vertical column of
/// persona cards (one per row). Reading `state.personas` here subscribes the
/// pane so it re-renders when the list changes (create / reorder / delete).
pub fn pane(state: NativeState) -> Element {
    let personas = state.personas.read().clone();
    let len = personas.len();
    let active = state.active_persona.read().clone();

    let mut col = rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .color(theme::INK)
        .padding(16.)
        .spacing(10.)
        .child(create_row(state));

    if personas.is_empty() {
        col = col.child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text("No personas yet — create one above."),
        );
    }
    for (idx, p) in personas.into_iter().enumerate() {
        let worn = active.as_deref() == Some(p.id.as_str());
        col = col.child(persona_card(state, p, idx, len, worn));
    }
    col.into()
}

/// The create row: a name input, a description input, and a Create button. It
/// reuses the `pe_name` / `pe_description` buffers (the same ones the editor
/// modal binds); since the editor is a modal shown over this pane, the two are
/// never edited at once, and a successful create clears them.
fn create_row(state: NativeState) -> Element {
    // Two rows: the name + description inputs on top, the Create button on its
    // own row beneath. A single horizontal row would push the button off-screen
    // at the 1100px native width — a `Size::fill` Input doesn't reserve room for
    // a fixed trailing sibling (Freya 0.4-rc quirk), so both inputs carry
    // bounded `Size::px` widths instead.
    let inputs = rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(6.)
        .child(
            Input::new(state.pe_name)
                .placeholder("persona name")
                .width(Size::px(180.0)),
        )
        .child(
            Input::new(state.pe_description)
                .placeholder("description")
                .width(Size::px(360.0)),
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
                .on_press(move |_| {
                    let n = state.pe_name.peek().clone();
                    let d = state.pe_description.peek().clone();
                    create_persona(state, n, d);
                })
                .child("Create persona"),
        )
        .into()
}

/// One persona character card: portrait, name (worn-tinted + badge),
/// description blurb, and an action row (Wear/Worn, Edit, Remove/Leave,
/// reorder up/down).
fn persona_card(
    state: NativeState,
    p: crate::protocol::PersonaSummary,
    idx: usize,
    len: usize,
    worn: bool,
) -> Element {
    let name_color = if p.color.is_empty() {
        theme::INK
    } else {
        Color::from_name(&p.color)
            .map(theme::tint)
            .unwrap_or(theme::INK)
    };

    // Portrait: the uploaded avatar over the authed session, else a monogram.
    let portrait: Element = match &p.avatar_id {
        Some(id) => RemoteImage {
            media_id: id.clone(),
            size: CARD_PORTRAIT,
            fallback: p.name.clone(),
            circle: false,
        }
        .into(),
        None => rect()
            .width(Size::px(CARD_PORTRAIT))
            .height(Size::px(CARD_PORTRAIT))
            .corner_radius(theme::RADIUS_SM)
            .background(theme::AVATAR_TILE)
            .color(theme::INK_SOFT)
            .center()
            .child(label().font_size(theme::FS_H3).text(monogram(&p.name)))
            .into(),
    };

    // Name row: name (worn-tinted) + a small "worn" badge when active.
    let mut name_row = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(6.)
        .child(
            label()
                .color(name_color)
                .font_size(theme::FS_H3)
                .font_weight(FontWeight::BOLD)
                .text(p.name.clone()),
        );
    if worn {
        name_row = name_row.child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .background(theme::GOLD)
                .color(theme::PARCHMENT_DEEP)
                .padding((1., 6.))
                .child(label().font_size(theme::FS_META).text("worn")),
        );
    }

    // Description blurb (rendered markup), or a muted placeholder.
    let blurb: Element = if p.description.trim().is_empty() {
        label()
            .color(theme::INK_MUTED)
            .font_size(theme::FS_META)
            .text("No description yet.")
            .into()
    } else {
        render_body(&p.description)
    };

    let info_col = rect()
        .vertical()
        .width(Size::fill())
        .spacing(3.)
        .child(name_row)
        .child(blurb)
        .child(card_actions(state, &p, idx, len, worn));

    rect()
        .horizontal()
        .width(Size::fill())
        .spacing(10.)
        .padding(10.)
        .background(theme::VELLUM)
        .corner_radius(theme::RADIUS)
        .child(portrait)
        .child(info_col)
        .into()
}

/// The per-card action row (Wear/Worn · Edit · Remove/Leave · reorder).
fn card_actions(
    state: NativeState,
    p: &crate::protocol::PersonaSummary,
    idx: usize,
    len: usize,
    worn: bool,
) -> Element {
    let mut row = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(6.)
        .padding((6., 0.));

    // Wear / Worn toggle — delegates to the shared `act::wear_persona` so the
    // per-channel write + picker-close matches the composer "speaking as" path.
    if worn {
        row = row.child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .background(theme::GOLD)
                .color(theme::PARCHMENT_DEEP)
                .padding((4., 10.))
                .on_press(move |_| act::wear_persona(state, None))
                .child(label().font_size(theme::FS_META).text("Worn \u{2713}")),
        );
    } else {
        let pid_wear = p.id.clone();
        row = row.child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .background(theme::VELLUM_2)
                .color(theme::INK_SOFT)
                .padding((4., 10.))
                .on_press(move |_| act::wear_persona(state, Some(pid_wear.clone())))
                .child(label().font_size(theme::FS_META).text("Wear")),
        );
    }

    // Edit → open the detail editor modal.
    let pid_edit = p.id.clone();
    let owned = p.owned;
    row = row.child(
        rect()
            .corner_radius(theme::RADIUS_SM)
            .background(theme::VELLUM_2)
            .color(theme::INK_SOFT)
            .padding((4., 10.))
            .on_press(move |_| open_editor(state, pid_edit.clone(), owned))
            .child(label().font_size(theme::FS_META).text("Edit")),
    );

    // Remove (owner) → confirm modal; Leave (shared) → leave immediately.
    if p.owned {
        let pid_del = p.id.clone();
        let name_del = p.name.clone();
        row = row.child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .background(theme::VELLUM_2)
                .color(theme::INK_DANGER)
                .padding((4., 10.))
                .on_press(move |_| {
                    *state.modal.write_unchecked() = Some(NativeModal::ConfirmDeletePersona {
                        pid: pid_del.clone(),
                        name: name_del.clone(),
                    });
                })
                .child(label().font_size(theme::FS_META).text("Remove")),
        );
    } else {
        let pid_leave = p.id.clone();
        row = row.child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .background(theme::VELLUM_2)
                .color(theme::INK_DANGER)
                .padding((4., 10.))
                .on_press(move |_| leave_persona(state, pid_leave.clone()))
                .child(label().font_size(theme::FS_META).text("Leave")),
        );
    }

    // Reorder up/down — active arrows are pressable; the ends render muted.
    row = row.child(reorder_arrow(state, idx, true, idx > 0));
    row = row.child(reorder_arrow(state, idx, false, idx + 1 < len));

    row.into()
}

/// A reorder arrow control. `up` picks the glyph; `enabled` decides whether it's
/// pressable (a disabled arrow is muted and has no handler).
fn reorder_arrow(state: NativeState, idx: usize, up: bool, enabled: bool) -> Element {
    let glyph = if up { "\u{2191}" } else { "\u{2193}" };
    if enabled {
        rect()
            .corner_radius(theme::RADIUS_SM)
            .background(theme::VELLUM_2)
            .color(theme::INK_SOFT)
            .padding((4., 10.))
            .on_press(move |_| reorder_persona(state, idx, up))
            .child(label().font_size(theme::FS_META).text(glyph))
            .into()
    } else {
        rect()
            .corner_radius(theme::RADIUS_SM)
            .background(theme::VELLUM)
            .color(theme::INK_MUTED)
            .padding((4., 10.))
            .child(label().font_size(theme::FS_META).text(glyph))
            .into()
    }
}

// ---------------------------------------------------------------------------
// Persona detail editor modal (dispatched from ui.rs `modal_view`).
// ---------------------------------------------------------------------------

/// Build the persona detail-editor modal for persona `pid`, wrapped in the
/// dismiss-on-backdrop overlay. The `ui.rs` `modal_view` `PersonaEditor` arm
/// calls this; `close` clears `state.modal`. `owned` (derived from the grid
/// entry) drives whether the sharing block shows.
pub fn editor_modal(
    state: NativeState,
    pid: String,
    close: impl Fn() + Clone + 'static,
) -> Element {
    let owned = state
        .personas
        .peek()
        .iter()
        .find(|p| p.id == pid)
        .map(|p| p.owned)
        .unwrap_or(false);
    let card = modal::modal_card(editor_card(state, pid, owned, close.clone()));
    modal::modal_overlay(card, close)
}

/// The editor card body: title, portrait + upload, gallery grid + add control,
/// name + description inputs, color picker, owner-only sharing, Save / Cancel.
fn editor_card(
    state: NativeState,
    pid: String,
    owned: bool,
    close: impl Fn() + Clone + 'static,
) -> Element {
    let avatar = state.pe_avatar_id.read().clone();
    let pe_name = state.pe_name.read().clone();

    // Portrait slot (current avatar or monogram).
    let portrait: Element = match &avatar {
        Some(id) => RemoteImage {
            media_id: id.clone(),
            size: EDITOR_PORTRAIT,
            fallback: pe_name.clone(),
            circle: false,
        }
        .into(),
        None => rect()
            .width(Size::px(EDITOR_PORTRAIT))
            .height(Size::px(EDITOR_PORTRAIT))
            .corner_radius(theme::RADIUS_SM)
            .background(theme::AVATAR_TILE)
            .color(theme::INK_SOFT)
            .center()
            .child(label().font_size(theme::FS_H1).text(monogram(&pe_name)))
            .into(),
    };

    let pid_avatar = pid.clone();
    let portrait_row = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(12.)
        .child(portrait)
        .child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .background(theme::VELLUM_2)
                .color(theme::INK_SOFT)
                .padding((6., 12.))
                .on_press(move |_| pick_and_set_avatar(state, pid_avatar.clone()))
                .child(label().font_size(theme::FS_META).text("Upload picture")),
        );

    let close_btn = close.clone();
    let head = rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .child(
            rect().width(Size::fill()).child(
                label()
                    .color(theme::INK)
                    .font_size(theme::FS_H3)
                    .font_weight(FontWeight::BOLD)
                    .text(if owned {
                        "Edit persona"
                    } else {
                        "Edit shared persona"
                    }),
            ),
        )
        .child(
            rect()
                .padding((2., 8.))
                .color(theme::INK_MUTED)
                .on_press(move |_| close_btn())
                .child(label().text("\u{2715}")),
        );

    let pid_save = pid.clone();
    let close_cancel = close.clone();
    let actions = rect()
        .horizontal()
        .width(Size::fill())
        .main_align(Alignment::End)
        .spacing(8.)
        .child(
            Button::new()
                .on_press(move |_| close_cancel())
                .child("Cancel"),
        )
        .child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .background(theme::GOLD)
                .color(theme::PARCHMENT_DEEP)
                .padding((6., 14.))
                .on_press(move |_| {
                    let n = state.pe_name.peek().clone();
                    let d = state.pe_description.peek().clone();
                    let c = state.pe_color.peek().clone();
                    update_persona(state, pid_save.clone(), n, d, c);
                })
                .child(label().text("Save")),
        );

    let mut body = rect()
        .vertical()
        .width(Size::fill())
        .spacing(10.)
        .child(head)
        .child(portrait_row)
        .child(field_label("Gallery"))
        .child(gallery_grid(state, &pid))
        .child(add_gallery_control(state, &pid))
        .child(field_label("Name"))
        .child(
            Input::new(state.pe_name)
                .placeholder("persona name")
                .width(Size::fill()),
        )
        .child(field_label("Description"))
        .child(
            Input::new(state.pe_description)
                .placeholder("describe this character")
                .width(Size::fill()),
        )
        .child(field_label("Name color"))
        .child(color_picker(state));

    if owned {
        body = body.child(field_label("Share with friends"));
        body = body.child(sharing_block(state, &pid));
    }

    body = body.child(actions);
    body.into()
}

/// A small field caption above an input/control.
fn field_label(text: &str) -> Element {
    label()
        .color(theme::INK_MUTED)
        .font_size(theme::FS_META)
        .text(text.to_string())
        .into()
}

/// The gallery grid: thumbnails laid out as a stack of plain rows (NOT a
/// ScrollView). Clicking a thumb sets it as the avatar (the current one gets a
/// GOLD ring); the x opens the remove-confirm modal.
fn gallery_grid(state: NativeState, pid: &str) -> Element {
    let imgs = state.pe_gallery.read().clone();
    if imgs.is_empty() {
        return label()
            .color(theme::INK_MUTED)
            .font_size(theme::FS_META)
            .text("No gallery images yet.")
            .into();
    }
    let current = state.pe_avatar_id.read().clone();
    let mut grid = rect().vertical().spacing(6.);
    let mut row = rect().horizontal().spacing(6.);
    let mut in_row = 0usize;
    for g in imgs {
        let is_avatar = current.as_deref() == Some(g.media_id.as_str());
        row = row.child(gallery_thumb(state, pid, &g, is_avatar));
        in_row += 1;
        if in_row == GALLERY_COLS {
            grid = grid.child(std::mem::replace(&mut row, rect().horizontal().spacing(6.)));
            in_row = 0;
        }
    }
    if in_row > 0 {
        grid = grid.child(row);
    }
    grid.into()
}

/// One gallery thumbnail: the image (click = set avatar), framed with a GOLD
/// ring when it's the current avatar, plus a small "x remove" control that opens
/// the remove-confirm modal.
fn gallery_thumb(
    state: NativeState,
    pid: &str,
    g: &crate::protocol::GalleryImage,
    is_avatar: bool,
) -> Element {
    let pid_set = pid.to_string();
    let media_set = g.media_id.clone();
    let pid_rm = pid.to_string();
    let img_id = g.id.clone();
    let frame_bg = if is_avatar {
        theme::GOLD
    } else {
        theme::VELLUM_2
    };

    rect()
        .vertical()
        .cross_align(Alignment::Center)
        .spacing(2.)
        // Faux ring: a tinted frame with 2px padding around the thumb.
        .background(frame_bg)
        .corner_radius(theme::RADIUS_SM)
        .padding(2.)
        .child(
            rect()
                .on_press(move |_| {
                    set_avatar_from_gallery(state, pid_set.clone(), media_set.clone())
                })
                .child(RemoteImage {
                    media_id: g.media_id.clone(),
                    size: GALLERY_THUMB,
                    fallback: String::new(),
                    circle: false,
                }),
        )
        .child(
            rect()
                .on_press(move |_| {
                    *state.modal.write_unchecked() = Some(NativeModal::ConfirmDeleteGalleryImage {
                        pid: pid_rm.clone(),
                        img_id: img_id.clone(),
                    });
                })
                .child(
                    label()
                        .color(theme::INK_DANGER)
                        .font_size(theme::FS_META)
                        .text("\u{2715} remove"),
                ),
        )
        .into()
}

/// The "add to gallery" control: an rfd multi-select picker → batch upload.
fn add_gallery_control(state: NativeState, pid: &str) -> Element {
    let pid = pid.to_string();
    rect()
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM_2)
        .color(theme::INK_SOFT)
        .padding((6., 12.))
        .on_press(move |_| pick_and_add_gallery(state, pid.clone()))
        .child(
            label()
                .font_size(theme::FS_META)
                .text("+ Add images to gallery"),
        )
        .into()
}

/// The name-color swatch picker: a Default swatch plus one per markup palette
/// color, using the theme tint consts. The active swatch gets an INK ring.
fn color_picker(state: NativeState) -> Element {
    let current = state.pe_color.read().clone();
    let mut row = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(6.);

    // Default (clears the tint).
    row = row.child(swatch(state, theme::AVATAR_TILE, current.is_empty(), None));

    for c in Color::ALL {
        let name = c.name();
        let active = current == name;
        row = row.child(swatch(state, theme::tint(c), active, Some(name)));
    }
    row.into()
}

/// One color swatch: a filled square ringed (INK frame) when active. `pick` is
/// the palette name to set (`None` = the Default/clear swatch).
fn swatch(state: NativeState, fill: theme::Rgb, active: bool, pick: Option<&str>) -> Element {
    let ring = if active { theme::INK } else { theme::RULE_LINE };
    let pick = pick.map(|s| s.to_string());
    rect()
        .background(ring)
        .corner_radius(theme::RADIUS_SM)
        .padding(2.)
        .on_press(move |_| {
            *state.pe_color.write_unchecked() = pick.clone().unwrap_or_default();
        })
        .child(
            rect()
                .width(Size::px(22.0))
                .height(Size::px(22.0))
                .corner_radius(theme::RADIUS_SM)
                .background(fill),
        )
        .into()
}

/// The owner-only sharing checklist: one row per friend with a grant/revoke
/// toggle (a checkbox-pill). Granted = the friend is in `pe_editors`.
fn sharing_block(state: NativeState, pid: &str) -> Element {
    let friends = state.pe_friends.read().clone();
    if friends.is_empty() {
        return label()
            .color(theme::INK_MUTED)
            .font_size(theme::FS_META)
            .text("No friends yet — add friends to share.")
            .into();
    }
    let granted: Vec<String> = state
        .pe_editors
        .read()
        .iter()
        .map(|e| e.account_id.clone())
        .collect();

    let mut col = rect().vertical().spacing(4.);
    for f in friends {
        let aid = f.account_id.clone();
        let checked = granted.contains(&aid);
        let pid_share = pid.to_string();
        let box_bg = if checked {
            theme::GOLD
        } else {
            theme::VELLUM_2
        };
        let box_fg = if checked {
            theme::PARCHMENT_DEEP
        } else {
            theme::INK_SOFT
        };
        let mark = if checked { "\u{2713}" } else { " " };
        col = col.child(
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(8.)
                .padding((2., 0.))
                .on_press(move |_| {
                    set_persona_share(state, pid_share.clone(), aid.clone(), !checked)
                })
                .child(
                    rect()
                        .width(Size::px(20.0))
                        .height(Size::px(20.0))
                        .corner_radius(theme::RADIUS_SM)
                        .background(box_bg)
                        .color(box_fg)
                        .center()
                        .child(label().font_size(theme::FS_META).text(mark)),
                )
                .child(label().color(theme::INK_SOFT).text(f.username)),
        );
    }
    col.into()
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// First letter of `name`, uppercased, for a monogram fallback.
fn monogram(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}
