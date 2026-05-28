//! Inline tokenizer: a UTF-8-boundary-safe linear scan that turns a raw
//! string into a sequence of [`Tok`]s for the tree builder to consume.
//!
//! Split from `src/markup.rs` in Wave 3; behavior preserved verbatim.
//! Leniency invariant: an unmatched/ill-formed marker collapses to its
//! literal character via the `buf.push(_)` fallback paths — never panics.

use super::Color;

pub(super) enum Tok {
    Text(String),
    Bold,
    Italic,
    ColorOpen(Color),
    ColorClose(Color),
    /// A fully-formed inline code span; contents are already literal.
    Code(String),
    /// `"` dialogue delimiter (toggles like bold/italic).
    Dialogue,
    /// `||` spoiler delimiter (toggles like bold/italic).
    Spoiler,
    /// A fully-formed `![alt](url)` image (alt, url).
    Image(String, String),
    /// A fully-formed `:name:` emoji shortcode (the bare name).
    Emoji(String),
}

pub(super) fn tokenize(s: &str) -> Vec<Tok> {
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
        } else if rest.starts_with("![") {
            if let Some((tok, len)) = parse_image(rest) {
                flush!();
                tokens.push(tok);
                i += len;
            } else {
                buf.push('!');
                i += 1;
            }
        } else if rest.starts_with("||") {
            flush!();
            tokens.push(Tok::Spoiler);
            i += 2;
        } else if rest.starts_with('"') {
            flush!();
            tokens.push(Tok::Dialogue);
            i += 1;
        } else if rest.starts_with(':') {
            if let Some((name, len)) = parse_emoji(rest) {
                flush!();
                tokens.push(Tok::Emoji(name));
                i += len;
            } else {
                buf.push(':');
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

/// If `rest` (starting with `![`) opens a well-formed `![alt](url)` image,
/// return the [`Tok::Image`] and the byte length consumed (through the closing
/// `)`). Any malformation (missing `]`, missing `(`, missing `)`) returns
/// `None`, so the leading `!` falls through to literal text. `alt` runs to the
/// first `]`; `url` runs to the first `)`.
fn parse_image(rest: &str) -> Option<(Tok, usize)> {
    let after_bang = &rest[1..]; // "[alt](url)…"
    let close_br = after_bang.find(']')?;
    let alt = &after_bang[1..close_br];
    let after_br = &after_bang[close_br + 1..]; // "(url)…"
    let rest_paren = after_br.strip_prefix('(')?;
    let close_par = rest_paren.find(')')?;
    let url = &rest_paren[..close_par];
    // '!' + '[' + alt + ']' + '(' + url + ')'
    let consumed = 1 + 1 + alt.len() + 1 + 1 + url.len() + 1;
    Some((Tok::Image(alt.to_string(), url.to_string()), consumed))
}

/// If `rest` (starting with `:`) opens a well-formed `:shortcode:` — one or more
/// of `[a-z0-9_]` between two colons — return the bare name and the byte length
/// consumed. An empty run, an unterminated run, or any other character returns
/// `None`, so the leading `:` falls through to literal text (keeping `12:30`,
/// `:)`, and `https://` intact).
fn parse_emoji(rest: &str) -> Option<(String, usize)> {
    let after = &rest[1..];
    let close = after.find(':')?;
    if close == 0 {
        return None;
    }
    let name = &after[..close];
    if name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        Some((name.to_string(), 1 + name.len() + 1))
    } else {
        None
    }
}
