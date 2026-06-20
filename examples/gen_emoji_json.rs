//! One-shot generator for `public/emoji.json` — the JSON dataset the hydrate
//! emoji picker / `:shortcode:` resolver fetches at runtime (M7/D1). Run by
//! hand whenever the upstream `emojis` crate ships a new unicode revision:
//!
//! ```text
//! cargo run --example gen_emoji_json
//! ```
//!
//! `emojis` is a `[dev-dependencies]` entry, so this is the **only** place it
//! enters the dep graph. The wasm hydrate build never compiles `emojis`; it
//! reads the static JSON asset over the wire instead, which trims ~79 KB of
//! brotli-compressed wasm out of the bundle.
//!
//! Output shape:
//! ```json
//! {
//!   "groups": ["Smileys & Emotion", "People & Body", …],
//!   "items":  [
//!     { "glyph": "😄", "shortcode": "smile",
//!       "group": "Smileys & Emotion", "name": "grinning face with smiling eyes" },
//!     …
//!   ]
//! }
//! ```
//! Only emoji that have at least one github-style shortcode are emitted (the
//! picker / autocomplete is keyed on shortcode; entries without one are not
//! addressable from the UI). Iteration order is `emojis`'s built-in stable
//! sort, so the file diff stays minimal between runs.

use std::io::Write;

const GROUPS: &[(&str, emojis::Group)] = &[
    ("Smileys & Emotion", emojis::Group::SmileysAndEmotion),
    ("People & Body", emojis::Group::PeopleAndBody),
    ("Animals & Nature", emojis::Group::AnimalsAndNature),
    ("Food & Drink", emojis::Group::FoodAndDrink),
    ("Travel & Places", emojis::Group::TravelAndPlaces),
    ("Activities", emojis::Group::Activities),
    ("Objects", emojis::Group::Objects),
    ("Symbols", emojis::Group::Symbols),
    ("Flags", emojis::Group::Flags),
];

fn main() -> std::io::Result<()> {
    let group_label = |g: emojis::Group| -> &'static str {
        GROUPS
            .iter()
            .find_map(|(label, gg)| (*gg == g).then_some(*label))
            .unwrap_or("")
    };

    // Manual JSON emission keeps the output stable (no derived serde key
    // ordering surprises) and avoids pulling serde_json into dev-deps for a
    // 50-line generator.
    let mut out = Vec::with_capacity(1 << 18);
    out.extend_from_slice(b"{\n  \"groups\": [");
    for (i, (label, _)) in GROUPS.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b", ");
        }
        write_json_string(&mut out, label)?;
    }
    out.extend_from_slice(b"],\n  \"items\": [\n");

    let mut first = true;
    for e in emojis::iter() {
        let Some(shortcode) = e.shortcode() else {
            continue;
        };
        if !first {
            out.extend_from_slice(b",\n");
        }
        first = false;
        out.extend_from_slice(b"    {\"glyph\":");
        write_json_string(&mut out, e.as_str())?;
        out.extend_from_slice(b",\"shortcode\":");
        write_json_string(&mut out, shortcode)?;
        out.extend_from_slice(b",\"group\":");
        write_json_string(&mut out, group_label(e.group()))?;
        out.extend_from_slice(b",\"name\":");
        write_json_string(&mut out, e.name())?;
        out.extend_from_slice(b"}");
    }
    out.extend_from_slice(b"\n  ]\n}\n");

    let path = std::env::var("EMOJI_JSON_OUT").unwrap_or_else(|_| "public/emoji.json".into());
    std::fs::write(&path, &out)?;
    let count = emojis::iter().filter(|e| e.shortcode().is_some()).count();
    eprintln!("wrote {} ({} entries, {} bytes)", path, count, out.len());
    Ok(())
}

/// Minimal JSON string encoder — handles the subset of escapes the emoji
/// corpus actually contains (mostly nothing; a handful of names with
/// backslash or quote would be wrong, and there are none).
fn write_json_string<W: Write>(w: &mut W, s: &str) -> std::io::Result<()> {
    w.write_all(b"\"")?;
    for c in s.chars() {
        match c {
            '\\' => w.write_all(b"\\\\")?,
            '"' => w.write_all(b"\\\"")?,
            '\n' => w.write_all(b"\\n")?,
            '\r' => w.write_all(b"\\r")?,
            '\t' => w.write_all(b"\\t")?,
            c if (c as u32) < 0x20 => {
                write!(w, "\\u{:04x}", c as u32)?;
            }
            c => {
                let mut buf = [0u8; 4];
                w.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    w.write_all(b"\"")?;
    Ok(())
}
