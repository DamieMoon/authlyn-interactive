//! Message rich-text markup — the shared, target-agnostic parser.
//!
//! Messages are stored server-side as plaintext `body` strings that may
//! contain a small, roleplay-aware markup. This module turns such a string
//! into an AST ([`Node`]); the browser renders that AST to styled spans and
//! the composer inserts the same syntax via toolbar buttons (build step 7).
//! Nothing here is gated to a feature — it compiles for both ssr and hydrate.
//!
//! ## Grammar (v1)
//! - **bold**: `**text**`
//! - *italic*: `*text*` (matches the RP convention where `*waves*` actions
//!   render italic)
//! - color: `[name]text[/name]` where `name` is one of the fixed [`Color`]
//!   palette (red, orange, yellow, green, blue, purple, pink, gray)
//!
//! Out of scope (deliberately): arbitrary hex colors, fonts.
//!
//! ## Leniency
//! The parser never fails. Unmatched openers, mismatched closers, and unknown
//! `[...]` tags are emitted as literal text, so any input renders as
//! *something* reasonable. Nesting is supported (`**[red]hi *there*[/red]**`);
//! bold/italic toggle against the innermost open span, so pathological
//! interleavings degrade to literal text rather than panicking.

/// The fixed color palette. No hex, by design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Pink,
    Gray,
}

impl Color {
    /// All palette entries, in display order (for a color-picker UI).
    pub const ALL: [Color; 8] = [
        Color::Red,
        Color::Orange,
        Color::Yellow,
        Color::Green,
        Color::Blue,
        Color::Purple,
        Color::Pink,
        Color::Gray,
    ];

    /// The tag name, e.g. `Color::Red` -> `"red"`.
    pub fn name(self) -> &'static str {
        match self {
            Color::Red => "red",
            Color::Orange => "orange",
            Color::Yellow => "yellow",
            Color::Green => "green",
            Color::Blue => "blue",
            Color::Purple => "purple",
            Color::Pink => "pink",
            Color::Gray => "gray",
        }
    }

    pub fn from_name(name: &str) -> Option<Color> {
        Color::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// A node in the parsed markup tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Text(String),
    Bold(Vec<Node>),
    Italic(Vec<Node>),
    Color(Color, Vec<Node>),
}

/// Parse a message body into a markup tree. Never fails (see module docs).
pub fn parse(input: &str) -> Vec<Node> {
    let tokens = tokenize(input);
    build_tree(tokens)
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

enum Tok {
    Text(String),
    Bold,
    Italic,
    ColorOpen(Color),
    ColorClose(Color),
}

fn tokenize(s: &str) -> Vec<Tok> {
    let mut tokens: Vec<Tok> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    macro_rules! flush {
        () => {
            if !buf.is_empty() {
                tokens.push(Tok::Text(std::mem::take(&mut buf)));
            }
        };
    }

    while i < s.len() {
        let rest = &s[i..];
        if let Some(after) = rest.strip_prefix("**") {
            let _ = after;
            flush!();
            tokens.push(Tok::Bold);
            i += 2;
        } else if rest.starts_with('*') {
            flush!();
            tokens.push(Tok::Italic);
            i += 1;
        } else if rest.starts_with('[') {
            if let Some((tok, len)) = parse_color_tag(rest) {
                flush!();
                tokens.push(tok);
                i += len;
            } else {
                buf.push('[');
                i += 1;
            }
        } else {
            // Consume one full char (UTF-8 safe; `i` stays on a boundary).
            let ch = rest.chars().next().expect("non-empty rest");
            buf.push(ch);
            i += ch.len_utf8();
        }
    }
    if !buf.is_empty() {
        tokens.push(Tok::Text(buf));
    }
    tokens
}

/// If `rest` (which starts with `[`) opens with a well-formed color tag,
/// return the token and the byte length consumed (through the closing `]`).
fn parse_color_tag(rest: &str) -> Option<(Tok, usize)> {
    let close = rest.find(']')?;
    let inner = &rest[1..close];
    let consumed = close + 1;
    if let Some(name) = inner.strip_prefix('/') {
        Color::from_name(name).map(|c| (Tok::ColorClose(c), consumed))
    } else {
        Color::from_name(inner).map(|c| (Tok::ColorOpen(c), consumed))
    }
}

// ---------------------------------------------------------------------------
// Tree builder (stack of open spans; unclosed spans unwind to literal text)
// ---------------------------------------------------------------------------

enum FrameKind {
    Root,
    Bold,
    Italic,
    Color(Color),
}

struct Frame {
    kind: FrameKind,
    /// The literal opener, re-emitted as text if this frame is never closed.
    opener: String,
    children: Vec<Node>,
}

fn build_tree(tokens: Vec<Tok>) -> Vec<Node> {
    let mut stack: Vec<Frame> = vec![Frame {
        kind: FrameKind::Root,
        opener: String::new(),
        children: Vec::new(),
    }];

    for tok in tokens {
        match tok {
            Tok::Text(s) => push_text(&mut top(&mut stack).children, &s),
            Tok::Bold => toggle(&mut stack, FrameKind::Bold, "**"),
            Tok::Italic => toggle(&mut stack, FrameKind::Italic, "*"),
            Tok::ColorOpen(c) => stack.push(Frame {
                kind: FrameKind::Color(c),
                opener: format!("[{}]", c.name()),
                children: Vec::new(),
            }),
            Tok::ColorClose(c) => {
                if matches!(top(&mut stack).kind, FrameKind::Color(open) if open == c) {
                    close_top(&mut stack);
                } else {
                    let lit = format!("[/{}]", c.name());
                    push_text(&mut top(&mut stack).children, &lit);
                }
            }
        }
    }

    // Unwind anything still open: its opener becomes literal text, its
    // children splice into the parent.
    while stack.len() > 1 {
        let frame = stack.pop().expect("len > 1");
        let parent = &mut top(&mut stack).children;
        push_text(parent, &frame.opener);
        for node in frame.children {
            push_node(parent, node);
        }
    }

    stack.pop().expect("root frame").children
}

fn top(stack: &mut [Frame]) -> &mut Frame {
    stack.last_mut().expect("stack always holds the root frame")
}

/// Toggle a bold/italic span: close it if it's the innermost open frame,
/// otherwise open a new one.
fn toggle(stack: &mut Vec<Frame>, kind: FrameKind, opener: &str) {
    let matches_top = matches!(
        (&top(stack).kind, &kind),
        (FrameKind::Bold, FrameKind::Bold) | (FrameKind::Italic, FrameKind::Italic)
    );
    if matches_top {
        close_top(stack);
    } else {
        stack.push(Frame {
            kind,
            opener: opener.to_string(),
            children: Vec::new(),
        });
    }
}

fn close_top(stack: &mut Vec<Frame>) {
    let frame = stack.pop().expect("close_top with a frame open");
    let node = match frame.kind {
        FrameKind::Bold => Node::Bold(frame.children),
        FrameKind::Italic => Node::Italic(frame.children),
        FrameKind::Color(c) => Node::Color(c, frame.children),
        FrameKind::Root => unreachable!("the root frame is never closed"),
    };
    push_node(&mut top(stack).children, node);
}

/// Append text, merging with a trailing `Text` node so the AST stays compact.
fn push_text(children: &mut Vec<Node>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Node::Text(last)) = children.last_mut() {
        last.push_str(s);
    } else {
        children.push(Node::Text(s.to_string()));
    }
}

fn push_node(children: &mut Vec<Node>, node: Node) {
    match node {
        Node::Text(s) => push_text(children, &s),
        other => children.push(other),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Node {
        Node::Text(s.to_string())
    }

    #[test]
    fn plain_text_is_one_node() {
        assert_eq!(parse("hello world"), vec![text("hello world")]);
    }

    #[test]
    fn bold_italic_and_color() {
        assert_eq!(parse("**b**"), vec![Node::Bold(vec![text("b")])], "bold");
        assert_eq!(parse("*i*"), vec![Node::Italic(vec![text("i")])], "italic");
        assert_eq!(
            parse("[red]r[/red]"),
            vec![Node::Color(Color::Red, vec![text("r")])],
            "color"
        );
    }

    #[test]
    fn rp_action_asterisks_become_italic() {
        // The roleplay convention: *she waves* renders italic.
        assert_eq!(
            parse("*she waves*"),
            vec![Node::Italic(vec![text("she waves")])]
        );
    }

    #[test]
    fn nested_color_with_bold_and_italic() {
        // "[blue]calm **and** *steady*[/blue]"
        assert_eq!(
            parse("[blue]calm **and** *steady*[/blue]"),
            vec![Node::Color(
                Color::Blue,
                vec![
                    text("calm "),
                    Node::Bold(vec![text("and")]),
                    text(" "),
                    Node::Italic(vec![text("steady")]),
                ]
            )]
        );
    }

    #[test]
    fn unmatched_bold_is_literal() {
        assert_eq!(parse("**oops"), vec![text("**oops")]);
    }

    #[test]
    fn stray_color_close_is_literal() {
        assert_eq!(parse("hi [/red] there"), vec![text("hi [/red] there")]);
    }

    #[test]
    fn unknown_color_name_is_literal() {
        assert_eq!(
            parse("[mauve]x[/mauve]"),
            vec![text("[mauve]x[/mauve]")],
            "unknown palette entries pass through untouched"
        );
    }

    #[test]
    fn lone_brackets_pass_through() {
        // Roleplayers type literal brackets/angles all the time.
        assert_eq!(parse("[not a tag]"), vec![text("[not a tag]")]);
        assert_eq!(parse("<grins> nervously"), vec![text("<grins> nervously")]);
    }

    #[test]
    fn unclosed_color_unwinds_to_literal_opener() {
        assert_eq!(
            parse("[green]hi"),
            vec![text("[green]hi")],
            "an unclosed color tag re-emits its opener as text"
        );
    }
}
