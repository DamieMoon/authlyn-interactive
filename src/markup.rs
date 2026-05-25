//! Message rich-text markup — the shared, target-agnostic parser.
//!
//! Messages are stored server-side as plaintext `body` strings that may
//! contain a small, roleplay-aware markup. This module turns such a string
//! into an AST ([`Node`]); the browser renders that AST to styled spans and
//! the composer inserts the same syntax via toolbar buttons (build step 7).
//! Nothing here is gated to a feature — it compiles for both ssr and hydrate.
//!
//! ## Grammar
//! Inline (anywhere within a line):
//! - **bold**: `**text**`
//! - *italic*: `*text*` (matches the RP convention where `*waves*` actions
//!   render italic)
//! - color: `[name]text[/name]` where `name` is one of the fixed [`Color`]
//!   palette (red, orange, yellow, green, blue, purple, pink, gray)
//! - inline code: `` `text` `` (monospace; no inline markup applies inside)
//!
//! Block / line-leading (Discord-style, marker must start a line + be followed
//! by a space):
//! - `# heading` → [`Node::Heading`] level 1
//! - `## heading` → level 2
//! - `### heading` → level 3
//! - `-# subtext` → [`Node::Subtext`] (small, muted)
//! - fenced code block: a line that is exactly ```` ``` ```` opens a block; it
//!   runs verbatim until the next ```` ``` ```` line (or end of input). No
//!   markup applies inside.
//!
//! Out of scope (deliberately): arbitrary hex colors, fonts.
//!
//! ## Leniency
//! The parser never fails. Unmatched openers, mismatched closers, unknown
//! `[...]` tags, a bare `#` with no trailing space, and an unterminated fence
//! are all emitted as literal text, so any input renders as *something*
//! reasonable. Inline nesting is supported (`**[red]hi *there*[/red]**`);
//! bold/italic toggle against the innermost open span, so pathological
//! interleavings degrade to literal text rather than panicking. Inline code
//! and fenced code render their contents verbatim — no other markup is applied
//! inside them.

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
///
/// `Text`, `Bold`, `Italic`, `Color` and `Code` are *inline* nodes; `Heading`,
/// `Subtext` and `CodeBlock` are *block* nodes that only appear at the top
/// level (one per source line / fence run).
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Text(String),
    Bold(Vec<Node>),
    Italic(Vec<Node>),
    Color(Color, Vec<Node>),
    /// Inline code span (`` `…` ``): contents are literal, monospace.
    Code(String),
    /// Line-leading header. `level` is 1, 2, or 3 (`#`, `##`, `###`).
    /// Children are parsed inline so a header can still hold bold/color/etc.
    Heading(u8, Vec<Node>),
    /// Line-leading `-#` subtext (small, muted). Children parsed inline.
    Subtext(Vec<Node>),
    /// A fenced ```` ``` ```` code block. Verbatim, no inner markup.
    CodeBlock(String),
}

/// Parse a message body into a markup tree. Never fails (see module docs).
///
/// Two-level parse: split into blocks by line (detecting line-leading markers
/// and ```` ``` ```` fences), then parse the inline content of each non-code
/// block with the inline tokenizer/tree-builder.
pub fn parse(input: &str) -> Vec<Node> {
    parse_blocks(input)
}

/// Parse `input` line-by-line into block-level nodes, recombining ordinary
/// text lines so a multi-line paragraph stays a single inline run (preserving
/// its embedded newlines, which the renderer shows via `white-space: pre-wrap`).
fn parse_blocks(input: &str) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    // Buffer of consecutive plain lines awaiting inline parsing as one run.
    let mut para: Vec<&str> = Vec::new();
    let mut lines = input.split('\n').peekable();

    // Flush the pending plain-line buffer as one inline run.
    let flush_para = |para: &mut Vec<&str>, out: &mut Vec<Node>| {
        if !para.is_empty() {
            let joined = para.join("\n");
            for node in build_tree(tokenize(&joined)) {
                push_node(out, node);
            }
            para.clear();
        }
    };

    while let Some(line) = lines.next() {
        if line.trim_end() == "```" {
            // Open a fence: consume verbatim until the closing ``` or EOF.
            flush_para(&mut para, &mut out);
            let mut body: Vec<&str> = Vec::new();
            let mut closed = false;
            for inner in lines.by_ref() {
                if inner.trim_end() == "```" {
                    closed = true;
                    break;
                }
                body.push(inner);
            }
            if closed {
                out.push(Node::CodeBlock(body.join("\n")));
            } else {
                // Unterminated fence: lenient fallback — re-emit the opening
                // line and its captured body as literal text.
                let mut lit = String::from("```");
                for b in body {
                    lit.push('\n');
                    lit.push_str(b);
                }
                push_node(&mut out, Node::Text(lit));
            }
            continue;
        }

        if let Some(block) = parse_line_block(line) {
            flush_para(&mut para, &mut out);
            out.push(block);
        } else {
            para.push(line);
        }
    }
    flush_para(&mut para, &mut out);
    out
}

/// If `line` is a line-leading block (`#`/`##`/`###` heading or `-#` subtext),
/// parse its inline content and return the block node. A marker not followed by
/// a space (e.g. a bare `#` or `#foo`) is *not* a block — returns `None` so the
/// line falls through to literal/inline handling.
fn parse_line_block(line: &str) -> Option<Node> {
    if let Some(rest) = line.strip_prefix("-# ") {
        return Some(Node::Subtext(build_tree(tokenize(rest))));
    }
    for (marker, level) in [("### ", 3u8), ("## ", 2), ("# ", 1)] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(Node::Heading(level, build_tree(tokenize(rest))));
        }
    }
    None
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
    /// A fully-formed inline code span; contents are already literal.
    Code(String),
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
        if let Some(after_tick) = rest.strip_prefix('`') {
            // Inline code: scan to the next backtick. Contents are literal —
            // the closing backtick must exist, else the opener is literal text.
            if let Some(close_rel) = after_tick.find('`') {
                flush!();
                let inner = &after_tick[..close_rel];
                tokens.push(Tok::Code(inner.to_string()));
                i += 1 + close_rel + 1;
            } else {
                buf.push('`');
                i += 1;
            }
        } else if let Some(after) = rest.strip_prefix("**") {
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
            Tok::Code(s) => top(&mut stack).children.push(Node::Code(s)),
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

    // --- block formats (Discord-style) -------------------------------------

    #[test]
    fn headings_by_level() {
        assert_eq!(
            parse("# big"),
            vec![Node::Heading(1, vec![text("big")])],
            "h1"
        );
        assert_eq!(
            parse("## mid"),
            vec![Node::Heading(2, vec![text("mid")])],
            "h2"
        );
        assert_eq!(
            parse("### small"),
            vec![Node::Heading(3, vec![text("small")])],
            "h3"
        );
    }

    #[test]
    fn subtext_block() {
        assert_eq!(
            parse("-# a footnote"),
            vec![Node::Subtext(vec![text("a footnote")])]
        );
    }

    #[test]
    fn heading_keeps_inline_markup() {
        assert_eq!(
            parse("## a **bold** title"),
            vec![Node::Heading(
                2,
                vec![text("a "), Node::Bold(vec![text("bold")]), text(" title"),]
            )]
        );
    }

    #[test]
    fn bare_hash_without_space_is_literal() {
        // No trailing space → not a heading marker.
        assert_eq!(parse("#nospace"), vec![text("#nospace")]);
        assert_eq!(parse("#"), vec![text("#")]);
        assert_eq!(parse("-#nope"), vec![text("-#nope")]);
    }

    #[test]
    fn inline_code_is_literal_inside() {
        assert_eq!(
            parse("run `**not bold**` now"),
            vec![
                text("run "),
                Node::Code("**not bold**".to_string()),
                text(" now"),
            ],
            "markup inside inline code stays literal"
        );
    }

    #[test]
    fn unterminated_inline_code_backtick_is_literal() {
        assert_eq!(parse("a ` b"), vec![text("a ` b")]);
    }

    #[test]
    fn fenced_code_block_verbatim() {
        let src = "```\nlet x = **5**;\n*y*\n```";
        assert_eq!(
            parse(src),
            vec![Node::CodeBlock("let x = **5**;\n*y*".to_string())],
            "fence contents are verbatim, no inner markup"
        );
    }

    #[test]
    fn fenced_code_block_among_text() {
        let src = "before\n```\ncode\n```\nafter";
        assert_eq!(
            parse(src),
            vec![
                text("before"),
                Node::CodeBlock("code".to_string()),
                text("after"),
            ]
        );
    }

    #[test]
    fn unterminated_fence_is_literal() {
        let src = "```\nstill open";
        assert_eq!(
            parse(src),
            vec![text("```\nstill open")],
            "an unclosed fence re-emits as literal text"
        );
    }

    #[test]
    fn plain_multiline_stays_one_run_with_newlines() {
        // Adjacent non-block lines join into a single inline run, newline kept.
        assert_eq!(
            parse("line one\nline two"),
            vec![text("line one\nline two")]
        );
    }

    #[test]
    fn block_lines_split_around_plain_text() {
        assert_eq!(
            parse("intro\n# Title\nbody"),
            vec![
                text("intro"),
                Node::Heading(1, vec![text("Title")]),
                text("body"),
            ]
        );
    }
}
