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
/// wardrobe leaf's [`crate::native::wardrobe::editor_modal`]). Every path closes
/// by writing `None` into `state.modal`.
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
            let confirm = move || {
                act::remove_gallery_image(state, pid.clone(), img_id.clone());
                *state.modal.write_unchecked() = None;
            };
            modal::confirm_modal(
                "Remove image",
                "Remove this image from the gallery?",
                "Remove",
                confirm,
                close,
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
    col.child(
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
    )
    .into()
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
            rect()
                .horizontal()
                .width(Size::fill())
                .cross_align(Alignment::Center)
                .child(
                    rect().width(Size::fill()).child(
                        label()
                            .color(theme::INK_MUTED)
                            .font_size(theme::FS_META)
                            .text("CHANNELS"),
                    ),
                )
                // Guild-scoped emoji manager entry (only meaningful with a guild
                // selected); switches the 3rd column to the emoji pane.
                .child(
                    rect()
                        .corner_radius(theme::RADIUS_SM)
                        .padding((2., 6.))
                        .on_press(move |_| {
                            *state.view.write_unchecked() = NativeView::EmojiManager;
                        })
                        .child(
                            label()
                                .color(theme::INK_MUTED)
                                .font_size(theme::FS_META)
                                .text("emoji"),
                        ),
                ),
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
    for m in state.messages.read().iter() {
        list = list.child(message_row(state, m));
    }

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
                .background(theme::VELLUM)
                .child(
                    rect().width(Size::fill()).child(
                        label()
                            .color(theme::INK)
                            .font_weight(FontWeight::BOLD)
                            .text(header),
                    ),
                )
                .child(
                    rect().on_press(move |_| act::logout(state)).child(
                        label()
                            .color(theme::INK_MUTED)
                            .font_size(theme::FS_META)
                            .text("log out"),
                    ),
                ),
        )
        .child(list)
        .child(typing_line(&typing))
        .child(persona_menu(state, menu_open))
        .child(attach_strip(state, strip_open))
        .child(emoji_popover(state, emoji_sugg))
        .child(composer(state))
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

fn message_row(state: NativeState, m: &MessageEnvelope) -> Element {
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
    if mine && !editing {
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
