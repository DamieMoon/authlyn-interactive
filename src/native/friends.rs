//! The friends pane (native mirror of `src/ui/shell/friends.rs`): add by
//! username, plus incoming / outgoing / accepted lists. Account-scoped, reached
//! from the rail's friends tile. A plain vertical column (not a `ScrollView`, so
//! the Accept/Remove presses fire — the wardrobe/sidebar precedent); a long list
//! clips at the bottom. Writes live in `act.rs` (shared with the confirm modal).

use freya::prelude::*;

use crate::native::state::{NativeModal, NativeState};
use crate::native::{act, theme};
use crate::protocol::FriendSummary;

/// The friends pane: the add-by-username row, then incoming requests (Accept),
/// outgoing requests (pending), and accepted friends (Remove).
pub fn pane(state: NativeState) -> Element {
    let f = state.friends.read().clone();

    let mut col = rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .color(theme::INK)
        .padding(16.)
        .spacing(10.)
        .child(add_row(state));

    if f.incoming.is_empty() && f.outgoing.is_empty() && f.friends.is_empty() {
        col = col.child(
            label()
                .color(theme::INK_MUTED)
                .text("No friends or requests yet."),
        );
    }
    for p in f.incoming {
        col = col.child(incoming_row(state, p));
    }
    for p in f.outgoing {
        col = col.child(outgoing_row(p));
    }
    for p in f.friends {
        col = col.child(friend_row(state, p));
    }
    col.into()
}

/// The "add by username" row: an input bound to `friend_add` + an Add button.
fn add_row(state: NativeState) -> Element {
    rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(6.)
        .child(
            Input::new(state.friend_add)
                .placeholder("add by username")
                .width(Size::px(240.0))
                .on_submit(move |t: String| act::add_friend(state, t)),
        )
        .child(
            Button::new()
                .on_press(move |_| {
                    let u = state.friend_add.peek().clone();
                    act::add_friend(state, u);
                })
                .child("Add"),
        )
        .into()
}

/// One incoming request: "wants to add" tag + username + Accept.
fn incoming_row(state: NativeState, p: FriendSummary) -> Element {
    let aid = p.account_id.clone();
    let action = rect()
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM_2)
        .color(theme::INK)
        .padding((4., 10.))
        .on_press(move |_| act::accept_friend(state, aid.clone()))
        .child(label().font_size(theme::FS_META).text("Accept"))
        .into();
    list_row(
        tag("wants to add", theme::TINT_RED),
        &p.username,
        Some(action),
    )
}

/// One outgoing request: "pending" tag + username (no action).
fn outgoing_row(p: FriendSummary) -> Element {
    list_row(tag("pending", theme::TINT_ORANGE), &p.username, None)
}

/// One accepted friend: "friend" tag + username + Remove (confirm modal).
fn friend_row(state: NativeState, p: FriendSummary) -> Element {
    let aid = p.account_id.clone();
    let username = p.username.clone();
    let action = rect()
        .corner_radius(theme::RADIUS_SM)
        .background(theme::VELLUM_2)
        .color(theme::INK_DANGER)
        .padding((4., 10.))
        .on_press(move |_| {
            *state.modal.write_unchecked() = Some(NativeModal::ConfirmRemoveFriend {
                aid: aid.clone(),
                username: username.clone(),
            });
        })
        .child(label().font_size(theme::FS_META).text("Remove"))
        .into();
    list_row(tag("friend", theme::TINT_GREEN), &p.username, Some(action))
}

/// A small colored pill tag (state label for the row).
fn tag(text: &str, fill: theme::Rgb) -> Element {
    rect()
        .corner_radius(theme::RADIUS_SM)
        .background(fill)
        .color(theme::PARCHMENT_DEEP)
        .padding((1., 6.))
        .child(label().font_size(theme::FS_META).text(text.to_string()))
        .into()
}

/// One list row: state tag + username (fill) + optional trailing action.
fn list_row(tag_el: Element, username: &str, action: Option<Element>) -> Element {
    // No `fill` between the tag and the trailing action: Freya's `fill` starves
    // fixed trailing siblings (it pushed the Accept/Remove buttons off-screen).
    // A bounded-width name keeps the actions aligned across rows.
    let mut row = rect()
        .horizontal()
        .width(Size::fill())
        .cross_align(Alignment::Center)
        .spacing(8.)
        .child(tag_el)
        .child(
            rect()
                .width(Size::px(220.0))
                .child(label().color(theme::INK).text(username.to_string())),
        );
    if let Some(a) = action {
        row = row.child(a);
    }
    row.into()
}
