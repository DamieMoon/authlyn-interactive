//! Render a message body's [`crate::markup`] AST to styled inline views.
//! Bold -> `<strong>`, italic -> `<em>`, color -> `<span class="mk-NAME">`
//! (the palette CSS lives in style/main.scss). Compiles for both targets.

use leptos::prelude::*;

use crate::markup::{self, Node};

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
    }
}
