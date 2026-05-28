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
//! - dialogue: `"text"` (RP speech; the renderer keeps the quotes, and a
//!   per-user toggle styles it at render time — see [`Node::Dialogue`])
//! - emoji: `:shortcode:` (a custom per-guild emoji or a standard unicode
//!   glyph; an unknown/ill-formed shortcode stays literal — see [`Node::Emoji`])
//! - image: `![alt](url)` (embedded inline image — see [`Node::Image`])
//! - spoiler: `||text||` (hidden until clicked — see [`Node::Spoiler`])
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
//!
//! ## Layout
//! - [`tokenize`] — UTF-8-boundary-safe scanner producing `Tok`s.
//! - [`tree`] — stack-based builder turning `Tok`s into `Node`s (with the
//!   leniency unwind).
//! - [`blocks`] — block-level pass (line-leading markers + fenced code),
//!   composing the inline pipeline for non-code content.

mod blocks;
mod tokenize;
mod tree;

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
    /// RP dialogue `"…"`. Children are parsed inline (nesting allowed). The
    /// renderer re-emits the surrounding quotes, so the text reads identically
    /// whether or not the per-user dialogue styling is active.
    Dialogue(Vec<Node>),
    /// Emoji shortcode `:name:` — the bare name. Resolved at render time to a
    /// custom per-guild emoji image or a standard unicode glyph (lenient: an
    /// unknown shortcode renders as the literal `:name:`).
    Emoji(String),
    /// Embedded image `![alt](url)`.
    Image(String, String),
    /// Spoiler `||…||`: hidden until clicked. Children parsed inline (nests).
    Spoiler(Vec<Node>),
}

/// Parse a message body into a markup tree. Never fails (see module docs).
///
/// Two-level parse: split into blocks by line (detecting line-leading markers
/// and ```` ``` ```` fences), then parse the inline content of each non-code
/// block with the inline tokenizer/tree-builder.
pub fn parse(input: &str) -> Vec<Node> {
    blocks::parse_blocks(input)
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

    // --- Wave B constructs: dialogue / emoji / image / spoiler -------------

    #[test]
    fn dialogue_quotes_become_a_node() {
        assert_eq!(
            parse("\"hello\""),
            vec![Node::Dialogue(vec![text("hello")])]
        );
    }

    #[test]
    fn dialogue_nests_inline_markup() {
        assert_eq!(
            parse("\"she **waves**\""),
            vec![Node::Dialogue(vec![
                text("she "),
                Node::Bold(vec![text("waves")]),
            ])]
        );
    }

    #[test]
    fn two_dialogue_runs_with_narration_between() {
        assert_eq!(
            parse("\"a\" then \"b\""),
            vec![
                Node::Dialogue(vec![text("a")]),
                text(" then "),
                Node::Dialogue(vec![text("b")]),
            ]
        );
    }

    #[test]
    fn unclosed_dialogue_is_literal_quote() {
        assert_eq!(parse("\"hello"), vec![text("\"hello")]);
    }

    #[test]
    fn emoji_shortcode_becomes_a_node() {
        assert_eq!(parse(":smile:"), vec![Node::Emoji("smile".to_string())]);
        assert_eq!(
            parse("hi :wave: there"),
            vec![text("hi "), Node::Emoji("wave".to_string()), text(" there"),]
        );
    }

    #[test]
    fn non_emoji_colons_stay_literal() {
        // Times, smileys and URLs must survive untouched.
        assert_eq!(parse("12:30"), vec![text("12:30")]);
        assert_eq!(parse(":)"), vec![text(":)")]);
        assert_eq!(parse("see https://x"), vec![text("see https://x")]);
        assert_eq!(parse("a :: b"), vec![text("a :: b")]);
        assert_eq!(
            parse(":Caps:"),
            vec![text(":Caps:")],
            "shortcodes are lowercase-only"
        );
    }

    #[test]
    fn image_becomes_a_node() {
        assert_eq!(
            parse("![a cat](/media/abc)"),
            vec![Node::Image("a cat".to_string(), "/media/abc".to_string())]
        );
    }

    #[test]
    fn malformed_image_is_literal() {
        assert_eq!(parse("![a]"), vec![text("![a]")], "no url");
        assert_eq!(parse("![a](u"), vec![text("![a](u")], "unterminated url");
        assert_eq!(parse("text ! bang"), vec![text("text ! bang")], "lone bang");
    }

    #[test]
    fn spoiler_becomes_a_node() {
        assert_eq!(
            parse("||secret||"),
            vec![Node::Spoiler(vec![text("secret")])]
        );
    }

    #[test]
    fn unclosed_spoiler_and_single_pipe_are_literal() {
        assert_eq!(parse("||oops"), vec![text("||oops")]);
        assert_eq!(parse("a | b"), vec![text("a | b")]);
    }

    #[test]
    fn spoiler_nests_inline_markup() {
        assert_eq!(
            parse("||a **secret**||"),
            vec![Node::Spoiler(vec![
                text("a "),
                Node::Bold(vec![text("secret")]),
            ])]
        );
    }
}
