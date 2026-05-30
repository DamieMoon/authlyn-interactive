//! Freya read-path UI — the native mirror of `src/ui/shell/`.
//!
//! Three-pane shell: guild rail · channel sidebar · channel pane (message list).
//! Coarse-grained: `rail`/`sidebar`/`channel_pane` are plain fns inlined into
//! `app`, so reading a signal subscribes the app scope and the tree re-renders on
//! change (fine at this data scale). Styling is per-element from `theme`. Avatars
//! are monograms in this step; real images are wired via `image.rs`.

use freya::prelude::*;

use crate::native::image::RemoteImage;
use crate::native::state::{use_native_state, NativeState};
use crate::native::{act, markup_view::render_body, theme};
use crate::protocol::MessageEnvelope;

/// Root component.
pub fn app() -> impl IntoElement {
    let state = use_native_state();

    use_hook(move || {
        spawn(async move { act::bootstrap(state).await });
        act::start_poll(state);
    });

    rect()
        .horizontal()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .color(theme::INK)
        .child(rail(state))
        .child(sidebar(state))
        .child(channel_pane(state))
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
    col.into()
}

fn sidebar(state: NativeState) -> Element {
    let sel_ch = state.sel_channel.read().as_ref().map(|c| c.id.clone());
    let mut col = rect()
        .vertical()
        .width(Size::px(theme::SIDEBAR_W))
        .height(Size::fill())
        .background(theme::VELLUM)
        .spacing(2.)
        .padding(10.)
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text("CHANNELS"),
        );

    for c in state.channels.read().iter() {
        let active = sel_ch.as_deref() == Some(c.id.as_str());
        let ch = c.clone();
        let sigil = if c.kind == "text" { "# " } else { "\u{1f4d6} " };
        col = col.child(
            rect()
                .padding((4., 8.))
                .corner_radius(theme::RADIUS_SM)
                .background(if active {
                    theme::VELLUM_2
                } else {
                    theme::VELLUM
                })
                .color(if active { theme::INK } else { theme::INK_SOFT })
                .on_press(move |_| act::open_channel(state, ch.clone()))
                .child(label().text(format!("{sigil}{}", c.name))),
        );
    }
    col.into()
}

fn channel_pane(state: NativeState) -> Element {
    let header = state
        .sel_channel
        .read()
        .as_ref()
        .map(|c| format!("# {}", c.name))
        .unwrap_or_else(|| state.status.read().clone());

    let typing = state.typing.read().clone();

    let mut list = ScrollView::new().spacing(2.);
    for m in state.messages.read().iter() {
        list = list.child(message_row(m));
    }

    rect()
        .vertical()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::PARCHMENT)
        .child(
            rect()
                .width(Size::fill())
                .padding((8., 12.))
                .background(theme::VELLUM)
                .child(
                    label()
                        .color(theme::INK)
                        .font_weight(FontWeight::BOLD)
                        .text(header),
                ),
        )
        .child(
            rect()
                .width(Size::fill())
                .height(Size::fill())
                .padding((4., 8.))
                .child(list),
        )
        .child(typing_line(&typing))
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
        .padding((2., 12.))
        .child(
            label()
                .color(theme::INK_MUTED)
                .font_size(theme::FS_META)
                .text(text),
        )
        .into()
}

fn message_row(m: &MessageEnvelope) -> Element {
    let who = display_name(m);

    let mut body_col = rect()
        .vertical()
        .width(Size::fill())
        .spacing(1.)
        .child(
            rect()
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
                ),
        )
        .child(render_body(&m.body));

    // Image attachments below the body.
    let images: Vec<&crate::protocol::Attachment> = m
        .attachments
        .iter()
        .filter(|a| a.mime.starts_with("image/"))
        .collect();
    if !images.is_empty() {
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
        .child(avatar(m, &who))
        .child(body_col)
        .into()
}

/// Persona avatar over the authed session, else a monogram tile.
fn avatar(m: &MessageEnvelope, who: &str) -> Element {
    match &m.persona_avatar_id {
        Some(id) => RemoteImage {
            media_id: id.clone(),
            size: theme::AVATAR,
            fallback: who.to_string(),
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
