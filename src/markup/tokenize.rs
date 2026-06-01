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
    /// A fully-formed hyperlink. Either an explicit `[text](url)` Markdown link
    /// or a bare-URL autolink (in which case `text == url`). The `url` is
    /// already protocol-checked to http/https by the tokenizer.
    Link(String, String),
    /// A fully-formed `:name:` emoji shortcode (the bare name).
    Emoji(String),
    /// A fully-formed `@username` mention (the bare username, no `@`). Resolution
    /// to a real guild member happens server-side; the parser only recognises the
    /// syntactic shape. See [`parse_mention`].
    Mention(String),
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
            // A `[` opens either a color tag or a `[text](url)` link. Try color
            // first (it has the narrower, palette-keyed grammar), then link.
            if let Some((tok, len)) = parse_color_tag(rest) {
                flush!();
                tokens.push(tok);
                i += len;
            } else if let Some((tok, len)) = parse_link(rest) {
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
        } else if rest.starts_with('@') {
            // A `@username` mention, but only when the `@` is NOT mid-word:
            // `parse@there` / `a@b` keep their `@` literal. The preceding char
            // (the last byte already in `buf`, since `@` is ASCII and word chars
            // are ASCII) decides — a word char (`[A-Za-z0-9_]`) before it means
            // mid-word, so fall through to literal.
            let after_word_char = buf
                .as_bytes()
                .last()
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
            match (after_word_char, parse_mention(rest)) {
                (false, Some((name, len))) => {
                    flush!();
                    tokens.push(Tok::Mention(name));
                    i += len;
                }
                _ => {
                    buf.push('@');
                    i += 1;
                }
            }
        } else if let Some(len) = autolink_len(rest) {
            // Bare http(s):// URL — emit as a link whose text is the URL itself.
            flush!();
            let url = rest[..len].to_string();
            tokens.push(Tok::Link(url.clone(), url));
            i += len;
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

/// If `rest` (starting with `[`) opens a well-formed `[text](url)` link whose
/// `url` is an http/https URL, return the [`Tok::Link`] and the byte length
/// consumed (through the closing `)`). Mirrors [`parse_image`] minus the `!`.
/// Any malformation (missing `]`, missing `(`, missing `)`) — or a URL whose
/// scheme is not http/https — returns `None`, so the leading `[` falls through
/// to literal text. `text` runs to the first `]`; `url` runs to the first `)`.
fn parse_link(rest: &str) -> Option<(Tok, usize)> {
    let close_br = rest.find(']')?;
    let text = &rest[1..close_br];
    let after_br = &rest[close_br + 1..]; // "(url)…"
    let rest_paren = after_br.strip_prefix('(')?;
    let close_par = rest_paren.find(')')?;
    let url = &rest_paren[..close_par];
    if !is_safe_url_scheme(url) {
        return None;
    }
    // '[' + text + ']' + '(' + url + ')'
    let consumed = 1 + text.len() + 1 + 1 + url.len() + 1;
    Some((Tok::Link(text.to_string(), url.to_string()), consumed))
}

/// Length of a bare `http://` / `https://` autolink at the start of `rest`, or
/// `None` if `rest` does not begin with such a URL. The URL runs to the first
/// ASCII whitespace or run-terminating char; common trailing punctuation
/// (`.,;:!?` and a closing `)`/`]`) is excluded so `(see https://x)` and
/// "go to https://x." keep their delimiter literal. A scheme with no host
/// (`https://` then whitespace/EOF) is rejected.
fn autolink_len(rest: &str) -> Option<usize> {
    let scheme = if rest.starts_with("https://") {
        "https://"
    } else if rest.starts_with("http://") {
        "http://"
    } else {
        return None;
    };
    let after = &rest[scheme.len()..];
    // Consume URL chars: stop at ASCII whitespace or characters that commonly
    // delimit a URL in prose. Operates on bytes (all stop chars are ASCII), so
    // `len` always lands on a UTF-8 boundary.
    let mut len = 0;
    for &b in after.as_bytes() {
        if b.is_ascii_whitespace() || matches!(b, b'<' | b'>' | b'"' | b'`' | b'|') {
            break;
        }
        len += 1;
    }
    // Trim trailing punctuation that is far more likely sentence/grouping
    // syntax than part of the URL.
    let host = &after.as_bytes()[..len];
    while let Some((&last, init)) = host[..len].split_last() {
        if matches!(last, b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']') {
            len = init.len();
        } else {
            break;
        }
    }
    if len == 0 {
        return None; // scheme with empty host — not a real URL.
    }
    Some(scheme.len() + len)
}

/// Whether `url` carries an http/https scheme (or is a scheme-relative/relative
/// reference, which inherits the page's safe origin). Rejects `javascript:`,
/// `data:`, `file:`, `vbscript:`, and any other explicit scheme. The check is
/// case-insensitive on the scheme; a URL with NO `:` before the first `/`, `?`,
/// `#` (or no `:` at all) is treated as relative and allowed.
fn is_safe_url_scheme(url: &str) -> bool {
    // Find a scheme: leading run of scheme chars followed by ':'. Per RFC 3986
    // a scheme is ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ).
    let mut scheme_end = None;
    for (idx, ch) in url.char_indices() {
        match ch {
            ':' => {
                scheme_end = Some(idx);
                break;
            }
            '/' | '?' | '#' => break, // path/query/fragment before any ':' → relative.
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') => {}
            _ => break, // a non-scheme char before ':' → not a scheme; relative.
        }
    }
    match scheme_end {
        // No scheme → relative reference, safe.
        None => true,
        Some(0) => false, // leading ':' → no scheme name, reject.
        Some(end) => {
            let scheme = &url[..end];
            scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
        }
    }
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

/// If `rest` (starting with `@`) opens a well-formed `@username` mention, return
/// the bare username and the byte length consumed (the `@` plus the run of
/// username chars). A mention is `@` then a leading ASCII letter or `_`,
/// followed by a run of ASCII `[A-Za-z0-9_]`. Requiring a non-digit first char
/// keeps `@123` literal (a bare number is far more likely "@ 123" prose than a
/// handle) while `@user123` works. An empty or digit-led run (`@`, `@@`, `@-`,
/// `@123`, `@ `) returns `None`, so the leading `@` falls through to literal
/// text. Case is preserved as typed; the server matches case-insensitively.
/// Operates on bytes (all recognised chars are ASCII), so `len` always lands on
/// a UTF-8 boundary.
fn parse_mention(rest: &str) -> Option<(String, usize)> {
    let after = &rest.as_bytes()[1..];
    // First char must be a letter or underscore (not a digit, not punctuation).
    match after.first() {
        Some(b) if b.is_ascii_alphabetic() || *b == b'_' => {}
        _ => return None,
    }
    let len = after
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
        .count();
    Some((rest[1..1 + len].to_string(), 1 + len))
}
