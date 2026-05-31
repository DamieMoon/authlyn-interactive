//! Reusable modal overlay — the Freya analogue of `src/ui/modal.rs` + the
//! `.modal-backdrop`/`.modal` spec in `style/_modal.scss`.
//!
//! There is no CSS and no DOM here: a modal is a `Global`-positioned full-
//! viewport scrim rect (the `.modal-backdrop`: `rgba(0,0,0,0.55)`, click-to-
//! dismiss) with a centered VELLUM card inside (`.modal`: a faux RULE_LINE
//! border, ~10px radius, ~448px / 28rem max width). The
//! `Position::new_global()` plus `Size::window_percent(100.)` plus `.center()`
//! recipe is the one freya-components' own `popup.rs` uses, so it lays out over
//! the rest of the shell regardless of the column tree.
//!
//! Helpers are state-agnostic: a leaf passes the card `Element` (or confirm
//! copy) plus close/confirm handlers, and owns the open/close signal itself
//! (`*state.modal.write_unchecked() = Some(..)/None`) exactly like the web's
//! caller-owned `close` closure.

use freya::prelude::*;

use crate::native::theme;

/// Max card width — `style/_modal.scss` `.modal { max-width: 28rem }` at the
/// 16px root = 448px.
const CARD_MAX_W: f32 = 448.0;

/// Wrap `card` in a dismiss-on-backdrop modal overlay. `on_close` fires when the
/// user presses the scrim outside the card (and is what the caller wires to
/// `*state.modal.write_unchecked() = None`). The card itself stops the press
/// from bubbling to the scrim, so a click inside never dismisses.
///
/// `on_close` is `Fn() + Clone` (the native analogue of the web's caller-owned
/// `Modal` close prop) — closures that capture `Copy` `State`s and/or owned
/// `String`s satisfy it, and `Clone` lets callers reuse one handler for both the
/// backdrop and an explicit Cancel button.
pub fn modal_overlay(card: Element, on_close: impl Fn() + Clone + 'static) -> Element {
    rect()
        .position(Position::new_global().top(0.).left(0.))
        .width(Size::window_percent(100.))
        .height(Size::window_percent(100.))
        .center()
        // `.modal-backdrop { background: rgba(0,0,0,0.55) }` — a black rect at
        // 55% opacity. Type the tuple `(u8,u8,u8)` so it picks the `Into<Fill>`
        // impl (untyped `(0,0,0)` literals infer `i32` and don't convert).
        .background((0u8, 0u8, 0u8))
        .opacity(0.55)
        .on_press(move |_| on_close())
        .child(card)
        .into()
}

/// Frame arbitrary `body` children as the centered VELLUM `.modal` card. The
/// card swallows its own press so clicking inside doesn't reach the scrim. Use
/// this to build a custom modal (e.g. the persona editor); for a yes/no prompt
/// use [`confirm_modal`].
pub fn modal_card(body: Element) -> Element {
    // `.modal { border: 1px solid var(--rule-line) }` is drawn as a 1px
    // RULE_LINE frame wrapping the VELLUM card (a faux border — the freya 0.4
    // `border` builder's `Border` value isn't ergonomically constructible here).
    rect()
        .background(theme::RULE_LINE)
        .corner_radius(10.)
        .padding(1.)
        // Block the backdrop's press so a click inside the card never dismisses.
        .on_press(|_| {})
        .child(
            rect()
                .vertical()
                .width(Size::px(CARD_MAX_W))
                .max_height(Size::window_percent(85.)) // .modal { max-height: 85vh }
                .background(theme::VELLUM)
                .color(theme::INK)
                .corner_radius(9.)
                .padding(16.)
                .spacing(10.)
                .child(body),
        )
        .into()
}

/// A yes/no confirm dialog mirroring `style/_modal.scss` `.confirm-modal`:
/// a bold title, a muted body line, and a right-aligned `[confirm] [Cancel]`
/// row. `on_confirm` runs the destructive action; `on_close` is Cancel /
/// backdrop dismiss. Both are `Fn() + Clone` so `on_close` can drive both the
/// Cancel button and the backdrop (the caller typically sets `state.modal` to
/// `None` in each, and dispatches the `act` fn in `on_confirm`).
pub fn confirm_modal(
    title: &str,
    body: &str,
    confirm_label: &str,
    on_confirm: impl Fn() + Clone + 'static,
    on_close: impl Fn() + Clone + 'static,
) -> Element {
    // The backdrop and the Cancel button both mean "close" — clone the handler
    // so each gets its own copy (`modal_overlay` also takes one).
    let on_close_btn = on_close.clone();
    let card = modal_card(
        rect()
            .vertical()
            .width(Size::fill())
            .spacing(10.)
            .child(
                label()
                    .color(theme::INK)
                    .font_size(theme::FS_H3)
                    .font_weight(FontWeight::BOLD)
                    .text(title.to_string()),
            )
            .child(
                label()
                    .color(theme::INK_MUTED)
                    .font_size(theme::FS_BODY)
                    .text(body.to_string()),
            )
            .child(
                rect()
                    .horizontal()
                    .width(Size::fill())
                    .main_align(Alignment::End)
                    .spacing(8.)
                    .child(
                        Button::new()
                            .on_press(move |_| on_close_btn())
                            .child("Cancel"),
                    )
                    .child(
                        rect()
                            .corner_radius(theme::RADIUS_SM)
                            .background(theme::INK_DANGER)
                            .color(theme::PARCHMENT_DEEP)
                            .padding((6., 12.))
                            .on_press(move |_| on_confirm())
                            .child(label().text(confirm_label.to_string())),
                    ),
            )
            .into(),
    );
    modal_overlay(card, on_close)
}

/// A titled single-input modal: a bold title, an `Input` bound to `buf`, an
/// optional `extra` child (e.g. a kind toggle), and a right-aligned
/// `[Cancel] [confirm_label]` row. `on_confirm` receives the current input text
/// (fired by Enter-to-submit OR the confirm button — Enter matters since the
/// headless harness is keyboard-only). `on_close` dismisses. Used by the
/// create/rename flows.
#[allow(clippy::too_many_arguments)]
pub fn input_modal(
    title: &str,
    buf: State<String>,
    placeholder: &str,
    confirm_label: &str,
    on_confirm: impl Fn(String) + Clone + 'static,
    on_close: impl Fn() + Clone + 'static,
    extra: Option<Element>,
) -> Element {
    let on_close_btn = on_close.clone();
    let on_submit = on_confirm.clone();
    let on_confirm_btn = on_confirm.clone();
    let mut body = rect()
        .vertical()
        .width(Size::fill())
        .spacing(10.)
        .child(
            label()
                .color(theme::INK)
                .font_size(theme::FS_H3)
                .font_weight(FontWeight::BOLD)
                .text(title.to_string()),
        )
        .child(
            Input::new(buf)
                .placeholder(placeholder.to_string())
                .width(Size::fill())
                .auto_focus(true)
                .on_submit(move |t: String| on_submit(t)),
        );
    if let Some(e) = extra {
        body = body.child(e);
    }
    let card = modal_card(
        body.child(
            rect()
                .horizontal()
                .width(Size::fill())
                .main_align(Alignment::End)
                .spacing(8.)
                .child(
                    Button::new()
                        .on_press(move |_| on_close_btn())
                        .child("Cancel"),
                )
                .child(
                    rect()
                        .corner_radius(theme::RADIUS_SM)
                        .background(theme::GOLD)
                        .color(theme::PARCHMENT_DEEP)
                        .padding((6., 12.))
                        .on_press(move |_| on_confirm_btn(buf.peek().clone()))
                        .child(label().text(confirm_label.to_string())),
                ),
        )
        .into(),
    );
    modal_overlay(card, on_close)
}
