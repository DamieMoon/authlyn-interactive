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
//! - link: `[text](url)` (a hyperlink — see [`Node::Link`]); bare `http://` /
//!   `https://` runs also autolink. `url` is whitelisted to http/https (or a
//!   scheme-relative reference) — `javascript:`/`data:`/`file:`/`vbscript:`
//!   never linkify and stay literal text.
//! - spoiler: `||text||` (hidden until clicked — see [`Node::Spoiler`])
//! - mention: `@username` (L-4; alphanumeric/underscore, must not start
//!   mid-word or with a digit — see [`Node::Mention`]); a malformed `@` stays
//!   literal text
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
    /// Hyperlink (`text`, `url`). Either an explicit `[text](url)` Markdown link
    /// or a bare-URL autolink (then `text == url`). `url` is guaranteed http/
    /// https or a scheme-relative reference by the tokenizer; the renderer emits
    /// an `<a href>` and Leptos escapes both fields.
    Link(String, String),
    /// Spoiler `||…||`: hidden until clicked. Children parsed inline (nests).
    Spoiler(Vec<Node>),
    /// Mention `@username` (L-4) — the bare username (no `@`). The parser only
    /// recognises the syntactic shape; whether the name names a real guild
    /// member is resolved server-side at send time (`pinged_users`). Rendered as
    /// a styled `<span class="mk-mention">@name</span>` (not a link in v1). A
    /// malformed `@` (`@`, `@@`, `@123`, mid-word `@`) never reaches here — it
    /// stays literal text (see the tokenizer's `parse_mention`).
    Mention(String),
}

/// Parse a message body into a markup tree. Never fails (see module docs).
///
/// Two-level parse: split into blocks by line (detecting line-leading markers
/// and ```` ``` ```` fences), then parse the inline content of each non-code
/// block with the inline tokenizer/tree-builder.
pub fn parse(input: &str) -> Vec<Node> {
    blocks::parse_blocks(input)
}

/// Collect the lowercased, de-duplicated set of `@username` mentions in `input`,
/// in first-appearance order (L-4). Walks the parsed AST so the same leniency
/// the renderer obeys applies here too: a `@name` inside an inline code span or
/// a fenced code block is verbatim text, NOT a mention, and a malformed `@`
/// never produces one. Lowercased because the server matches member usernames
/// case-insensitively. The returned names are syntactic only — resolution to a
/// real guild member happens server-side (`posting.rs`).
pub fn collect_mentions(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_mentions_nodes(&parse(input), &mut out, &mut seen);
    out
}

fn collect_mentions_nodes(
    nodes: &[Node],
    out: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    for node in nodes {
        match node {
            Node::Mention(name) => {
                let lower = name.to_lowercase();
                if seen.insert(lower.clone()) {
                    out.push(lower);
                }
            }
            // Recurse into every inline/block container so a mention nested in
            // bold/italic/color/dialogue/spoiler/heading/subtext is still found.
            Node::Bold(c)
            | Node::Italic(c)
            | Node::Color(_, c)
            | Node::Heading(_, c)
            | Node::Subtext(c)
            | Node::Dialogue(c)
            | Node::Spoiler(c) => collect_mentions_nodes(c, out, seen),
            // Leaf nodes that never contain a mention (code is verbatim).
            Node::Text(_)
            | Node::Code(_)
            | Node::CodeBlock(_)
            | Node::Emoji(_)
            | Node::Image(_, _)
            | Node::Link(_, _) => {}
        }
    }
}

/// Drop `[name]`/`[/name]` color tags from a message body, leaving every other
/// markup token intact. Used by the per-message "copy as markdown" action so
/// the copied source can be re-sent under a different persona without
/// dragging the original speaker's palette choice along (Foxtrot feedback,
/// ctx 019e6f23-fcfc).
///
/// Lenient (see module docs): unknown `[…]` tags pass through unchanged. The
/// scanner walks UTF-8 boundaries and treats `![alt](url)` images verbatim so
/// an image whose alt text happens to be a color name (e.g. `![red](u)`)
/// doesn't get mangled.
pub fn strip_color_tokens(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        // Skip past `![alt](url)` images verbatim so `[alt]` inside them
        // isn't mistaken for a color open.
        if let Some(len) = image_run_len(rest) {
            out.push_str(&rest[..len]);
            i += len;
            continue;
        }
        // Drop a well-formed color open/close tag.
        if rest.starts_with('[') {
            if let Some(len) = color_tag_len(rest) {
                i += len;
                continue;
            }
        }
        // Otherwise copy one full UTF-8 char (boundary-safe).
        let ch = rest.chars().next().expect("non-empty rest");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Byte length of a `[name]` or `[/name]` color tag at the start of `rest`,
/// or `None` if the tag is malformed or names an unknown color.
fn color_tag_len(rest: &str) -> Option<usize> {
    if !rest.starts_with('[') {
        return None;
    }
    let close = rest.find(']')?;
    let inner = &rest[1..close];
    let name = inner.strip_prefix('/').unwrap_or(inner);
    Color::from_name(name).map(|_| close + 1)
}

/// Byte length of a well-formed `![alt](url)` image at the start of `rest`,
/// or `None` if the syntax is malformed (any malformation falls through to
/// per-char copying, matching the tokenizer's leniency).
fn image_run_len(rest: &str) -> Option<usize> {
    let after_bang = rest.strip_prefix('!')?;
    let after_open_br = after_bang.strip_prefix('[')?;
    let close_br = after_open_br.find(']')?;
    let after_close_br = &after_open_br[close_br + 1..];
    let in_paren = after_close_br.strip_prefix('(')?;
    let close_par = in_paren.find(')')?;
    // '!' + '[' + alt + ']' + '(' + url + ')'
    Some(1 + 1 + close_br + 1 + 1 + close_par + 1)
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
        // A non-http(s) scheme keeps its `:` literal and is NOT autolinked
        // (only http/https bare URLs linkify — see the L-2 link tests below).
        assert_eq!(parse("see ftp://x"), vec![text("see ftp://x")]);
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

    // --- L-2 hyperlinks: explicit `[text](url)` + bare-URL autolink ---------

    fn link(t: &str, u: &str) -> Node {
        Node::Link(t.to_string(), u.to_string())
    }

    #[test]
    fn explicit_markdown_link_becomes_a_link_node() {
        assert_eq!(parse("[text](http://x)"), vec![link("text", "http://x")]);
        assert_eq!(
            parse("see [the docs](https://example.com/a) now"),
            vec![
                text("see "),
                link("the docs", "https://example.com/a"),
                text(" now"),
            ]
        );
    }

    #[test]
    fn malformed_link_brackets_or_parens_stay_literal() {
        assert_eq!(parse("[text]("), vec![text("[text](")], "no closing paren");
        // Unterminated link (no closing `)`): the `[text](` prefix stays literal.
        // A relative target is used so no bare-URL autolink fires on the tail.
        assert_eq!(
            parse("[text](/path"),
            vec![text("[text](/path")],
            "unterminated url"
        );
        assert_eq!(
            parse("[text]"),
            vec![text("[text]")],
            "no paren group at all"
        );
        assert_eq!(parse("[not a tag]"), vec![text("[not a tag]")]);
    }

    #[test]
    fn bare_http_and_https_urls_autolink() {
        assert_eq!(
            parse("http://example.com"),
            vec![link("http://example.com", "http://example.com")]
        );
        assert_eq!(
            parse("go to https://example.com/path now"),
            vec![
                text("go to "),
                link("https://example.com/path", "https://example.com/path"),
                text(" now"),
            ]
        );
    }

    #[test]
    fn autolink_trims_trailing_sentence_punctuation_and_parens() {
        // Trailing prose punctuation is kept literal, not absorbed into the URL.
        assert_eq!(
            parse("visit https://x.com."),
            vec![
                text("visit "),
                link("https://x.com", "https://x.com"),
                text(".")
            ]
        );
        assert_eq!(
            parse("(see https://x.com)"),
            vec![
                text("(see "),
                link("https://x.com", "https://x.com"),
                text(")")
            ]
        );
    }

    #[test]
    fn malformed_bare_scheme_is_literal() {
        // A single slash, or a scheme with no host, must not linkify.
        assert_eq!(parse("https:/x"), vec![text("https:/x")]);
        assert_eq!(parse("see https:// here"), vec![text("see https:// here")]);
        // A non-http(s) bare scheme is never autolinked.
        assert_eq!(parse("ftp://host/f"), vec![text("ftp://host/f")]);
    }

    #[test]
    fn javascript_and_other_unsafe_schemes_never_linkify() {
        // The explicit-link path rejects any non-http(s) scheme: the whole
        // `[...](...)` stays literal text, never an `<a href>`.
        assert_eq!(
            parse("[click](javascript:alert(1))"),
            vec![text("[click](javascript:alert(1))")],
            "javascript: is not a link"
        );
        assert_eq!(
            parse("[x](JavaScript:alert(1))"),
            vec![text("[x](JavaScript:alert(1))")],
            "scheme check is case-insensitive"
        );
        assert_eq!(
            parse("[x](data:text/html,foo)"),
            vec![text("[x](data:text/html,foo)")],
            "data: is not a link"
        );
        assert_eq!(parse("[x](file:///etc)"), vec![text("[x](file:///etc)")]);
        assert_eq!(
            parse("[x](vbscript:msgbox)"),
            vec![text("[x](vbscript:msgbox)")]
        );
    }

    #[test]
    fn relative_link_target_is_allowed() {
        // A scheme-relative path has no dangerous scheme → it links.
        assert_eq!(parse("[home](/guilds/1)"), vec![link("home", "/guilds/1")]);
    }

    #[test]
    fn link_nested_inside_bold_and_color_parses() {
        assert_eq!(
            parse("**[t](http://x)**"),
            vec![Node::Bold(vec![link("t", "http://x")])]
        );
        assert_eq!(
            parse("[blue]see [t](https://x)[/blue]"),
            vec![Node::Color(
                Color::Blue,
                vec![text("see "), link("t", "https://x")]
            )]
        );
    }

    #[test]
    fn url_inside_inline_code_stays_literal() {
        // Code spans are verbatim — no autolinking inside.
        assert_eq!(
            parse("`http://x`"),
            vec![Node::Code("http://x".to_string())]
        );
    }

    #[test]
    fn image_still_parses_and_is_not_a_link() {
        // The `![alt](url)` image path is unaffected by the new link path.
        assert_eq!(
            parse("![a cat](/media/abc)"),
            vec![Node::Image("a cat".to_string(), "/media/abc".to_string())]
        );
    }

    // --- L-4 mentions: `@username` -> Node::Mention, lenient on malformed ---

    fn mention(name: &str) -> Node {
        Node::Mention(name.to_string())
    }

    #[test]
    fn bare_mention_becomes_a_node() {
        assert_eq!(parse("@alice"), vec![mention("alice")]);
        assert_eq!(
            parse("hi @bob there"),
            vec![text("hi "), mention("bob"), text(" there")]
        );
        // Underscores and digits (after a leading letter) are part of a handle.
        assert_eq!(parse("@user_123"), vec![mention("user_123")]);
        assert_eq!(parse("@_hidden"), vec![mention("_hidden")]);
    }

    #[test]
    fn mention_preserves_case_in_node() {
        // The node keeps the case as typed; the SERVER lowercases for matching
        // (the parser stays purely syntactic).
        assert_eq!(parse("@CAPS"), vec![mention("CAPS")]);
        assert_eq!(parse("@MixedCase"), vec![mention("MixedCase")]);
    }

    #[test]
    fn malformed_at_signs_stay_literal() {
        assert_eq!(parse("@"), vec![text("@")], "lone @");
        assert_eq!(parse("@@"), vec![text("@@")], "double @");
        assert_eq!(
            parse("@123"),
            vec![text("@123")],
            "digit-led is not a handle"
        );
        assert_eq!(parse("@-"), vec![text("@-")], "@ then punctuation");
        assert_eq!(parse("@ space"), vec![text("@ space")], "@ then space");
        assert_eq!(parse("trailing @"), vec![text("trailing @")], "trailing @");
    }

    #[test]
    fn mid_word_at_stays_literal() {
        // An `@` preceded by a word char is an email-ish / handle-tail, not a
        // mention: keep it literal.
        assert_eq!(parse("parse@there"), vec![text("parse@there")]);
        assert_eq!(parse("a@b"), vec![text("a@b")]);
        assert_eq!(
            parse("email me at user@host.com"),
            vec![text("email me at user@host.com")]
        );
        // But an `@` after a non-word char (punctuation/space) still mentions.
        assert_eq!(
            parse("(@nick)"),
            vec![text("("), mention("nick"), text(")")]
        );
    }

    #[test]
    fn mention_nested_in_bold_and_color() {
        assert_eq!(
            parse("**@alice**"),
            vec![Node::Bold(vec![mention("alice")])]
        );
        assert_eq!(
            parse("[red]ping @bob[/red]"),
            vec![Node::Color(Color::Red, vec![text("ping "), mention("bob")])]
        );
    }

    #[test]
    fn mention_inside_inline_code_stays_literal() {
        // Code spans are verbatim — no mention scanning inside.
        assert_eq!(parse("`@alice`"), vec![Node::Code("@alice".to_string())]);
        assert_eq!(
            parse("run `@cmd` now"),
            vec![text("run "), Node::Code("@cmd".to_string()), text(" now")]
        );
    }

    #[test]
    fn mention_terminates_at_non_word_char() {
        // The handle stops at the first non-`[A-Za-z0-9_]` char.
        assert_eq!(
            parse("@alice, hello"),
            vec![mention("alice"), text(", hello")]
        );
        assert_eq!(parse("@bob's turn"), vec![mention("bob"), text("'s turn")]);
    }

    // --- L-4 collect_mentions: AST-walk extraction for the server send path ---

    #[test]
    fn collect_mentions_lowercases_and_dedupes_in_order() {
        assert_eq!(
            collect_mentions("hey @Alice and @bob and @ALICE again"),
            vec!["alice".to_string(), "bob".to_string()],
            "case-insensitive, de-duplicated, first-appearance order"
        );
    }

    #[test]
    fn collect_mentions_finds_nested_but_skips_code() {
        assert_eq!(
            collect_mentions("**@bold** [blue]@tint[/blue]"),
            vec!["bold".to_string(), "tint".to_string()]
        );
        // A mention inside inline code or a fence is literal text, NOT a mention.
        assert_eq!(collect_mentions("`@nope`"), Vec::<String>::new());
        assert_eq!(collect_mentions("```\n@nope\n```"), Vec::<String>::new());
    }

    #[test]
    fn collect_mentions_empty_when_none() {
        assert_eq!(collect_mentions("no pings here"), Vec::<String>::new());
        assert_eq!(collect_mentions("@123 @ @@"), Vec::<String>::new());
    }

    // ---- strip_color_tokens (copy-as-markdown helper, ctx 019e6f23-fcfc) ----

    #[test]
    fn strip_color_passes_plain_text_through() {
        assert_eq!(strip_color_tokens("hello world"), "hello world");
        assert_eq!(strip_color_tokens(""), "");
    }

    #[test]
    fn strip_color_drops_matched_open_close_pairs() {
        assert_eq!(strip_color_tokens("[red]hi[/red]"), "hi");
        assert_eq!(
            strip_color_tokens("[blue]calm **and** *steady*[/blue]"),
            "calm **and** *steady*",
            "siblings of the color tags survive verbatim"
        );
    }

    #[test]
    fn strip_color_handles_nested_pairs() {
        assert_eq!(
            strip_color_tokens("[blue]a [orange]b[/orange] c[/blue]"),
            "a b c"
        );
    }

    #[test]
    fn strip_color_drops_unmatched_tags_too() {
        // The renderer is lenient with unmatched tags; the strip helper mirrors
        // that — drop whatever LOOKS like a color tag and let downstream callers
        // re-parse the result.
        assert_eq!(strip_color_tokens("[red]hi"), "hi");
        assert_eq!(strip_color_tokens("hi[/red]"), "hi");
    }

    #[test]
    fn strip_color_preserves_non_color_bracket_runs() {
        // Unknown-palette tags pass through unchanged (they were going to render
        // as literal text anyway).
        assert_eq!(
            strip_color_tokens("[invalidcolor]x[/invalidcolor]"),
            "[invalidcolor]x[/invalidcolor]"
        );
        assert_eq!(strip_color_tokens("[]empty"), "[]empty");
        assert_eq!(strip_color_tokens("price [$5]"), "price [$5]");
        assert_eq!(strip_color_tokens("just a [ bracket"), "just a [ bracket");
    }

    #[test]
    fn strip_color_keeps_image_syntax_intact_even_with_color_alt() {
        // `![alt](url)` with alt = "red" mustn't have its `[red]` chunk
        // mistaken for a color open and dropped.
        assert_eq!(strip_color_tokens("![red](u)"), "![red](u)");
        assert_eq!(
            strip_color_tokens("![red](u) and [red]actually red[/red]"),
            "![red](u) and actually red"
        );
    }

    #[test]
    fn strip_color_keeps_other_markup_around_tags() {
        // Bold/italic/spoiler/code spans are preserved verbatim — only color
        // brackets disappear.
        assert_eq!(
            strip_color_tokens("**bold** *it* `code` ||spoil||"),
            "**bold** *it* `code` ||spoil||"
        );
        assert_eq!(
            strip_color_tokens("**[red]bold red[/red]**"),
            "**bold red**"
        );
    }

    #[test]
    fn strip_color_is_utf8_boundary_safe() {
        // Multibyte chars between tags must not split.
        assert_eq!(strip_color_tokens("[red]héllo 🦀[/red]"), "héllo 🦀");
    }

    #[test]
    fn strip_color_drops_all_eight_palette_names() {
        // Cover every palette entry — guards against a future name change in
        // the Color enum desyncing the strip helper.
        for c in Color::ALL {
            let name = c.name();
            let body = format!("[{name}]x[/{name}]");
            assert_eq!(strip_color_tokens(&body), "x", "{name}");
        }
    }

    #[test]
    fn strip_color_round_trip_through_parse_drops_color_nodes() {
        // The downstream invariant: after stripping, re-parsing must produce
        // an AST with no `Node::Color` left anywhere at the top level.
        let stripped = strip_color_tokens("[red]a[/red] [blue]b[/blue]");
        let ast = parse(&stripped);
        for n in &ast {
            assert!(!matches!(n, Node::Color(_, _)), "found color node: {n:?}");
        }
    }
}
