//! Inline-SVG icon components (W2 design system). Stroke-based, 24×24
//! viewBox, `currentColor` — they inherit text color and scale with
//! font-size via `width/height: 1em` units in CSS (size with font-size or
//! an explicit class). Always-on module: pure view code, zero ssr crates.
//! W3/W4 replace the legacy text glyphs (↑ ↓ ⤒ ⤓ ✕ ✓ 🗑) with these.

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
icon!(/// Settings / preferences.
    IconSettings, r#"<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M19.1 4.9L17 7M7 17l-2.1 2.1"/>"#);
icon!(/// Chat tab (W3 mobile nav).
    IconChat, r#"<path d="M21 12a8 8 0 0 1-8 8H5l-2 2V12a8 8 0 0 1 8-8h2a8 8 0 0 1 8 8z"/>"#);
icon!(/// Servers tab (W3 mobile nav).
    IconServers, r#"<rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/>"#);
icon!(/// Friends tab (W3 mobile nav).
    IconFriends, r#"<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0M16 4.6a3.5 3.5 0 0 1 0 6.8M21.5 20a6.5 6.5 0 0 0-4.5-6.2"/>"#);
icon!(/// Personas tab (W3 mobile nav).
    IconPersonas, r#"<path d="M12 3l2.5 5 5.5.8-4 3.9.9 5.5-4.9-2.6-4.9 2.6.9-5.5-4-3.9L9.5 8z"/>"#);
icon!(/// Notifications / bell.
    IconBell, r#"<path d="M18 9a6 6 0 1 0-12 0c0 6-2 7-2 7h16s-2-1-2-7M10.5 20a1.7 1.7 0 0 0 3 0"/>"#);

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
