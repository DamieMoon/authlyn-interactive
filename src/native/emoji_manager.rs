//! The per-guild custom-emoji manager pane (Phase 4b leaf) — the native mirror
//! of `src/ui/shell/emoji_manager.rs`: list the open guild's emoji, upload an
//! image plus name it (client-side `^[a-z0-9_]{2,32}$` check, server
//! re-validates), and delete one over the `/guilds/{gid}/emoji*` REST endpoints.
//!
//! The list rows reuse [`crate::native::image::RemoteImage`] for the authed
//! thumbnail and open the shared `ConfirmDeleteEmoji` modal (wired in `ui.rs`
//! `modal_view` to `act::delete_guild_emoji`) for removal. Delete is offered for
//! every emoji and authorization is re-derived server-side on the mutate (a
//! non-manager gets a privacy-404, surfaced as a friendly `status` line, never a
//! crash) — the native client has no local guild-role signal to gate on, in line
//! with the "never trust client state for access" invariant.
//!
//! The add row stages an uploaded media id in `emoji_staged_media` (with the raw
//! bytes in `emoji_staged_bytes` for an instant local preview) and only enables
//! "Add" once an image is staged AND the typed name is valid. Name validity and
//! the inline error are derived per-render from the `Input`-bound `emoji_new_name`
//! (no separate change handler — Freya's `Input` two-way-binds), so no stored
//! error field is kept. The live error surfaces only HARD problems while typing
//! (bad chars / >32); the 2-char minimum is enforced by the disabled Add button
//! and on submit, not as a premature "too short" message. A duplicate name
//! (server 409) or any other failure is surfaced as a friendly message rather
//! than a panic.
//!
//! The emoji `act`-style fns (refresh / create / image-upload) live IN this file
//! (the leaf owns its richer flows; the shared destructive `delete_guild_emoji`
//! already lives in `act.rs`), porting `src/ui/shell/act/emoji.rs`. Async
//! closures read signals with `.peek()` (no reactive context inside a task).

use freya::prelude::*;

use crate::native::api::client;
use crate::native::image::{hash_id, RemoteImage};
use crate::native::state::{NativeModal, NativeState};
use crate::native::theme;

/// A custom emoji name is 2..=32 chars, each lowercase ascii / digit / `_`.
/// Mirrors the server rule `^[a-z0-9_]{2,32}$` (and the web manager's
/// `valid_emoji_name`) without pulling in a regex crate. Gates the Add button
/// and the submit; the 2-char minimum is NOT surfaced as a live "too short"
/// error (see [`emoji_name_live_error`]).
fn valid_emoji_name(name: &str) -> bool {
    let len = name.chars().count();
    (2..=32).contains(&len)
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The live (per-keystroke) name error to show under the input — HARD errors
/// only: a disallowed character or a name longer than 32 chars. The 2-char
/// minimum is deliberately NOT flagged here (it would fire "too short" on the
/// very first keystroke); that floor is still enforced by [`valid_emoji_name`]
/// gating the Add button and the submit. Empty = no error shown.
fn emoji_name_live_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return None;
    }
    let len = name.chars().count();
    let bad_char = name
        .chars()
        .any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
    if bad_char {
        Some("Use only a\u{2013}z, 0\u{2013}9, _")
    } else if len > 32 {
        Some("Name must be at most 32 chars")
    } else {
        None
    }
}

/// The custom-emoji manager pane rendered in the shell's 3rd column when
/// `state.view == NativeView::EmojiManager`. Lists the open guild's emoji (thumb
/// plus `:name:` plus a delete control) and an add row (upload image plus a
/// validated name plus an "Add" button). No-op-friendly with no guild selected.
pub fn pane(state: NativeState) -> Element {
    let gid = state.sel_server.read().clone();
    let emoji = state.guild_emoji.read().clone();
    // Surfaces upload / create / delete failures (e.g. the 409 duplicate-name
    // message) the same way the rest of the shell reports problems.
    let status = state.status.read().clone();

    let Some(gid) = gid else {
        return rect()
            .vertical()
            .width(Size::fill())
            .height(Size::fill())
            .background(theme::PARCHMENT)
            .color(theme::INK)
            .padding(16.)
            .spacing(10.)
            .child(
                label()
                    .color(theme::INK_MUTED)
                    .font_size(theme::FS_META)
                    .text("Select a guild to manage its custom emoji."),
            )
            .into();
    };

    // -- The existing-emoji list. A plain rect column (NOT a ScrollView) so the
    // per-row delete `on_press` fires (a ScrollView swallows child presses under
    // the bare-rect press path — same reason as the persona menu / emoji popover).
    let mut list = rect().vertical().width(Size::fill()).spacing(4.);
    if emoji.is_empty() {
        list = list.child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text("No custom emoji yet."),
        );
    }
    for e in emoji.iter() {
        list = list.child(emoji_row(state, &gid, &e.media_id, &e.name));
    }

    rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .color(theme::INK)
        .padding(16.)
        .spacing(12.)
        .child(list)
        .child(add_row(state, &gid))
        .child(status_line(&status))
        .into()
}

/// One existing-emoji row: an authed thumbnail, the `:name:` shortcode, and a
/// delete control that opens the shared `ConfirmDeleteEmoji` confirm dialog
/// (`ui.rs` `modal_view` dispatches `act::delete_guild_emoji` on confirm).
fn emoji_row(state: NativeState, gid: &str, media_id: &str, name: &str) -> Element {
    let gid = gid.to_string();
    let name = name.to_string();
    let del_name = name.clone();
    rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(10.)
        .padding((4., 8.))
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM)
        .child(RemoteImage {
            media_id: media_id.to_string(),
            size: 32.0,
            fallback: name.clone(),
            circle: false,
        })
        .child(
            rect().width(Size::fill()).child(
                label()
                    .color(theme::INK_SOFT)
                    .font_size(theme::FS_BODY)
                    .text(format!(":{name}:")),
            ),
        )
        .child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .padding((4., 8.))
                .on_press(move |_| {
                    *state.modal.write_unchecked() = Some(NativeModal::ConfirmDeleteEmoji {
                        gid: gid.clone(),
                        name: del_name.clone(),
                    });
                })
                .child(
                    label()
                        .color(theme::INK_DANGER)
                        .font_size(theme::FS_META)
                        .text("delete"),
                ),
        )
        .into()
}

/// The add row: an "upload image" control (rfd pick → `upload_media`, staging the
/// id plus bytes), a thumbnail of the staged image, a name `Input` (validity
/// derived per-render), the inline error, and an "Add" control enabled only once
/// an image is staged AND the name is valid.
fn add_row(state: NativeState, gid: &str) -> Element {
    let gid = gid.to_string();
    let staged_bytes = state.emoji_staged_bytes.read().clone();
    let staged_media = state.emoji_staged_media.read().clone();
    let name = state.emoji_new_name.read().clone();

    // Derive validity + the inline error from the live `Input`-bound name on
    // every render (the web's `name_valid` / `name_error` closures):
    // `valid_emoji_name` gates the Add button (incl. the 2-char minimum), while
    // `emoji_name_live_error` only surfaces HARD errors while typing (bad chars /
    // >32) so a short-but-still-being-typed name doesn't show a premature "too
    // short" message. `Input::new(state.field)` two-way-binds, so the signal
    // updates per keystroke and re-renders us — no separate change handler needed
    // (and none exists ergonomically in this rc).
    let name_valid = valid_emoji_name(&name);
    let error = emoji_name_live_error(&name).unwrap_or("");

    let can_add = staged_media.is_some() && name_valid;

    // The staged-image preview (instant, from local bytes) or a placeholder tile.
    let preview: Element = match staged_bytes {
        Some(bytes) => ImageViewer::new(ImageSource::Bytes(hash_id("emoji-staged"), bytes))
            .width(Size::px(40.0))
            .height(Size::px(40.0))
            .corner_radius(theme::RADIUS_SM)
            .into(),
        None => rect()
            .width(Size::px(40.0))
            .height(Size::px(40.0))
            .corner_radius(theme::RADIUS_SM)
            .background(theme::AVATAR_TILE)
            .color(theme::INK_MUTED)
            .center()
            .child(label().font_size(theme::FS_META).text("\u{1f5bc}"))
            .into(),
    };

    let add_bg = if can_add {
        theme::GOLD
    } else {
        theme::AVATAR_TILE
    };
    let add_fg = if can_add {
        theme::PARCHMENT_DEEP
    } else {
        theme::INK_MUTED
    };
    let gid_for_add = gid.clone();

    let mut col = rect()
        .vertical()
        .width(Size::fill())
        .spacing(8.)
        .padding(10.)
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM)
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text("Add a custom emoji"),
        )
        .child(
            rect()
                .horizontal()
                .width(Size::fill())
                .cross_align(Alignment::Center)
                .spacing(8.)
                .child(preview)
                .child(
                    rect()
                        .corner_radius(theme::RADIUS_SM)
                        .padding((6., 10.))
                        .background(theme::INPUT_BG)
                        .color(theme::INK_SOFT)
                        .on_press(move |_| pick_and_stage_emoji_image(state))
                        .child(label().font_size(theme::FS_META).text("upload image")),
                )
                .child(
                    rect().width(Size::fill()).child(
                        Input::new(state.emoji_new_name)
                            .placeholder("name (a-z 0-9 _)")
                            .width(Size::fill()),
                    ),
                )
                .child(
                    rect()
                        .corner_radius(theme::RADIUS_SM)
                        .padding((6., 14.))
                        .background(add_bg)
                        .color(add_fg)
                        .on_press(move |_| {
                            if !can_add {
                                return;
                            }
                            create_emoji(state, gid_for_add.clone());
                        })
                        .child(label().font_weight(FontWeight::BOLD).text("Add")),
                ),
        );

    if !error.is_empty() {
        col = col.child(
            label()
                .color(theme::INK_DANGER)
                .font_size(theme::FS_META)
                .text(error.to_string()),
        );
    }
    col.into()
}

/// A bottom status line echoing `state.status` (upload / create / delete result),
/// zero-height when empty so the layout doesn't jump.
fn status_line(status: &str) -> Element {
    if status.is_empty() {
        return rect().height(Size::px(0.0)).into();
    }
    label()
        .color(theme::INK_MUTED)
        .font_size(theme::FS_META)
        .text(status.to_string())
        .into()
}

// ---------------------------------------------------------------------------
// Emoji actions — the native port of `src/ui/shell/act/emoji.rs`. The shared
// destructive `delete_guild_emoji` lives in `act.rs` (it's dispatched by the
// `ConfirmDeleteEmoji` modal wiring); the create / refresh / image-upload flows
// the manager owns live here. Async closures read signals with `.peek()`.
// ---------------------------------------------------------------------------

/// Open the OS file picker (images only), upload the chosen image over the
/// authenticated session, and stage its media id (`emoji_staged_media`) plus raw
/// bytes (`emoji_staged_bytes`, for an instant preview). Runs in a `spawn` task
/// so the winit/Skia event loop never blocks (rfd's sync dialog would freeze the
/// window — only `AsyncFileDialog` is safe). Mirrors the web's
/// `upload_emoji_image`, staging into the manager's buffers rather than the
/// composer's attachment list.
pub fn pick_and_stage_emoji_image(state: NativeState) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
            .set_title("Emoji image")
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
        match client().upload_media(bytes.clone(), name, mime).await {
            Ok(id) => {
                *state.emoji_staged_bytes.write_unchecked() = Some(bytes::Bytes::from(bytes));
                *state.emoji_staged_media.write_unchecked() = Some(id);
                *state.status.write_unchecked() = String::new();
            }
            Err(e) => *state.status.write_unchecked() = format!("upload failed: {e}"),
        }
    });
}

/// Create a named custom emoji from the staged media id, then refresh the guild's
/// emoji list so the new emoji is immediately usable and clear the add row. A
/// duplicate name (server 409) or any other failure is surfaced as a friendly
/// `status` message — never a crash. Re-checks validity at click time (the button
/// is already gated, but a stale read is cheap to guard against).
pub fn create_emoji(state: NativeState, gid: String) {
    let name = state.emoji_new_name.peek().trim().to_string();
    let Some(media_id) = state.emoji_staged_media.peek().clone() else {
        return;
    };
    if !valid_emoji_name(&name) {
        return;
    }
    *state.status.write_unchecked() = String::new();
    spawn(async move {
        match client().create_emoji(&gid, &name, &media_id).await {
            Ok(_) => {
                // Clear the add row, then reload so the resolver/picker see it.
                *state.emoji_new_name.write_unchecked() = String::new();
                *state.emoji_staged_media.write_unchecked() = None;
                *state.emoji_staged_bytes.write_unchecked() = None;
                refresh(state, gid).await;
            }
            Err(e) => *state.status.write_unchecked() = create_error_message(&e),
        }
    });
}

/// Reload the open guild's custom emoji into `guild_emoji` (drives this list, the
/// composer `:`-autocomplete, and `:name:` render resolution). Inline (no nested
/// `spawn`) so it can be awaited from another task — see [`create_emoji`].
async fn refresh(state: NativeState, gid: String) {
    if let Ok(r) = client().list_guild_emoji(&gid).await {
        *state.guild_emoji.write_unchecked() = r.emoji;
    }
}

/// Map a create failure to a friendly message: the common case is a 409 when the
/// shortcode is already taken in this guild.
fn create_error_message(e: &crate::native::api::ApiError) -> String {
    match e.status() {
        Some(409) => "That emoji name is already taken in this guild.".to_string(),
        Some(403) | Some(404) => "You don't have permission to add emoji here.".to_string(),
        _ => format!("Could not add emoji: {e}"),
    }
}

/// Infer an upload MIME from the file extension, matching the server's image
/// allowlist (`server/media.rs`); the server reads the multipart part's
/// Content-Type and re-validates, rejecting a spoofed extension with 415. A
/// duplicate of `act::mime_from_name` (private there), kept local to this leaf.
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
