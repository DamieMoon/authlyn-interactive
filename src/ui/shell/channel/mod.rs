//! The channel message pane: the message list and the markup composer.
//!
//! Pure-helper carve-outs live in sibling submodules:
//! - [`avatar`] — `chat_avatar` (circular monogram-fallback portrait) +
//!   `format_local_time` (RFC3339 → viewer locale).
//! - [`attachments`] — `attachment_grid` (image/video grid with lightbox).
//! - [`emoji_suggest`] — the `:`-autocomplete primitives (`Suggestion`,
//!   `emoji_suggestions`, `active_shortcode_token`,
//!   `replace_shortcode_token`) + the picker-grid buttons
//!   (`custom_emoji_btn`, `unicode_emoji_btn`). Its `active_shortcode_token`
//!   unit test is co-located there.
//! - [`lightbox`] — `LightboxState`/`LbTransform` + `lightbox_view` (the
//!   near-fullscreen viewer and its pointer gesture engine). Its transform-
//!   math unit tests are co-located there.
//!
//! This file owns `ChannelPane` itself (the message-list/composer view), the
//! composer's caret-aware `apply_markup`, the touch-vs-desktop Enter helper,
//! the small `deleted_message_row`, and the re-entry divider rows
//! (`new_divider_row`/`date_divider_row`, UX evolution #9).

mod attachments;
mod avatar;
mod emoji_suggest;
mod lightbox;
mod manager;
mod meta;
mod radial;
mod skeleton;

pub(crate) use manager::ChannelManagerModal;

use attachments::attachment_grid;
use avatar::{chat_avatar, format_local_time};
#[cfg(feature = "hydrate")]
use emoji_suggest::active_shortcode_token;
use emoji_suggest::{
    custom_emoji_btn, emoji_suggestions, replace_shortcode_token, unicode_emoji_btn,
};
use lightbox::{lightbox_view, LbTransform, LightboxState};
use meta::{message_meta, system_message_meta};
use skeleton::{should_show_skeletons, skeleton_rows};

use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use super::COMPOSER_MAX_ATTACHMENTS;
use super::{act, Shell};
#[cfg(feature = "hydrate")]
use crate::client::api;
use crate::markup::Color;
use crate::protocol::MessageEnvelope;
use crate::ui::emoji::data::{self, GROUPS};
use crate::ui::markup_view::render_body;
use crate::ui::modal::Modal;
use crate::ui::AuthCtx;

/// The display name to render for a message — the worn persona's name when
/// present, otherwise the message author's display name (Discord-style). Used
/// in 3 places: the live row, the deleted-message trash row, and the persona-
/// info popup, so it stays a small named helper rather than getting re-inlined.
fn display_name(m: &MessageEnvelope) -> String {
    m.persona_name
        .clone()
        .unwrap_or_else(|| m.author_display.clone())
}

/// The composer effect picker's cycle order (W4/T5): no effect → whisper →
/// shout → spell → back to none. Values match the server's validated set
/// (`MESSAGE_EFFECTS` in `server/messages/posting.rs`).
fn next_effect(cur: Option<&str>) -> Option<&'static str> {
    match cur {
        None => Some("whisper"),
        Some("whisper") => Some("shout"),
        Some("shout") => Some("spell"),
        _ => None,
    }
}

/// Glyph shown on the effect-picker button per mode (◌ = no effect).
fn effect_glyph(cur: Option<&str>) -> &'static str {
    match cur {
        Some("whisper") => "🤫",
        Some("shout") => "📣",
        Some("spell") => "✨",
        _ => "◌",
    }
}

/// `title`/`aria-label` for the effect-picker button: names the CURRENT mode
/// (what the next send will do) and the cycling affordance.
fn effect_label(cur: Option<&str>) -> &'static str {
    match cur {
        Some("whisper") => "Message effect: whisper — blurred until tapped. Click to change.",
        Some("shout") => "Message effect: shout — shake and warm tint. Click to change.",
        Some("spell") => "Message effect: spell — glow and sparks. Click to change.",
        _ => "Message effect: none. Click to cycle whisper, shout, spell.",
    }
}

/// Per-kind action affordances for one message row — THE single source for
/// what a message offers, shared by the hover `.msg-actions` row (`meta.rs`)
/// and the touch radial (`radial.rs`) so the two surfaces can never drift.
/// Built by [`message_actions`].
#[derive(Clone, Copy)]
struct MessageActions {
    reply: bool,
    copy: bool,
    edit: bool,
    delete: bool,
}

impl MessageActions {
    /// Number of offered actions — zero means "never arm the radial"; the
    /// count also picks the radial's n2/n4 arc spread.
    fn count(self) -> usize {
        usize::from(self.reply)
            + usize::from(self.copy)
            + usize::from(self.edit)
            + usize::from(self.delete)
    }
}

/// Map a message `kind` (+ viewer ownership) to its action affordances.
/// Conservative on purpose: ONLY `kind='user'` is mutable (edit/delete,
/// owner-gated); `system` (Nova DOT) offers nothing — immutable, not
/// repliable, matching its actionless meta row exactly; `roll` (W4/T6 Fate
/// Engine) is reply+copy only; any UNKNOWN/future kind gets reply+copy but
/// NEVER edit/delete.
fn message_actions(kind: &str, mine: bool) -> MessageActions {
    match kind {
        "user" => MessageActions {
            reply: true,
            copy: true,
            edit: mine,
            delete: mine,
        },
        "system" => MessageActions {
            reply: false,
            copy: false,
            edit: false,
            delete: false,
        },
        // Rolls are FULLY immutable even for their author — the server 403s
        // both edit and delete on kind='roll' (server/messages/editing.rs,
        // cheating-proof) — so never offer edit/delete here; reply+copy stay.
        "roll" => MessageActions {
            reply: true,
            copy: true,
            edit: false,
            delete: false,
        },
        _ => MessageActions {
            reply: true,
            copy: true,
            edit: false,
            delete: false,
        },
    }
}

/// Channel-switch disarm hook for the radial: cancels a pending long-press
/// and closes an open menu (`act::channel::open_channel_at` calls this so a
/// press straddling the switch can't open a menu carrying the OLD channel's
/// envelope). Hydrate-only — its only caller is the hydrate `open_channel_at`.
#[cfg(feature = "hydrate")]
pub(super) fn disarm_radial() {
    radial::disarm();
}

/// The "NEW" unread-frontier divider (UX evolution #9): a virtual row marking
/// where unread began when the channel was opened, rendered above the first
/// row strictly past the captured baseline. Sentinel discipline (the
/// skeleton-row rule): it is NOT a message — its dom id is deliberately NOT
/// `msg-`-prefixed, so the delegated radial handlers and the message-anchor
/// lookups can never resolve it, and it never enters seen/cursor bookkeeping.
/// The id doubles as the unread jump's anchor target
/// (`act::reentry::NEW_DIVIDER_ANCHOR`), so landing there shows the frontier
/// line itself, not an unmarked message among look-alike cards.
fn new_divider_row() -> impl IntoView {
    view! {
        <li class="msg-divider new-divider" id=act::reentry::NEW_DIVIDER_ANCHOR
            role="separator" aria-label="new messages">
            <span class="divider-line" aria-hidden="true"></span>
            <span class="divider-label">"NEW"</span>
            <span class="divider-line" aria-hidden="true"></span>
        </li>
    }
}

/// A date-separator row (UX evolution #9) between messages from different
/// days. `label` is the ISO `YYYY-MM-DD` date — locale-stable on every device
/// (and the native Swedish date shape), computed against the VIEWER's local
/// midnight on hydrate (`act::reentry::date_label`). Same sentinel discipline
/// as the NEW divider: no `msg-` id, never in seen/cursor bookkeeping.
fn date_divider_row(label: String) -> impl IntoView {
    let aria = format!("messages from {label}");
    view! {
        <li class="msg-divider date-divider" role="separator" aria-label=aria>
            <span class="divider-line" aria-hidden="true"></span>
            <span class="divider-label">{label}</span>
            <span class="divider-line" aria-hidden="true"></span>
        </li>
    }
}

/// The clickable reply quote rendered ABOVE a reply's body (L-3): the parent
/// author + a short body snippet. Clicking it scrolls the parent into view via
/// the same `msg-{id}` anchor the deep-link / unread-pill / older-history paths
/// use. A non-reply message renders no quote (the caller only calls this for
/// `Some(reply_to)`).
fn reply_quote(r: crate::protocol::ReplyPreview) -> impl IntoView {
    let pid = r.id.clone();
    view! {
        <button class="reply-quote" title="jump to replied message"
            on:click=move |_| {
                #[cfg(feature = "hydrate")]
                if let Some(el) = leptos::prelude::document()
                    .get_element_by_id(&format!("msg-{pid}"))
                {
                    el.scroll_into_view();
                }
                #[cfg(not(feature = "hydrate"))]
                let _ = &pid;
            }>
            <span class="reply-quote-who">{r.author_display}</span>
            <span class="reply-quote-body">{r.body_snippet}</span>
        </button>
    }
}

/// One row in the deleted-messages panel: the message snippet plus a Restore button.
fn deleted_message_row(s: Shell, m: MessageEnvelope, auth_id: Option<String>) -> impl IntoView {
    let cid = s
        .sel
        .sel_channel
        .get_untracked()
        .map(|c| c.id)
        .unwrap_or_default();
    let mid_restore = m.id.clone();
    let who = display_name(&m);
    let when = format_local_time(&m.sent_at);
    let body_preview: String = m.body.chars().take(120).collect();
    // Only the message's own author can restore it (mirrors server-side require_own_message).
    let is_mine = auth_id.as_deref() == Some(m.author_id.as_str());
    view! {
        <li class="trash-item trash-msg-item">
            <div class="trash-msg-meta">
                <span class="trash-msg-who">{who}</span>
                <time class="when trash-msg-when">{when}</time>
            </div>
            <p class="trash-msg-body">{body_preview}</p>
            {is_mine.then(|| {
                let cid = cid.clone();
                view! {
                    <button class="trash-restore"
                        on:click=move |_| act::restore_deleted_message(s, cid.clone(), mid_restore.clone())>
                        "Restore"
                    </button>
                }
            })}
        </li>
    }
    .into_any()
}

/// True on touch-primary (coarse-pointer) devices — phones/tablets. Shared
/// touch detection for the composer's Enter behaviour and the W4/T4
/// long-press radial menu (`radial.rs` calls it via `super::is_touch`).
#[cfg(feature = "hydrate")]
fn is_touch() -> bool {
    leptos::web_sys::window()
        .and_then(|w| w.match_media("(pointer: coarse)").ok().flatten())
        .map(|m| m.matches())
        .unwrap_or(false)
}

/// True on touch-primary devices (phones/tablets), where the on-screen
/// keyboard's Enter must insert a newline rather than send — there's no
/// Shift+Enter, so Enter-to-send would make multi-line messages impossible.
/// Desktop (fine pointer) keeps Enter-to-send / Shift+Enter-for-newline.
#[cfg(feature = "hydrate")]
fn enter_inserts_newline() -> bool {
    is_touch()
}

/// Insert markup around the textarea's current selection. With a non-empty
/// selection the chosen `open`/`close` wrap it; with no selection an empty
/// `open``close` is inserted and the caret is placed between the two markers.
/// Hydrate-only DOM work (selection ranges are in UTF-16 units, so we splice in
/// JS-string space); the ssr fallback just appends the markers.
#[cfg(feature = "hydrate")]
pub(super) fn apply_markup(
    s: Shell,
    ta_ref: NodeRef<leptos::html::Textarea>,
    open: &str,
    close: &str,
) {
    let Some(el) = ta_ref.get() else {
        s.composer.compose.update(|c| {
            c.push_str(open);
            c.push_str(close);
        });
        return;
    };
    let start = el.selection_start().ok().flatten().unwrap_or(0);
    let end = el.selection_end().ok().flatten().unwrap_or(start);
    let v = js_sys::JsString::from(el.value().as_str());
    let before = v.slice(0, start).as_string().unwrap_or_default();
    let sel = v.slice(start, end).as_string().unwrap_or_default();
    let after = v.slice(end, v.length()).as_string().unwrap_or_default();
    s.composer
        .compose
        .set(format!("{before}{open}{sel}{close}{after}"));

    let open_u = open.encode_utf16().count() as u32;
    let close_u = close.encode_utf16().count() as u32;
    // Empty selection → caret between the markers; otherwise just past the close.
    let caret = if start == end {
        start + open_u
    } else {
        end + open_u + close_u
    };
    // Defer the caret set until after Leptos rewrites prop:value on the next
    // tick (writing .value otherwise resets the cursor to the end).
    leptos::task::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(0).await;
        let _ = el.set_selection_range(caret, caret);
        let _ = el.focus();
    });
}

#[cfg(not(feature = "hydrate"))]
pub(super) fn apply_markup(
    s: Shell,
    _ta_ref: NodeRef<leptos::html::Textarea>,
    open: &str,
    close: &str,
) {
    s.composer.compose.update(|c| {
        c.push_str(open);
        c.push_str(close);
    });
}

/// Apply a color swatch: record it into the quick-swap history (move-to-front,
/// dedup, cap-3) + persist, then wrap the selection in `[name]…[/name]` via
/// [`apply_markup`] (unchanged). Shared by the inline quick swatches and the
/// full popover.
fn apply_color(s: Shell, ta_ref: NodeRef<leptos::html::Textarea>, name: &str) {
    let next = act::record_color(&s.composer.last_used_colors.get_untracked(), name);
    act::save_color_history(&next);
    s.composer.last_used_colors.set(next);
    apply_markup(s, ta_ref, &format!("[{name}]"), &format!("[/{name}]"));
}

/// Feature-detect `field-sizing: content` via the CSS Support API. When
/// supported, the composer textarea grows + shrinks natively (see SCSS) and
/// the JS auto-grow Effect can short-circuit — avoiding the per-keystroke
/// `style.height="auto" → measure` flicker that surfaces as a composer
/// shake on Android Chrome (feedback row bzuypww1phg0lc1eju6p).
///
/// Reflection-driven through `window.CSS.supports("field-sizing", "content")`
/// so we don't need a new web-sys feature. Returns false on any failure
/// (CSS object missing, supports() throws, return value not boolean) so the
/// JS fallback runs — a strict superset of today's behaviour.
#[cfg(feature = "hydrate")]
fn supports_field_sizing_content() -> bool {
    use wasm_bindgen::{JsCast, JsValue};
    (|| -> Option<bool> {
        let win = leptos::web_sys::window()?;
        let css = js_sys::Reflect::get(&win, &JsValue::from_str("CSS")).ok()?;
        if css.is_undefined() || css.is_null() {
            return None;
        }
        let supports: js_sys::Function = js_sys::Reflect::get(&css, &JsValue::from_str("supports"))
            .ok()?
            .dyn_into()
            .ok()?;
        let r = supports
            .call2(
                &css,
                &JsValue::from_str("field-sizing"),
                &JsValue::from_str("content"),
            )
            .ok()?;
        r.as_bool()
    })()
    .unwrap_or(false)
}

/// The `<html>` element, for the measured `--composer-h` custom property
/// (UX evolution #11 placement contract): set on the document ROOT so both
/// the fixed toast host (a shell-level child) and the channel floats inside
/// `.channel-view` inherit the same anchor var.
#[cfg(feature = "hydrate")]
fn doc_root() -> Option<leptos::web_sys::HtmlElement> {
    use wasm_bindgen::JsCast;
    leptos::web_sys::window()?
        .document()?
        .document_element()?
        .dyn_into::<leptos::web_sys::HtmlElement>()
        .ok()
}

#[component]
pub(crate) fn ChannelPane() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");

    // Composer emoji picker + `:`-autocomplete + live preview state (all
    // component-local). `ac_token` holds the active `:query` token as
    // (start_utf16, end_utf16, query); `ac_index` is the highlighted suggestion.
    let emoji_open = RwSignal::new(false);
    let emoji_query = RwSignal::new(String::new());
    // Composer color picker: the full 8-swatch popover toggle (the 3 quick
    // swatches render inline). Mirrors the emoji popover's open/backdrop pattern.
    let color_open = RwSignal::new(false);
    let preview_on = RwSignal::new(act::compose_preview_enabled());
    let ac_token = RwSignal::new(None::<(u32, u32, String)>);
    let ac_index = RwSignal::new(0usize);
    // W4/T5 whisper reveal: ids of whispered messages the viewer has tapped
    // open. A pane-level set rather than per-row signals (the markup_view
    // spoiler pattern) because every poll/ingest re-renders the whole row map,
    // which would reset per-row state mid-conversation. Tapping the blurred
    // text toggles membership; message ids are globally unique, so entries
    // from other channels are harmless.
    let revealed = RwSignal::new(std::collections::HashSet::<String>::new());
    // W4/T2 charging send button: fraction of a "full" message composed,
    // driving the Send button's conic-gradient ring via the `--charge`
    // custom property. ~280 chars FEELS full — it is not a length limit.
    // Counted in chars (not bytes) so multibyte text/emoji don't over-fill,
    // and TRIMMED to mirror `send_message`'s guard — whitespace-only compose
    // must not light a ring on a button whose send path no-ops. (`.charging`
    // below is `charge > 0`, so it follows the same trimmed predicate.
    // Attachments-only stays 0: the ring reflects text length.)
    let charge = Memo::new(move |_| {
        (s.composer
            .compose
            .with(|c| c.trim().chars().count())
            .min(280) as f64)
            / 280.0
    });
    // Typing-ping throttle (#19): epoch-ms of the last `POST /typing` we fired,
    // so on:input pings at most once every ~2s while the user types instead of
    // every keystroke. `StoredValue` (not a signal) — it's plumbing, not UI.
    #[cfg(feature = "hydrate")]
    let last_typing_ping = StoredValue::new(0.0_f64);

    // Auto-grow the composer to fit its content, up to the CSS max-height
    // (then it scrolls). Tracking `compose` covers both typing and the
    // programmatic clear after send. Hydrate-only; ssr leaves it min-height.
    //
    // PRIMARY: `field-sizing: content` in `style/_content.scss` handles this
    // natively in modern browsers (Chrome 123+ / Safari 17.4+ / Firefox 124+,
    // all March 2024). When that path is active, the textarea grows + shrinks
    // without any JS, and the per-keystroke shake reported on Foxtrot's
    // Android Chrome (feedback row bzuypww1phg0lc1eju6p) disappears because
    // we're no longer running a `style.height="auto" → measure → style.height`
    // dance at every keystroke.
    //
    // FALLBACK: when `CSS.supports("field-sizing", "content")` is false (older
    // browsers), keep the JS measurement so the textarea still grows. The
    // deferred-microtask measure stays — without it the post-send clear (#28)
    // reads stale `scroll_height` on mobile and the textarea stays super-tall
    // until the next keystroke.
    let composer_ref = NodeRef::<leptos::html::Textarea>::new();
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        s.composer.compose.track();
        let Some(el) = composer_ref.get() else {
            return;
        };
        if supports_field_sizing_content() {
            return;
        }
        leptos::task::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(0).await;
            // Deref to web_sys::HtmlElement so its inherent `style()` wins over
            // tachys' `ElementExt::style` (both in scope via leptos prelude).
            let style = (*el).style();
            let _ = style.set_property("height", "auto");
            let _ = style.set_property("height", &format!("{}px", el.scroll_height()));
        });
    });

    // Measured composer-height var (UX evolution #11 placement contract — the
    // judges' risk line): a ResizeObserver mirrors the composer band's REAL
    // height into `--composer-h` on `<html>`, so the toast capsule
    // (`_toast.scss`) and the channel floats (.unread-pill / .jump-bottom,
    // `_content.scss`) all anchor to the composer's actual top edge. Wrapped
    // toolbar rows on narrow phones, the growing textarea, reply/edit banners
    // and the attachment strip all move the floats with them instead of
    // being overlapped — fluid measured geometry, no hardcoded band height.
    // Cleared on unmount (pane switch / logout) so the SCSS fallbacks apply
    // wherever no composer exists.
    let composer_box = NodeRef::<leptos::html::Div>::new();
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;
        let Some(el) = composer_box.get() else {
            return;
        };
        let target = el.clone();
        let set_var = move || {
            if let Some(root) = doc_root() {
                // Inherent web_sys `style()` (same deref note as above).
                let _ = root
                    .style()
                    .set_property("--composer-h", &format!("{}px", el.offset_height()));
            }
        };
        set_var();
        let cb = Closure::<dyn FnMut()>::new(set_var);
        let observer = leptos::web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()).ok();
        if let Some(o) = &observer {
            o.observe(&target);
        }
        // `SendWrapper` carries the non-`Send` wasm types across `on_cleanup`'s
        // `Send + Sync` bound (the `ui/modal.rs` convention) — WASM is
        // single-threaded, so the wrapper's same-thread assert always holds.
        let held = send_wrapper::SendWrapper::new((observer, cb));
        on_cleanup(move || {
            let (observer, cb) = held.take();
            if let Some(o) = observer {
                o.disconnect();
            }
            drop(cb); // the observer is gone; now the JS shim may drop too
            if let Some(root) = doc_root() {
                let _ = root.style().remove_property("--composer-h");
            }
        });
    });

    // Click-the-name info popup: which message's persona/controller to show.
    let info = RwSignal::new(None::<MessageEnvelope>);

    // W4/T4 radial long-press menu: the message whose touch action menu is
    // open (None when closed). Channel-pane-local — the delegated `<ul>`
    // long-press handlers and the menu render live in this component, so it
    // never needs to ride Shell state. `long_press` is the generation-counter
    // timer tracker (see `radial::LongPress`); `radial_armed` is the
    // manufactured-click guard for the menu's backdrop/buttons, created ONCE
    // here because a per-render StoredValue would leak an arena slot per open.
    let radial = RwSignal::new(None::<radial::RadialState>);
    let long_press = radial::LongPress::new();
    let radial_armed = StoredValue::new(false);
    // Channel switches must disarm a pending press / close an open menu —
    // act::channel::open_channel_at reaches the pane-local state through this
    // registration (see radial::disarm).
    #[cfg(feature = "hydrate")]
    radial::register_disarm(long_press, radial);
    // Lightbox: the clicked message's IMAGE attachments + the index currently
    // shown, or None when closed. Arrow/swipe step the index within this list
    // (images only — videos keep their own inline controls and never enter the
    // gallery); see `LightboxState`. The grid click seeds `idx` to the clicked
    // image; `lb_tf` is the image's CSS transform (scale + pan translate, see
    // `LbTransform`), reset to identity on every open and gallery step. Both
    // live as component-level signals so stepping `idx` / zoom-panning
    // re-renders only the <img>, never the focusable container (which would
    // steal focus and break the arrow-key handler).
    let lightbox = RwSignal::new(None::<LightboxState>);
    let lb_tf = RwSignal::new(LbTransform::default());

    // Auto-scroll. `last_dist` is the px distance from the bottom recorded on
    // the user's last scroll (i.e. pre-append). On a new message: your own →
    // follow when NEAR the bottom; someone else's → only when EXACTLY at the
    // bottom; otherwise leave the scroll position alone (reading history).
    let list_ref = NodeRef::<leptos::html::Ul>::new();
    let last_dist = StoredValue::new(0.0_f64);

    // Scroll/unread aids (all component-local):
    //  - `scrolled_up` toggles the jump-to-bottom arrow's visibility, set from
    //    the on:scroll handler when the user is more than ~200px from bottom.
    //  - `unread` / `first_unread_id` track messages that arrived *while the
    //    user was scrolled up and weren't their own*, so the unread pill can
    //    jump back to the earliest one.
    //  - `prev_count` is the message count seen on the previous effect run; it
    //    distinguishes genuinely-appended messages from the initial load and
    //    from in-place edits/deletes (which don't grow the list).
    let scrolled_up = RwSignal::new(false);
    let unread = RwSignal::new(0_usize);
    // Only read/written from hydrate-gated code (the append effect and the pill
    // click); on ssr they'd be unused, so gate the declarations too.
    #[cfg(feature = "hydrate")]
    let first_unread_id = RwSignal::new(None::<String>);
    #[cfg(feature = "hydrate")]
    let prev_count = StoredValue::new(None::<usize>);

    // `at_bottom` clears the unread state and hides the arrow. Called from the
    // jump-to-bottom click and from the on:scroll handler when the user is back
    // at (or very near) the bottom of the list.
    #[cfg(feature = "hydrate")]
    let mark_seen = move || {
        unread.set(0);
        first_unread_id.set(None);
        scrolled_up.set(false);
    };

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let msgs = s.msg.messages.get();
        // An older-history prepend grows the list at the FRONT; skip the
        // append/scroll/unread logic here (the anchor effect below repositions
        // the viewport instead), but keep prev_count in sync for the next real
        // append.
        if s.msg.anchor_to.get_untracked().is_some() {
            prev_count.set_value(Some(msgs.len()));
            return;
        }
        let me = auth.user.get_untracked().map(|u| u.account_id);
        let mine = msgs
            .last()
            .zip(me.as_ref())
            .map(|(m, id)| &m.author_id == id)
            .unwrap_or(false);

        // Detect genuinely-new appended messages (count grew since last run)
        // vs. the initial load (no previous count) or an edit/delete (count
        // same or smaller). `prev` is the count *before* this batch.
        let count = msgs.len();
        let prev = prev_count.get_value();
        prev_count.set_value(Some(count));

        // The user is "scrolled up" reading history when more than a small
        // slack from the bottom at the moment new messages land.
        let was_scrolled_up = last_dist.get_value() > 4.0;

        if let Some(prev) = prev {
            if count > prev && was_scrolled_up {
                // New arrivals while away from the bottom. Count only the
                // newly-appended messages that aren't the current user's own,
                // and remember the earliest such id for the pill's jump target.
                let mut newly_unread = 0_usize;
                for m in msgs.iter().skip(prev) {
                    let is_mine = me.as_deref() == Some(m.author_id.as_str());
                    if !is_mine {
                        if first_unread_id.get_untracked().is_none() && newly_unread == 0 {
                            first_unread_id.set(Some(m.id.clone()));
                        }
                        newly_unread += 1;
                    }
                }
                if newly_unread > 0 {
                    unread.update(|n| *n += newly_unread);
                }
            }
        }

        let threshold = if mine { 120.0 } else { 4.0 };
        if last_dist.get_value() <= threshold {
            last_dist.set_value(0.0);
            // Following the bottom on this append also clears any unread state.
            mark_seen();
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(0).await;
                if let Some(el) = list_ref.get_untracked() {
                    el.set_scroll_top(el.scroll_height());
                }
            });
        }
    });

    // After an older-history prepend, bring the previously-top message back
    // into view so the viewport doesn't jump, then clear the request.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let Some(id) = s.msg.anchor_to.get() else {
            return;
        };
        leptos::task::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(0).await;
            if let Some(el) = leptos::web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| {
                    // Re-entry (UX evolution #9): the unread jump lands AT
                    // the NEW divider — the sentinel anchor resolves to the
                    // divider row's own dom id, so the frontier line itself
                    // is visible above the first unread message. Every other
                    // anchor is a real `msg-{id}` row, unchanged.
                    if id == act::reentry::NEW_DIVIDER_ANCHOR {
                        d.get_element_by_id(act::reentry::NEW_DIVIDER_ANCHOR)
                    } else {
                        d.get_element_by_id(&format!("msg-{id}"))
                    }
                })
            {
                el.scroll_into_view();
            }
            s.msg.anchor_to.set(None);
        });
    });

    view! {
        <div class="channel-view">
            <ul class="messages" node_ref=list_ref
                // W4/T4 radial long-press: DELEGATED listeners — 5 on this
                // <ul>, not 5 per row (this build has no tachys event
                // delegation, so per-row `on:` means per-row addEventListener
                // calls that the non-keyed list re-attaches wholesale on
                // every message change). The handlers resolve the pressed
                // row from `ev.target().closest("li[id^='msg-']")` and look
                // the envelope up by id only when a press actually fires;
                // pointermove past the slop radius and pointerup/-cancel
                // (the browser claiming the gesture for scrolling) disarm
                // the pending press, so scrolls never fire it. System rows
                // never arm and keep the NATIVE context menu so their text
                // stays copyable on touch. Desktop is untouched — `down`
                // no-ops on a fine pointer; right-click stays native.
                on:pointerdown=move |ev| long_press.down(&ev, s, auth, radial)
                on:pointermove=move |ev| long_press.moved(&ev)
                on:pointerup=move |_| long_press.cancel()
                on:pointercancel=move |_| long_press.cancel()
                on:contextmenu=move |ev| radial::suppress_touch_context_menu(&ev)
                on:scroll=move |_ev| {
                    #[cfg(feature = "hydrate")]
                    if let Some(el) = list_ref.get_untracked() {
                        let dist =
                            (el.scroll_height() - el.scroll_top() - el.client_height()) as f64;
                        last_dist.set_value(dist);
                        // Show the jump arrow once the user is meaningfully up
                        // the history; clear unread state when they reach the
                        // bottom again.
                        scrolled_up.set(dist > 200.0);
                        if dist <= 4.0 {
                            mark_seen();
                        }
                        // Near the top → backfill the previous page of history.
                        if el.scroll_top() < 200 {
                            act::load_older(s);
                        }
                    }
                    #[cfg(not(feature = "hydrate"))]
                    let _ = (&last_dist, &_ev);
                }>
                // Ephemeral loading skeletons (F-7): shown only while the
                // first page is in flight AND no real rows exist yet. Leptos
                // diffing drops them the instant `messages` becomes non-empty.
                // Sentinel `skeleton-N` ids only — never enter seen/cursor.
                {move || should_show_skeletons(
                    s.msg.loading_initial.get(),
                    s.msg.messages.with(Vec::len),
                ).then(skeleton_rows)}
                {move || {
                    let me = auth.user.get().map(|u| u.account_id);
                    let cid = s.sel.sel_channel.get().map(|c| c.id);
                    let msgs = s.msg.messages.get();
                    // Re-entry dividers (UX evolution #9) — pure render-time
                    // grouping over the composite-ordered list (never
                    // re-sorted client-side): `new_before` is the id of the
                    // first row strictly past the unread baseline captured at
                    // channel open; `prev_date` threads the running local-date
                    // label so a separator lands wherever consecutive rows
                    // cross midnight. `prev_date` starting `None` means the
                    // FIRST loaded row always gets a label — deliberately: it
                    // names the day the loaded window opens on (the unloaded
                    // row above may share the date), and a backfill reruns
                    // this whole grouping so it self-corrects to true
                    // crossings as history loads (review; standard chat
                    // behaviour — suppressing it would leave the window's top
                    // dateless). Neither divider is a message: no `msg-`
                    // dom id, never in seen/cursor bookkeeping (sentinel
                    // discipline) — see `new_divider_row`/`date_divider_row`.
                    let new_before = s
                        .msg
                        .new_divider
                        .get()
                        .and_then(|baseline| act::reentry::first_past_baseline(&msgs, &baseline));
                    let mut prev_date: Option<String> = None;
                    msgs.into_iter().map(|m| {
                        let date = act::reentry::date_label(&m.sent_at);
                        let date_row = (prev_date.as_deref() != Some(date.as_str()))
                            .then(|| date_divider_row(date.clone()));
                        prev_date = Some(date);
                        // Date separator ABOVE the NEW divider when both land
                        // on the same row ("a new day — and here is where you
                        // left off").
                        let new_row =
                            (new_before.as_deref() == Some(m.id.as_str())).then(new_divider_row);
                        let atts = m.attachments.clone();
                        let mine = me.is_some() && me.as_deref() == Some(m.author_id.as_str());
                        let body = m.body.clone();
                        let cid = cid.clone();
                        let dom_id = format!("msg-{}", m.id);
                        let reply_quote = m.reply_to.clone().map(reply_quote);
                        // System (Nova DOT) messages get a distinct row + a stripped
                        // meta line (no edit/reply/persona-popup); everything else is
                        // a normal authored message. The `system` class doubles as
                        // the marker the delegated long-press handlers (radial.rs)
                        // check, so system rows never arm the radial and keep the
                        // native context menu.
                        let is_system = m.kind == "system";
                        // Roll results (W4/T6): an authored action (normal meta
                        // row — persona/author name and reply/copy stay) whose
                        // body renders as an animated glass chip below.
                        let is_roll = m.kind == "roll";
                        // W4/T5 delivery effect: a known effect adds an
                        // `effect-{name}` class to the row. Re-whitelisted here
                        // (the server already validates) so an unexpected wire
                        // value can never inject an arbitrary class.
                        let effect = m
                            .effect
                            .as_deref()
                            .filter(|e| matches!(*e, "whisper" | "shout" | "spell"));
                        let is_whisper = effect == Some("whisper");
                        // Directional bubbles: the viewer's own messages carry
                        // `.own` (right-aligned in CSS). System rows are authored
                        // by the Nova DOT account, never the viewer — but branch
                        // order makes them never-"own" regardless. Effects don't
                        // change the base composition (or the full-width
                        // exemptions) — they only append.
                        let base_class = if is_system {
                            "msg system"
                        } else if is_roll {
                            // Full-width banner like the system row (exempt
                            // from the directional-bubble squeeze, never
                            // `.own`) — a roll is table-facing, not a bubble.
                            "msg roll"
                        } else if mine {
                            "msg own"
                        } else {
                            "msg"
                        };
                        let li_class = match effect {
                            Some(e) => format!("{base_class} effect-{e}"),
                            None => base_class.to_string(),
                        };
                        let meta = if is_system {
                            system_message_meta(&m).into_any()
                        } else {
                            message_meta(s, &m, &cid, mine, info).into_any()
                        };
                        // Whisper reveal: the blur sits on `.text` only (the
                        // meta row stays readable); tapping the text toggles
                        // this message's id in the pane-level `revealed` set,
                        // which flips the row's `.revealed` class. A plain
                        // class flip — state, not motion — so it works under
                        // reduced-motion too.
                        let mid = m.id.clone();
                        let text_view = if is_roll {
                            // Glass result chip: die glyph + the
                            // server-generated result text, rendered VERBATIM
                            // (never markup-parsed — the body is the server's
                            // formatted outcome, not user markup).
                            view! {
                                <span class="text roll-chip">
                                    <span class="roll-die" aria-hidden="true">"🎲"</span>
                                    <span class="roll-text">{body.clone()}</span>
                                </span>
                            }
                            .into_any()
                        } else if is_whisper {
                            let mid = m.id.clone();
                            view! {
                                <span class="text"
                                    title="whispered — tap to reveal"
                                    on:click=move |_| revealed.update(|r| {
                                        if !r.insert(mid.clone()) {
                                            r.remove(&mid);
                                        }
                                    })>
                                    {render_body(&body)}
                                </span>
                            }
                            .into_any()
                        } else {
                            view! { <span class="text">{render_body(&body)}</span> }.into_any()
                        };
                        // Whisper veil for media (review): the attachment grid
                        // is a SIBLING of `.text`, so while hidden the CSS
                        // (_content.scss) blurs it and drops its
                        // pointer-events — a tap on the media area then lands
                        // on this wrapper, which REVEALS instead of opening
                        // the lightbox. Insert-only (never a toggle): once
                        // revealed, a lightbox click bubbling back up here
                        // must not re-hide the row mid-open. Non-whisper rows
                        // render the bare grid, no wrapper.
                        let atts_view = (!atts.is_empty()).then(|| {
                            let grid = attachment_grid(atts.clone(), lightbox);
                            if is_whisper {
                                let mid = m.id.clone();
                                view! {
                                    <div class="atts-veil"
                                        title="whispered — tap to reveal"
                                        on:click=move |_| revealed.update(|r| {
                                            r.insert(mid.clone());
                                        })>
                                        {grid}
                                    </div>
                                }
                                .into_any()
                            } else {
                                grid.into_any()
                            }
                        });
                        view! {
                            // Virtual divider rows render as SIBLINGS above the
                            // real row (a multi-root view flattens into the
                            // <ul>) — exactly where the date flips or the
                            // unread frontier begins.
                            {date_row}
                            {new_row}
                            // Long-press handling is delegated to the <ul> above —
                            // no per-row listeners (and no per-row envelope clone);
                            // the row only needs its `msg-{id}` dom id (+ the
                            // `system` class) for the handlers to resolve it at
                            // fire time.
                            <li class=li_class id=dom_id
                                class:revealed=move || {
                                    is_whisper && revealed.with(|r| r.contains(&mid))
                                }>
                                {meta}
                                {reply_quote}
                                // Editing happens in the main composer (✎ →
                                // act::start_edit), not inline, so the body is
                                // always just rendered markup (whisper rows add
                                // the tap-to-reveal handler above).
                                {text_view}
                                {atts_view}
                            </li>
                        }
                    }).collect_view()
                }}
                // Live draft preview (opt-in via the 👁 toggle): a non-persisted
                // "ghost" row at the bottom of the list rendering the composer
                // draft exactly as it'll appear when sent. Re-renders reactively
                // off `s.composer.compose`; vanishes when the draft is empty or after send.
                {move || (preview_on.get() && !s.composer.compose.get().trim().is_empty()).then(|| {
                    // Use the currently-worn persona's name + avatar; fall back to
                    // the signed-in account's display name (matching real-message
                    // resolution) with no avatar if no persona is worn.
                    let (who, avatar_id) = s.social
                        .active_persona
                        .get()
                        .and_then(|pid| {
                            s.social.personas
                                .get()
                                .into_iter()
                                .find(|p| p.id == pid)
                                .map(|p| (p.name, p.avatar_id))
                        })
                        .unwrap_or_else(|| {
                            let name = auth
                                .user
                                .get()
                                .map(|u| u.display_name)
                                .unwrap_or_default();
                            (name, None)
                        });
                    let avatar_el = chat_avatar(&avatar_id, &who, false);
                    view! {
                        <li class="msg msg-draft">
                            <div class="meta">
                                {avatar_el}
                                <span class="who">{who}</span>
                            </div>
                            <span class="text">{render_body(&s.composer.compose.get())}</span>
                        </li>
                    }
                })}
                // Ghost Quill rows (W4/T7): OTHER members' live drafts, below
                // the real messages. Their own `ghost_drafts` signal (never
                // the message list), fetched from the permission-checked
                // /typing-drafts endpoint on SSE nudges, rendered ONLY while
                // the receiver's pref is on (toggling off hides them at
                // once). Draft text renders as plain text — clearly-not-real
                // styling (dashed, italic) over fidelity. Static rows, no
                // animation, so nothing to kill for reduced motion.
                {move || s.prefs.ghost_quill.get().then(|| {
                    s.msg.ghost_drafts.get().into_iter().map(|g| {
                        view! {
                            <li class="msg msg-ghost">
                                <div class="meta">
                                    <span class="who">{g.display_name} " ✒️"</span>
                                </div>
                                <span class="text">{g.draft}</span>
                            </li>
                        }
                    }).collect_view()
                })}
            </ul>

            // Unread pill — shown only when messages arrived while the user was
            // scrolled up. Clicking it scrolls the earliest unread message into
            // view (and clears the unread state).
            {move || {
                (unread.get() > 0).then(|| {
                    let n = unread.get();
                    let label = if n == 1 {
                        "1 new message ↓".to_string()
                    } else {
                        format!("{n} new messages ↓")
                    };
                    view! {
                        <button class="unread-pill"
                            on:click=move |_| {
                                #[cfg(feature = "hydrate")]
                                {
                                    if let Some(id) = first_unread_id.get_untracked() {
                                        if let Some(el) = leptos::prelude::document()
                                            .get_element_by_id(&format!("msg-{id}"))
                                        {
                                            el.scroll_into_view();
                                        }
                                    }
                                    mark_seen();
                                }
                            }>
                            {label}
                        </button>
                    }
                })
            }}

            // Jump-to-bottom arrow — shown only when scrolled up past the
            // threshold. Clicking it jumps to the bottom and clears unread.
            {move || {
                scrolled_up.get().then(|| {
                    view! {
                        <button class="jump-bottom" title="jump to latest"
                            on:click=move |_| {
                                #[cfg(feature = "hydrate")]
                                {
                                    if let Some(el) = list_ref.get_untracked() {
                                        el.set_scroll_top(el.scroll_height());
                                    }
                                    last_dist.set_value(0.0);
                                    mark_seen();
                                }
                            }>
                            "↓"
                        </button>
                    }
                })
            }}

            // Deleted-messages panel — shown when "Show deleted" is toggled.
            {move || s.trash.show_msg_trash.get().then(|| {
                let me = auth.user.get().map(|u| u.account_id);
                let msgs = s.trash.deleted_messages.get();
                view! {
                    <div class="trash-msg-panel">
                        <div class="trash-panel-header">
                            <span>"🗑 Deleted messages"</span>
                        </div>
                        {if msgs.is_empty() {
                            view! { <p class="muted pad">"No deleted messages."</p> }.into_any()
                        } else {
                            view! {
                                <ul class="trash-list">
                                    {msgs.into_iter().map(|m| {
                                        deleted_message_row(s, m, me.clone())
                                    }).collect_view()}
                                </ul>
                            }.into_any()
                        }}
                    </div>
                }
            })}

            // "%name% is typing…" line (#19), fed by the message poll. Renders
            // nothing when nobody else is typing. A constellation of orbiting
            // stars (one per typist, capped at 3; W4/T1) decorates the line —
            // purely decorative (aria-hidden) ALONGSIDE the names text, which
            // stays for accessibility. Per-star stagger/color is nth-child
            // CSS; the typing payload carries no per-persona color, so stars
            // alternate the shared accent/mint hues.
            {move || {
                let names = s.msg.typing.get();
                let line = match names.len() {
                    0 => return ().into_any(),
                    1 => format!("{} is typing…", names[0]),
                    2 => format!("{} and {} are typing…", names[0], names[1]),
                    _ => "Several people are typing…".to_string(),
                };
                let stars = (0..names.len().min(3))
                    .map(|_| view! { <span class="star"></span> })
                    .collect_view();
                view! {
                    <div class="typing-indicator">
                        <span class="constellation" aria-hidden="true">{stars}</span>
                        {line}
                    </div>
                }
                .into_any()
            }}

            // node_ref: the ResizeObserver above mirrors this band's measured
            // height into the `--composer-h` anchor var (UX evolution #11).
            <div class="composer" node_ref=composer_box>
                // "Replying to X" banner (L-3): shown while a reply target is
                // staged; the ✕ clears it back to a normal send.
                {move || s.composer.replying_to.get().map(|r| {
                    let snippet = r.body_snippet.clone();
                    view! {
                        <div class="reply-banner">
                            <span class="reply-banner-text">
                                "Replying to "<strong>{r.author_display}</strong>
                                <span class="reply-banner-snippet">{snippet}</span>
                            </span>
                            <button class="reply-banner-cancel" type="button" title="cancel reply"
                                on:click=move |_| act::cancel_reply(s)>"✕"</button>
                        </div>
                    }
                })}
                // "Editing message" banner: shown while a message is loaded into
                // the composer for editing; the ✕ (or Esc) cancels and restores
                // the stashed draft. The Send button reads "Save" meanwhile.
                {move || s.composer.editing.get().map(|_| {
                    view! {
                        <div class="edit-banner">
                            <span class="edit-banner-text">"Editing message"</span>
                            <button class="edit-banner-cancel" type="button" title="cancel edit"
                                on:click=move |_| act::cancel_edit(s)>"✕"</button>
                        </div>
                    }
                })}
                <div class="toolbar">
                    // Attach images: a hidden multi-file input behind a 📎 label.
                    // Each pick uploads immediately and stages the media id.
                    <label class="fmt attach" title="attach a file">
                        "📎"
                        // NO `accept`: on Android a media `accept` hint makes Chrome
                        // launch the system photo picker (Google Photos on this
                        // device), which the user doesn't want; omitting it gives the
                        // generic Files chooser instead. A PWA can't target a specific
                        // gallery app, so this is the better of the two reachable
                        // options. Any file type the server allowlist accepts can be
                        // attached; the server (`is_allowed_attachment_mime`) is the
                        // authority and rejects script-capable types.
                        <input type="file" multiple style="display:none"
                            on:change=move |_ev| {
                                #[cfg(feature = "hydrate")]
                                {
                                    use leptos::wasm_bindgen::JsCast;
                                    if let Some(input) = _ev.target().and_then(|t| {
                                        t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok()
                                    }) {
                                        if let Some(files) = input.files() {
                                            // Soft cap (W7/B1-client): refuse to queue uploads
                                            // beyond COMPOSER_MAX_ATTACHMENTS so the user gets a
                                            // toast instead of an upload-then-server-reject
                                            // roundtrip. The server enforces the same ceiling
                                            // (`MAX_ATTACHMENTS` in src/server/messages/mod.rs).
                                            let mut current =
                                                s.composer.compose_attachments.get_untracked().len();
                                            let mut overflowed = false;
                                            // Collect the picked files first, then upload the
                                            // whole pick at once so the staged order matches the
                                            // selection order (mnjs2ljw…), not upload-completion
                                            // order. Any file type is queued client-side; the
                                            // server allowlist (`is_allowed_attachment_mime`) is
                                            // the authority and rejects script-capable types.
                                            let mut picked: Vec<web_sys::File> = Vec::new();
                                            for i in 0..files.length() {
                                                if let Some(file) = files.get(i) {
                                                    if current >= COMPOSER_MAX_ATTACHMENTS {
                                                        overflowed = true;
                                                        break;
                                                    }
                                                    picked.push(file);
                                                    current += 1;
                                                }
                                            }
                                            act::add_compose_attachments(s, picked);
                                            if overflowed {
                                                s.composer.status.set(format!(
                                                    "Attachment limit ({COMPOSER_MAX_ATTACHMENTS}) reached"
                                                ));
                                            }
                                        }
                                        // Clear so re-picking the same file re-fires.
                                        input.set_value("");
                                    }
                                }
                                #[cfg(not(feature = "hydrate"))]
                                {
                                    let _ = &_ev;
                                    act::add_compose_attachment(s);
                                }
                            }/>
                    </label>
                    <button class="fmt" title="bold"
                        on:click=move |_| apply_markup(s, composer_ref, "**", "**")>
                        <strong>"B"</strong>
                    </button>
                    <button class="fmt" title="italic"
                        on:click=move |_| apply_markup(s, composer_ref, "*", "*")>
                        <em>"i"</em>
                    </button>
                    // Discord-style block formats. Headers / subtext are
                    // line-leading prefixes (insert the marker, no closer);
                    // inline code wraps the selection, the fence opens/closes
                    // a block.
                    <button class="fmt" title="heading"
                        on:click=move |_| apply_markup(s, composer_ref, "# ", "")>
                        "H"
                    </button>
                    <button class="fmt" title="subtext"
                        on:click=move |_| apply_markup(s, composer_ref, "-# ", "")>
                        <small>"-#"</small>
                    </button>
                    <button class="fmt" title="inline code"
                        on:click=move |_| apply_markup(s, composer_ref, "`", "`")>
                        <code>"</>"</code>
                    </button>
                    <button class="fmt" title="code block"
                        on:click=move |_| apply_markup(s, composer_ref, "```\n", "\n```")>
                        <code>"{}"</code>
                    </button>
                    // Quick-swap color swatches: only the 3 last-used colors
                    // render inline (compressed when history < 3); the ▼ toggle
                    // opens a popover with the full palette (feedback
                    // rli3tsora4ho7lsi9q31).
                    {move || {
                        s.composer.last_used_colors.get().into_iter()
                            .filter(|n| Color::from_name(n).is_some())
                            .take(3)
                            .map(|name| view! {
                                <button class=format!("swatch mk-bg-{name}") title=name.clone()
                                    on:click=move |_| apply_color(s, composer_ref, &name)>
                                </button>
                            })
                            .collect_view()
                    }}
                    <button class="fmt color-more" title="more colors"
                        on:click=move |_| color_open.update(|o| *o = !*o)>
                        "▼"
                    </button>
                    // Emoji picker toggle + live-preview toggle. The preview
                    // toggle persists per-user (localStorage) like the other
                    // composer prefs.
                    <button class="fmt" title="emoji"
                        on:click=move |_| emoji_open.update(|o| *o = !*o)>
                        "😀"
                    </button>
                    <button class="fmt" title="preview"
                        on:click=move |_| {
                            let v = !preview_on.get_untracked();
                            preview_on.set(v);
                            act::set_compose_preview(v);
                        }>
                        "👁"
                    </button>
                </div>
                // Color palette popover: all 8 swatches in a small grid; a
                // full-viewport backdrop closes it on an outside click (mirrors
                // the emoji popover). Click-to-apply only for v1.
                {move || color_open.get().then(|| view! {
                    <div class="emoji-backdrop" on:click=move |_| color_open.set(false)></div>
                    <div class="color-picker">
                        {Color::ALL.into_iter().map(|col| {
                            let name = col.name();
                            view! {
                                <button class=format!("swatch mk-bg-{name}") title=name
                                    on:click=move |_| {
                                        apply_color(s, composer_ref, name);
                                        color_open.set(false);
                                    }>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                })}
                // Emoji picker popover: a search box over a categorised grid of
                // the open guild's custom emoji plus the standard-unicode set.
                // A full-viewport backdrop closes it on an outside click.
                {move || emoji_open.get().then(|| view! {
                    <div class="emoji-backdrop" on:click=move |_| emoji_open.set(false)></div>
                    <div class="emoji-picker">
                        <input class="emoji-search" placeholder="search emoji"
                            prop:value=move || emoji_query.get()
                            on:input=move |ev| emoji_query.set(event_target_value(&ev))/>
                        <div class="emoji-grid">
                            {move || {
                                let q = emoji_query.get().trim().to_lowercase();
                                let custom = s.sel.guild_emoji.get();
                                if q.is_empty() {
                                    // Server custom emoji first, then each unicode category.
                                    let server = (!custom.is_empty()).then(|| view! {
                                        <div class="emoji-cat">"Server"</div>
                                        <div class="emoji-cat-items">
                                            {custom.iter().cloned().map(|e| custom_emoji_btn(
                                                s, composer_ref, emoji_open, e.name, e.media_id,
                                            )).collect_view()}
                                        </div>
                                    });
                                    let cats = GROUPS.iter().map(|label| {
                                        let items = data::by_group(label);
                                        view! {
                                            <div class="emoji-cat">{*label}</div>
                                            <div class="emoji-cat-items">
                                                {items.into_iter().map(|e| unicode_emoji_btn(
                                                    s, composer_ref, emoji_open,
                                                    e.shortcode, e.glyph,
                                                )).collect_view()}
                                            </div>
                                        }
                                    }).collect_view();
                                    view! { {server} {cats} }.into_any()
                                } else {
                                    // Filtered: matching custom emoji, then unicode hits.
                                    let custom_hits = custom.into_iter()
                                        .filter(|e| e.name.to_lowercase().contains(&q))
                                        .map(|e| custom_emoji_btn(
                                            s, composer_ref, emoji_open, e.name, e.media_id,
                                        ))
                                        .collect_view();
                                    let std_hits = data::search(&q, 80).into_iter()
                                        .map(|e| unicode_emoji_btn(
                                            s, composer_ref, emoji_open, e.shortcode, e.glyph,
                                        ))
                                        .collect_view();
                                    view! {
                                        <div class="emoji-cat-items">{custom_hits}{std_hits}</div>
                                    }.into_any()
                                }
                            }}
                        </div>
                    </div>
                })}
                // Pending attachments: thumbnails of staged uploads, each with
                // a per-item upload-progress overlay (F-8) + a remove button,
                // and a retry button when the upload failed. The `Ready` slots'
                // media ids are sent (and cleared) on the next message.
                {move || {
                    use super::state::UploadStatus;
                    let atts = s.composer.compose_attachments.get();
                    (!atts.is_empty()).then(|| view! {
                        <div class="compose-attachments">
                            {atts.into_iter().map(|st| {
                                let key = st.key;
                                let id = st.att.id.clone();
                                let is_video = st.att.mime.starts_with("video/");
                                let ready = st.status == UploadStatus::Ready;
                                // Thumbnail only resolves once the media id is real
                                // (`Ready`); while uploading/failed the slot shows a
                                // neutral placeholder behind the progress overlay.
                                let thumb = if !ready {
                                    view! { <div class="pending-att-placeholder"></div> }.into_any()
                                } else if is_video {
                                    view! {
                                        <video src=format!("/media/{id}") muted preload="metadata"></video>
                                    }.into_any()
                                } else {
                                    // GIFs raw so the preview animates; the ?w= thumb
                                    // would flatten them to a static JPEG frame.
                                    let src = if st.att.mime == "image/gif" {
                                        format!("/media/{id}")
                                    } else {
                                        format!("/media/{id}?w=256")
                                    };
                                    view! {
                                        <img src=src alt="pending attachment"/>
                                    }.into_any()
                                };
                                let overlay = match &st.status {
                                    UploadStatus::Uploading(frac) => {
                                        let pct = (frac.unwrap_or(0.0) * 100.0).round() as i32;
                                        let indeterminate = frac.is_none();
                                        view! {
                                            <div class="att-progress"
                                                class:indeterminate=indeterminate
                                                role="progressbar"
                                                aria-label="uploading">
                                                <div class="att-progress-bar"
                                                    style=format!("width:{pct}%")></div>
                                            </div>
                                        }.into_any()
                                    }
                                    UploadStatus::Failed(msg) => {
                                        let msg = msg.clone();
                                        view! {
                                            <div class="att-failed" title=msg>
                                                <button class="att-retry" type="button" title="retry"
                                                    on:click=move |_| act::retry_compose_attachment(s, key)>
                                                    "↻"
                                                </button>
                                            </div>
                                        }.into_any()
                                    }
                                    UploadStatus::Ready => ().into_any(),
                                };
                                view! {
                                    <div class="pending-att"
                                        class:uploading=matches!(st.status, UploadStatus::Uploading(_))
                                        class:failed=matches!(st.status, UploadStatus::Failed(_))>
                                        {thumb}
                                        {overlay}
                                        <button class="att-remove" type="button" title="remove"
                                            on:click=move |_| act::remove_compose_attachment(s, key)>
                                            "✕"
                                        </button>
                                    </div>
                                }
                            }).collect_view()}
                        </div>
                    })
                }}
                <textarea
                    node_ref=composer_ref
                    prop:value=move || s.composer.compose.get()
                    on:input=move |ev| {
                        let value = event_target_value(&ev);
                        s.composer.compose.set(value.clone());
                        // Persist the current channel's draft on every keystroke
                        // so a reload / PWA close doesn't lose unsent typing.
                        act::channel::save_draft(s, &value);
                        // Track the trailing `:query` token under the caret to
                        // drive the autocomplete popover.
                        #[cfg(feature = "hydrate")]
                        {
                            if let Some(el) = composer_ref.get() {
                                let caret = el.selection_start().ok().flatten().unwrap_or(0);
                                let before = js_sys::JsString::from(el.value().as_str())
                                    .slice(0, caret)
                                    .as_string()
                                    .unwrap_or_default();
                                match active_shortcode_token(&before) {
                                    Some((q, len)) => {
                                        ac_token.set(Some((caret - len, caret, q)));
                                        ac_index.set(0);
                                    }
                                    None => ac_token.set(None),
                                }
                            }
                            // Throttled "is typing" ping (#19): fire at most once
                            // per ~2s while typing. Fire-and-forget; ignore errors.
                            // Ghost Quill (W4/T7): with the SENDER's pref on, the
                            // ping carries the compose text as `draft` (empty text
                            // included — the server clears the entry on it); with
                            // the pref off it stays the classic bare ping.
                            let now = js_sys::Date::now();
                            if now - last_typing_ping.get_value() >= 2000.0 {
                                if let Some(cid) = s.sel.sel_channel.get_untracked().map(|c| c.id) {
                                    last_typing_ping.set_value(now);
                                    let draft = s
                                        .prefs
                                        .ghost_quill
                                        .get_untracked()
                                        .then(|| value.clone());
                                    leptos::task::spawn_local(async move {
                                        let _ = api::post_typing(&cid, draft).await;
                                    });
                                }
                            }
                        }
                    }
                    on:paste=move |_ev| {
                        // Paste-to-upload images (#27): stage any image items on
                        // the clipboard and suppress their default text paste.
                        // Same `image/*` filter as the gallery (B4). The helper
                        // lives in [`crate::ui::clipboard`] (W7/B2 extraction).
                        #[cfg(feature = "hydrate")]
                        {
                            let files = crate::ui::clipboard::read_pasted_images(&_ev);
                            let handled = !files.is_empty();
                            // Soft cap (W7/B1-client) — same ceiling as the file
                            // picker. Drop overflow files and toast once.
                            let current =
                                s.composer.compose_attachments.get_untracked().len();
                            let slots_left =
                                COMPOSER_MAX_ATTACHMENTS.saturating_sub(current);
                            let overflowed = files.len() > slots_left;
                            for file in files.into_iter().take(slots_left) {
                                act::add_compose_attachment(s, file);
                            }
                            if overflowed {
                                s.composer.status.set(format!(
                                    "Attachment limit ({COMPOSER_MAX_ATTACHMENTS}) reached"
                                ));
                            }
                            if handled {
                                _ev.prevent_default();
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = &_ev;
                    }
                    on:keydown=move |ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            // While the autocomplete popover is open it owns the
                            // arrow/Enter/Tab/Escape keys; only when it's closed
                            // does Enter fall through to send.
                            if let Some((st, en, q)) = ac_token.get() {
                                let sugg = emoji_suggestions(s, &q);
                                match ev.key().as_str() {
                                    "ArrowDown" => {
                                        ev.prevent_default();
                                        let max = sugg.len().saturating_sub(1);
                                        ac_index.update(|i| *i = (*i + 1).min(max));
                                        return;
                                    }
                                    "ArrowUp" => {
                                        ev.prevent_default();
                                        ac_index.update(|i| *i = i.saturating_sub(1));
                                        return;
                                    }
                                    "Escape" => {
                                        ev.prevent_default();
                                        ac_token.set(None);
                                        return;
                                    }
                                    "Tab" => {
                                        ev.prevent_default();
                                        if let Some(sg) = sugg.get(ac_index.get_untracked()) {
                                            replace_shortcode_token(
                                                s, composer_ref, st, en, &sg.name,
                                            );
                                        }
                                        ac_token.set(None);
                                        return;
                                    }
                                    // Enter accepts the highlighted suggestion rather
                                    // than sending the raw `:query` (review F-D13-1).
                                    // Guarded by !is_composing so an IME confirm is
                                    // left alone; with nothing to accept it falls
                                    // through to the normal Enter-to-send below.
                                    "Enter" if !ev.is_composing() => {
                                        if let Some(sg) = sugg.get(ac_index.get_untracked()) {
                                            ev.prevent_default();
                                            replace_shortcode_token(
                                                s, composer_ref, st, en, &sg.name,
                                            );
                                            ac_token.set(None);
                                            return;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // Esc cancels an in-progress message edit (restores
                            // the stashed draft); only relevant in edit mode, so
                            // a stray Esc otherwise does nothing here.
                            if ev.key() == "Escape"
                                && s.composer.editing.get_untracked().is_some()
                            {
                                ev.prevent_default();
                                act::cancel_edit(s);
                                return;
                            }
                            // Send on plain Enter only on desktop. On touch
                            // devices (no Shift) and mid-IME-composition, Enter
                            // falls through to insert a newline; use the Send button.
                            if ev.key() == "Enter"
                                && !ev.shift_key()
                                && !ev.is_composing()
                                && !enter_inserts_newline()
                            {
                                ev.prevent_default();
                                act::send_message(s);
                                // Close any lingering autocomplete popover so it
                                // doesn't hover over the now-cleared composer.
                                ac_token.set(None);
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = &ev;
                    }
                    // Short enough to FIT one line on a narrow phone (mobile
                    // finding #50b — the old string's `[red]color[/red]` tail
                    // wrapped it into an ugly two-line block); the
                    // `::placeholder` nowrap/ellipsis guard in _content.scss
                    // degrades anything narrower gracefully.
                    placeholder="type a message — **bold**, *italic*"
                ></textarea>
                // `:`-autocomplete popover: matches for the trailing `:query`
                // under the caret. Arrow/Enter/Tab navigate (handled in
                // on:keydown); a click accepts directly.
                {move || ac_token.get().map(|(st, en, q)| {
                    let sugg = emoji_suggestions(s, &q);
                    view! {
                        <ul class="emoji-suggest">
                            {sugg.into_iter().enumerate().map(|(i, sg)| {
                                let name = sg.name.clone();
                                let title = format!(":{name}:");
                                let icon = match (sg.media, sg.glyph) {
                                    (Some(media), _) => view! {
                                        <img class="inline-emoji"
                                            src=format!("/media/{media}?w=32") alt=title.clone()/>
                                    }.into_any(),
                                    (None, Some(g)) => g.into_any(),
                                    (None, None) => title.clone().into_any(),
                                };
                                view! {
                                    <li class:active=move || ac_index.get() == i
                                        on:click=move |_| {
                                            replace_shortcode_token(s, composer_ref, st, en, &name);
                                            ac_token.set(None);
                                        }>
                                        <span class="sg-emoji">{icon}</span>
                                        <span class="sg-code">{title}</span>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }
                })}
                // W4/T5: effect picker — cycles the NEXT message's delivery
                // effect none → whisper → shout → spell (then back). The mode
                // rides the send as `SendMessageRequest::effect` and RESETS to
                // none after each send (act::send_message). Distinct glyph per
                // mode; title/aria-label name the current mode.
                <button class="effect-pick" type="button"
                    class:active=move || s.composer.effect_mode.get().is_some()
                    title=move || effect_label(s.composer.effect_mode.get().as_deref())
                    aria-label=move || effect_label(s.composer.effect_mode.get().as_deref())
                    on:click=move |_| s.composer.effect_mode.update(|e| {
                        *e = next_effect(e.as_deref()).map(str::to_string);
                    })>
                    {move || effect_glyph(s.composer.effect_mode.get().as_deref())}
                </button>
                // The charge ring (W4/T2): `--charge` (0..1) fills the conic
                // ::before ring as the compose grows; `.charging` shows it
                // only while something is typed; `.sent` plays the one-shot
                // post-send pulse (flipped by act::send_message).
                <button class="send"
                    // Braced: a bare `>` would close the <button> tag in rstml.
                    class:charging={move || charge.get() > 0.0}
                    class:sent=move || s.composer.sent.get()
                    style=("--charge", move || format!("{:.3}", charge.get()))
                    on:click=move |_| {
                    act::send_message(s);
                    // Close any lingering `:`-autocomplete popover — on touch the
                    // Send button is the only send path (Enter inserts a newline),
                    // so this is where a `:3`-style send must dismiss it.
                    ac_token.set(None);
                }>
                    // "Save" while editing a message, "Send" for a normal compose.
                    {move || if s.composer.editing.get().is_some() { "Save" } else { "Send" }}
                </button>
            </div>

            // Persona info popup — opened by clicking a message's author name.
            {move || info.get().map(|m| {
                // For a personaless message the "default" identity is the
                // controlling account's nickname.
                let persona = display_name(&m);
                let portrait = chat_avatar(&m.persona_avatar_id, &persona, true);
                let desc = m.persona_description.clone().filter(|d| !d.trim().is_empty());
                let author = m.author_name.clone();
                view! {
                    <Modal class="persona-info" close=move || info.set(None)>
                        <div class="detail-head">
                            <h4>{persona}</h4>
                            <button class="row-edit" title="close"
                                on:click=move |_| info.set(None)>"✕"</button>
                        </div>
                        // Persona's send-time avatar snapshot (#26), monogram fallback.
                        <div class="info-portrait">{portrait}</div>
                        {match desc {
                            // Description supports the same markup as chat (#18).
                            Some(d) => view! { <p class="card-desc">{render_body(&d)}</p> }.into_any(),
                            None => view! { <p class="card-desc muted">"No description."</p> }.into_any(),
                        }}
                        <p class="muted">"Controlled by "<strong>{author}</strong></p>
                    </Modal>
                }
            })}

            // Attachment lightbox — the clicked image near-fullscreen; tap the
            // backdrop (or the ✕, or Esc, or drag down) to close. Loads the
            // full original, not the grid thumbnail. Within a multi-image
            // message: ◀/▶ buttons, Left/Right arrow keys, and pointer-swipe
            // step the gallery (clamped, no wrap); pinch/double-tap/wheel and
            // +/-/0 zoom-and-pan via a single CSS transform (see lightbox.rs).
            {lightbox_view(lightbox, lb_tf)}

            // W4/T4 radial long-press action menu (touch) — the glass arc of
            // reply/copy(/edit/delete) buttons blossoming at the press point,
            // opened by the delegated <ul> pointer handlers above.
            {radial::radial_menu(s, radial, radial_armed)}
        </div>
    }
}
