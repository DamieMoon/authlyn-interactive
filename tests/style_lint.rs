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
//!    exemption removed in M5/P0 Task 0.2 once its keyframe became transform-only.)
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
/// until M5/P0 Task 0.2 (#54) rewrote it to a `transform: translateX` sweep.
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

// ───────────────────────────────────────────────────────────────────────────
// Deck-bug-class regression guards (M5 → M7).
//
// One recurring class of defect kept slipping through: orbit chrome shipped
// "demo-grade" — compile-, clippy-, ssr-test- AND Chromium/iOS-sim-green — yet
// was a real WebKit/touch/visual defect the owner caught only on his physical
// iPhone. Worse, a fixed property did not survive the next rewrite (other-author
// bubble parity was fixed pre-orbit at 61ca832 and the orbit rebuild dropped it
// again as B1, ea2f6f1). These guards are the EXTERNAL signal per error class
// (not a self-reminder): each pins a property by a pure static file scan, and
// each is validated to turn RED on the pre-fix state of the named commit. No
// browser, no DB — the deleted visual-gate tooling must NOT return.
// ───────────────────────────────────────────────────────────────────────────

/// Every `style/*.scss` file as `(filename, raw source)`.
fn scss_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("style");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).expect("style/ dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("scss") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, fs::read_to_string(&path).expect("read scss")));
        }
    }
    out
}

/// Every `src/**/*.rs` file as `(path, raw source)`, recursively.
fn rs_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path.to_string_lossy().into_owned();
                out.push((rel, fs::read_to_string(&path).expect("read rs")));
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

/// Read one source file under the crate root by relative path.
fn read_rel(rel: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|_| panic!("read {rel}"))
}

/// Brace-matched body (braces included) of the FIRST rule whose head contains
/// `anchor` (e.g. `".content.sk-orbit-content {"`). `None` if not found.
fn brace_body(src: &str, anchor: &str) -> Option<String> {
    let start = src.find(anchor)?;
    let open = start + src[start..].find('{')?;
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    for i in open..bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(src[open..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// `var(--scrim)` (the opaque modal dim) may sit ONLY on a true modal backdrop.
/// fx-max is unconditional (`mod.rs` renders `class="app fx-max"` since 161baa0),
/// so an opaque scrim on a NON-modal popover catcher is a permanent full-app
/// blackout behind a tiny menu — that is B4 (`.sk-orbit-blossom-scrim`) and its
/// twin `.radial-backdrop` (fixed: both are `background: transparent`).
/// RED at 6c90d20^ (blossom scrim was `var(--scrim)`).
#[test]
fn scrim_only_on_modal_backdrops() {
    // The legitimate full-cover modal backdrops, by their trailing class token.
    const MODAL_SCRIM_ALLOWLIST: &[&str] = &["modal-backdrop", "holopanel-scrim", "sk-orbit-hints"];
    let mut violations = Vec::new();
    for (name, raw) in scss_sources() {
        let src = strip_line_comments(&raw);
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.contains("var(--scrim)") {
                continue;
            }
            // Nearest preceding selector head (a line ending in `{`).
            let token = (0..=i)
                .rev()
                .map(|j| lines[j])
                .find(|l| l.trim_end().ends_with('{'))
                .and_then(|sel| sel.rsplit('.').next())
                .map(|t| t.trim().trim_end_matches('{').trim().to_string())
                .unwrap_or_default();
            if !MODAL_SCRIM_ALLOWLIST.contains(&token.as_str()) {
                violations.push(format!(
                    "{name}:{} — var(--scrim) on non-modal `.{token}`",
                    i + 1
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "var(--scrim) (the opaque modal dim) is only allowed on a true modal backdrop \
         ({MODAL_SCRIM_ALLOWLIST:?}); a non-modal popover/menu catcher must be \
         `background: transparent` — under unconditional fx-max an opaque scrim blacks \
         out the whole chat behind it (B4):\n{}",
        violations.join("\n")
    );
}

/// `.sk-orbit-content` (the 100dvh shell) must clip BOTH axes with `overflow:
/// clip` — never a mixed `overflow-x: clip` + implicit `overflow-y: visible`,
/// the iOS-only phantom side/vertical scroll BOTH the sim and pw-webkit reported
/// green — and never `contain: paint` (which traps the fixed orb/help/composer).
/// RED at 66270c9^ (was `overflow-x: clip` only).
#[test]
fn sk_orbit_content_clips_both_axes_no_paint_containment() {
    let src = strip_line_comments(&read_rel("style/_sk_orbit_chrome.scss"));
    let body = brace_body(&src, ".content.sk-orbit-content {")
        .expect(".content.sk-orbit-content base rule present");
    assert!(
        body.contains("overflow: clip"),
        "`.sk-orbit-content` must `overflow: clip` (BOTH axes); a per-axis split \
         re-opens the iOS hardware-only side-scroll (66270c9/43458c6)."
    );
    assert!(
        !body.contains("overflow-x") && !body.contains("overflow-y"),
        "`.sk-orbit-content` must not re-split overflow per-axis — the mixed \
         overflow-x:clip + overflow-y:visible combo is the hardware-only side-scroll."
    );
    assert!(
        !body.contains("contain: paint"),
        "`.sk-orbit-content` must not use `contain: paint` — it makes the content a \
         containing block for fixed descendants and breaks the fixed orb/help/composer."
    );
}

/// No permanently-dead `:not(.fx-max)` selector: fx-max is rendered
/// unconditionally (`mod.rs` `class="app fx-max"`, 161baa0), so a Standard-tier
/// `:not(.fx-max)` fallback never matches. Pin it gone so a rewrite can't add a
/// dead fallback path back. GREEN now (already purged) — a pure regression guard.
#[test]
fn no_dead_fx_max_negation() {
    let mut violations = Vec::new();
    for (name, raw) in scss_sources() {
        for (i, line) in strip_line_comments(&raw).lines().enumerate() {
            if line.contains(":not(.fx-max") {
                violations.push(format!("{name}:{}", i + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "fx-max is unconditional, so every `:not(.fx-max)` selector is permanently \
         dead CSS:\n{}",
        violations.join("\n")
    );
}

/// No HTML5 drag-and-drop in the UI: iOS WebKit does not implement it, so reorder
/// must use the Pointer-Events grip pattern (manager.rs / wardrobe.rs). The only
/// allowed `draggable` is the exact `"false"` native-drag suppression.
/// RED at bacbcf4^ (wardrobe.rs had a conditional `draggable` + on:dragstart/drop).
#[test]
fn no_html5_drag_and_drop_in_ui() {
    const DRAG_HANDLERS: &[&str] = &[
        "on:dragstart",
        "on:dragover",
        "on:dragend",
        "on:dragenter",
        "on:dragleave",
        "on:drop",
    ];
    let mut violations = Vec::new();
    for (path, raw) in rs_sources() {
        let src = strip_line_comments(&raw);
        for (i, line) in src.lines().enumerate() {
            for h in DRAG_HANDLERS {
                if line.contains(h) {
                    violations.push(format!(
                        "{path}:{} — `{h}` (iOS WebKit has no HTML5 DnD)",
                        i + 1
                    ));
                }
            }
            if let Some(idx) = line.find("draggable=") {
                let rest = line[idx + "draggable=".len()..].trim_start();
                if !rest.starts_with("\"false\"") {
                    violations.push(format!(
                        "{path}:{} — `draggable=` must be the literal `\"false\"` \
                         (a conditional/true draggable is iOS-dead; use the grip)",
                        i + 1
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "HTML5 drag-and-drop is dead on iOS WebKit — reorder must use the \
         pointer-capture grip pattern (bacbcf4):\n{}",
        violations.join("\n")
    );
}

/// The three whole-surface swipe/panel pointer-gesture engines must BAIL before
/// `set_pointer_capture` when the press starts on an interactive control —
/// otherwise a captured pointer steals the trailing `click` from child controls
/// on desktop (dead buttons). RED at 569be68^ (modal/holopanel captured
/// unconditionally; drag.rs bailed only on `.composer`).
#[test]
fn swipe_engines_bail_pointer_capture_on_controls() {
    const ENGINES: &[&str] = &[
        "src/ui/shell/sk_orbit/drag.rs",
        "src/ui/shell/holopanel.rs",
        "src/ui/modal.rs",
    ];
    // The shared bail predicate — the exact control selector all three pass to
    // `.closest(...)`. drag.rs prefixes `.composer, ` but still contains this.
    const BAIL: &str = r#"button, a[href], input, textarea, select, label, [role=\"button\"]"#;
    let mut violations = Vec::new();
    for rel in ENGINES {
        let src = strip_line_comments(&read_rel(rel));
        if !src.contains("set_pointer_capture") {
            violations.push(format!(
                "{rel} — no set_pointer_capture (engine moved? re-verify the bail)"
            ));
        }
        if !src.contains(BAIL) {
            violations.push(format!(
                "{rel} — the pointer-capture `down` path does not bail on interactive \
                 controls (desktop click-dead, 569be68)"
            ));
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// Each per-user pref toggle is rendered by exactly ONE checkbox. A second render
/// site is the Ghost-Quill duplicate (a toggle that lived in both Account →
/// Preferences and the orbit Station pane). RED at 66f5e84^ (ghost_quill rendered
/// twice).
#[test]
fn each_pref_toggle_is_rendered_exactly_once() {
    // Keep this list in sync with the bool prefs in state.rs `PrefsState`.
    const PREF_TOGGLES: &[&str] = &["dialogue_style", "ghost_quill", "haptic_vibrate"];
    let corpus: String = rs_sources()
        .iter()
        .map(|(_, s)| strip_line_comments(s))
        .collect::<Vec<_>>()
        .join("\n");
    let mut violations = Vec::new();
    for pref in PREF_TOGGLES {
        let needle = format!("prop:checked=move || s.prefs.{pref}.get()");
        let count = corpus.matches(&needle).count();
        if count != 1 {
            violations.push(format!(
                "`{pref}` toggle rendered {count}× (expected exactly 1 — a duplicate is the \
                 Ghost-Quill dup class, 66f5e84)"
            ));
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// Leaving a root-mounted management surface returns to the orbit MAP, not the
/// channel underneath. Each `swipe_close=true` management modal (Account, Server,
/// Wardrobe) must call `act::show_orbit_map` in BOTH its `close=` and its
/// `<ModalHead on_close=...>` dismiss closures. Keying on `swipe_close=true`
/// selects exactly those three and excludes the deferred channel-manager and the
/// data-mutating sub-modals. RED at 66f5e84^ (settings-exit dropped into a channel).
#[test]
fn management_modal_dismiss_returns_to_orbit_map() {
    const FILES: &[&str] = &[
        "src/ui/shell/account.rs",
        "src/ui/shell/server.rs",
        "src/ui/shell/mod.rs",
    ];
    let mut count = 0usize;
    let mut violations = Vec::new();
    for rel in FILES {
        let src = strip_line_comments(&read_rel(rel));
        let mut from = 0;
        while let Some(rel_idx) = src[from..].find("swipe_close=true") {
            let start = from + rel_idx;
            // Region = from `swipe_close=true` through the end of the following
            // `<ModalHead ... />` element (its close= line + the ModalHead line).
            let mh = src[start..].find("<ModalHead").map(|x| start + x);
            let end = mh
                .and_then(|m| src[m..].find("/>").map(|x| m + x + 2))
                .unwrap_or(src.len());
            let region = &src[start..end];
            count += 1;
            let occ = region.matches("act::show_orbit_map").count();
            if occ < 2 {
                violations.push(format!(
                    "{rel} — a swipe_close management modal calls act::show_orbit_map {occ}× \
                     in its dismiss region (need both close= and ModalHead on_close=)"
                ));
            }
            from = end;
        }
    }
    assert_eq!(
        count, 3,
        "expected exactly 3 swipe_close management modals (Account/Server/Wardrobe), found {count} \
         — a new one must also return to the orbit map on dismiss"
    );
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// Every named interactive orbit-chrome control inherits the shared Liquid-Glass
/// material via `@include glass-holo` (or `glass-live`) in at least one of its
/// rules — never geometry-only/flat (B2 was a bare `.sk-orbit-account-btn`). The
/// union-of-rules check tolerates the reduced-motion / fx-max sibling rules that
/// legitimately carry no glass. RED at 6c90d20^ (account-btn had no material).
#[test]
fn orbit_chrome_controls_inherit_glass_material() {
    // Interactive chrome controls only — NOT the luminous radial discs
    // (.sk-orbit-core/.sk-orbit-far, by-design no frosted glass) or glyph/label spans.
    const MATERIAL_CONTROLS: &[&str] = &[
        "sk-orbit-pill",
        "sk-orbit-help",
        "sk-orbit-pane-back",
        "sk-orbit-node",
        "sk-orbit-sat",
        "sk-orbit-orb",
        "sk-orbit-chip",
        "sk-orbit-station-close",
        "sk-orbit-persona-card",
        "sk-orbit-account-btn",
    ];
    let src = strip_line_comments(&read_rel("style/_sk_orbit_chrome.scss"));
    let mut violations = Vec::new();
    for ctrl in MATERIAL_CONTROLS {
        // `.<ctrl> {` (literal space-brace) matches only rules where the control
        // is the LAST simple selector — the base rule AND its fx-max/reduced-motion
        // siblings — but not `.<ctrl>-name`/descendant rules. The material may live
        // in ANY of those rules, so union them.
        let anchor = format!(".{ctrl} {{");
        let has_material = all_bodies(&src, &anchor)
            .iter()
            .any(|b| b.contains("@include glass-holo") || b.contains("@include glass-live"));
        if !has_material {
            violations.push(format!(
                ".{ctrl} — no @include glass-holo/glass-live in any of its rules \
                 (geometry-only chrome silently lacks the glow every peer has, B2)"
            ));
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// Dispatch-pane controls (the Friends / Members / wardrobe-card / persona-editor
/// action buttons that live in `_wave_b.scss` / `_wardrobe.scss`, NOT the orbit
/// shell chrome) carry the same shared Liquid-Glass material via
/// `@include glass-holo` (or `glass-live`) at their BASE definition — so every
/// surface reads with the electric-blue glow, never flat (B2). Each anchor is the
/// exact selector head as it appears in the file; a nested `@include` counts
/// because `all_bodies` returns the whole block body.
#[test]
fn dispatch_pane_controls_inherit_glass_material() {
    const PANE_CONTROLS: &[(&str, &str)] = &[
        ("style/_wave_b.scss", ".add-row button {"),
        ("style/_wave_b.scss", ".flist button {"),
        ("style/_wave_b.scss", ".flist label {"),
        ("style/_wave_b.scss", ".member-role-btn {"),
        ("style/_wave_b.scss", ".member-kick {"),
        ("style/_wardrobe.scss", ".card-actions {"),
        ("style/_wardrobe.scss", ".detail-actions {"),
    ];
    let mut violations = Vec::new();
    for (file, anchor) in PANE_CONTROLS {
        let src = strip_line_comments(&read_rel(file));
        let bodies = all_bodies(&src, anchor);
        if bodies.is_empty() {
            violations.push(format!(
                "{file}: `{anchor}` not found (renamed? update the guard)"
            ));
        } else if !bodies
            .iter()
            .any(|b| b.contains("@include glass-holo") || b.contains("@include glass-live"))
        {
            violations.push(format!(
                "{file}: `{anchor}` — no @include glass-holo/glass-live in its rule body \
                 (dispatch-pane control silently lacks the glow every peer has, B2)"
            ));
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// `glass-holo` / `glass-live` OWN the control's background — a consumer must NOT
/// restate a top-level `background:` after the `@include`. The mixin emits its
/// `@supports (backdrop-filter…) { background: …glass… }` block AFTER the rule
/// body, so on backdrop-filter engines (WebKit/iOS — the primary target) a
/// restated top-level background is a DEAD declaration: the control ships the
/// blue accent-glass fill, not the authored one, while reading correctly on
/// Chromium (the UI-fidelity class — invisible to fmt/clippy). The B2 reference
/// peer `.sk-orbit-account-btn` lets the mixin own the background; the dispatch-
/// pane consumers must too. RED if `.member-kick` / `.member-role-btn` restate
/// `background: var(--surface)` after the include (the defect this guards).
#[test]
fn glass_holo_consumers_let_the_mixin_own_the_background() {
    // The same dispatch-pane consumers as `dispatch_pane_controls_inherit_glass_material`.
    const GLASS_CONSUMERS: &[(&str, &str)] = &[
        ("style/_wave_b.scss", ".add-row button {"),
        ("style/_wave_b.scss", ".flist button {"),
        ("style/_wave_b.scss", ".flist label {"),
        ("style/_wave_b.scss", ".member-role-btn {"),
        ("style/_wave_b.scss", ".member-kick {"),
        ("style/_wardrobe.scss", ".card-actions {"),
        ("style/_wardrobe.scss", ".detail-actions {"),
    ];
    let mut violations = Vec::new();
    for (file, anchor) in GLASS_CONSUMERS {
        let src = strip_line_comments(&read_rel(file));
        for body in all_bodies(&src, anchor) {
            let includes_glass =
                body.contains("@include glass-holo") || body.contains("@include glass-live");
            if includes_glass && has_top_level_background(&body) {
                violations.push(format!(
                    "{file}: `{anchor}` restates a top-level `background:` after \
                     @include glass-holo/glass-live — DEAD on backdrop-filter engines \
                     (WebKit/iOS); let the mixin own the background (cf. .sk-orbit-account-btn)"
                ));
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// True if `body` (as returned by `brace_body`, i.e. INCLUDING its outer braces,
/// so the rule's own declarations sit at brace-depth 1) declares a `background`
/// at depth 1 — a sibling of the `@include`, not one nested in `&:hover` /
/// `@supports` / `@media` (those are depth >= 2 and legitimate).
fn has_top_level_background(body: &str) -> bool {
    let mut depth = 0i32;
    for line in body.lines() {
        let l = line.trim();
        if depth == 1 && (l.starts_with("background:") || l.starts_with("background-color:")) {
            return true;
        }
        for ch in line.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
    }
    false
}

/// Every brace-matched body whose rule head contains `anchor`, in document order.
fn all_bodies(src: &str, anchor: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = src[from..].find(anchor) {
        let at = from + rel;
        match brace_body(&src[at..], anchor) {
            Some(body) => {
                from = at + body.len();
                out.push(body);
            }
            None => break,
        }
    }
    out
}

/// Parse an absolute CSS length to rem (1rem = 16px). `None` for non-absolute
/// units (%, vh, vw, calc, auto, env) so they are SKIPPED, never read as 0.
fn len_to_rem(value: &str) -> Option<f32> {
    let v = value.split("//").next().unwrap_or(value);
    let v = v.trim().trim_end_matches(';').trim();
    if let Some(n) = v.strip_suffix("rem") {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = v.strip_suffix("px") {
        n.trim().parse::<f32>().ok().map(|px| px / 16.0)
    } else {
        None
    }
}

/// At least one line of `body` declares a >= 2.75rem (44px) HEIGHT floor
/// (`min-height` or `height`). Square icon buttons floor via `height`.
fn declares_touch_floor(body: &str) -> bool {
    body.lines().any(|line| {
        let l = line.trim();
        for prop in ["min-height:", "height:"] {
            if let Some(rest) = l.strip_prefix(prop) {
                return len_to_rem(rest).is_some_and(|rem| rem >= 2.75 - 1e-3);
            }
        }
        false
    })
}

/// Every registered interactive control declares a >= 44px (2.75rem) HEIGHT
/// floor — Mendicant Bias is touch-first PRODUCT-WIDE (owner ruling 2026-06-17,
/// ctx 019ed33e): compact desktop-density controls are the regression the product
/// exists to retire. The registry IS the allowlist by construction: deferred
/// controls (.persona-reorder/.channel-reorder/.gallery-remove, pending the
/// wardrobe rebuild) and bespoke geometry (composer orb / map nodes / swipe-strip,
/// .member-avatar <img>) are simply not members. RED at e6c7845^ / 844fe5e^
/// (a control shipped below the floor). New floored controls join the registry by
/// hand — a curated list that grows deliberately, never an auto button-scan (which
/// false-positives on image tiles, inline spans, list rows).
#[test]
fn registered_interactive_controls_declare_44px_touch_floor() {
    const FLOOR_CONTROLS: &[(&str, &str)] = &[
        ("style/_sk_orbit_chrome.scss", "sk-orbit-pill"),
        ("style/_sk_orbit_chrome.scss", "sk-orbit-sat"),
        ("style/_sk_orbit_chrome.scss", "sk-orbit-chip"),
        ("style/_sk_orbit_chrome.scss", "sk-orbit-orb"),
        ("style/_sk_orbit_chrome.scss", "sk-orbit-station-close"),
        ("style/_sk_orbit_chrome.scss", "sk-orbit-persona-card"),
        ("style/_sk_orbit_chrome.scss", "sk-orbit-account-btn"),
        ("style/_sk_orbit_chrome.scss", "sk-orbit-pane-back"),
        ("style/_foundation.scss", "accent-swatch"),
        ("style/_modal.scss", "account-logout"),
        ("style/_modal.scss", "account-save"),
        ("style/_modal.scss", "row-edit"),
        ("style/_trash.scss", "trash-toggle"),
        ("style/_trash.scss", "trash-restore"),
        ("style/_wave_b.scss", "member-role-btn"),
        ("style/_wave_b.scss", "member-kick"),
        ("style/_wave_b.scss", "add-row button"),
        ("style/_wave_b.scss", "flist button"),
        ("style/_wave_b.scss", "flist label"),
        // Card-/detail-action buttons floor via a nested `button {}` rule, so the
        // registry holds the PARENT block selector — `all_bodies` returns the whole
        // block body (nested `button` min-height included) for `declares_touch_floor`.
        ("style/_wardrobe.scss", "card-actions"),
        ("style/_wardrobe.scss", "detail-actions"),
        ("style/_toast.scss", "toast-action"),
        ("style/_wardrobe.scss", "persona-grip"),
        ("style/_lorebook.scss", "lore-grip"),
    ];
    let mut violations = Vec::new();
    for (file, ctrl) in FLOOR_CONTROLS {
        let src = strip_line_comments(&read_rel(file));
        let bodies = all_bodies(&src, &format!(".{ctrl} {{"));
        if bodies.is_empty() {
            violations.push(format!(
                "{file}: `.{ctrl}` not found (renamed? update the registry)"
            ));
        } else if !bodies.iter().any(|b| declares_touch_floor(b)) {
            violations.push(format!(
                "{file}: `.{ctrl}` — no >=2.75rem (44px) min-height/height floor"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "Mendicant Bias is touch-first product-wide (>=44px tap targets, owner ruling \
         2026-06-17): a registered control dropped below the floor:\n{}",
        violations.join("\n")
    );
}
