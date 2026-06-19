//! Static style-doctrine guards (pure file scan; no DB, runs in every feature
//! graph). Two rules:
//!
//! 1. **Motion doctrine (#43):** `@keyframes` blocks may animate ONLY
//!    transform/translate/rotate/scale/opacity. Paint-class properties
//!    (box-shadow, background-position, filter, width, height, top, left)
//!    inside a `@keyframes` body force per-frame repaint on the POCO C3 floor
//!    and are forbidden — every future fx- effect must be born composite-cheap.
//!    A narrow, documented `EXEMPT_KEYFRAMES` allowlist (by name) carves out the
//!    brief loading-placeholder shimmer loaders. (`fx-warp` was a TIME-BOXED
//!    exemption removed in W5/P0 Task 0.2 once its keyframe became transform-only.)
//!    See the const for the rationale.
//!
//! 2. **WebKit 1:1 (`backdrop-filter`) doctrine** — owner ruling 2026-06-15,
//!    "Real Liquid Glass as the default": Apple Liquid Glass (`backdrop-filter`) is
//!    now the orbit-chrome DEFAULT (the `glass-holo` mixin), NOT an fx-max-only
//!    escalation. This REVERSES the earlier "no `backdrop-filter` at Standard"
//!    doctrine for orbit chrome. The remaining guardrail is the one that bit us
//!    on iOS (CLAUDE.md F1, the WebKit 1:1 standing rule): **every standard
//!    `backdrop-filter` declaration MUST be paired with a `-webkit-backdrop-filter`
//!    sibling** so Safari/WebKit — the mobile-first PRIMARY target — never
//!    silently loses the glass. Enforced per-file by count-equality (comments
//!    stripped); `backdrop-filter` is NOT in the keyframes `FORBIDDEN` list and is
//!    deliberately permitted outside keyframes (it's the Liquid Glass default).
//!    box-shadow/filter inside `@keyframes` stay forbidden (rule 1) — the glow is
//!    static even though the glass now blurs.

use std::fs;
use std::path::Path;

/// Properties that trigger layout or paint when animated. `top:`/`left:`
/// carry the colon so a `transform: translate(...left...)` substring can't
/// false-positive; the others are property names that never appear as a
/// transform sub-token.
const FORBIDDEN: &[&str] = &[
    "box-shadow",
    "background-position",
    "filter:",
    "width:",
    "height:",
    "top:",
    "left:",
];

/// Keyframes EXEMPT from the doctrine, by NAME.
///
/// `shimmer` / `gallery-skeleton-shimmer` are the brief loading-placeholder
/// shimmer loaders: the textbook `background-position` sweep over a
/// `background-size: 200%` gradient. They're short-lived loading skeletons
/// (the plan's distinct "loading-placeholder skeleton" class — see #43 +
/// the `sk-` prefix rule), NOT perpetual decorative `fx-` effects on the
/// live interactive hot path, so they are intentionally carved out.
///
/// (`fx-warp` was a TIME-BOXED exemption — it animated `background-position`
/// until W5/P0 Task 0.2 (#54) rewrote it to a `transform: translateX` sweep.
/// Now composite-only, it is enforced by this guard like every other `fx-`.)
const EXEMPT_KEYFRAMES: &[&str] = &["shimmer", "gallery-skeleton-shimmer"];

/// Return every `@keyframes <name> { ... }` as `(name, body)`, brace-matched.
fn keyframes_blocks(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = src[search_from..].find("@keyframes") {
        let kw = search_from + rel;
        // Find the opening brace after the keyframes name.
        if let Some(open_rel) = src[kw..].find('{') {
            let open = kw + open_rel;
            // The name is the text between `@keyframes` and `{`.
            let name = src[kw + "@keyframes".len()..open].trim().to_string();
            let mut depth = 0i32;
            let mut i = open;
            let mut end = open;
            while i < bytes.len() {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            out.push((name, src[open..=end].to_string()));
            search_from = end + 1;
        } else {
            break;
        }
    }
    out
}

#[test]
fn no_keyframes_animate_paint_or_layout_properties() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("style");
    let mut violations = Vec::new();
    for entry in fs::read_dir(&dir).expect("style/ dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("scss") {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read scss");
        for (name, body) in keyframes_blocks(&src) {
            if EXEMPT_KEYFRAMES.contains(&name.as_str()) {
                continue;
            }
            for prop in FORBIDDEN {
                if body.contains(prop) {
                    violations.push(format!(
                        "{}: @keyframes `{}` body animates forbidden `{}`",
                        path.file_name().unwrap().to_string_lossy(),
                        name,
                        prop
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "motion doctrine (#43) violations:\n{}",
        violations.join("\n")
    );
}

/// Strip `//` line comments so a `backdrop-filter` mentioned in prose can't
/// false-positive. (SCSS uses `//` line comments; `/* */` block comments are
/// not used for these declarations, and stripping `//` is enough for the
/// declaration-counting this guard does.)
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Count `backdrop-filter:` DECLARATIONS, split into (standard, webkit).
///
/// A declaration is the property name at the START of a (trimmed) line followed
/// by a colon: `backdrop-filter:` / `-webkit-backdrop-filter:`. Start-anchoring
/// is what distinguishes a real declaration from a feature query, where the
/// property sits INSIDE parens — `@supports (backdrop-filter: blur(1px))` and
/// `@media (prefers-reduced-transparency: ...)` begin with `@`/`(`, never with
/// the bare property, so they're correctly ignored. Comments are stripped by the
/// caller. (Authoring convention here is one declaration per line, matched by
/// the source-of-truth grep used to design this guard.)
fn count_backdrop_decls(src: &str) -> (usize, usize) {
    let mut std_count = 0;
    let mut webkit_count = 0;
    for line in src.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("-webkit-backdrop-filter") {
            if rest.trim_start().starts_with(':') {
                webkit_count += 1;
            }
        } else if let Some(rest) = t.strip_prefix("backdrop-filter") {
            if rest.trim_start().starts_with(':') {
                std_count += 1;
            }
        }
    }
    (std_count, webkit_count)
}

/// WebKit 1:1 doctrine: every standard `backdrop-filter` declaration must have a
/// matching `-webkit-backdrop-filter` sibling so Safari/WebKit (the mobile-first
/// PRIMARY target) renders the Apple Liquid Glass 1:1. Owner ruling 2026-06-15
/// made Liquid Glass the orbit-chrome DEFAULT (`glass-holo`), so this is the
/// load-bearing guardrail now — checked per-file by count-equality. (Both
/// declaration orderings exist in-tree — `-webkit-` before OR after the standard
/// line — so a positional/adjacency check would be brittle; count-equality is
/// order- and gap-independent.)
#[test]
fn backdrop_filter_always_has_webkit_sibling() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("style");
    let mut violations = Vec::new();
    for entry in fs::read_dir(&dir).expect("style/ dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("scss") {
            continue;
        }
        let src = strip_line_comments(&fs::read_to_string(&path).expect("read scss"));
        let (std_count, webkit_count) = count_backdrop_decls(&src);
        if std_count != webkit_count {
            violations.push(format!(
                "{}: {} `backdrop-filter` decl(s) but {} `-webkit-backdrop-filter` sibling(s) \
                 — WebKit 1:1 doctrine requires one `-webkit-` sibling per standard decl",
                path.file_name().unwrap().to_string_lossy(),
                std_count,
                webkit_count
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "WebKit 1:1 backdrop-filter doctrine violations:\n{}",
        violations.join("\n")
    );
}

/// Documents + machine-encodes the reversed doctrine at its source: `glass-holo`
/// is now REAL Apple Liquid Glass as the Standard default — it MUST carry a
/// `backdrop-filter` upgrade and MUST NOT layer the `--frost-noise` grain (the
/// "TV static" the owner rejected, 2026-06-15). Guards against a regression that
/// re-buries the orbit chrome under noise or drops the blur.
#[test]
fn glass_holo_is_liquid_glass_not_frost_noise() {
    let foundation =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("style/_foundation.scss"))
            .expect("read _foundation.scss");
    let src = strip_line_comments(&foundation);

    // Isolate the `@mixin glass-holo(...) { ... }` body (brace-matched).
    let start = src
        .find("@mixin glass-holo")
        .expect("glass-holo mixin present");
    let open = start + src[start..].find('{').expect("glass-holo opening brace");
    let bytes = src.as_bytes();
    let (mut depth, mut i, mut end) = (0i32, open, open);
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let body = &src[open..=end];

    assert!(
        body.contains("backdrop-filter"),
        "glass-holo must emit a backdrop-filter Liquid Glass upgrade (2026-06-15 default)"
    );
    assert!(
        body.contains("-webkit-backdrop-filter"),
        "glass-holo's backdrop-filter needs its -webkit- sibling (WebKit 1:1)"
    );
    assert!(
        !body.contains("--frost-noise"),
        "glass-holo must NOT layer --frost-noise grain (the 'TV static' removed 2026-06-15)"
    );
}

/// iOS scroll-lock doctrine (CLAUDE.md F1 / fidelity gate 2026-06-16): the orbit
/// map (`.sk-orbit-map`) is a full-cover `position:fixed` body-portal overlay.
/// The app's `<body>` stays pannable (`_base.scss` `touch-action: manipulation`,
/// shared with non-orbit routes — it can't be globally locked like the prototype's
/// `body{position:fixed}`), so WITHOUT `touch-action: none` on the map root, iOS
/// WebKit routes vertical drags on the overlay to the document and rubber-bands
/// the chat behind it (owner found this on his real iPhone; Playwright reported
/// green). `touch-action` intersects along the touch hit-path to the root, so the
/// declaration on the full-cover root covers the whole map subtree (scrim / nodes
/// / dock). The prototype's `#orbitMap` (a-orbit.html:163) carries it; this pins
/// that the app keeps it.
#[test]
fn orbit_map_overlay_blocks_touch_scroll() {
    let src = strip_line_comments(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("style/_sk_orbit_chrome.scss"),
        )
        .expect("read _sk_orbit_chrome.scss"),
    );

    // Isolate the base `.sk-orbit-map { ... }` rule (brace-matched). The exact
    // `.sk-orbit-map {` selector (space before brace) excludes `.sk-orbit-map-scrim`
    // / `-dock` / `-hint` (a `-` follows `map`) and `.sk-orbit-map.diving` (a `.`
    // follows); `find` returns the first match = the top-level base rule.
    let start = src
        .find(".sk-orbit-map {")
        .expect(".sk-orbit-map base rule present");
    let open = start + src[start..].find('{').expect(".sk-orbit-map opening brace");
    let bytes = src.as_bytes();
    let (mut depth, mut i, mut end) = (0i32, open, open);
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let body = &src[open..=end];

    assert!(
        body.contains("touch-action: none"),
        "`.sk-orbit-map` must carry `touch-action: none` so the full-cover fixed \
         overlay swallows touch and iOS WebKit can't scroll/rubber-band the chat \
         behind it (prototype #orbitMap a-orbit.html:163; CLAUDE.md F1)."
    );
}
