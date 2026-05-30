//! Grimoire design tokens transcribed from `style/_tokens.scss` into Rust.
//!
//! Freya has no CSS; the read UI is styled per-element with these constants
//! (`.background(..)`/`.color(..)` accept an `(u8,u8,u8)` tuple). Colors are RGB
//! tuples so they're `const`; keep the names aligned with the SCSS `--tokens`.

/// An RGB color as Freya's element builders accept it (`impl Into<Color>`).
pub type Rgb = (u8, u8, u8);

// Surfaces / backgrounds
pub const PARCHMENT: Rgb = (0x22, 0x1c, 0x16); // page surface
pub const PARCHMENT_DEEP: Rgb = (0x1a, 0x16, 0x12); // darkest base / rail bg
pub const VELLUM: Rgb = (0x2b, 0x23, 0x1a); // raised panels (sidebar/modal)
pub const VELLUM_2: Rgb = (0x35, 0x2a, 0x1f); // hover/active; the message card bg
pub const INPUT_BG: Rgb = (0x1d, 0x18, 0x14);
pub const ATTACHMENT_BG: Rgb = (0x1e, 0x18, 0x12);
pub const AVATAR_TILE: Rgb = (0x3a, 0x2f, 0x20); // monogram fallback frame

// Borders
pub const RULE_LINE: Rgb = (0x3d, 0x30, 0x25);

// Text / ink
pub const INK: Rgb = (0xef, 0xe6, 0xd3); // primary text
pub const INK_SOFT: Rgb = (0xd6, 0xc7, 0xa4); // secondary / author name
pub const INK_MUTED: Rgb = (0x8a, 0x7d, 0x63); // tertiary / timestamps

// Accent (gold)
pub const GOLD: Rgb = (0xc8, 0x9b, 0x3c);
pub const GOLD_WARM: Rgb = (0xe6, 0xb3, 0x5c);

// Danger (terracotta)
pub const INK_DANGER: Rgb = (0xd5, 0x7a, 0x6b);

// Persona / markup tint palette (readable on VELLUM_2), index-aligned to markup::Color
pub const TINT_RED: Rgb = (0xc8, 0x7a, 0x6b);
pub const TINT_ORANGE: Rgb = (0xc8, 0x85, 0x6b);
pub const TINT_YELLOW: Rgb = (0xc8, 0xa3, 0x56);
pub const TINT_GREEN: Rgb = (0x7e, 0xa0, 0x71);
pub const TINT_BLUE: Rgb = (0x6b, 0x8e, 0xc8);
pub const TINT_PURPLE: Rgb = (0xa0, 0x7e, 0xc8);
pub const TINT_PINK: Rgb = (0xc8, 0x76, 0xa3);
pub const TINT_GRAY: Rgb = (0xa8, 0x9d, 0x83);

// Layout dimensions (px; SCSS rem→px at 16px root)
pub const RAIL_W: f32 = 72.0;
pub const SIDEBAR_W: f32 = 240.0; // clamp(180,22vw,260) → fixed mid for native
pub const GUILD_TILE: f32 = 46.0;
pub const AVATAR: f32 = 38.0;
pub const RADIUS: f32 = 6.0; // 0.375rem default
pub const RADIUS_SM: f32 = 4.0;

// Font sizes
pub const FS_BODY: f32 = 16.0;
pub const FS_META: f32 = 13.0; // ~0.8rem timestamps/author meta
pub const FS_H1: f32 = 24.0;
pub const FS_H2: f32 = 20.0;
pub const FS_H3: f32 = 18.0;
pub const FS_SUBTEXT: f32 = 13.0;

/// Map a parsed markup color to its Grimoire tint.
pub fn tint(color: crate::markup::Color) -> Rgb {
    use crate::markup::Color::*;
    match color {
        Red => TINT_RED,
        Orange => TINT_ORANGE,
        Yellow => TINT_YELLOW,
        Green => TINT_GREEN,
        Blue => TINT_BLUE,
        Purple => TINT_PURPLE,
        Pink => TINT_PINK,
        Gray => TINT_GRAY,
    }
}
