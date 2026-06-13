//! W5/P0 #43 motion doctrine guard: `@keyframes` blocks may animate ONLY
//! transform/translate/rotate/scale/opacity. Paint-class properties
//! (box-shadow, background-position, filter, width, height, top, left)
//! inside a @keyframes body force per-frame repaint on the POCO C3 floor
//! and are forbidden — every future fx- effect must be born composite-cheap.
//! Pure file scan; no DB, runs in every feature graph.
//!
//! A narrow, documented `EXEMPT_KEYFRAMES` allowlist (by name) carves out the
//! brief loading-placeholder shimmer loaders + the one TIME-BOXED pending
//! offender (`fx-warp`, retrofitted in W5/P0 Task 0.2 — its exemption is
//! removed there). See the const for the rationale.

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
/// `fx-warp` is a TIME-BOXED exemption: it still animates `background-position`
/// in the current tree and is rewritten to a transform sweep in W5/P0 Task 0.2
/// (#54, `fx-warp keyframe → translateX`). REMOVE `"fx-warp"` from this list in
/// Task 0.2 once the keyframe is composite-only — at which point this guard
/// enforces the doctrine on fx-warp too (Step 0.2.8's real assertion).
const EXEMPT_KEYFRAMES: &[&str] = &["shimmer", "gallery-skeleton-shimmer", "fx-warp"];

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
