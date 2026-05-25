//! The channel message pane: the message list and the markup composer.

use leptos::prelude::*;

use super::{act, short_id, Shell};
use crate::markup::Color;
use crate::protocol::MessageEnvelope;
use crate::ui::markup_view::render_body;
use crate::ui::AuthCtx;

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
        if let Some(el) = composer_ref.get() {
            // Deref to web_sys::HtmlElement so its inherent `style()` wins over
            // tachys' `ElementExt::style` (both in scope via leptos prelude).
            let style = (*el).style();
            let _ = style.set_property("height", "auto");
            let _ = style.set_property("height", &format!("{}px", el.scroll_height()));
        }
    });

    // Click-the-name info popup: which message's persona/controller to show.
    let info = RwSignal::new(None::<MessageEnvelope>);

    // Auto-scroll. `last_dist` is the px distance from the bottom recorded on
    // the user's last scroll (i.e. pre-append). On a new message: your own →
    // follow when NEAR the bottom; someone else's → only when EXACTLY at the
    // bottom; otherwise leave the scroll position alone (reading history).
    let list_ref = NodeRef::<leptos::html::Ul>::new();
    let last_dist = StoredValue::new(0.0_f64);
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let msgs = s.messages.get();
        let mine = msgs
            .last()
            .zip(auth.user.get_untracked())
            .map(|(m, u)| m.author_id == u.account_id)
            .unwrap_or(false);
        let threshold = if mine { 120.0 } else { 4.0 };
        if last_dist.get_value() <= threshold {
            last_dist.set_value(0.0);
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(0).await;
                if let Some(el) = list_ref.get_untracked() {
                    el.set_scroll_top(el.scroll_height());
                }
            });
        }
    });

    view! {
        <div class="channel-view">
            <ul class="messages" node_ref=list_ref
                on:scroll=move |_ev| {
                    #[cfg(feature = "hydrate")]
                    if let Some(el) = list_ref.get_untracked() {
                        last_dist.set_value(
                            (el.scroll_height() - el.scroll_top() - el.client_height()) as f64,
                        );
                    }
                    #[cfg(not(feature = "hydrate"))]
                    let _ = (&last_dist, &_ev);
                }>
                {move || {
                    let me = auth.user.get().map(|u| u.account_id);
                    let cid = s.sel_channel.get().map(|c| c.id);
                    s.messages.get().into_iter().map(|m| {
                        let who = m.persona_name.clone().unwrap_or_else(|| short_id(&m.author_id));
                        let when = format_local_time(&m.sent_at);
                        let info_m = m.clone();
                        let mine = me.is_some() && me.as_deref() == Some(m.author_id.as_str());
                        let mid = m.id.clone();
                        let body = m.body.clone();
                        let cid = cid.clone();
                        view! {
                            <li class="msg">
                                <div class="meta">
                                    <button class="who" title="persona info"
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
                                                            act::delete_message(s, cid, del_mid.clone());
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
                            </li>
                        }
                    }).collect_view()
                }}
            </ul>
            <div class="composer">
                <div class="toolbar">
                    <button class="fmt" title="bold"
                        on:click=move |_| apply_markup(s, composer_ref, "**", "**")>
                        <strong>"B"</strong>
                    </button>
                    <button class="fmt" title="italic"
                        on:click=move |_| apply_markup(s, composer_ref, "*", "*")>
                        <em>"i"</em>
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
                <textarea
                    node_ref=composer_ref
                    prop:value=move || s.compose.get()
                    on:input=move |ev| s.compose.set(event_target_value(&ev))
                    on:keydown=move |ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            if ev.key() == "Enter" && !ev.shift_key() {
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
                let persona = m.persona_name.clone().unwrap_or_else(|| "(no persona)".to_string());
                let desc = m.persona_description.clone().filter(|d| !d.trim().is_empty());
                let author = m.author_name.clone();
                view! {
                    <div class="modal-backdrop" on:click=move |_| info.set(None)>
                        <div class="modal" on:click=move |_ev| {
                            #[cfg(feature = "hydrate")]
                            _ev.stop_propagation();
                        }>
                            <div class="detail-head">
                                <h4>{persona}</h4>
                                <button class="row-edit" title="close"
                                    on:click=move |_| info.set(None)>"✕"</button>
                            </div>
                            {match desc {
                                Some(d) => view! { <p class="card-desc">{d}</p> }.into_any(),
                                None => view! { <p class="card-desc muted">"No description."</p> }.into_any(),
                            }}
                            <p class="muted">"Controlled by "<strong>{author}</strong></p>
                        </div>
                    </div>
                }
            })}
        </div>
    }
}
