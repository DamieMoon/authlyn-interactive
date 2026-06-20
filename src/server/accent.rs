//! M5/P2 per-server accent palette (Open Question #5). The accent is a name
//! from the same 8-color markup palette as `persona.color` / the `--tint-*`
//! CSS tokens (`style/_tokens.scss`). Validated here server-side (the schema
//! field carries no ASSERT, mirroring persona.color); the empty string clears
//! the accent back to the default. Always-on module — imports zero ssr/hydrate
//! crates so the same names are reachable from any graph if ever needed.

/// The 8 valid accent names — the markup palette / `--tint-*` vocabulary.
pub const ACCENT_PALETTE: [&str; 8] = [
    "red", "orange", "yellow", "green", "blue", "purple", "pink", "gray",
];

/// Normalize a client-sent accent into the stored form, or reject it.
/// - trims + lowercases first;
/// - empty (after trim) ⇒ `Some(String::new())` (clears the accent);
/// - a palette name ⇒ `Some(name)`;
/// - anything else ⇒ `None` (caller returns 400).
pub fn normalize_accent(raw: &str) -> Option<String> {
    let v = raw.trim().to_lowercase();
    if v.is_empty() {
        return Some(String::new());
    }
    if ACCENT_PALETTE.contains(&v.as_str()) {
        Some(v)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_names_normalize_to_themselves() {
        for name in ACCENT_PALETTE {
            assert_eq!(normalize_accent(name).as_deref(), Some(name));
        }
    }

    #[test]
    fn empty_clears_the_accent() {
        assert_eq!(normalize_accent("").as_deref(), Some(""));
        assert_eq!(normalize_accent("   ").as_deref(), Some(""));
    }

    #[test]
    fn case_and_whitespace_are_normalized() {
        assert_eq!(normalize_accent("  PURPLE ").as_deref(), Some("purple"));
        assert_eq!(normalize_accent("Red").as_deref(), Some("red"));
    }

    #[test]
    fn out_of_palette_is_rejected() {
        assert_eq!(normalize_accent("chartreuse"), None);
        assert_eq!(normalize_accent("#ff00ff"), None);
        assert_eq!(normalize_accent("blue; DROP TABLE"), None);
    }
}
