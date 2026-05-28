//! Emoji `:`-autocomplete primitives + picker-grid buttons.
//!
//! - [`Suggestion`] + [`emoji_suggestions`] back the popover and the picker
//!   grid: custom-guild emoji first, standard-unicode glyphs second, capped
//!   at 8.
//! - [`active_shortcode_token`] scans the text before the caret for a
//!   trailing `:query` token; pure (not cfg-gated) so the unit test reaches
//!   it.
//! - [`replace_shortcode_token`] splices the chosen `:name: ` into the
//!   composer, working in UTF-16 / JS-string space (selection ranges are
//!   UTF-16 units) and deferring `set_selection_range` past Leptos' next
//!   prop:value flush. THIS IS LOAD-BEARING — do not convert to byte offsets.
//! - [`custom_emoji_btn`] / [`unicode_emoji_btn`] are the picker-grid tiles;
//!   they call back into `super::apply_markup` (the composer's caret-aware
//!   splicer) and close the picker.

use leptos::prelude::*;

use super::super::Shell;
use crate::ui::emoji::data;

/// One `:`-autocomplete row: a guild custom emoji (image) or a standard-unicode
/// glyph. `name` is the shortcode (sans colons) that gets inserted on accept.
pub(super) struct Suggestion {
    pub name: String,
    pub media: Option<String>,
    pub glyph: Option<&'static str>,
}

/// Autocomplete candidates for a `:query`: the open guild's custom emoji whose
/// name starts with the query first, then the standard-unicode matches, capped
/// at 8. `data::search` is empty on ssr, so on the server this only ever returns
/// custom-emoji rows (and the popover never renders without a caret anyway).
pub(super) fn emoji_suggestions(s: Shell, query: &str) -> Vec<Suggestion> {
    let q = query.to_lowercase();
    let mut out: Vec<Suggestion> = s
        .sel
        .guild_emoji
        .get()
        .into_iter()
        .filter(|e| e.name.to_lowercase().starts_with(&q))
        .map(|e| Suggestion {
            name: e.name,
            media: Some(e.media_id),
            glyph: None,
        })
        .collect();
    for e in data::search(query, 8) {
        out.push(Suggestion {
            name: e.shortcode.to_string(),
            media: None,
            glyph: Some(e.glyph),
        });
    }
    out.truncate(8);
    out
}

/// Trailing emoji token `:query` before the caret (`:` then ≥1 of [a-z0-9_],
/// the `:` not preceded by an alphanumeric so `12:30`/`http:` don't trigger).
/// Returns (query, token_len) where token_len = the `:`+query length (ASCII,
/// == UTF-16 units).
///
/// Pure (not cfg-gated) so the unit tests reach it, but only *called* from the
/// hydrate-only composer handlers — hence dead on the ssr non-test build.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(super) fn active_shortcode_token(before: &str) -> Option<(String, u32)> {
    let b = before.as_bytes();
    let mut i = b.len();
    while i > 0 && (b[i - 1].is_ascii_lowercase() || b[i - 1].is_ascii_digit() || b[i - 1] == b'_')
    {
        i -= 1;
    }
    if i == b.len() || i == 0 || b[i - 1] != b':' {
        return None;
    }
    let colon = i - 1;
    if colon > 0 && b[colon - 1].is_ascii_alphanumeric() {
        return None;
    }
    Some((before[i..].to_string(), (before.len() - colon) as u32))
}

/// Replace the `start..end` (UTF-16) `:query` token in the composer with the
/// chosen `:name: ` and place the caret just after it. Hydrate-only DOM work
/// (selection ranges are UTF-16 units, so we splice in JS-string space).
#[cfg(feature = "hydrate")]
pub(super) fn replace_shortcode_token(
    s: Shell,
    ta: NodeRef<leptos::html::Textarea>,
    start: u32,
    end: u32,
    name: &str,
) {
    let Some(el) = ta.get() else { return };
    let v = js_sys::JsString::from(el.value().as_str());
    let before = v.slice(0, start).as_string().unwrap_or_default();
    let after = v.slice(end, v.length()).as_string().unwrap_or_default();
    let insert = format!(":{name}: ");
    s.composer.compose.set(format!("{before}{insert}{after}"));
    let caret = start + insert.encode_utf16().count() as u32;
    leptos::task::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(0).await;
        let _ = el.set_selection_range(caret, caret);
        let _ = el.focus();
    });
}

#[cfg(not(feature = "hydrate"))]
pub(super) fn replace_shortcode_token(
    _s: Shell,
    _ta: NodeRef<leptos::html::Textarea>,
    _start: u32,
    _end: u32,
    _name: &str,
) {
}

/// One custom-emoji button in the picker grid: its image, inserting `:name: `
/// at the caret and closing the picker on click.
pub(super) fn custom_emoji_btn(
    s: Shell,
    composer_ref: NodeRef<leptos::html::Textarea>,
    emoji_open: RwSignal<bool>,
    name: String,
    media_id: String,
) -> impl IntoView {
    let title = format!(":{name}:");
    let alt = title.clone();
    let src = format!("/media/{media_id}?w=32");
    view! {
        <button class="emoji-btn" title=title
            on:click=move |_| {
                super::apply_markup(s, composer_ref, &format!(":{name}: "), "");
                emoji_open.set(false);
            }>
            <img src=src alt=alt/>
        </button>
    }
}

/// One standard-unicode emoji button in the picker grid: its glyph, inserting
/// `:shortcode: ` at the caret and closing the picker on click.
pub(super) fn unicode_emoji_btn(
    s: Shell,
    composer_ref: NodeRef<leptos::html::Textarea>,
    emoji_open: RwSignal<bool>,
    shortcode: &'static str,
    glyph: &'static str,
) -> impl IntoView {
    view! {
        <button class="emoji-btn" title=format!(":{shortcode}:")
            on:click=move |_| {
                super::apply_markup(s, composer_ref, &format!(":{shortcode}: "), "");
                emoji_open.set(false);
            }>
            {glyph}
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::active_shortcode_token;

    #[test]
    fn detects_a_trailing_shortcode_token() {
        // Pure-ASCII so UTF-8 byte length == UTF-16 unit length and the test
        // can match the (utf16) token_len that the caret splice will use.
        assert_eq!(active_shortcode_token(":smi"), Some(("smi".into(), 4)));
        // Allow digits + underscores after the colon, but not as the first char.
        assert_eq!(active_shortcode_token("x :tada"), Some(("tada".into(), 5)));
        assert_eq!(active_shortcode_token(":joy_2"), Some(("joy_2".into(), 6)));
    }

    #[test]
    fn rejects_non_tokens() {
        // No colon → not a token at all.
        assert_eq!(active_shortcode_token("hi"), None);
        // `:` preceded by alphanumeric (URL scheme, time) → not a shortcode.
        assert_eq!(active_shortcode_token("12:30"), None);
        assert_eq!(active_shortcode_token("http:smile"), None);
        // Just a `:` or `name:` (cursor on the closing) → no body yet.
        assert_eq!(active_shortcode_token(":"), None);
        // `foo:` — colon at end, no following ascii-lowercase body.
        assert_eq!(active_shortcode_token("foo:"), None);
    }
}
