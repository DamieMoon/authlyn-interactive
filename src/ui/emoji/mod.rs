//! Emoji rendering — the per-guild custom-emoji resolver + the standard-unicode
//! dataset ([`data`]).
//!
//! A `:shortcode:` in a message resolves, in order, to:
//!   1. a **custom per-guild** emoji image (`/media/{id}`),
//!   2. a **standard unicode** glyph (the [`data`] table, hydrate-only), or
//!   3. the **literal** `:name:` if unknown (the markup parser stays lenient).
//!
//! [`EmojiResolver`] is a `Copy` Leptos context provided once by `AppShell`, so
//! the markup renderer (`markup_view::render_node`) resolves emoji via
//! `use_context` without threading a parameter through every render call site.
//! Outside a guild (or on ssr) the context is absent / empty and `:name:`
//! renders literally.

use std::collections::HashMap;

use leptos::prelude::*;

pub mod data;

/// Resolves an emoji `:shortcode:` to a view. Carries the active guild's custom
/// emoji map (shortcode → media id) as a reactive [`Memo`]; the standard-unicode
/// fallback lives in [`data`] (hydrate-only).
#[derive(Clone, Copy)]
pub struct EmojiResolver {
    custom: Memo<HashMap<String, String>>,
}

impl EmojiResolver {
    pub fn new(custom: Memo<HashMap<String, String>>) -> Self {
        Self { custom }
    }

    /// Render `:name:`. Custom guild emoji win over standard unicode; an unknown
    /// shortcode renders as the literal text. Read untracked: a render pass sees
    /// a stable map, and newly-uploaded emoji surface on the next message/render
    /// rather than re-rendering the whole history.
    pub fn resolve(&self, name: &str) -> AnyView {
        if let Some(media_id) = self.custom.get_untracked().get(name) {
            let src = format!("/media/{media_id}?w=32");
            let label = format!(":{name}:");
            return view! {
                <img class="inline-emoji" src=src alt=label.clone() title=label />
            }
            .into_any();
        }
        if let Some(glyph) = data::glyph(name) {
            return glyph.into_any();
        }
        format!(":{name}:").into_any()
    }
}
