//! The guild member-management pane (native mirror of `src/ui/shell/members.rs`).
//!
//! Every viewer sees the roster (avatar + name + role badge). Mirroring the web,
//! only the guild OWNER gets per-row controls — promote a member to admin /
//! demote an admin to member, and kick — and the owner's own row is fixed (the
//! backend rejects ownership transfer / self-kick regardless, so a UI gate slip
//! is cosmetic). Ownership is `state.me == state.sel_owner`; the roster lives in
//! `state.members`, loaded by `act::show_members` (and refreshed by
//! `act::open_server` when this pane is open) since native has no web-style
//! mount Effect keyed on the selected guild. Plain column, not a `ScrollView`,
//! so the controls fire; a long roster clips.

use freya::prelude::*;

use crate::native::image::RemoteImage;
use crate::native::state::{NativeModal, NativeState};
use crate::native::{act, theme};
use crate::protocol::MemberSummary;

/// The members pane: the open guild's roster, with owner-only role/kick controls.
pub fn pane(state: NativeState) -> Element {
    let me_id = state.me.read().as_ref().map(|m| m.account_id.clone());
    let owner = state.sel_owner.read().clone();
    let is_owner = me_id.is_some() && me_id == owner;
    let gid = state.sel_server.read().clone().unwrap_or_default();

    let mut col = rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .color(theme::INK)
        .padding(16.)
        .spacing(8.);

    let members = state.members.read().clone();
    if members.is_empty() {
        col = col.child(label().color(theme::INK_MUTED).text("No members."));
    }
    for m in members {
        col = col.child(member_row(state, m, is_owner, &gid));
    }
    col.into()
}

/// One roster row: avatar + name + role badge, plus owner-only controls on every
/// row except the owner's own.
fn member_row(state: NativeState, m: MemberSummary, is_owner: bool, gid: &str) -> Element {
    let name = if m.display_name.trim().is_empty() {
        m.username.clone()
    } else {
        m.display_name.clone()
    };

    let portrait: Element = match &m.avatar_id {
        Some(id) => RemoteImage {
            media_id: id.clone(),
            size: theme::AVATAR,
            fallback: name.clone(),
            circle: true,
        }
        .into(),
        None => rect()
            .width(Size::px(theme::AVATAR))
            .height(Size::px(theme::AVATAR))
            .corner_radius(theme::AVATAR / 2.0)
            .background(theme::AVATAR_TILE)
            .color(theme::INK_SOFT)
            .center()
            .child(label().text(monogram(&name)))
            .into(),
    };

    let mut row = rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(10.)
        .padding((6., 8.))
        .background(theme::VELLUM)
        .corner_radius(theme::RADIUS)
        .child(portrait)
        // Bounded (not `fill`) name: `fill` starves the trailing role badge +
        // controls, squashing them to vertical text at the row's right edge.
        .child(
            rect()
                .width(Size::px(220.0))
                .child(label().color(theme::INK).text(name.clone())),
        )
        .child(role_badge(&m.role));

    if is_owner && m.role != "owner" {
        row = row
            .child(role_button(state, &m, gid))
            .child(kick_button(state, &m, &name, gid));
    }
    row.into()
}

/// The role pill (owner = gold, admin = blue, member = muted tile).
fn role_badge(role: &str) -> Element {
    let fill = match role {
        "owner" => theme::GOLD,
        "admin" => theme::TINT_BLUE,
        _ => theme::AVATAR_TILE,
    };
    rect()
        .corner_radius(theme::RADIUS_SM)
        .background(fill)
        .color(theme::PARCHMENT_DEEP)
        .padding((1., 6.))
        .child(label().font_size(theme::FS_META).text(role.to_string()))
        .into()
}

/// "Make admin" (for a member) / "Demote" (for an admin) — flips the role.
fn role_button(state: NativeState, m: &MemberSummary, gid: &str) -> Element {
    let is_admin = m.role == "admin";
    let next = if is_admin { "member" } else { "admin" };
    let text = if is_admin { "Demote" } else { "Make admin" };
    let gid = gid.to_string();
    let aid = m.account_id.clone();
    rect()
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM_2)
        .color(theme::INK)
        .padding((4., 10.))
        .on_press(move |_| act::set_member_role(state, gid.clone(), aid.clone(), next.to_string()))
        .child(label().font_size(theme::FS_META).text(text))
        .into()
}

/// The kick control (✕) — opens the confirm-kick modal.
fn kick_button(state: NativeState, m: &MemberSummary, name: &str, gid: &str) -> Element {
    let gid = gid.to_string();
    let aid = m.account_id.clone();
    let name = name.to_string();
    rect()
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM_2)
        .color(theme::INK_DANGER)
        .padding((4., 10.))
        .on_press(move |_| {
            *state.modal.write_unchecked() = Some(NativeModal::ConfirmKickMember {
                gid: gid.clone(),
                aid: aid.clone(),
                name: name.clone(),
            });
        })
        .child(label().font_size(theme::FS_META).text("\u{2715}"))
        .into()
}

/// First character, uppercased — the avatar fallback (mirrors `ui::monogram`).
fn monogram(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}
