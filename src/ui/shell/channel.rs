//! The channel message pane: the message list and the markup composer.

use leptos::prelude::*;

use super::{act, short_id, Shell};
use crate::markup::Color;
use crate::ui::markup_view::render_body;
use crate::ui::AuthCtx;

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

    view! {
        <div class="channel-view">
            <ul class="messages">
                {move || {
                    let me = auth.user.get().map(|u| u.account_id);
                    let cid = s.sel_channel.get().map(|c| c.id);
                    s.messages.get().into_iter().map(|m| {
                        let who = m.persona_name.clone().unwrap_or_else(|| short_id(&m.author_id));
                        let when = format_local_time(&m.sent_at);
                        let mine = me.is_some() && me.as_deref() == Some(m.author_id.as_str());
                        let mid = m.id.clone();
                        let body = m.body.clone();
                        let cid = cid.clone();
                        view! {
                            <li class="msg">
                                <div class="meta">
                                    <span class="who">{who}</span>
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
                        on:click=move |_| s.compose.update(|c| c.push_str("**bold**"))>
                        <strong>"B"</strong>
                    </button>
                    <button class="fmt" title="italic"
                        on:click=move |_| s.compose.update(|c| c.push_str("*italic*"))>
                        <em>"i"</em>
                    </button>
                    {Color::ALL.into_iter().map(|col| {
                        let name = col.name();
                        view! {
                            <button class=format!("swatch mk-bg-{name}") title=name
                                on:click=move |_| s.compose.update(|c| {
                                    c.push_str(&format!("[{name}]text[/{name}]"));
                                })>
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
        </div>
    }
}
