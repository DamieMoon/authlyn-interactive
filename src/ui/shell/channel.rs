//! The channel message pane: the message list and the markup composer.

use leptos::prelude::*;

use super::{act, PendingDelete, Shell};
use crate::markup::Color;
use crate::protocol::MessageEnvelope;
use crate::ui::markup_view::render_body;
use crate::ui::AuthCtx;

/// One row in the deleted-messages panel: the message snippet plus a Restore button.
fn deleted_message_row(s: Shell, m: MessageEnvelope, auth_id: Option<String>) -> impl IntoView {
    let cid = s
        .sel_channel
        .get_untracked()
        .map(|c| c.id)
        .unwrap_or_default();
    let mid_restore = m.id.clone();
    let who = m
        .persona_name
        .clone()
        .unwrap_or_else(|| m.author_display.clone());
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
fn apply_markup(s: Shell, ta_ref: NodeRef<leptos::html::Textarea>, open: &str, close: &str) {
    let Some(el) = ta_ref.get() else {
        s.compose.update(|c| {
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
    s.compose.set(format!("{before}{open}{sel}{close}{after}"));

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
fn apply_markup(s: Shell, _ta_ref: NodeRef<leptos::html::Textarea>, open: &str, close: &str) {
    s.compose.update(|c| {
        c.push_str(open);
        c.push_str(close);
    });
}

/// Format an RFC3339 timestamp for display beside the author name.
///
/// On hydrate (browser) we hand the string to JavaScript's `Date`, which
/// parses RFC3339 and renders in the viewer's local timezone + locale.
/// On ssr (native) there is no browser timezone, so we fall back to the
/// raw timestamp — the value is replaced by the localized one as soon as
/// the client hydrates.
#[cfg(feature = "hydrate")]
fn format_local_time(sent_at: &str) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(sent_at));
    // NaN time => unparseable input; keep the raw string rather than show
    // "Invalid Date".
    if date.get_time().is_nan() {
        return sent_at.to_string();
    }
    let undef = wasm_bindgen::JsValue::UNDEFINED;
    let day = String::from(date.to_locale_date_string("default", &undef));
    let time = String::from(date.to_locale_time_string("default"));
    format!("{day} {time}")
}

#[cfg(not(feature = "hydrate"))]
fn format_local_time(sent_at: &str) -> String {
    sent_at.to_string()
}

/// A circular persona avatar for chat: the send-time snapshot image (served at
/// `/media/{id}`) when present, else the name's first letter as a monogram.
/// `fill` true makes it fill its parent slot (the info popup's `.info-portrait`);
/// false renders a fixed small inline circle (the per-message meta row). Styled
/// inline because `main.scss` is owned by a parallel work stream.
fn chat_avatar(avatar_id: &Option<String>, name: &str, fill: bool) -> impl IntoView {
    let frame = if fill {
        "width:100%;height:100%;border-radius:inherit;overflow:hidden;display:flex;\
         align-items:center;justify-content:center"
            .to_string()
    } else {
        "width:2.5rem;height:2.5rem;border-radius:50%;overflow:hidden;flex:0 0 auto;\
         display:inline-flex;align-items:center;justify-content:center;\
         background:#3a3550;color:#cdb8f0;font-weight:600;font-size:1.05rem;\
         vertical-align:middle;margin-right:0.5rem"
            .to_string()
    };
    match avatar_id {
        Some(id) => {
            // Request a downscaled JPEG thumbnail instead of the full upload so
            // avatars load fast: the small row circle needs ~128px, the popup ~256.
            let tw = if fill { 256 } else { 128 };
            let src = format!("/media/{id}?w={tw}");
            view! {
                <span class="chat-avatar" style=frame>
                    <img src=src alt="" style="width:100%;height:100%;object-fit:cover"/>
                </span>
            }
            .into_any()
        }
        None => {
            let monogram = name
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string();
            view! { <span class="chat-avatar" style=frame>{monogram}</span> }.into_any()
        }
    }
}

/// Render a message's inline image attachments as a Discord-style grid: the
/// more images, the more compact (column count climbs, cells go square).
/// Clicking one opens it in the lightbox. Thumbnails pull a downscaled JPEG
/// (`?w=512`); the lightbox loads the full original.
#[cfg(feature = "hydrate")]
fn attachment_grid(ids: Vec<String>, lightbox: RwSignal<Option<String>>) -> impl IntoView {
    let cols = match ids.len() {
        1 => 1,
        2 | 4 => 2,
        _ => 3,
    };
    view! {
        <div class=format!("attachments cols-{cols}")>
            {ids.into_iter().map(|id| {
                let open_id = id.clone();
                view! {
                    <img class="att-thumb" loading="lazy" alt="attachment"
                        src=format!("/media/{id}?w=512")
                        on:click=move |_| lightbox.set(Some(open_id.clone()))/>
                }
            }).collect_view()}
        </div>
    }
}

/// SSR build has no lightbox interaction; render the grid as plain links so the
/// markup still hydrates identically.
#[cfg(not(feature = "hydrate"))]
fn attachment_grid(ids: Vec<String>, _lightbox: RwSignal<Option<String>>) -> impl IntoView {
    let cols = match ids.len() {
        1 => 1,
        2 | 4 => 2,
        _ => 3,
    };
    view! {
        <div class=format!("attachments cols-{cols}")>
            {ids.into_iter().map(|id| view! {
                <img class="att-thumb" alt="attachment" src=format!("/media/{id}?w=512")/>
            }).collect_view()}
        </div>
    }
}

#[component]
pub(crate) fn ChannelPane(s: Shell) -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");
    // Inline edit state, shared across message rows like the channel-rename
    // pattern: which message id is being edited (if any), and its buffer.
    let editing_msg = RwSignal::new(None::<String>);
    let msg_edit_buf = RwSignal::new(String::new());

    // Auto-grow the composer to fit its content, up to the CSS max-height
    // (then it scrolls). Tracking `compose` covers both typing and the
    // programmatic clear after send. Hydrate-only; ssr leaves it min-height.
    let composer_ref = NodeRef::<leptos::html::Textarea>::new();
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        s.compose.track();
        let Some(el) = composer_ref.get() else {
            return;
        };
        // Measure AFTER Leptos flushes prop:value to the DOM on the next tick.
        // On send the composer is cleared to "" (#28): measuring synchronously
        // here reads the stale, still-large content and the textarea stays
        // super-tall until the next keystroke — especially visible on mobile
        // after a big message + keyboard close. Deferring lets scroll_height
        // reflect the current value, so an emptied composer collapses to its
        // CSS min-height.
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
    let lightbox = RwSignal::new(None::<String>);

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
        let msgs = s.messages.get();
        // An older-history prepend grows the list at the FRONT; skip the
        // append/scroll/unread logic here (the anchor effect below repositions
        // the viewport instead), but keep prev_count in sync for the next real
        // append.
        if s.anchor_to.get_untracked().is_some() {
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
        let Some(id) = s.anchor_to.get() else {
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
            s.anchor_to.set(None);
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
                {move || {
                    let me = auth.user.get().map(|u| u.account_id);
                    let cid = s.sel_channel.get().map(|c| c.id);
                    s.messages.get().into_iter().map(|m| {
                        // Worn persona's frozen name, else the "default" identity
                        // (the controlling account's nickname).
                        let who = m.persona_name.clone()
                            .unwrap_or_else(|| m.author_display.clone());
                        // Tint the name with the persona's chosen palette color
                        // (validated against the markup palette before trusting it).
                        let who_class = m.persona_color.as_deref()
                            .filter(|c| Color::from_name(c).is_some())
                            .map(|c| format!("who mk-{c}"))
                            .unwrap_or_else(|| "who".to_string());
                        let when = format_local_time(&m.sent_at);
                        // Circular persona avatar (send-time snapshot) left of the name.
                        let avatar_el = chat_avatar(&m.persona_avatar_id, &who, false);
                        let atts = m.attachments.clone();
                        let info_m = m.clone();
                        let mine = me.is_some() && me.as_deref() == Some(m.author_id.as_str());
                        let mid = m.id.clone();
                        let body = m.body.clone();
                        let cid = cid.clone();
                        let dom_id = format!("msg-{}", m.id);
                        view! {
                            <li class="msg" id=dom_id>
                                <div class="meta">
                                    {avatar_el}
                                    <button class=who_class title="persona info"
                                        on:click=move |_| info.set(Some(info_m.clone()))>{who}</button>
                                    <time class="when">{when}</time>
                                    {mine.then(|| {
                                        let edit_mid = mid.clone();
                                        let edit_body = body.clone();
                                        let del_mid = mid.clone();
                                        let del_cid = cid.clone();
                                        view! {
                                            <span class="msg-actions">
                                                <button class="row-edit" title="edit"
                                                    on:click=move |_| {
                                                        msg_edit_buf.set(edit_body.clone());
                                                        editing_msg.set(Some(edit_mid.clone()));
                                                    }>"✎"</button>
                                                <button class="row-edit" title="delete"
                                                    on:click=move |_| {
                                                        if let Some(cid) = del_cid.clone() {
                                                            // Message deletes confirm unless the
                                                            // user opted out in account settings;
                                                            // other deletes always confirm.
                                                            if act::confirm_delete_message_enabled() {
                                                                act::ask_delete(
                                                                    s,
                                                                    "Delete this message? This cannot be undone."
                                                                        .to_string(),
                                                                    PendingDelete::Message {
                                                                        cid,
                                                                        mid: del_mid.clone(),
                                                                    },
                                                                );
                                                            } else {
                                                                act::delete_message(s, cid, del_mid.clone());
                                                            }
                                                        }
                                                    }>"🗑"</button>
                                            </span>
                                        }
                                    })}
                                </div>
                                {move || {
                                    let mid = mid.clone();
                                    let body = body.clone();
                                    let cid = cid.clone();
                                    if editing_msg.get().as_deref() == Some(mid.as_str()) {
                                        let save_mid = mid.clone();
                                        let save_cid = cid.clone();
                                        view! {
                                            <div class="msg-edit">
                                                <textarea class="rename-input"
                                                    prop:value=move || msg_edit_buf.get()
                                                    on:input=move |ev| msg_edit_buf.set(event_target_value(&ev))></textarea>
                                                <button class="row-edit" title="save" on:click=move |_| {
                                                    if let Some(cid) = save_cid.clone() {
                                                        act::edit_message(s, cid, save_mid.clone(), msg_edit_buf.get_untracked());
                                                    }
                                                    editing_msg.set(None);
                                                }>"✓"</button>
                                                <button class="row-edit" title="cancel"
                                                    on:click=move |_| editing_msg.set(None)>"✕"</button>
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
            {move || s.show_msg_trash.get().then(|| {
                let me = auth.user.get().map(|u| u.account_id);
                let msgs = s.deleted_messages.get();
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

            <div class="composer">
                <div class="toolbar">
                    // Attach images: a hidden multi-file input behind a 📎 label.
                    // Each pick uploads immediately and stages the media id.
                    <label class="fmt attach" title="attach image">
                        "📎"
                        <input type="file" accept="image/*" multiple style="display:none"
                            on:change=move |_ev| {
                                #[cfg(feature = "hydrate")]
                                {
                                    use leptos::wasm_bindgen::JsCast;
                                    if let Some(input) = _ev.target().and_then(|t| {
                                        t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok()
                                    }) {
                                        if let Some(files) = input.files() {
                                            for i in 0..files.length() {
                                                if let Some(file) = files.get(i) {
                                                    act::add_compose_attachment(s, file);
                                                }
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
                    {Color::ALL.into_iter().map(|col| {
                        let name = col.name();
                        view! {
                            <button class=format!("swatch mk-bg-{name}") title=name
                                on:click=move |_|
                                    apply_markup(s, composer_ref, &format!("[{name}]"), &format!("[/{name}]"))>
                            </button>
                        }
                    }).collect_view()}
                </div>
                // Pending attachments: thumbnails of staged uploads, each with a
                // remove button. Sent (and cleared) on the next message.
                {move || {
                    let atts = s.compose_attachments.get();
                    (!atts.is_empty()).then(|| view! {
                        <div class="compose-attachments">
                            {atts.into_iter().map(|id| {
                                let rid = id.clone();
                                view! {
                                    <div class="pending-att">
                                        <img src=format!("/media/{id}?w=256") alt="pending attachment"/>
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
                    prop:value=move || s.compose.get()
                    on:input=move |ev| s.compose.set(event_target_value(&ev))
                    on:paste=move |_ev| {
                        // Paste-to-upload images (#27): stage any image items on
                        // the clipboard and suppress their default text paste.
                        #[cfg(feature = "hydrate")]
                        {
                            if let Some(dt) = _ev.clipboard_data() {
                                let items = dt.items();
                                let mut handled = false;
                                for i in 0..items.length() {
                                    let Some(item) = items.get(i) else { continue };
                                    if item.type_().starts_with("image/") {
                                        if let Ok(Some(file)) = item.get_as_file() {
                                            act::add_compose_attachment(s, file);
                                            handled = true;
                                        }
                                    }
                                }
                                if handled {
                                    _ev.prevent_default();
                                }
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = &_ev;
                    }
                    on:keydown=move |ev| {
                        #[cfg(feature = "hydrate")]
                        {
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
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = &ev;
                    }
                    placeholder="type a message — **bold**, *italic*, [red]color[/red]"
                ></textarea>
                <button class="send" on:click=move |_| act::send_message(s)>"Send"</button>
            </div>

            // Persona info popup — opened by clicking a message's author name.
            {move || info.get().map(|m| {
                // For a personaless message the "default" identity is the
                // controlling account's nickname.
                let persona = m.persona_name.clone().unwrap_or_else(|| m.author_display.clone());
                let portrait = chat_avatar(&m.persona_avatar_id, &persona, true);
                let desc = m.persona_description.clone().filter(|d| !d.trim().is_empty());
                let author = m.author_name.clone();
                view! {
                    <div class="modal-backdrop" on:click=move |_| info.set(None)>
                        <div class="modal persona-info" on:click=move |_ev| {
                            #[cfg(feature = "hydrate")]
                            _ev.stop_propagation();
                        }>
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
                        </div>
                    </div>
                }
            })}

            // Attachment lightbox — the clicked image near-fullscreen; click
            // anywhere (or the ✕) to close. Loads the full original, not the
            // grid thumbnail.
            {move || lightbox.get().map(|id| {
                view! {
                    <div class="lightbox" on:click=move |_| lightbox.set(None)>
                        <button class="lightbox-close" title="close"
                            on:click=move |_| lightbox.set(None)>"✕"</button>
                        <img class="lightbox-img" src=format!("/media/{id}") alt="attachment"/>
                    </div>
                }
            })}
        </div>
    }
}
