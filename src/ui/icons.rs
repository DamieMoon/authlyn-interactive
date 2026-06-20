//! Inline-SVG icon components (M2 design system). Stroke-based, 24×24
//! viewBox, `currentColor` — they inherit text color and scale with
//! font-size via `width/height: 1em` units in CSS (size with font-size or
//! an explicit class). Always-on module: pure view code, zero ssr crates.
//! M3/M4 replace the legacy text glyphs (↑ ↓ ⤒ ⤓ ✕ ✓ 🗑) with these.

use leptos::prelude::*;

macro_rules! icon {
    ($(#[$doc:meta])* $name:ident, $paths:expr) => {
        $(#[$doc])*
        #[component]
        pub fn $name(
            /// Extra CSS classes merged after the base `icon` class, so call
            /// sites can size/position an icon (`<IconX class="topbar-icon"/>`).
            /// Defaults to empty — `<IconX/>` stays valid.
            #[prop(optional, into)]
            class: String,
        ) -> impl IntoView {
            view! {
                <svg class=format!("icon {class}") viewBox="0 0 24 24" fill="none"
                    stroke="currentColor" stroke-width="1.8"
                    stroke-linecap="round" stroke-linejoin="round"
                    aria-hidden="true" inner_html=$paths></svg>
            }
        }
    };
}

icon!(/// Close / dismiss (replaces "✕").
    IconClose, r#"<path d="M6 6l12 12M18 6L6 18"/>"#);
icon!(/// Confirm (replaces "✓").
    IconCheck, r#"<path d="M5 13l4 4L19 7"/>"#);
icon!(/// Delete / trash (replaces "🗑").
    IconTrash, r#"<path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m-9 0l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13M10 11v6M14 11v6"/>"#);
icon!(/// Send message.
    IconSend, r#"<path d="M12 19V5M5 12l7-7 7 7"/>"#);
icon!(/// Add / create.
    IconPlus, r#"<path d="M12 5v14M5 12h14"/>"#);
icon!(/// Edit / rename.
    IconEdit, r#"<path d="M4 20l4-1L20 7a2 2 0 0 0-3-3L5 16l-1 4z"/>"#);
icon!(/// Reply to a message.
    IconReply, r#"<path d="M9 14L4 9l5-5M4 9h10a6 6 0 0 1 6 6v4"/>"#);
icon!(/// Copy to clipboard.
    IconCopy, r#"<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/>"#);
icon!(/// Reorder: up one step (replaces "↑").
    IconUp, r#"<path d="M12 19V5M6 11l6-6 6 6"/>"#);
icon!(/// Reorder: down one step (replaces "↓").
    IconDown, r#"<path d="M12 5v14M6 13l6 6 6-6"/>"#);
icon!(/// Reorder: to top (replaces "⤒").
    IconToTop, r#"<path d="M5 5h14M12 19V9M7 13l5-5 5 5"/>"#);
icon!(/// Reorder: to bottom (replaces "⤓").
    IconToBottom, r#"<path d="M5 19h14M12 5v10M7 11l5 5 5-5"/>"#);
icon!(/// Settings / preferences — a toothed cog. (The earlier radiating-line
    /// version read as a SUN at ~12px; owner ruling 2026-06-17 — redraw as a
    /// proper gear: a lobed/toothed outline around a center hole.)
    IconSettings, r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#);
icon!(/// Chat tab (M3 mobile nav).
    IconChat, r#"<path d="M21 12a8 8 0 0 1-8 8H5l-2 2V12a8 8 0 0 1 8-8h2a8 8 0 0 1 8 8z"/>"#);
icon!(/// Servers tab (M3 mobile nav).
    IconServers, r#"<rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/>"#);
icon!(/// Friends tab (M3 mobile nav).
    IconFriends, r#"<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0M16 4.6a3.5 3.5 0 0 1 0 6.8M21.5 20a6.5 6.5 0 0 0-4.5-6.2"/>"#);
icon!(/// Personas tab (M3 mobile nav).
    IconPersonas, r#"<path d="M12 3l2.5 5 5.5.8-4 3.9.9 5.5-4.9-2.6-4.9 2.6.9-5.5-4-3.9L9.5 8z"/>"#);
icon!(/// Notifications / bell.
    IconBell, r#"<path d="M18 9a6 6 0 1 0-12 0c0 6-2 7-2 7h16s-2-1-2-7M10.5 20a1.7 1.7 0 0 0 3 0"/>"#);

// M6 affordance migration (owner directive 2026-06-17 — "Everything, incl.
// dingbats"): every emoji/glyph UI control becomes an inline-SVG icon. The
// set below replaces the inline composer/orbit/menu glyphs that the M2 system
// never reached. Genuine emoji CONTENT (the picker grid, custom-emoji manager,
// emoji typed into messages) is NOT an affordance and stays untouched.
icon!(/// Attach a file (replaces "📎") — paperclip.
    IconAttach, r#"<path d="M21 11l-9 9a5 5 0 0 1-7-7l9-9a3.5 3.5 0 0 1 5 5l-9 9a2 2 0 0 1-3-3l8-8"/>"#);
icon!(/// Emoji picker trigger (replaces "😀") — smiley outline. NB: the affordance,
    /// not emoji content; the picker grid stays a real emoji surface.
    IconEmoji, r#"<circle cx="12" cy="12" r="9"/><path d="M8.5 14a4 4 0 0 0 7 0"/><path d="M9 9.5h.01M15 9.5h.01"/>"#);
icon!(/// Draft-preview toggle (replaces "👁") — eye.
    IconEye, r#"<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>"#);
icon!(/// Disclosure chevron (replaces "▼") — color-swatch / popover toggles.
    IconChevronDown, r#"<path d="M6 9l6 6 6-6"/>"#);
icon!(/// Roll a die (replaces "🎲") — die face with pips.
    IconDie, r#"<rect x="4" y="4" width="16" height="16" rx="3"/><path d="M8.5 8.5h.01M15.5 8.5h.01M12 12h.01M8.5 15.5h.01M15.5 15.5h.01"/>"#);
icon!(/// Whisper effect (replaces "🌫"/"🤫") — a brief, quiet speech bubble.
    IconWhisper, r#"<path d="M21 11.5a8.4 8.4 0 0 1-11.7 7.7L3 21l1.8-6.3A8.4 8.4 0 1 1 21 11.5z"/><path d="M8.5 11.5h7"/>"#);
icon!(/// Shout effect (replaces "📣") — megaphone with sound waves.
    IconShout, r#"<path d="M3 11v2a1 1 0 0 0 1 1h2l4 4V6L6 10H4a1 1 0 0 0-1 1z"/><path d="M14.5 9a4 4 0 0 1 0 6M17 7a7 7 0 0 1 0 10"/>"#);
icon!(/// Spell effect (replaces "✨") — sparkles.
    IconSpell, r#"<path d="M12 3l1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8z"/><path d="M18.5 15l.6 1.6 1.6.6-1.6.6-.6 1.6-.6-1.6-1.6-.6 1.6-.6z"/>"#);
icon!(/// Four-point brand star (replaces "✦") — orbit nucleus / menu bullet.
    IconStar, r#"<path d="M12 2l2.2 7.8L22 12l-7.8 2.2L12 22l-2.2-7.8L2 12l7.8-2.2z"/>"#);
icon!(/// Drag grip (replaces "⠿") — six dots, the finger-reorder handle.
    IconGrip, r#"<path d="M9 6h.01M9 12h.01M9 18h.01M15 6h.01M15 12h.01M15 18h.01"/>"#);
icon!(/// Empty / off state (replaces "○" and the "◌" no-effect mode) — outline circle.
    IconCircle, r#"<circle cx="12" cy="12" r="7"/>"#);
icon!(/// Members roster (replaces "👥") — two side-by-side people; distinct from
    /// IconFriends so the station's Friends / Members entries read apart.
    IconMembers, r#"<circle cx="8" cy="9.5" r="2.5"/><circle cx="16" cy="9.5" r="2.5"/><path d="M3.5 18a4.5 4.5 0 0 1 9 0M11.5 18a4.5 4.5 0 0 1 9 0"/>"#);
icon!(/// Swipe sideways (replaces "↔") — orbit help legend.
    IconSwipe, r#"<path d="M3 12h18M7 8l-4 4 4 4M17 8l4 4-4 4"/>"#);
icon!(/// The compose orb (replaces "◉") — orbit help legend; a ringed core.
    IconOrb, r#"<circle cx="12" cy="12" r="8"/><circle cx="12" cy="12" r="3" fill="currentColor"/>"#);
icon!(/// Press-and-hold gesture (replaces "⏺") — orbit help legend.
    IconHold, r#"<circle cx="12" cy="12" r="3.5" fill="currentColor"/><path d="M12 4v2.5M12 17.5V20M4 12h2.5M17.5 12H20"/>"#);
icon!(/// Previous (replaces "‹") — lightbox gallery nav.
    IconChevronLeft, r#"<path d="M15 6l-6 6 6 6"/>"#);
icon!(/// Next (replaces "›") — lightbox gallery nav.
    IconChevronRight, r#"<path d="M9 6l6 6-6 6"/>"#);
icon!(/// Zoom out (replaces "−") — lightbox.
    IconMinus, r#"<path d="M5 12h14"/>"#);
icon!(/// Reset / fit (replaces "⤢") — lightbox zoom reset; corner brackets.
    IconZoomReset, r#"<path d="M9 4H4v5M15 4h5v5M15 20h5v-5M9 20H4v-5"/>"#);
icon!(/// Back (replaces "←") — left arrow, e.g. the orbit station close.
    IconBack, r#"<path d="M19 12H5M11 6l-6 6 6 6"/>"#);
icon!(/// File / document download tile (replaces "📄").
    IconFile, r#"<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>"#);
icon!(/// Retry / refresh (replaces "↻") — failed-upload re-try.
    IconRefresh, r#"<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v5h-5"/>"#);
icon!(/// Lorebook channel sigil (replaces "📖") — open book.
    IconBook, r#"<path d="M12 6.5v13"/><path d="M3 5.5a9 9 0 0 1 9 1 9 9 0 0 1 9-1V17a9 9 0 0 0-9 1 9 9 0 0 0-9-1z"/>"#);
icon!(/// Ghost Quill author marker (replaces "✒️") — a feather quill.
    IconQuill, r#"<path d="M20 4C10.5 6 6.5 12 4.5 20l3-1c1.2-6 5-10 12.5-12z"/><path d="M14 7l-7 7"/>"#);
icon!(/// Filled disc (replaces "●") — a colour swatch dot; fills in currentColor
    /// so a `.mk-*` swatch shows its hue. The empty/none swatch uses IconCircle.
    IconDisc, r#"<circle cx="12" cy="12" r="7" fill="currentColor" stroke="none"/>"#);

/// Nova DOT system avatar — the Superintendent-inspired civic-AI orb (spec
/// §3:98, M6/P3). Unlike the stroke icons above this is the bundled BRAND
/// asset: the art lives in `public/nova-dot.svg` (the owner is the visual
/// oracle — retune it there, no Rust edit) and is `include_str!`'d so SSR and
/// hydrate emit byte-identical markup. The SVG keeps the static body and the
/// `.nova-orb-ring` as DISTINCT elements so CSS (`fx-nova-ring`, _motion.scss)
/// spins the ring without touching the body. Decorative → the wrapper is
/// `aria-hidden`; the adjacent author name ("Nova DOT") is the accessible
/// label. Always-on view code: zero ssr/hydrate-only crates, no media upload —
/// it is a brand asset, not user content.
#[component]
pub fn NovaOrb(
    /// Extra CSS classes merged after the base `nova-orb` class, mirroring the
    /// `icon!` convention so call sites can size/position the orb. Defaults to
    /// empty — `<NovaOrb/>` stays valid.
    #[prop(optional, into)]
    class: String,
) -> impl IntoView {
    view! {
        <span class=format!("nova-orb {class}") aria-hidden="true"
            inner_html=include_str!("../../public/nova-dot.svg")></span>
    }
}
