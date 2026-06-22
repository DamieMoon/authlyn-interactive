//! M5/P2 per-server accent → CSS token mapper (Open Question #5). Maps a
//! markup-palette accent name (red…gray, the `server/accent.rs` vocabulary) to
//! the CSS custom-property values bound on the `.app` root, so the warp-jump
//! streak (#A) and accent family (#G) render the guild's color. An empty or
//! unknown name returns `String::new()` → the inline `style:` binding sets
//! nothing and the `style/_tokens.scss` defaults (--glow-accent / --accent)
//! win. Always-on (used by the shell view); imports zero ssr/hydrate crates.

/// `var(--tint-NAME)` solid for the accent name, or empty for default/unknown.
/// Reuses the existing `--tint-*` tokens so there is one palette source.
pub fn accent_var_css(name: &str) -> String {
    if is_palette(name) {
        format!("var(--tint-{name})")
    } else {
        String::new()
    }
}

/// The `--glow-accent` rgba (alpha 0.55, matching `_tokens.scss:67`) for the
/// accent name, or empty for default/unknown. Hardcoded rgba mirrors the
/// `--tint-*` hexes so the glow tints with the same color the solid uses.
pub fn accent_glow_css(name: &str) -> String {
    let rgb = match name {
        "red" => "255, 138, 150",
        "orange" => "255, 180, 127",
        "yellow" => "255, 212, 127",
        "green" => "142, 230, 200",
        "blue" => "127, 182, 255",
        "purple" => "196, 168, 255",
        "pink" => "255, 154, 213",
        "gray" => "154, 167, 189",
        _ => return String::new(),
    };
    format!("rgba({rgb}, 0.55)")
}

fn is_palette(name: &str) -> bool {
    matches!(
        name,
        "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "pink" | "gray"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_map_to_tint_var_and_rgba() {
        assert_eq!(accent_var_css("purple"), "var(--tint-purple)");
        assert_eq!(accent_glow_css("purple"), "rgba(196, 168, 255, 0.55)");
        assert_eq!(accent_var_css("green"), "var(--tint-green)");
        assert_eq!(accent_glow_css("green"), "rgba(142, 230, 200, 0.55)");
    }

    #[test]
    fn empty_and_unknown_return_blank_so_token_default_wins() {
        assert_eq!(accent_var_css(""), "");
        assert_eq!(accent_glow_css(""), "");
        assert_eq!(accent_var_css("chartreuse"), "");
        assert_eq!(accent_glow_css("chartreuse"), "");
    }
}
