//! The channel message pane: the message list and the markup composer.

use leptos::prelude::*;

use super::{act, short_id, Shell};
use crate::markup::Color;
use crate::ui::markup_view::render_body;

#[component]
pub(crate) fn ChannelPane(s: Shell) -> impl IntoView {
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
                {move || s.messages.get().into_iter().map(|m| {
                    let who = m.persona_name.clone().unwrap_or_else(|| short_id(&m.author_id));
                    view! {
                        <li class="msg">
                            <span class="who">{who}</span>
                            <span class="text">{render_body(&m.body)}</span>
                        </li>
                    }
                }).collect_view()}
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
