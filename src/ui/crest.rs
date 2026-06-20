//! M7/P3 Crest view: renders a deterministic heraldic [`Blazon`] (computed in
//! the always-on `markup::crest` algebra) as an inline SVG escutcheon. Pure
//! Leptos view — ssr + hydrate, zero web-sys — so it renders identically on the
//! server's first paint and after hydration (the blazon is a pure function of
//! the persona's name + debut, so there is nothing to hydrate).
//!
//! Render strategy (the 100+-card wardrobe grid): the blazon is derived ONCE at
//! construction and the SVG built as a single `inner_html` String — NO
//! per-crest `RwSignal`/`Memo`, so a grid of crests allocates no reactive graph
//! nodes. Tinctures are emitted as `fill="var(--tint-NAME)"` (the same token
//! source as guild accents), so a theme/accent change reflows the crest via CSS
//! without re-running the component.
//!
//! The escutcheon is a rounded square (the `.crest` CSS rounds + clips it); a
//! pointed shield silhouette is a deck-pass refinement — it would need a
//! per-crest `clipPath` (id collides when the same persona's crest shows in both
//! the wardrobe card and its open detail modal), so it is deferred (see the
//! M7/P3 plan's open decisions).

use leptos::prelude::*;

use crate::markup::crest::{Blazon, Division, Ordinary};

/// Inline-SVG heraldic crest for a persona, derived deterministically from
/// `name` (+ the optional `debut` date string that diverges like-named
/// personas). `class` is merged after the base `crest` class so call sites can
/// size it (`<Crest name=… class="card-portrait-crest"/>`).
#[component]
pub fn Crest(
    name: String,
    #[prop(optional, into)] debut: String,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let blazon = Blazon::derive(&name, &debut);
    let inner = render_blazon(&blazon);
    let label = format!("{name} crest");
    view! {
        <svg class=format!("crest {class}") viewBox="0 0 64 64"
            preserveAspectRatio="xMidYMid slice"
            role="img" aria-label=label inner_html=inner></svg>
    }
}

/// Build the crest's inner SVG markup from a resolved blazon. Pure string
/// assembly — no DOM — so it runs identically on ssr and hydrate.
fn render_blazon(b: &Blazon) -> String {
    let field = b.field.name();
    let contrast = b.contrast.name();
    // 1) The field fills the whole tile.
    let mut svg = format!(r#"<rect width="64" height="64" fill="var(--tint-{field})"/>"#);
    // 2) The division paints part of the tile in the contrast tincture.
    svg.push_str(&division_svg(b.division, contrast));
    // 3) The ordinary is an ink charge laid over both tinctures (always legible).
    svg.push_str(ordinary_svg(b.ordinary));
    // 4) The initial, centred.
    svg.push_str(&format!(
        r#"<text x="32" y="33" text-anchor="middle" dominant-baseline="central" font-size="30" font-weight="700" fill="var(--crest-ink)" fill-opacity="0.85">{}</text>"#,
        escape_text(b.initial)
    ));
    svg
}

/// The contrast-tincture region for each field division (all within the 64×64
/// box, so nothing overflows the CSS-rounded tile).
fn division_svg(d: Division, contrast: &str) -> String {
    let fill = format!(r#"fill="var(--tint-{contrast})""#);
    match d {
        Division::Plain => String::new(),
        Division::PerPale => format!(r#"<rect x="32" width="32" height="64" {fill}/>"#),
        Division::PerFess => format!(r#"<rect y="32" width="64" height="32" {fill}/>"#),
        Division::PerBend => format!(r#"<polygon points="0,0 64,0 64,64" {fill}/>"#),
        Division::Quarterly => format!(
            r#"<rect x="32" width="32" height="32" {fill}/><rect y="32" width="32" height="32" {fill}/>"#
        ),
    }
}

/// The ordinary as ink stroke(s) — a fixed ink color (not a tincture) so the
/// charge stays legible whichever tincture it crosses.
fn ordinary_svg(o: Ordinary) -> &'static str {
    match o {
        Ordinary::None => "",
        Ordinary::Chief => {
            r#"<line x1="0" y1="10" x2="64" y2="10" stroke="var(--crest-ink)" stroke-opacity="0.34" stroke-width="12"/>"#
        }
        Ordinary::Fess => {
            r#"<line x1="0" y1="32" x2="64" y2="32" stroke="var(--crest-ink)" stroke-opacity="0.34" stroke-width="12"/>"#
        }
        Ordinary::Pale => {
            r#"<line x1="32" y1="0" x2="32" y2="64" stroke="var(--crest-ink)" stroke-opacity="0.34" stroke-width="12"/>"#
        }
        Ordinary::Bend => {
            r#"<line x1="6" y1="58" x2="58" y2="6" stroke="var(--crest-ink)" stroke-opacity="0.34" stroke-width="12"/>"#
        }
        Ordinary::Cross => {
            r#"<line x1="32" y1="0" x2="32" y2="64" stroke="var(--crest-ink)" stroke-opacity="0.3" stroke-width="9"/><line x1="0" y1="32" x2="64" y2="32" stroke="var(--crest-ink)" stroke-opacity="0.3" stroke-width="9"/>"#
        }
        Ordinary::Saltire => {
            r#"<line x1="8" y1="8" x2="56" y2="56" stroke="var(--crest-ink)" stroke-opacity="0.3" stroke-width="9"/><line x1="56" y1="8" x2="8" y2="56" stroke="var(--crest-ink)" stroke-opacity="0.3" stroke-width="9"/>"#
        }
    }
}

/// Escape a single char for embedding as raw SVG `<text>` content (the rest of
/// the markup is fixed-palette, so only the persona-derived initial needs it).
fn escape_text(c: char) -> String {
    match c {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        other => other.to_string(),
    }
}
