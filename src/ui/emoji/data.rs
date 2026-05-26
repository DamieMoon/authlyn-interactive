//! Standard-unicode emoji dataset access, isolated so the `emojis` crate (a
//! large static table, ~1900 entries) is referenced from exactly **one** place
//! and only on the hydrate (wasm) target. ssr gets inert stubs, so the server
//! renders an unknown `:name:` literally and hydrate swaps in the glyph — the
//! same text-content pattern the channel view already uses for local-time
//! timestamps, so there's no jarring hydration mismatch.
//!
//! Keeping every `emojis::*` reference behind this module is what lets a later
//! optimisation (lazy-fetch the set as a JSON asset, if the rodata proves heavy
//! in the bundle) stay local to this file.

/// One standard-unicode emoji surfaced to the picker / autocomplete. `Copy` and
/// all-`&'static` so it threads cheaply through view code.
#[derive(Clone, Copy)]
pub struct StdEmoji {
    /// The unicode glyph, e.g. `"😄"`.
    pub glyph: &'static str,
    /// The primary github-style shortcode, without colons, e.g. `"smile"`.
    pub shortcode: &'static str,
}

/// Emoji group display labels, in picker order. Mirrors `emojis::Group`.
pub const GROUPS: &[&str] = &[
    "Smileys & Emotion",
    "People & Body",
    "Animals & Nature",
    "Food & Drink",
    "Travel & Places",
    "Activities",
    "Objects",
    "Symbols",
    "Flags",
];

#[cfg(feature = "hydrate")]
pub use real::{by_group, glyph, search};

#[cfg(not(feature = "hydrate"))]
pub use stub::{by_group, glyph, search};

#[cfg(feature = "hydrate")]
mod real {
    use super::StdEmoji;

    /// The unicode glyph for a github `:shortcode:`, if it is a standard emoji.
    /// `None` for unknown or custom shortcodes (the caller falls back to either
    /// a custom guild emoji or the literal text).
    pub fn glyph(shortcode: &str) -> Option<&'static str> {
        emojis::get_by_shortcode(shortcode).map(|e| e.as_str())
    }

    fn to_std(e: &'static emojis::Emoji) -> Option<StdEmoji> {
        Some(StdEmoji {
            glyph: e.as_str(),
            shortcode: e.shortcode()?,
        })
    }

    /// Up to `limit` standard emoji whose CLDR name or any shortcode contains
    /// the (lowercased) query. A linear scan over the static table — trivial at
    /// this size, cheaper than the grid's DOM render, so no index is built.
    pub fn search(query: &str, limit: usize) -> Vec<StdEmoji> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        emojis::iter()
            .filter(|e| {
                e.name().to_lowercase().contains(q.as_str())
                    || e.shortcodes().any(|sc| sc.contains(q.as_str()))
            })
            .filter_map(to_std)
            .take(limit)
            .collect()
    }

    /// Every standard emoji in the group named by `label` (one of [`GROUPS`]),
    /// for the picker's category sections.
    pub fn by_group(label: &str) -> Vec<StdEmoji> {
        let Some(group) = group_from_label(label) else {
            return Vec::new();
        };
        group.emojis().filter_map(to_std).collect()
    }

    fn group_from_label(label: &str) -> Option<emojis::Group> {
        use emojis::Group::*;
        Some(match label {
            "Smileys & Emotion" => SmileysAndEmotion,
            "People & Body" => PeopleAndBody,
            "Animals & Nature" => AnimalsAndNature,
            "Food & Drink" => FoodAndDrink,
            "Travel & Places" => TravelAndPlaces,
            "Activities" => Activities,
            "Objects" => Objects,
            "Symbols" => Symbols,
            "Flags" => Flags,
            _ => return None,
        })
    }
}

// ssr never compiles the `emojis` crate (it's gated to the `hydrate` feature),
// so these inert stubs keep the shared call sites type-checking on the server.
#[cfg(not(feature = "hydrate"))]
mod stub {
    use super::StdEmoji;

    pub fn glyph(_shortcode: &str) -> Option<&'static str> {
        None
    }
    pub fn search(_query: &str, _limit: usize) -> Vec<StdEmoji> {
        Vec::new()
    }
    pub fn by_group(_label: &str) -> Vec<StdEmoji> {
        Vec::new()
    }
}
