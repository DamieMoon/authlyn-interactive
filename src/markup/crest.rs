//! M7/P3 Crests (Vapenskölden, design spec §9.7): deterministic heraldic
//! blazonry derived purely from a persona's name + debut date. This is the
//! always-on, dependency-free ALGEBRA half — pure math, no leptos/web-sys/DOM —
//! so it compiles to every graph (ssr, hydrate/wasm32, nova) like the rest of
//! `markup/`. The SVG VIEW that renders a [`Blazon`] lives in `ui/crest.rs`.
//!
//! A crest's identity is a 64-bit FNV-1a hash of the lowercased name (+ the
//! debut string), sliced into disjoint bit-fields that pick the field tincture,
//! a contrasting tincture, a field division, an ordinary (charge band), and the
//! initial letter. The reachable blazon space is
//! 8 fields × 7 contrasts × 5 divisions × 7 ordinaries ≈ 1 960 layouts, times
//! ~26 common initials ≈ 50 000 visually-distinct crests — so visible heraldic
//! repeats appear (by the birthday bound) around ~226 personas. That is a
//! property of heraldry, not a hash collision: the 64-bit hash itself only
//! birthday-collides near ~5×10⁹ inputs, far past any persona population.
//!
//! Tinctures reuse the existing 8-name markup [`Color`] palette (→ the
//! `--tint-*` CSS tokens) — there is exactly one palette source in the codebase.

use super::Color;

/// How the shield FIELD is divided between the two tinctures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Division {
    /// Solid field, no division.
    Plain,
    /// Split vertically (party per pale).
    PerPale,
    /// Split horizontally (party per fess).
    PerFess,
    /// Split on the diagonal (party per bend).
    PerBend,
    /// Four quarters.
    Quarterly,
}

const DIVISIONS: [Division; 5] = [
    Division::Plain,
    Division::PerPale,
    Division::PerFess,
    Division::PerBend,
    Division::Quarterly,
];

/// The ORDINARY — a bold geometric charge laid over the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ordinary {
    /// No ordinary (field + initial only).
    None,
    /// A band across the top (chief).
    Chief,
    /// A horizontal band across the middle (fess).
    Fess,
    /// A vertical band down the middle (pale).
    Pale,
    /// A diagonal band (bend).
    Bend,
    /// An upright cross.
    Cross,
    /// A diagonal cross (saltire).
    Saltire,
}

const ORDINARIES: [Ordinary; 7] = [
    Ordinary::None,
    Ordinary::Chief,
    Ordinary::Fess,
    Ordinary::Pale,
    Ordinary::Bend,
    Ordinary::Cross,
    Ordinary::Saltire,
];

/// A fully-resolved heraldic blazon: everything the SVG view needs to draw a
/// crest, all derived deterministically from `(name, debut)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blazon {
    /// The base field tincture.
    pub field: Color,
    /// The contrasting tincture (always `!= field`) for the division + ordinary.
    pub contrast: Color,
    /// How the field is split.
    pub division: Division,
    /// The ordinary charged over the field.
    pub ordinary: Ordinary,
    /// The upper-case initial centred on the shield.
    pub initial: char,
}

impl Blazon {
    /// Derive the crest for a persona `name` (+ its `debut` date string, which
    /// diverges two like-named personas). Pure and deterministic.
    pub fn derive(name: &str, debut: &str) -> Blazon {
        let h = name_hash(name, debut);

        let field_idx = (h & 0x7) as usize;
        let field = Color::ALL[field_idx];

        // A second tincture, re-rolled to differ from the field so the division
        // and ordinary are always legible against it.
        let mut contrast_idx = ((h >> 8) & 0x7) as usize;
        if contrast_idx == field_idx {
            contrast_idx = (contrast_idx + 1) % Color::ALL.len();
        }
        let contrast = Color::ALL[contrast_idx];

        let division = DIVISIONS[((h >> 16) & 0x7) as usize % DIVISIONS.len()];
        let ordinary = ORDINARIES[((h >> 24) & 0xF) as usize % ORDINARIES.len()];

        Blazon {
            field,
            contrast,
            division,
            ordinary,
            initial: initial(name),
        }
    }
}

/// 64-bit FNV-1a over the lowercased/trimmed `name` followed by the `debut`
/// string. Hand-rolled (no dep, and deliberately NOT `DefaultHasher` /
/// `RandomState` — those seed randomly per process, which would make the crest
/// differ across reloads and between the ssr render and the hydrate render).
fn name_hash(name: &str, debut: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for &b in name.trim().to_lowercase().as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    // Unit separator so ("ab", "") and ("a", "b") can't hash alike.
    h ^= 0x1f;
    h = h.wrapping_mul(PRIME);
    for &b in debut.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Upper-case first character of `name` (the shield's charge), `'?'` when empty.
/// Returns a single `char` (a blazon carries exactly one initial), so a
/// multi-`char` uppercase expansion keeps only its first — e.g. German `ß` → `S`
/// where `ui::avatar::monogram` (which returns a `String`) would yield `SS`.
/// Duplicated rather than imported from `ui/`: that tree pulls leptos and isn't
/// compiled for the nova graph, while this module is always-on.
fn initial(name: &str) -> char {
    name.trim()
        .chars()
        .next()
        .and_then(|c| c.to_uppercase().next())
        .unwrap_or('?')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic() {
        let a = Blazon::derive("Mendicant Bias", "2026-06-20T00:00:00Z");
        let b = Blazon::derive("Mendicant Bias", "2026-06-20T00:00:00Z");
        assert_eq!(a, b, "same (name, debut) must yield the same blazon");
    }

    #[test]
    fn contrast_always_differs_from_field() {
        // Sweep a thousand synthetic names; the contrast tincture must never
        // equal the field, or the division/ordinary would be invisible.
        for i in 0..1000u32 {
            let name = format!("persona-{i}");
            let bl = Blazon::derive(&name, "");
            assert_ne!(bl.field, bl.contrast, "field == contrast for {name}");
        }
    }

    #[test]
    fn debut_diverges_like_named_personas() {
        // Two same-named personas with different birthdays diverge in the
        // blazon (the debut folds into the hash). Not guaranteed for EVERY
        // date pair, but these two concrete dates do.
        let early = Blazon::derive("Nova", "2025-01-01T00:00:00Z");
        let late = Blazon::derive("Nova", "2026-12-31T00:00:00Z");
        assert_ne!(early, late);
    }

    #[test]
    fn name_is_case_and_whitespace_insensitive() {
        // The crest is the persona's identity, not its exact casing/spacing.
        assert_eq!(
            Blazon::derive("Alice", "d"),
            Blazon::derive("  alice  ", "d")
        );
    }

    #[test]
    fn initial_is_uppercase_first_char_with_fallback() {
        assert_eq!(initial("alice"), 'A');
        assert_eq!(initial("  bob"), 'B');
        assert_eq!(initial(""), '?');
        assert_eq!(initial("   "), '?');
    }

    #[test]
    fn distribution_touches_many_blazons_without_panic() {
        // No index ever panics, and the space is actually spread (not all
        // names collapsing onto a handful of crests). Dedup via the Debug
        // string so the test needs no Hash on the foreign `Color`.
        let mut seen = std::collections::HashSet::new();
        for i in 0..500u32 {
            let bl = Blazon::derive(&format!("name{i}"), "");
            seen.insert(format!("{bl:?}"));
        }
        assert!(
            seen.len() > 200,
            "only {} distinct crests for 500 names — distribution is too narrow",
            seen.len()
        );
    }
}
