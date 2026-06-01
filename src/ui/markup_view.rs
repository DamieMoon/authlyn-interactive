//! Render a message body's [`crate::markup`] AST to styled views.
//! Inline: bold -> `<strong>`, italic -> `<em>`, color -> `<span
//! class="mk-NAME">`, inline code -> `<code>`. Block: headings -> `<h1>`/`<h2>`
//! /`<h3>`, subtext -> `<small class="mk-subtext">`, fenced code -> `<pre><code>`
//! (the markup CSS lives in style/main.scss). Compiles for both targets.

use leptos::prelude::*;

use crate::markup::{self, Node};
use crate::ui::emoji::EmojiResolver;

/// Parse `body` and render its markup. Unknown/unmatched markers render as
/// literal text (the parser is lenient).
pub fn render_body(body: &str) -> AnyView {
    render_nodes(markup::parse(body))
}

fn render_nodes(nodes: Vec<Node>) -> AnyView {
    nodes.into_iter().map(render_node).collect_view().into_any()
}

fn render_node(node: Node) -> AnyView {
    match node {
        Node::Text(s) => s.into_any(),
        Node::Bold(children) => view! { <strong>{render_nodes(children)}</strong> }.into_any(),
        Node::Italic(children) => view! { <em>{render_nodes(children)}</em> }.into_any(),
        Node::Color(color, children) => {
            let class = format!("mk-{}", color.name());
            view! { <span class=class>{render_nodes(children)}</span> }.into_any()
        }
        Node::Code(s) => view! { <code class="mk-code">{s}</code> }.into_any(),
        Node::Heading(level, children) => match level {
            1 => view! { <h1 class="mk-h1">{render_nodes(children)}</h1> }.into_any(),
            2 => view! { <h2 class="mk-h2">{render_nodes(children)}</h2> }.into_any(),
            _ => view! { <h3 class="mk-h3">{render_nodes(children)}</h3> }.into_any(),
        },
        Node::Subtext(children) => {
            view! { <small class="mk-subtext">{render_nodes(children)}</small> }.into_any()
        }
        Node::CodeBlock(s) => view! { <pre class="mk-pre"><code>{s}</code></pre> }.into_any(),
        // The quotes are re-emitted so the text reads identically whether or not
        // the per-user dialogue styling (a `.dialogue-style` root class) is on.
        Node::Dialogue(children) => {
            view! { <span class="mk-dialogue">{"\""}{render_nodes(children)}{"\""}</span> }
                .into_any()
        }
        // Custom guild emoji → image, standard unicode → glyph, else literal —
        // via the `EmojiResolver` context provided by `AppShell`. An absent
        // context (ssr / outside a guild) falls back to the literal `:name:`.
        Node::Emoji(name) => use_context::<EmojiResolver>()
            .map(|r| r.resolve(&name))
            .unwrap_or_else(|| format!(":{name}:").into_any()),
        Node::Image(alt, url) => {
            view! { <img class="mk-image" src=url alt=alt loading="lazy" /> }.into_any()
        }
        // The tokenizer guarantees `url` is http/https (or scheme-relative);
        // Leptos escapes both the href attribute and the link text. `rel` hardens
        // the new tab against `window.opener` tampering and referrer leakage.
        Node::Link(text, url) => view! {
            <a
                class="mk-link"
                href=url
                target="_blank"
                rel="noopener noreferrer nofollow"
            >
                {text}
            </a>
        }
        .into_any(),
        // Hidden until clicked: a per-node signal flips a `revealed` class. No
        // web_sys needed, so it compiles for ssr too (inert until hydrated).
        Node::Spoiler(children) => {
            let revealed = RwSignal::new(false);
            view! {
                <span
                    class="mk-spoiler"
                    class:revealed=move || revealed.get()
                    on:click=move |_| revealed.set(true)
                >
                    {render_nodes(children)}
                </span>
            }
            .into_any()
        }
    }
}
