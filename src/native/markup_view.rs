//! Native renderer for the roleplay markup AST — the Freya analogue of
//! `src/ui/markup_view.rs`. The PARSER is reused verbatim (`crate::markup::parse`);
//! only the AST→view mapping is new. Inline runs become styled `Span`s inside a
//! `paragraph()`; block nodes (heading/subtext/code-block) become their own
//! stacked elements.
//!
//! Phase 2 fidelity: text styles + colors + headings + code. Deferred — custom-
//! emoji resolution (renders `:shortcode:`), interactive spoiler reveal (rendered
//! muted), and inline images (placeholder; real images land via `image.rs`).

use freya::prelude::*;

use crate::markup::{self, Node};
use crate::native::theme;

/// Parse `body` and render it as a vertical stack of paragraphs/blocks.
pub fn render_body(body: &str) -> Element {
    let nodes = markup::parse(body);
    let mut col = rect().direction(Direction::Vertical).spacing(3.);
    let mut inline: Vec<Node> = Vec::new();

    for node in nodes {
        if is_block(&node) {
            if !inline.is_empty() {
                col = col.child(inline_paragraph(std::mem::take(&mut inline)));
            }
            col = col.child(render_block(node));
        } else {
            inline.push(node);
        }
    }
    if !inline.is_empty() {
        col = col.child(inline_paragraph(inline));
    }
    col.into()
}

fn is_block(node: &Node) -> bool {
    matches!(
        node,
        Node::Heading(..) | Node::Subtext(..) | Node::CodeBlock(..) | Node::Image(..)
    )
}

/// Accumulated inline style pushed down the AST to each text leaf.
#[derive(Clone, Copy, Default)]
struct Style {
    color: Option<theme::Rgb>,
    size: Option<f32>,
    bold: bool,
    italic: bool,
}

fn styled_span(text: String, style: Style) -> Span<'static> {
    let mut s = Span::new(text).color(style.color.unwrap_or(theme::INK));
    if let Some(sz) = style.size {
        s = s.font_size(sz);
    }
    if style.bold {
        s = s.font_weight(FontWeight::BOLD);
    }
    if style.italic {
        s = s.font_slant(FontSlant::Italic);
    }
    s
}

fn push_spans(node: &Node, style: Style, out: &mut Vec<Span<'static>>) {
    match node {
        Node::Text(s) => out.push(styled_span(s.clone(), style)),
        Node::Bold(children) => {
            let st = Style {
                bold: true,
                ..style
            };
            children.iter().for_each(|c| push_spans(c, st, out));
        }
        Node::Italic(children) => {
            let st = Style {
                italic: true,
                ..style
            };
            children.iter().for_each(|c| push_spans(c, st, out));
        }
        Node::Color(color, children) => {
            let st = Style {
                color: Some(theme::tint(*color)),
                ..style
            };
            children.iter().for_each(|c| push_spans(c, st, out));
        }
        // Inline code: distinct accent color (a monospace face is deferred).
        Node::Code(s) => out.push(styled_span(
            s.clone(),
            Style {
                color: Some(theme::GOLD_WARM),
                ..style
            },
        )),
        Node::Dialogue(children) => {
            let st = Style {
                color: Some(theme::INK_SOFT),
                ..style
            };
            out.push(styled_span("\u{201c}".to_string(), st));
            children.iter().for_each(|c| push_spans(c, st, out));
            out.push(styled_span("\u{201d}".to_string(), st));
        }
        // Spoiler: rendered muted for now (no click-to-reveal yet).
        Node::Spoiler(children) => {
            let st = Style {
                color: Some(theme::INK_MUTED),
                ..style
            };
            children.iter().for_each(|c| push_spans(c, st, out));
        }
        Node::Emoji(name) => out.push(styled_span(format!(":{name}:"), style)),
        // Block nodes shouldn't reach here, but degrade to their inline text.
        Node::Heading(_, children) | Node::Subtext(children) => {
            children.iter().for_each(|c| push_spans(c, style, out));
        }
        Node::CodeBlock(s) => out.push(styled_span(
            s.clone(),
            Style {
                color: Some(theme::GOLD_WARM),
                ..style
            },
        )),
        Node::Image(alt, _url) => out.push(styled_span(
            format!("[image: {alt}]"),
            Style {
                color: Some(theme::INK_MUTED),
                ..style
            },
        )),
    }
}

fn inline_paragraph(nodes: Vec<Node>) -> Element {
    let mut spans = Vec::new();
    for n in &nodes {
        push_spans(n, Style::default(), &mut spans);
    }
    paragraph().spans_iter(spans.into_iter()).into()
}

fn render_block(node: Node) -> Element {
    match node {
        Node::Heading(level, children) => {
            let size = match level {
                1 => theme::FS_H1,
                2 => theme::FS_H2,
                _ => theme::FS_H3,
            };
            let mut spans = Vec::new();
            let st = Style {
                bold: true,
                size: Some(size),
                ..Default::default()
            };
            children.iter().for_each(|c| push_spans(c, st, &mut spans));
            paragraph().spans_iter(spans.into_iter()).into()
        }
        Node::Subtext(children) => {
            let mut spans = Vec::new();
            let st = Style {
                color: Some(theme::INK_MUTED),
                size: Some(theme::FS_SUBTEXT),
                ..Default::default()
            };
            children.iter().for_each(|c| push_spans(c, st, &mut spans));
            paragraph().spans_iter(spans.into_iter()).into()
        }
        Node::CodeBlock(s) => rect()
            .background(theme::INPUT_BG)
            .padding(8.)
            .corner_radius(theme::RADIUS_SM)
            .child(label().color(theme::INK_SOFT).text(s))
            .into(),
        // Real inline images are wired through image.rs in a later step.
        Node::Image(alt, _url) => label()
            .color(theme::INK_MUTED)
            .text(format!("[image: {alt}]"))
            .into(),
        _ => label().text("").into(),
    }
}
