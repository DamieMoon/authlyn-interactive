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
//!
//! This file owns `ChannelPane` itself (the message-list/composer view), the
//! composer's caret-aware `apply_markup`, the touch-vs-desktop Enter helper,
//! and the small `deleted_message_row`.

mod attachments;
mod avatar;
mod emoji_suggest;
mod meta;
mod skeleton;

use attachments::attachment_grid;
use avatar::{chat_avatar, format_local_time};
#[cfg(feature = "hydrate")]
use emoji_suggest::active_shortcode_token;
use emoji_suggest::{
    custom_emoji_btn, emoji_suggestions, replace_shortcode_token, unicode_emoji_btn,
};
use meta::message_meta;
use skeleton::{should_show_skeletons, skeleton_rows};

use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use super::COMPOSER_MAX_ATTACHMENTS;
use super::{act, Shell};
#[cfg(feature = "hydrate")]
use crate::client::api;
use crate::markup::Color;
use crate::protocol::{Attachment, MessageEnvelope};
use crate::ui::emoji::data::{self, GROUPS};
use crate::ui::inline_rename::InlineRename;
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

/// True on touch-primary devices (phones/tablets), where the on-screen
/// keyboard's Enter must insert a newline rather than send — there's no
/// Shift+Enter, so Enter-to-send would make multi-line messages impossible.
/// Desktop (fine pointer) keeps Enter-to-send / Shift+Enter-for-newline.
#[cfg(feature = "hydrate")]
fn enter_inserts_newline() -> bool {
    leptos::web_sys::window()
        .and_then(|w| w.match_media("(pointer: coarse)").ok().flatten())
        .map(|m| m.matches())
        .unwrap_or(false)
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

#[component]
pub(crate) fn ChannelPane() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    // Inline edit state, shared across message rows: which message id is
    // being edited (if any). The draft buffer lives inside <InlineRename>
    // (W6/C7).
    let editing_msg = RwSignal::new(None::<String>);

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

    // Click-the-name info popup: which message's persona/controller to show.
    let info = RwSignal::new(None::<MessageEnvelope>);
    // Lightbox: media id of the attachment opened near-fullscreen, if any.
    let lightbox = RwSignal::new(None::<Attachment>);

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
                .and_then(|d| d.get_element_by_id(&format!("msg-{id}")))
            {
                el.scroll_into_view();
            }
            s.msg.anchor_to.set(None);
        });
    });

    view! {
        <div class="channel-view">
            <ul class="messages" node_ref=list_ref
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
                    s.msg.messages.get().into_iter().map(|m| {
                        let atts = m.attachments.clone();
                        let mine = me.is_some() && me.as_deref() == Some(m.author_id.as_str());
                        let mid = m.id.clone();
                        let body = m.body.clone();
                        let cid = cid.clone();
                        let dom_id = format!("msg-{}", m.id);
                        let meta = message_meta(s, &m, &cid, mine, editing_msg, info);
                        view! {
                            <li class="msg" id=dom_id>
                                {meta}
                                {move || {
                                    let mid = mid.clone();
                                    let body = body.clone();
                                    let cid = cid.clone();
                                    if editing_msg.get().as_deref() == Some(mid.as_str()) {
                                        let save_mid = mid.clone();
                                        let save_cid = cid.clone();
                                        view! {
                                            <div class="msg-edit">
                                                <InlineRename
                                                    value=body.clone()
                                                    multiline=true
                                                    submit_on_enter=true
                                                    on_save=move |v| {
                                                        if let Some(cid) = save_cid.clone() {
                                                            act::edit_message(s, cid, save_mid.clone(), v);
                                                        }
                                                        editing_msg.set(None);
                                                    }
                                                    on_cancel=move || editing_msg.set(None)
                                                />
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <span class="text">{render_body(&body)}</span>
                                        }.into_any()
                                    }
                                }}
                                {(!atts.is_empty()).then(|| attachment_grid(atts.clone(), lightbox))}
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
            // nothing when nobody else is typing.
            {move || {
                let names = s.msg.typing.get();
                let line = match names.len() {
                    0 => return ().into_any(),
                    1 => format!("{} is typing…", names[0]),
                    2 => format!("{} and {} are typing…", names[0], names[1]),
                    _ => "Several people are typing…".to_string(),
                };
                view! { <div class="typing-indicator">{line}</div> }.into_any()
            }}

            <div class="composer">
                <div class="toolbar">
                    // Attach images: a hidden multi-file input behind a 📎 label.
                    // Each pick uploads immediately and stages the media id.
                    <label class="fmt attach" title="attach image or video">
                        "📎"
                        // NO `accept`: on Android a media `accept` hint makes Chrome
                        // launch the system photo picker (Google Photos on this
                        // device), which the user doesn't want; omitting it gives the
                        // generic Files chooser instead. A PWA can't target a specific
                        // gallery app, so this is the better of the two reachable
                        // options. We filter to image/video client-side below.
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
                                            let mut skipped = false;
                                            let mut overflowed = false;
                                            // Collect the accepted files first, then upload the
                                            // whole pick at once so the staged order matches the
                                            // selection order (mnjs2ljw…), not upload-completion
                                            // order.
                                            let mut picked: Vec<web_sys::File> = Vec::new();
                                            for i in 0..files.length() {
                                                if let Some(file) = files.get(i) {
                                                    // Generic picker can return any file;
                                                    // only images and videos are valid.
                                                    let t = file.type_();
                                                    if !(t.starts_with("image/")
                                                        || t.starts_with("video/"))
                                                    {
                                                        skipped = true;
                                                        continue;
                                                    }
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
                                            } else if skipped {
                                                s.composer.status.set(
                                                    "Only images or videos can be attached."
                                                        .to_string(),
                                                );
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
                // Pending attachments: thumbnails of staged uploads, each with a
                // remove button. Sent (and cleared) on the next message.
                {move || {
                    let atts = s.composer.compose_attachments.get();
                    (!atts.is_empty()).then(|| view! {
                        <div class="compose-attachments">
                            {atts.into_iter().map(|att| {
                                let rid = att.id.clone();
                                let id = att.id.clone();
                                let is_video = att.mime.starts_with("video/");
                                let thumb = if is_video {
                                    view! {
                                        <video src=format!("/media/{id}") muted preload="metadata"></video>
                                    }.into_any()
                                } else {
                                    // GIFs raw so the preview animates; the ?w= thumb
                                    // would flatten them to a static JPEG frame.
                                    let src = if att.mime == "image/gif" {
                                        format!("/media/{id}")
                                    } else {
                                        format!("/media/{id}?w=256")
                                    };
                                    view! {
                                        <img src=src alt="pending attachment"/>
                                    }.into_any()
                                };
                                view! {
                                    <div class="pending-att">
                                        {thumb}
                                        <button class="att-remove" type="button" title="remove"
                                            on:click=move |_| act::remove_compose_attachment(s, rid.clone())>
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
                            let now = js_sys::Date::now();
                            if now - last_typing_ping.get_value() >= 2000.0 {
                                if let Some(cid) = s.sel.sel_channel.get_untracked().map(|c| c.id) {
                                    last_typing_ping.set_value(now);
                                    leptos::task::spawn_local(async move {
                                        let _ = api::post_typing(&cid).await;
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
                    placeholder="type a message — **bold**, *italic*, [red]color[/red]"
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
                <button class="send" on:click=move |_| {
                    act::send_message(s);
                    // Close any lingering `:`-autocomplete popover — on touch the
                    // Send button is the only send path (Enter inserts a newline),
                    // so this is where a `:3`-style send must dismiss it.
                    ac_token.set(None);
                }>"Send"</button>
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

            // Attachment lightbox — the clicked image near-fullscreen; click
            // anywhere (or the ✕) to close. Loads the full original, not the
            // grid thumbnail.
            {move || lightbox.get().map(|att| {
                let id = att.id.clone();
                let is_video = att.mime.starts_with("video/");
                // Full original (no `?w=512`); video gets autoplay+controls.
                let media = if is_video {
                    view! {
                        <video class="lightbox-img" controls autoplay
                            src=format!("/media/{id}")></video>
                    }.into_any()
                } else {
                    view! {
                        <img class="lightbox-img" src=format!("/media/{id}") alt="attachment"/>
                    }.into_any()
                };
                view! {
                    <div class="lightbox" on:click=move |_| lightbox.set(None)>
                        <button class="lightbox-close" title="close"
                            on:click=move |_| lightbox.set(None)>"✕"</button>
                        {media}
                    </div>
                }
            })}
        </div>
    }
}
