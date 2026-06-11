//! Freya read-path UI — the native mirror of `src/ui/shell/`.
//!
//! Three-pane shell: guild rail · channel sidebar · channel pane (message list).
//! Coarse-grained: `rail`/`sidebar`/`channel_pane` are plain fns inlined into
//! `app`, so reading a signal subscribes the app scope and the tree re-renders on
//! change (fine at this data scale). Styling is per-element from `theme`. Avatars
//! are monograms in this step; real images are wired via `image.rs`.

use freya::prelude::*;

use crate::native::image::RemoteImage;
use crate::native::state::{use_native_state, NativeModal, NativeState, NativeView};
use crate::native::{act, markup_view::render_body, modal, theme};
use crate::protocol::MessageEnvelope;

/// Root component. Branches between the login form and the authenticated shell;
/// `bootstrap` auto-logs-in only when env creds are set (dev/headless).
pub fn app() -> impl IntoElement {
    let state = use_native_state();

    use_hook(move || {
        spawn(async move { act::bootstrap(state).await });
        act::start_poll(state);
    });

    let view: Element = if *state.authed.read() {
        shell(state)
    } else {
        login_view(state)
    };
    view
}

/// The authenticated three-pane shell. The 3rd column dispatches on
/// `state.view`: the channel reader, the wardrobe, or the emoji manager. Any
/// open `state.modal` is rendered as a Global overlay on top of everything.
fn shell(state: NativeState) -> Element {
    let view = *state.view.read();
    let third: Element = match view {
        NativeView::Channel => channel_pane(state),
        NativeView::Wardrobe => {
            let body = crate::native::wardrobe::pane(state);
            pane_with_back(state, "Wardrobe", body)
        }
        NativeView::EmojiManager => {
            let body = crate::native::emoji_manager::pane(state);
            pane_with_back(state, "Custom emoji", body)
        }
        NativeView::Friends => {
            let body = crate::native::friends::pane(state);
            pane_with_back(state, "Friends", body)
        }
        NativeView::Members => {
            let body = crate::native::members::pane(state);
            pane_with_back(state, "Members", body)
        }
        NativeView::Trash => {
            let body = crate::native::trash::pane(state);
            pane_with_back(state, "Trash", body)
        }
    };

    let mut root = rect()
        .horizontal()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .color(theme::INK)
        .child(rail(state))
        .child(sidebar(state))
        .child(third);

    if let Some(m) = state.modal.read().clone() {
        root = root.child(modal_view(state, m));
    }
    root.into()
}

/// Wrap a non-Channel pane with a slim header bar carrying a "← Back" control
/// that returns the 3rd column to the channel reader.
fn pane_with_back(state: NativeState, title: &str, body: Element) -> Element {
    rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .child(
            rect()
                .horizontal()
                .width(Size::fill())
                .height(Size::px(44.0))
                .cross_align(Alignment::Center)
                .padding((0., 12.))
                .spacing(10.)
                .background(theme::VELLUM)
                .child(
                    rect()
                        .corner_radius(theme::RADIUS_SM)
                        .padding((4., 8.))
                        .background(theme::VELLUM_2)
                        .color(theme::INK_SOFT)
                        .on_press(move |_| {
                            *state.view.write_unchecked() = NativeView::Channel;
                        })
                        .child(label().font_size(theme::FS_META).text("\u{2190} Back")),
                )
                .child(
                    label()
                        .color(theme::INK)
                        .font_weight(FontWeight::BOLD)
                        .text(title.to_string()),
                ),
        )
        .child(body)
        .into()
}

/// Render an open modal: a destructive-confirm prompt for the three
/// `ConfirmDelete*` variants, or the persona detail editor (built by the
/// wardrobe leaf's [`crate::native::wardrobe::editor_modal`]). Most paths close
/// by writing `None` into `state.modal`; the `ConfirmDeleteGalleryImage` arm is
/// the exception — both its close paths RESTORE the `PersonaEditor` so dismissing
/// or confirming the gallery-remove returns to the still-open editor (web parity).
fn modal_view(state: NativeState, m: NativeModal) -> Element {
    let close = move || *state.modal.write_unchecked() = None;
    match m {
        NativeModal::ConfirmDeletePersona { pid, name } => {
            let confirm = move || {
                act::delete_persona(state, pid.clone());
                *state.modal.write_unchecked() = None;
            };
            modal::confirm_modal(
                "Delete persona",
                &format!("Delete the persona \u{201c}{name}\u{201d}? This cannot be undone."),
                "Delete",
                confirm,
                close,
            )
        }
        NativeModal::ConfirmDeleteGalleryImage { pid, img_id } => {
            // pending_remove is editor-local in the web (wardrobe.rs PersonaDetail):
            // dismissing OR confirming the gallery-remove returns to the OPEN editor.
            // The native modal is a single Option slot, so both close paths must
            // RESTORE the PersonaEditor here (not write None) — otherwise the editor
            // is dismissed and re-opening it re-seeds the pe_* buffers from the grid,
            // clobbering unsaved name/description edits.
            let restore_pid = pid.clone();
            let dismiss = move || {
                *state.modal.write_unchecked() = Some(NativeModal::PersonaEditor {
                    pid: restore_pid.clone(),
                });
            };
            let confirm = move || {
                act::remove_gallery_image(state, pid.clone(), img_id.clone());
                *state.modal.write_unchecked() =
                    Some(NativeModal::PersonaEditor { pid: pid.clone() });
            };
            modal::confirm_modal(
                "Remove image",
                "Remove this image from the gallery?",
                "Remove",
                confirm,
                dismiss,
            )
        }
        NativeModal::ConfirmDeleteEmoji { gid, name } => {
            // Build the prompt before `name` moves into the confirm closure.
            let body = format!("Delete the custom emoji \u{201c}:{name}:\u{201d}?");
            let confirm = move || {
                act::delete_guild_emoji(state, gid.clone(), name.clone());
                *state.modal.write_unchecked() = None;
            };
            modal::confirm_modal("Delete emoji", &body, "Delete", confirm, close)
        }
        NativeModal::ConfirmRemoveFriend { aid, username } => {
            let body = format!("Remove \u{201c}{username}\u{201d} from your friends?");
            let confirm = move || {
                act::remove_friend(state, aid.clone());
                *state.modal.write_unchecked() = None;
            };
            modal::confirm_modal("Remove friend", &body, "Remove", confirm, close)
        }
        NativeModal::ConfirmKickMember { gid, aid, name } => {
            let body = format!("Kick \u{201c}{name}\u{201d} from this guild?");
            let confirm = move || {
                act::remove_member(state, gid.clone(), aid.clone());
                *state.modal.write_unchecked() = None;
            };
            modal::confirm_modal("Kick member", &body, "Kick", confirm, close)
        }
        NativeModal::CreateGuild => {
            let confirm = move |name: String| act::create_guild(state, name);
            modal::input_modal(
                "Create guild",
                state.guild_new_name,
                "Guild name",
                "Create",
                confirm,
                close,
                None,
            )
        }
        NativeModal::RenameGuild { gid } => {
            let confirm = move |name: String| act::rename_guild(state, gid.clone(), name);
            modal::input_modal(
                "Rename guild",
                state.guild_rename_buf,
                "New name",
                "Save",
                confirm,
                close,
                None,
            )
        }
        NativeModal::ConfirmDeleteGuild { gid, name } => {
            let body = format!(
                "Delete the guild \u{201c}{name}\u{201d}? It moves to the trash (auto-purged \
                 after 30 days) and can be restored."
            );
            let confirm = move || {
                act::delete_guild(state, gid.clone());
                *state.modal.write_unchecked() = None;
            };
            modal::confirm_modal("Delete guild", &body, "Delete", confirm, close)
        }
        NativeModal::CreateChannel { gid } => {
            let confirm = move |name: String| {
                let kind = state.channel_new_kind.peek().clone();
                act::create_channel(state, gid.clone(), name, kind);
            };
            let toggle = kind_toggle(state);
            modal::input_modal(
                "Create channel",
                state.channel_new_name,
                "Channel name",
                "Create",
                confirm,
                close,
                Some(toggle),
            )
        }
        NativeModal::RenameChannel { gid, cid } => {
            let confirm =
                move |name: String| act::rename_channel(state, gid.clone(), cid.clone(), name);
            modal::input_modal(
                "Rename channel",
                state.channel_rename_buf,
                "New name",
                "Save",
                confirm,
                close,
                None,
            )
        }
        NativeModal::ConfirmDeleteChannel { gid, cid, name } => {
            let body =
                format!("Delete the channel \u{201c}{name}\u{201d}? It moves to the trash and can be restored.");
            let confirm = move || {
                act::delete_channel(state, gid.clone(), cid.clone());
                *state.modal.write_unchecked() = None;
            };
            modal::confirm_modal("Delete channel", &body, "Delete", confirm, close)
        }
        // The persona detail editor: built by the wardrobe leaf. Its close also
        // clears the `pe_name`/`pe_description` buffers it shares with the
        // wardrobe create row, so dismissing the editor doesn't leave the edited
        // persona's values bleeding into the create row.
        NativeModal::PersonaEditor { pid } => {
            let close_editor = move || {
                *state.modal.write_unchecked() = None;
                *state.pe_name.write_unchecked() = String::new();
                *state.pe_description.write_unchecked() = String::new();
            };
            crate::native::wardrobe::editor_modal(state, pid, close_editor)
        }
    }
}

/// Pre-shell login / register form, centered on the page. NOTE: Freya 0.4-rc
/// `Input` has no obscured/password mode, so the password shows as plain text —
/// a known limitation until Freya gains masking (or a custom widget lands).
fn login_view(state: NativeState) -> Element {
    let register = *state.auth_register.read();
    let err = state.auth_error.read().clone();
    let submit_label = if register {
        "Create account"
    } else {
        "Sign in"
    };
    let toggle_label = if register {
        "Have an account? Sign in"
    } else {
        "New here? Create an account"
    };

    let mut card = rect()
        .vertical()
        .width(Size::px(320.0))
        .spacing(10.)
        .padding(20.)
        .background(theme::VELLUM)
        .corner_radius(theme::RADIUS)
        .child(
            label()
                .color(theme::INK)
                .font_size(theme::FS_H2)
                .font_weight(FontWeight::BOLD)
                .text(if register {
                    "Create account"
                } else {
                    "Welcome back"
                }),
        )
        .child(
            Input::new(state.auth_user)
                .placeholder("username")
                .width(Size::fill())
                .auto_focus(true),
        )
        .child(
            Input::new(state.auth_pass)
                .placeholder("password")
                .width(Size::fill())
                .on_submit(move |_: String| act::submit_login(state)),
        )
        .child(
            Button::new()
                .on_press(move |_| act::submit_login(state))
                .child(submit_label),
        )
        .child(
            rect()
                .on_press(move |_| {
                    let r = *state.auth_register.peek();
                    *state.auth_register.write_unchecked() = !r;
                    *state.auth_error.write_unchecked() = String::new();
                })
                .child(
                    label()
                        .color(theme::GOLD)
                        .font_size(theme::FS_META)
                        .text(toggle_label),
                ),
        );
    if !err.is_empty() {
        card = card.child(
            label()
                .color(theme::INK_DANGER)
                .font_size(theme::FS_META)
                .text(err),
        );
    }

    rect()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .center()
        .child(card)
        .into()
}

fn rail(state: NativeState) -> Element {
    let sel = state.sel_server.read().clone();
    let mut col = rect()
        .vertical()
        .width(Size::px(theme::RAIL_W))
        .height(Size::fill())
        .background(theme::PARCHMENT_DEEP)
        .cross_align(Alignment::Center)
        .spacing(8.)
        .padding((10., 0.));

    for g in state.guilds.read().iter() {
        let active = sel.as_deref() == Some(g.id.as_str());
        let gid = g.id.clone();
        col = col.child(
            rect()
                .width(Size::px(theme::GUILD_TILE))
                .height(Size::px(theme::GUILD_TILE))
                .corner_radius(theme::GUILD_TILE / 2.0)
                .background(if active {
                    theme::GOLD
                } else {
                    theme::AVATAR_TILE
                })
                .color(if active {
                    theme::PARCHMENT_DEEP
                } else {
                    theme::INK_SOFT
                })
                .center()
                .on_press(move |_| act::open_server(state, gid.clone()))
                .child(monogram(g.name.as_str())),
        );
    }

    // Account-scoped Wardrobe entry, pinned at the bottom of the rail. Refreshes
    // the persona list, then switches the 3rd column to the wardrobe pane.
    let wardrobe_active = *state.view.read() == NativeView::Wardrobe;
    col = col.child(
        rect()
            .width(Size::px(theme::GUILD_TILE))
            .height(Size::px(theme::GUILD_TILE))
            .corner_radius(theme::GUILD_TILE / 2.0)
            .background(if wardrobe_active {
                theme::GOLD
            } else {
                theme::AVATAR_TILE
            })
            .color(if wardrobe_active {
                theme::PARCHMENT_DEEP
            } else {
                theme::INK_SOFT
            })
            .center()
            .on_press(move |_| {
                act::refresh_personas(state);
                *state.view.write_unchecked() = NativeView::Wardrobe;
            })
            .child(label().text("\u{1f9e5}")),
    );

    // Account-scoped Friends entry, pinned beneath the wardrobe. Loads the
    // friend lists, then switches the 3rd column to the friends pane.
    let friends_active = *state.view.read() == NativeView::Friends;
    col = col.child(
        rect()
            .width(Size::px(theme::GUILD_TILE))
            .height(Size::px(theme::GUILD_TILE))
            .corner_radius(theme::GUILD_TILE / 2.0)
            .background(if friends_active {
                theme::GOLD
            } else {
                theme::AVATAR_TILE
            })
            .color(if friends_active {
                theme::PARCHMENT_DEEP
            } else {
                theme::INK_SOFT
            })
            .center()
            .on_press(move |_| act::show_friends(state))
            .child(label().text("\u{1f465}")),
    );

    // Create-guild "+" tile (account-scoped — any user may create a guild).
    col.child(
        rect()
            .width(Size::px(theme::GUILD_TILE))
            .height(Size::px(theme::GUILD_TILE))
            .corner_radius(theme::GUILD_TILE / 2.0)
            .background(theme::AVATAR_TILE)
            .color(theme::INK_SOFT)
            .center()
            .on_press(move |_| {
                *state.guild_new_name.write_unchecked() = String::new();
                *state.modal.write_unchecked() = Some(NativeModal::CreateGuild);
            })
            .child(label().font_size(theme::FS_H3).text("+")),
    )
    .into()
}

fn sidebar(state: NativeState) -> Element {
    let sel_ch = state.sel_channel.read().as_ref().map(|c| c.id.clone());
    let me_id = state.me.read().as_ref().map(|m| m.account_id.clone());
    let is_owner = me_id.is_some() && me_id.as_deref() == state.sel_owner.read().as_deref();
    let have_guild = state.sel_server.read().is_some();

    // CHANNELS header: label + owner-only create-channel "+" + members + emoji.
    // Plain horizontal siblings (no `fill` spacer — Freya's `fill` starves the
    // narrow sidebar's fixed trailing controls, squashing them to vertical text).
    let mut header = rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(4.)
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text("CHANNELS"),
        );
    if is_owner {
        if let Some(gid) = state.sel_server.read().clone() {
            header = header.child(
                rect()
                    .corner_radius(theme::RADIUS_SM)
                    .padding((2., 3.))
                    .on_press(move |_| {
                        *state.channel_new_name.write_unchecked() = String::new();
                        *state.channel_new_kind.write_unchecked() = "text".to_string();
                        *state.modal.write_unchecked() =
                            Some(NativeModal::CreateChannel { gid: gid.clone() });
                    })
                    .child(
                        label()
                            .color(theme::INK_MUTED)
                            .font_size(theme::FS_META)
                            .text("+"),
                    ),
            );
        }
    }
    header = header
        .child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .padding((2., 3.))
                .on_press(move |_| act::show_members(state))
                .child(
                    label()
                        .color(theme::INK_MUTED)
                        .font_size(theme::FS_META)
                        .text("members"),
                ),
        )
        .child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .padding((2., 3.))
                .on_press(move |_| {
                    *state.view.write_unchecked() = NativeView::EmojiManager;
                })
                .child(
                    label()
                        .color(theme::INK_MUTED)
                        .font_size(theme::FS_META)
                        .text("emoji"),
                ),
        );

    let mut col = rect()
        .vertical()
        .width(Size::px(theme::SIDEBAR_W))
        .height(Size::fill())
        .background(theme::VELLUM)
        .spacing(2.)
        .padding(10.);
    if have_guild {
        col = col.child(guild_header(state, is_owner));
    }
    col = col.child(header);

    let channels: Vec<crate::protocol::ChannelSummary> = state.channels.read().clone();
    let count = channels.len();
    for (idx, c) in channels.into_iter().enumerate() {
        let active = sel_ch.as_deref() == Some(c.id.as_str());
        let sigil = if c.kind == "text" { "# " } else { "\u{1f4d6} " };
        let ch = c.clone();
        let name_area = rect()
            .padding((4., 8.))
            .corner_radius(theme::RADIUS_SM)
            .background(if active {
                theme::VELLUM_2
            } else {
                theme::VELLUM
            })
            .color(if active { theme::INK } else { theme::INK_SOFT })
            .on_press(move |_| act::open_channel(state, ch.clone()))
            .child(label().text(format!("{sigil}{}", c.name)));
        // Owner-only inline reorder/rename/delete controls. The row has no
        // `on_press` of its own (the name_area carries the open press), mirroring
        // `message_row` so a control press never also opens the channel.
        if is_owner {
            col = col.child(
                rect()
                    .horizontal()
                    .width(Size::fill())
                    .cross_align(Alignment::Center)
                    .spacing(4.)
                    .child(name_area)
                    .child(channel_controls(state, &c, idx, count)),
            );
        } else {
            col = col.child(name_area);
        }
    }
    col.into()
}

/// The sidebar's guild header (shown when a guild is open): the guild name and,
/// for the owner, a controls row — rename, move up/down in the rail order,
/// delete, and a trash entry. Two rows so the controls get the full sidebar width
/// (the narrow sidebar can't fit name + 5 controls on one line).
fn guild_header(state: NativeState, is_owner: bool) -> Element {
    let gname = {
        let sel = state.sel_server.read().clone();
        state
            .guilds
            .read()
            .iter()
            .find(|g| Some(g.id.as_str()) == sel.as_deref())
            .map(|g| g.name.clone())
            .unwrap_or_default()
    };
    let mut block = rect()
        .vertical()
        .width(Size::fill())
        .spacing(4.)
        .padding((0., 2.))
        .child(
            label()
                .color(theme::INK)
                .font_weight(FontWeight::BOLD)
                .text(gname.clone()),
        );
    if is_owner {
        let name_r = gname.clone();
        let name_d = gname.clone();
        let controls = rect()
            .horizontal()
            .spacing(6.)
            .cross_align(Alignment::Center)
            .child(ctrl_btn("rename", move || {
                if let Some(gid) = state.sel_server.peek().clone() {
                    *state.guild_rename_buf.write_unchecked() = name_r.clone();
                    *state.modal.write_unchecked() = Some(NativeModal::RenameGuild { gid });
                }
            }))
            .child(ctrl_btn("\u{2191}", move || {
                if let Some(idx) = guild_index(state) {
                    act::swap_guild(state, idx, true);
                }
            }))
            .child(ctrl_btn("\u{2193}", move || {
                if let Some(idx) = guild_index(state) {
                    act::swap_guild(state, idx, false);
                }
            }))
            .child(danger_btn("delete", move || {
                if let Some(gid) = state.sel_server.peek().clone() {
                    *state.modal.write_unchecked() = Some(NativeModal::ConfirmDeleteGuild {
                        gid,
                        name: name_d.clone(),
                    });
                }
            }))
            .child(ctrl_btn("trash", move || act::show_trash(state)));
        block = block.child(controls);
    }
    block.into()
}

/// Index of the open guild within the rail (`state.guilds`), for `swap_guild`.
fn guild_index(state: NativeState) -> Option<usize> {
    let sel = state.sel_server.peek().clone()?;
    state.guilds.peek().iter().position(|g| g.id == sel)
}

/// Owner-only inline channel controls: move up/down (when not at an edge),
/// rename, and delete.
fn channel_controls(
    state: NativeState,
    c: &crate::protocol::ChannelSummary,
    idx: usize,
    count: usize,
) -> Element {
    let cid = c.id.clone();
    let name = c.name.clone();
    let mut row = rect()
        .horizontal()
        .spacing(4.)
        .cross_align(Alignment::Center);
    if idx > 0 {
        row = row.child(ctrl_btn("\u{2191}", move || {
            act::swap_channel(state, idx, true)
        }));
    }
    if idx + 1 < count {
        row = row.child(ctrl_btn("\u{2193}", move || {
            act::swap_channel(state, idx, false)
        }));
    }
    {
        let cid = cid.clone();
        let name = name.clone();
        row = row.child(ctrl_btn("edit", move || {
            if let Some(gid) = state.sel_server.peek().clone() {
                *state.channel_rename_buf.write_unchecked() = name.clone();
                *state.modal.write_unchecked() = Some(NativeModal::RenameChannel {
                    gid,
                    cid: cid.clone(),
                });
            }
        }));
    }
    {
        let cid = cid.clone();
        let name = name.clone();
        row = row.child(danger_btn("\u{00d7}", move || {
            if let Some(gid) = state.sel_server.peek().clone() {
                *state.modal.write_unchecked() = Some(NativeModal::ConfirmDeleteChannel {
                    gid,
                    cid: cid.clone(),
                    name: name.clone(),
                });
            }
        }));
    }
    row.into()
}

/// A small muted text control used in the sidebar headers/rows.
fn ctrl_btn(text: &str, on_press: impl Fn() + 'static) -> Element {
    rect()
        .corner_radius(theme::RADIUS_SM)
        .padding((2., 6.))
        .on_press(move |_| on_press())
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text(text.to_string()),
        )
        .into()
}

/// Like [`ctrl_btn`] but rendered in the danger tint (destructive controls).
fn danger_btn(text: &str, on_press: impl Fn() + 'static) -> Element {
    rect()
        .corner_radius(theme::RADIUS_SM)
        .padding((2., 6.))
        .on_press(move |_| on_press())
        .child(
            label()
                .color(theme::INK_DANGER)
                .font_size(theme::FS_META)
                .text(text.to_string()),
        )
        .into()
}

/// The text/lorebook kind selector shown in the Create-channel modal.
fn kind_toggle(state: NativeState) -> Element {
    let kind = state.channel_new_kind.read().clone();
    let opt = |label_txt: &str, value: &str, active: bool| -> Element {
        let value = value.to_string();
        rect()
            .corner_radius(theme::RADIUS_SM)
            .padding((4., 10.))
            .background(if active { theme::GOLD } else { theme::VELLUM_2 })
            .color(if active {
                theme::PARCHMENT_DEEP
            } else {
                theme::INK_SOFT
            })
            .on_press(move |_| *state.channel_new_kind.write_unchecked() = value.clone())
            .child(
                label()
                    .font_size(theme::FS_META)
                    .text(label_txt.to_string()),
            )
            .into()
    };
    rect()
        .horizontal()
        .spacing(8.)
        .child(opt("# Text", "text", kind == "text"))
        .child(opt("\u{1f4d6} Lorebook", "lorebook", kind == "lorebook"))
        .into()
}

fn channel_pane(state: NativeState) -> Element {
    // A lorebook channel renders the lore editor in place of the message reader
    // (web parity); the message list / composer / poll don't apply to it.
    let lore_title = state
        .sel_channel
        .read()
        .as_ref()
        .filter(|c| c.kind == "lorebook")
        .map(|c| format!("\u{1f4d6} {}", c.name));
    if let Some(title) = lore_title {
        return rect()
            .vertical()
            .width(Size::fill())
            .height(Size::fill())
            .background(theme::PARCHMENT)
            .child(channel_header(state, title))
            .child(crate::native::lorebook::pane(state))
            .into();
    }

    let header = state
        .sel_channel
        .read()
        .as_ref()
        .map(|c| format!("# {}", c.name))
        .unwrap_or_else(|| state.status.read().clone());

    let typing = state.typing.read().clone();

    // Freya 0.4-rc `Size::flex`/`fill` don't reserve space for fixed siblings in
    // a column (the scroll area grows and pushes the composer off-screen), so the
    // message list gets a definite height sized to the fixed window: 860 minus the
    // header (44) + typing (20) + composer (60). The "speaking as" picker, when
    // open, takes a fixed slice that the list gives back so the total stays put.
    let menu_open = *state.persona_menu.read();
    let menu_h = if menu_open { PERSONA_MENU_H } else { 0.0 };
    let strip_open = !state.staged_attachments.read().is_empty();
    let strip_h = if strip_open { ATTACH_STRIP_H } else { 0.0 };
    // `:`-emoji suggestions for the current composer token (reading `compose`
    // here subscribes the scope so the popover updates as the user types).
    let emoji_sugg = match act::active_shortcode_token(&state.compose.read()) {
        Some((q, _)) => act::emoji_suggestions(state, &q),
        None => Vec::new(),
    };
    let emoji_h = if emoji_sugg.is_empty() {
        0.0
    } else {
        EMOJI_POPOVER_H
    };
    let mut list = ScrollView::new()
        .spacing(2.)
        .width(Size::fill())
        .height(Size::px(720.0 - menu_h - strip_h - emoji_h));
    // While a modal is open, suppress message images: the Skia `ImageViewer`
    // draws on a layer above the `Global` modal overlay (a Freya 0.4-rc z-order
    // limitation), so an attachment/avatar image would punch through the scrim
    // and over the dialog. Monogram fallbacks (plain rects) are unaffected.
    let modal_open = state.modal.read().is_some();
    for m in state.messages.read().iter() {
        list = list.child(message_row(state, m, modal_open));
    }

    rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .child(channel_header(state, header))
        .child(list)
        .child(typing_line(&typing))
        .child(persona_menu(state, menu_open))
        .child(attach_strip(state, strip_open))
        .child(emoji_popover(state, emoji_sugg))
        .child(composer(state))
        .into()
}

/// The slim channel header bar: the title (fill) + a "log out" control. Shared
/// by the message reader and the lorebook editor.
fn channel_header(state: NativeState, title: String) -> Element {
    rect()
        .horizontal()
        .width(Size::fill())
        .height(Size::px(44.0))
        .cross_align(Alignment::Center)
        .padding((0., 12.))
        .background(theme::VELLUM)
        .child(
            rect().width(Size::fill()).child(
                label()
                    .color(theme::INK)
                    .font_weight(FontWeight::BOLD)
                    .text(title),
            ),
        )
        .child(
            rect().on_press(move |_| act::logout(state)).child(
                label()
                    .color(theme::INK_MUTED)
                    .font_size(theme::FS_META)
                    .text("log out"),
            ),
        )
        .into()
}

/// Height of the `:`-emoji suggestion popover when shown.
const EMOJI_POPOVER_H: f32 = 160.0;

/// A list of custom-emoji suggestions for the active `:query` token; clicking a
/// row splices `:name: ` into the composer. Zero-height when no suggestions.
/// Plain column (not a ScrollView) so child `on_press` fires (see persona menu).
fn emoji_popover(state: NativeState, suggestions: Vec<crate::protocol::CustomEmoji>) -> Element {
    if suggestions.is_empty() {
        return rect().height(Size::px(0.0)).into();
    }
    let mut col = rect()
        .vertical()
        .spacing(2.)
        .width(Size::fill())
        .height(Size::px(EMOJI_POPOVER_H))
        .padding(6.)
        .background(theme::VELLUM_2);
    for e in suggestions {
        let name = e.name.clone();
        col = col.child(
            rect()
                .horizontal()
                .width(Size::fill())
                .cross_align(Alignment::Center)
                .spacing(8.)
                .padding((4., 8.))
                .corner_radius(theme::RADIUS_SM)
                .background(theme::VELLUM)
                .color(theme::INK_SOFT)
                .on_press(move |_| act::apply_emoji(state, &name))
                .child(RemoteImage {
                    media_id: e.media_id.clone(),
                    size: 20.0,
                    fallback: e.name.clone(),
                    circle: false,
                })
                .child(label().text(format!(":{}:", e.name))),
        );
    }
    col.into()
}

/// Height of the staged-attachment thumbnail strip when shown.
const ATTACH_STRIP_H: f32 = 72.0;

/// A horizontal strip of staged-attachment thumbnails (local bytes → instant
/// preview), each with a remove control. Zero-height when nothing is staged.
fn attach_strip(state: NativeState, open: bool) -> Element {
    if !open {
        return rect().height(Size::px(0.0)).into();
    }
    let mut strip = rect()
        .horizontal()
        .spacing(8.)
        .padding((6., 12.))
        .height(Size::px(ATTACH_STRIP_H))
        .background(theme::VELLUM);
    for a in state.staged_attachments.read().iter() {
        let rid = a.id.clone();
        strip = strip.child(
            rect()
                .vertical()
                .spacing(2.)
                .cross_align(Alignment::Center)
                .child(
                    ImageViewer::new(ImageSource::Bytes(
                        crate::native::image::hash_id(&a.id),
                        a.bytes.clone(),
                    ))
                    .width(Size::px(44.0))
                    .height(Size::px(44.0))
                    .corner_radius(theme::RADIUS_SM),
                )
                .child(
                    rect()
                        .on_press(move |_| act::remove_staged_attachment(state, rid.clone()))
                        .child(
                            label()
                                .color(theme::INK_DANGER)
                                .font_size(theme::FS_META)
                                .text("remove"),
                        ),
                ),
        );
    }
    strip.into()
}

/// Height of the "speaking as" picker panel when open.
const PERSONA_MENU_H: f32 = 160.0;

/// The "speaking as" picker: a scrollable list of the caller's personas plus an
/// "as yourself" option. Rendered with zero height when closed so the column
/// layout is unchanged. Selecting an entry wears it in the open channel.
fn persona_menu(state: NativeState, open: bool) -> Element {
    if !open {
        return rect().height(Size::px(0.0)).into();
    }
    let active = state.active_persona.read().clone();
    // A plain column, NOT a ScrollView: under the bare-rect press path a
    // ScrollView swallows child `on_press` (the proven sidebar channel rows are
    // plain rects too). Personas are few; overflow past PERSONA_MENU_H clips.
    let mut menu = rect()
        .vertical()
        .spacing(2.)
        .width(Size::fill())
        .height(Size::px(PERSONA_MENU_H));
    menu = menu.child(persona_option(
        state,
        None,
        "Speak as yourself",
        active.is_none(),
    ));
    for p in state.personas.read().iter() {
        let chosen = active.as_deref() == Some(p.id.as_str());
        menu = menu.child(persona_option(state, Some(p.id.clone()), &p.name, chosen));
    }
    rect()
        .vertical()
        .width(Size::fill())
        .background(theme::VELLUM_2)
        .padding(6.)
        .child(menu)
        .into()
}

/// One row in the persona picker.
fn persona_option(state: NativeState, pid: Option<String>, name: &str, chosen: bool) -> Element {
    rect()
        .width(Size::fill())
        .padding((5., 8.))
        .corner_radius(theme::RADIUS_SM)
        .background(if chosen { theme::GOLD } else { theme::VELLUM })
        .color(if chosen {
            theme::PARCHMENT_DEEP
        } else {
            theme::INK_SOFT
        })
        .on_press(move |_| act::wear_persona(state, pid.clone()))
        .child(label().text(name.to_string()))
        .into()
}

/// Bottom composer: a "speaking as" persona button + a text input (Enter
/// submits) + a Send button.
fn composer(state: NativeState) -> Element {
    rect()
        .horizontal()
        .width(Size::fill())
        .height(Size::px(60.0))
        .cross_align(Alignment::Center)
        .padding((8., 12.))
        .spacing(6.)
        .background(theme::VELLUM)
        .child(persona_button(state))
        .child(
            rect()
                .height(Size::px(36.0))
                .corner_radius(theme::RADIUS_SM)
                .background(theme::INPUT_BG)
                .color(theme::INK_SOFT)
                .center()
                .padding((0., 12.))
                .on_press(move |_| act::pick_and_stage_attachments(state))
                .child(label().text("+")),
        )
        .child(
            Input::new(state.compose)
                .placeholder("Write a message\u{2026}")
                .width(Size::fill())
                .auto_focus(true)
                .on_submit(move |t: String| act::send_message(state, t)),
        )
        .child(
            Button::new()
                .on_press(move |_| {
                    let t = state.compose.peek().clone();
                    act::send_message(state, t);
                })
                .child("Send"),
        )
        .into()
}

/// The composer's "speaking as" button: shows the worn persona's name (or
/// "you"), and toggles the picker panel.
fn persona_button(state: NativeState) -> Element {
    let active = state.active_persona.read().clone();
    let name = match &active {
        Some(pid) => state
            .personas
            .read()
            .iter()
            .find(|p| &p.id == pid)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "persona".to_string()),
        None => "you".to_string(),
    };
    rect()
        .height(Size::px(36.0))
        .corner_radius(theme::RADIUS_SM)
        .background(theme::INPUT_BG)
        .color(theme::INK_SOFT)
        .cross_align(Alignment::Center)
        .padding((0., 10.))
        .on_press(move |_| {
            let cur = *state.persona_menu.peek();
            *state.persona_menu.write_unchecked() = !cur;
        })
        .child(label().font_size(theme::FS_META).text(format!("as {name}")))
        .into()
}

fn typing_line(typing: &[String]) -> Element {
    let text = if typing.is_empty() {
        String::new()
    } else {
        format!("{} typing\u{2026}", typing.join(", "))
    };
    rect()
        .width(Size::fill())
        .height(Size::px(20.0))
        .padding((0., 12.))
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text(text),
        )
        .into()
}

fn message_row(state: NativeState, m: &MessageEnvelope, modal_open: bool) -> Element {
    let who = display_name(m);
    let me_id = state.me.read().as_ref().map(|me| me.account_id.clone());
    let mine = me_id.as_deref() == Some(m.author_id.as_str());
    let editing = state.editing.read().as_deref() == Some(m.id.as_str());

    // Meta row: name · time · (edit/del for your own messages).
    let mut meta = rect()
        .horizontal()
        .spacing(6.)
        .child(
            label()
                .color(theme::INK_SOFT)
                .font_weight(FontWeight::SEMI_BOLD)
                .text(who.clone()),
        )
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text(short_time(&m.sent_at)),
        );
    // Only kind='user' is mutable — system broadcasts and kind='roll' results
    // reject edit/delete server-side (403, forge-proof rolls), so offering the
    // buttons would be a dead-end affordance (mirrors the web's
    // message_actions predicate).
    if mine && !editing && m.kind == "user" {
        let (eid, ebody) = (m.id.clone(), m.body.clone());
        let did = m.id.clone();
        meta = meta
            .child(
                rect()
                    .on_press(move |_| {
                        *state.edit_buf.write_unchecked() = ebody.clone();
                        *state.editing.write_unchecked() = Some(eid.clone());
                    })
                    .child(
                        label()
                            .color(theme::INK_MUTED)
                            .font_size(theme::FS_META)
                            .text("edit"),
                    ),
            )
            .child(
                rect()
                    .on_press(move |_| act::delete_message(state, did.clone()))
                    .child(
                        label()
                            .color(theme::INK_DANGER)
                            .font_size(theme::FS_META)
                            .text("delete"),
                    ),
            );
    }

    // Body: inline edit input when editing, else the rendered markup.
    let content: Element = if editing {
        let sid = m.id.clone();
        rect()
            .horizontal()
            .width(Size::fill())
            .spacing(6.)
            .child(
                Input::new(state.edit_buf)
                    .width(Size::fill())
                    .on_submit(move |t: String| act::edit_message(state, sid.clone(), t)),
            )
            .child(
                Button::new()
                    .on_press(move |_| *state.editing.write_unchecked() = None)
                    .child("cancel"),
            )
            .into()
    } else {
        render_body(&m.body)
    };

    let mut body_col = rect()
        .vertical()
        .width(Size::fill())
        .spacing(1.)
        .child(meta)
        .child(content);

    // Image attachments below the body.
    let images: Vec<&crate::protocol::Attachment> = m
        .attachments
        .iter()
        .filter(|a| a.mime.starts_with("image/"))
        .collect();
    if !images.is_empty() && !modal_open {
        let mut grid = rect().horizontal().spacing(6.).padding((4., 0.));
        for a in images {
            grid = grid.child(RemoteImage {
                media_id: a.id.clone(),
                size: 180.0,
                fallback: String::new(),
                circle: false,
            });
        }
        body_col = body_col.child(grid);
    }

    rect()
        .horizontal()
        .width(Size::fill())
        .spacing(8.)
        .padding((4., 8.))
        .child(avatar(m, &who, modal_open))
        .child(body_col)
        .into()
}

/// Persona avatar over the authed session, else a monogram tile.
fn avatar(m: &MessageEnvelope, who: &str, suppress_image: bool) -> Element {
    // `suppress_image` forces the monogram branch while a modal is open (the
    // ImageViewer-over-overlay z-order limitation; see `channel_pane`).
    match (&m.persona_avatar_id, suppress_image) {
        (Some(id), false) => RemoteImage {
            media_id: id.clone(),
            size: theme::AVATAR,
            fallback: who.to_string(),
            circle: true,
        }
        .into(),
        _ => rect()
            .width(Size::px(theme::AVATAR))
            .height(Size::px(theme::AVATAR))
            .corner_radius(theme::AVATAR / 2.0)
            .background(theme::AVATAR_TILE)
            .color(theme::INK_SOFT)
            .center()
            .child(monogram(who))
            .into(),
    }
}

// --- small pure helpers (ports of avatar.rs / channel/avatar.rs) ---

fn monogram(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn display_name(m: &MessageEnvelope) -> String {
    m.persona_name
        .clone()
        .unwrap_or_else(|| m.author_display.clone())
}

/// `HH:MM` from the RFC3339 `sent_at` (UTC; local-tz formatting is deferred —
/// the web parses with the browser `Date`, unavailable natively without chrono).
fn short_time(sent_at: &str) -> String {
    sent_at.get(11..16).unwrap_or("").to_string()
}
