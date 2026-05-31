//! The Trash pane (Phase 4c PR2): the caller's soft-deleted guilds and the open
//! guild's soft-deleted channels, each restorable. Mirrors the friends/members
//! pane shape (a plain column rendered via `pane_with_back`). Restore is the only
//! action — the server auto-purges trash after a window (guild 30d, channel 1d),
//! so there is no user-facing purge.

use freya::prelude::*;

use crate::native::state::NativeState;
use crate::native::{act, theme};

/// The trash pane body: a "Deleted guilds" section (account-scoped) followed by a
/// "Deleted channels" section for the open guild, if any.
pub fn pane(state: NativeState) -> Element {
    let guilds = state.deleted_guilds.read().clone();
    let channels = state.deleted_channels.read().clone();
    let have_guild = state.sel_server.read().is_some();

    let mut col = rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .padding(16.)
        .spacing(10.)
        .child(section_title("Deleted guilds"));

    if guilds.is_empty() {
        col = col.child(muted_line("No deleted guilds."));
    } else {
        for g in guilds {
            let gid = g.id.clone();
            col = col.child(trash_row(&g.name, move || {
                act::restore_guild(state, gid.clone())
            }));
        }
    }

    if have_guild {
        col = col.child(section_title("Deleted channels (this guild)"));
        if channels.is_empty() {
            col = col.child(muted_line("No deleted channels."));
        } else {
            for c in channels {
                let gid = state.sel_server.peek().clone().unwrap_or_default();
                let cid = c.id.clone();
                let sigil = if c.kind == "text" { "# " } else { "\u{1f4d6} " };
                col = col.child(trash_row(&format!("{sigil}{}", c.name), move || {
                    act::restore_channel(state, gid.clone(), cid.clone())
                }));
            }
        }
    }
    col.into()
}

/// A small bold muted section label.
fn section_title(text: &str) -> Element {
    rect()
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .font_weight(FontWeight::BOLD)
                .text(text.to_string()),
        )
        .into()
}

/// A muted body line (empty-state copy).
fn muted_line(text: &str) -> Element {
    rect()
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_BODY)
                .text(text.to_string()),
        )
        .into()
}

/// One trash row: the name (fill) + a Restore button.
fn trash_row(name: &str, on_restore: impl Fn() + 'static) -> Element {
    rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(8.)
        .padding((6., 8.))
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM)
        .child(
            rect()
                .width(Size::fill())
                .child(label().color(theme::INK).text(name.to_string())),
        )
        .child(
            rect()
                .corner_radius(theme::RADIUS_SM)
                .background(theme::GOLD)
                .color(theme::PARCHMENT_DEEP)
                .padding((4., 10.))
                .on_press(move |_| on_restore())
                .child(label().font_size(theme::FS_META).text("Restore")),
        )
        .into()
}
