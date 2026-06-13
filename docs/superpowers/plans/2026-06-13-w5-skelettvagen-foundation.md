# Mendicant Bias W5 (Skelettvågen): Foundation Plan — Phase 0 Prerequisites + Phase 1 Switch Infrastructure

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax. Run the test BEFORE the implementation where a test is specified (Run → FAIL → implement → Run → PASS), then build, then a single commit per task. The per-task review pattern is implementer → spec-review → quality-review (as in the W4 plan).

> **Part of the W5 wave** — see the roadmap: `docs/superpowers/plans/2026-06-13-w5-skelettvagen-roadmap.md` (the master index for Phases 0–7 and the 6-doc decomposition). This Foundation doc is the canonical executable doc for Phase 0 + Phase 1.

**Goal:** Land the six ux-evolution prerequisite items (#43 motion doctrine, #54 transform-free `.content` + body portals, #20 etched glass, #36 content-visibility, #49 HoloPanel engine, #19 visual haptics) and the theme-switch + onboarding-ceremony machinery (`authlyn.skeleton` pref, `.app.sk-*` root class, pref-less three-way choice at first authenticated mount). This unblocks all three W5 skeletons (`sk-orbit`/`sk-deck`/`sk-hud`) without yet authoring any one of them.

**Architecture:** Two appearance axes are introduced at the foundation level: **Skeleton** (structure — a `String` client-pref → `.app.sk-*` root class) and the existing **Tier** (`.fx-max`). The shared core (message stream, W4 effects, composer, `act/` layer, `message_actions(kind, mine)`) is untouched. The new switch state is lifted ABOVE any skeleton branch so switching never remounts the shell. Foundation also converts the chrome material from live `backdrop-filter` to a compositor-cheap etched-glass default (live refraction survives as an fx-max escalation), relocates the three fixed overlays to body-level `<Portal>`s so they stop depending on `.content`'s transform, rebases the warp transition off `.content`, codifies the keyframe motion doctrine as a lint, builds the HoloPanel gesture primitive (engine only; first real consumer is Phase 3), and ships the visual-haptics feedback vocabulary.

**Tech Stack:** Rust/Leptos 0.8 full-stack. Touched: `src/ui/shell/act/prefs.rs` (+ `act/mod.rs` re-export), `src/ui/shell/act/haptics.rs` (NEW), `src/ui/shell/state.rs` (`Prefs`), `src/ui/shell/mod.rs` (`AppShell` constructor + root render + ceremony), `src/ui/shell/account.rs` (picker + haptic toggle), `src/ui/shell/channel/mod.rs` (portal relocation of radial/lightbox/emoji), `src/ui/shell/holopanel.rs` (NEW), `src/ui/shell/ceremony.rs` (NEW), `style/_foundation.scss` (glass split, warp rebase), `style/_motion.scss` (fx-glow-pulse retrofit, fx-warp rebase, vh-* family), `style/_content.scss` (content-visibility, warp dip move, glow `::before`), `style/_tokens.scss` (frost tokens), `style/_holopanel.scss` (NEW), `style/_ceremony.scss` (NEW), `style/main.scss` (@use registration), `tests/style_lint.rs` (NEW — `std::fs` keyframe-doctrine guard), `tests/skeleton_switch.rs` (NEW — pref + switch-invariant guard), `.githooks/pre-commit` (keyframe grep).

**Spec:** `docs/superpowers/specs/2026-06-10-mendicant-bias-design.md` §1 (appearance axes — skeleton × tier, nine effects), §2 (navigation — W3 retired by W5), §12 Phase (0)+(1), §13 verification gates. Prototypes: `assets/2026-06-12-skelettvagen/` (the three skeleton prototypes share the always-on protocol/markup/SSE invariants). Touchpoints inventory (verified against current code): fx-max pref precedent `act/prefs.rs:59-73` + ssr stubs `:120-125`, re-export `act/mod.rs:107-110`, shell mount/constructor `mod.rs:219-224`, root render `mod.rs:389`, account picker `account.rs:184-226`. SCSS map (token→typography→base→foundation→motion→…→chrome layering per `main.scss:9-32`; `glass()` mixin `_foundation.scss:9-46`; aurora layer + warp; reduced-motion kill list `_motion.scss:243-260`).

**Gates (full gate, run at every task close):** `cargo fmt --all --check`; `cargo clippy --features ssr`; `cargo clippy --features hydrate --target wasm32-unknown-unknown`; `cargo clippy --features freya`; `cargo test --features ssr` (live SurrealDB on 127.0.0.1:8000, root/root, 0 FAILED); `cargo leptos build --release`; `cargo build --bin authlyn-native --features freya`. New tests: `tests/style_lint.rs`, `tests/skeleton_switch.rs`. WASM bundle baseline recorded at P0 start (Task 0.0).

**Branch:** continue on `mendicant-bias` (do NOT push to `main` — push to `main` is a live fenrir deploy; owner sign-off only).

**WASM bundle budget baseline (record at P0 start, before any code; see Task 0.0):** run `cargo leptos build --release`, then record raw + gzip bytes of `target/site/pkg/authlyn-interactive.wasm`. This number is the W5 plan-header baseline; **the owner signs the budget ceiling (Open Question #1)**; the Phase-(7) gate re-measures with the same command. (Confirmed: that path is the wasm artifact in this tree.)

---

## Open Questions (owner decisions) — DO NOT silently resolve

These are flagged owner decisions surfaced during the adversarial plan review. Each step that touches one carries an inline pointer. They are NOT blockers for authoring; they are decisions to sign before the affected phase ships.

1. **WASM bundle-budget ceiling.** Owner-signed, not yet a number. Task 0.0 records the baseline (raw + gzip bytes of `target/site/pkg/authlyn-interactive.wasm`). The owner must sign the allowed growth before Phase 7 re-measures. *(Touched by: Task 0.0, Phase-7 gate.)*
2. **Standard-tier chrome look (etched glass).** The `--frost-noise` PNG and the `--frost-top` / `--frost-bottom` precomputed tints need an owner eyeball to confirm Standard-tier chrome reads ~90% identical to live blur. Concrete sample values ship in Task 0.3; flagged for an eyeball pass. *(Touched by: Task 0.3.)*
3. **Ceremony localStorage-writability detection approach.** The plan uses a throwaway probe key (`_authlyn_pref_test`) set-then-deleted to detect writability WITHOUT touching `authlyn.skeleton`, preserving "no silent default". The owner may prefer a cleaner detection (e.g. a single try/catch on the real write). *(Touched by: Task 1.3.)*
4. **Automated headed-Playwright switch-invariant guard, now vs Phase 7.** The §13 switch invariant (no SSE drop / no draft loss / no selection reset on skeleton switch) is pinned STRUCTURALLY in Task 1.6; the LIVE behavioral check is currently booked into the Phase-7 real-device gate. The owner may want it automated sooner via headed Playwright on the MacBook. *(Touched by: Task 1.6, Phase-7 gate.)*
5. **`guild.accent_color` schema field does not exist yet.** Per-server accent (spec §1 wow-effect #G) needs a `guild.accent_color` schema field that is **absent from `src/storage/schema.surql` today**. This is OUT of Foundation scope, but someone must author it under the SCHEMAFULL NONE-coercion + enum-OVERWRITE invariants (CLAUDE.md) BEFORE any skeleton renders the accent. Flagged here so it is not forgotten; the warp directional tint (Task 0.2) deliberately uses the generic `--glow-accent` until this lands. *(Touched by: Task 0.2 inline note; a non-Foundation substrate task.)*
6. **Exact `web_sys` API surface for the once-listener + vibrate.** web-sys is pinned `0.3` (Cargo.lock resolves `0.3.85`). `set_pointer_capture(pointer_id)` is confirmed in-tree (`lightbox.rs:530`). `AddEventListenerOptions::new().once(true)` + `add_event_listener_with_callback_and_add_event_listener_options`, and `navigator.vibrate_with_duration` are NOT yet used anywhere in this codebase — **confirm the exact binding names against web-sys 0.3.85 at execution** and, if they differ, mirror the proven `leptos::ev` / `leptos::web_sys` / `wasm_bindgen::JsCast` pattern already used in `radial.rs:244-253` / `lightbox.rs:283-291`. *(Touched by: Task 0.5b, Task 0.6.)*

---

**Invariant watch:**
- **fx-max pref pattern is the template** — `authlyn.skeleton` MUST mirror `act/prefs.rs` exactly: gloo-storage JSON-encodes, so values are read back via `LocalStorage::get::<String>()` (the stored string is quoted); hydrate-real fn + ssr stub; never raw localStorage. (`act/prefs.rs:59-73` eyecandy, `:104-133` stubs.)
- **No silent default** (spec §1) — a pref-less device gets the ceremony, NOT a quietly-applied skeleton. The ONLY exception is the in-wave dev build (Task 1.5 scaffolding): a *localStorage-unavailable* device boots `sk-orbit` for the session under the still-no-op `.app.sk-orbit` selector, so the W3 chrome shows through; that scaffolding is deleted in Phase 6.
- **localStorage-unavailable fallback** — when localStorage cannot persist (private mode / disabled), boot `sk-orbit` for the session WITHOUT ceremony (spec §1 fallback). Detected via a throwaway probe key (Open Question #3), never by writing `authlyn.skeleton`.
- **Theme switch never drops SSE / composer draft / selection** (spec §13) — the switch flips one `RwSignal<Option<String>>` driving a root class; it MUST NOT remount `AppShell`, reset `Composer.compose`, or clear `Selection.sel_channel`. Pinned structurally by `tests/skeleton_switch.rs` (Task 1.6); the live behavioral check is the Phase-7 gate (Open Question #4).
- **Motion doctrine (#43)** — `@keyframes` may animate ONLY transform/translate/rotate/scale/opacity. `box-shadow`/`background-position`/`filter`/`width`/`height`/`top:`/`left:` inside a `@keyframes` block is a lint failure (`tests/style_lint.rs` is the authoritative brace-aware guard + `.githooks/pre-commit` is the fast gate). Existing `fx-glow-pulse` violates this (it animates `box-shadow` at `_motion.scss:26-34`) and is retrofitted in Task 0.1 — note it has **two** consumers (`.send.sent` and the fx-max typing star), both addressed there.
- **Glass split keeps the opaque fallback** (#20) — `glass-etched` must still degrade to opaque/Canvas under `forced-colors`; the existing `@supports`/`prefers-reduced-transparency`/`forced-colors` guards on the old mixin (`_foundation.scss:9-46`) must survive on `glass-live`.
- **Portal relocation keeps the safe-area + z-index contract** (#54) — the three relocated overlays (radial, lightbox, mobile emoji) own their z-index at the body level and must not regress reduced-motion kills (`_motion.scss:243-260` lists `.radial-menu`, `.composer .send.sent`, the warp `::after`) or the lightbox z-100 layering.
- **`sk-` prefix rule** — structural shells are `sk-`-prefixed (`.app.sk-*`, `_sk_*.scss`, `sk_*/mod.rs`); the loading-placeholder skeleton (`_skeleton.scss`, `channel/skeleton.rs`) keeps unprefixed names. NEVER collide them.
- **Disjointness** — `prefs.rs`/`haptics.rs` skeleton+haptic helpers are hydrate-gated with ssr stubs; `HoloPanel`/`ceremony` are shared UI but import ZERO ssr crates; gloo-storage stays out of the ssr graph.

---

## Phase 0 — Prerequisite Cluster

### Task 0.0: Record WASM bundle baseline (verification, no code)

**Files:** none (records a number into this plan header + the eventual Phase-7 gate). **Open Question #1.**

- [ ] **Step 0.0.1: Clean release build.** Run `cargo leptos build --release` from the repo root. Expect it to complete (server bin + `site/`).
- [ ] **Step 0.0.2: Measure.** Run `wc -c target/site/pkg/authlyn-interactive.wasm` for raw bytes and `gzip -c target/site/pkg/authlyn-interactive.wasm | wc -c` for gzip bytes. Record BOTH numbers in this plan's header under "WASM bundle budget baseline" and flag for the owner to sign a ceiling (Open Question #1). No commit (measurement only).

---

### Task 0.1: #43 Motion doctrine — keyframe lint + fx-glow-pulse retrofit (CSS/lint, STD)

**Files:** `style/_motion.scss` (`@keyframes fx-glow-pulse` at lines 26-34; the doctrine header at lines 1-2; reduced-motion kill list `:243-260`), `style/_content.scss` (the TWO `fx-glow-pulse` consumers — the send-button pulse `.composer .send.sent` at `:1113-1116` and the fx-max typing-star glow at `:716-719` — relocate both box-shadow animations to a static `::before`), `tests/style_lint.rs` (NEW — `std::fs` guard reading the SCSS source), `.githooks/pre-commit` (grep gate). Note: there is no SCSS-asserting test today; `tests/media.rs` already uses `std::fs`, so the pattern is sanctioned.

- [ ] **Step 0.1.1: Locate the fx-glow-pulse consumers AND extract the real timing.** Run `grep -rn "fx-glow-pulse" style/` to find every site. Confirmed sites (current tree): the keyframe at `_motion.scss:26`, and TWO consumers in `_content.scss` — the send-confirm pulse at `:1115` and the fx-max typing-star at `:718`. Extract the exact `.send.sent` timing the retrofit must preserve:
  ```
  grep -A2 "&.sent {" style/_content.scss | grep animation:
  # current tree → `animation: fx-glow-pulse 0.4s ease-out 1;`  (USE 0.4s/1)
  ```
  and the fx-max typing-star timing:
  ```
  grep -n "fx-glow-pulse" style/_content.scss
  # :718 → `animation: fx-glow-pulse 2.4s ease-in-out infinite;` (a glow BREATHE)
  ```
  The illustrative durations below (0.4s for `.send.sent`, 2.4s for the star) are the actual measured values from the current tree — re-confirm with the greps above before editing, in case the lines drifted.

- [ ] **Step 0.1.2: Write the failing lint test FIRST.** Create `tests/style_lint.rs`:
```rust
//! W5/P0 #43 motion doctrine guard: `@keyframes` blocks may animate ONLY
//! transform/translate/rotate/scale/opacity. Paint-class properties
//! (box-shadow, background-position, filter, width, height, top, left)
//! inside a @keyframes body force per-frame repaint on the POCO C3 floor
//! and are forbidden — every future fx- effect must be born composite-cheap.
//! Pure file scan; no DB, runs in every feature graph.

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

/// Return every `@keyframes { ... }` body found in `src`, brace-matched.
fn keyframes_bodies(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = src[search_from..].find("@keyframes") {
        let kw = search_from + rel;
        // Find the opening brace after the keyframes name.
        if let Some(open_rel) = src[kw..].find('{') {
            let open = kw + open_rel;
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
            out.push(src[open..=end].to_string());
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
        for body in keyframes_bodies(&src) {
            for prop in FORBIDDEN {
                if body.contains(prop) {
                    violations.push(format!(
                        "{}: @keyframes body animates forbidden `{}`",
                        path.file_name().unwrap().to_string_lossy(),
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
```
- [ ] **Step 0.1.3: Run → FAIL.** Run `cargo test --features ssr --test style_lint`. Expect FAIL naming `_motion.scss: @keyframes body animates forbidden \`box-shadow\`` (fx-glow-pulse animates box-shadow at 0%/50%/100%). This proves the guard catches the real existing violation.

- [ ] **Step 0.1.4: Retrofit fx-glow-pulse to opacity.** In `_motion.scss`, replace the `fx-glow-pulse` keyframe body so it animates ONLY opacity:
```scss
// W5/P0 #43: a glow PULSE must animate opacity, never box-shadow (a paint
// property → per-frame repaint on the floor device). Each consumer carries
// the MAX glow STATICALLY on a `::before`; this keyframe just breathes its
// opacity 0.3 → 1 → 0.3. House technique: "pre-rendered glow" — see header.
@keyframes fx-glow-pulse {
	0%,
	100% {
		opacity: 0.3;
	}
	50% {
		opacity: 1;
	}
}
```
- [ ] **Step 0.1.5: Move BOTH consumers' static MAX glow to a `::before`.** The keyframe now drives opacity, so each consumer that previously animated its own box-shadow must carry the glow statically on a pseudo-element. There are TWO (Step 0.1.1).

  **(a) The send-confirm pulse** — in `_content.scss`, the `&.sent { animation: fx-glow-pulse 0.4s ease-out 1; }` rule at `:1115` (inside the `.composer .send` block). Replace it so the glow lives on a `::before` and the keyframe breathes its opacity (timing from Step 0.1.1: 0.4s ease-out, one iteration):
```scss
// W5/P0 #43 pre-rendered-glow pattern: the send-confirm pulse no longer
// animates box-shadow. A `::before` carries the MAX glow statically; the
// keyframe only breathes its opacity. Identical look, composite-only cost.
// (The `.send` rule already sets `position: relative` for the charge ring.)
&.sent::after {
	content: "";
	position: absolute;
	inset: 0;
	border-radius: inherit;
	pointer-events: none;
	box-shadow: 0 0 16px var(--glow-accent); // MAX glow, STATIC
	animation: fx-glow-pulse 0.4s ease-out 1;
}
```
  (`.send` already owns `::before` for the charge ring — use `::after` for the glow so the two pseudo-elements don't collide. Verify with `grep -n "send::before\|&::before\|&.charging::before" style/_content.scss` before choosing the pseudo-element.)

  **(b) The fx-max typing-star breathe** — in `_content.scss`, the `.app.fx-max & { box-shadow: 0 0 18px var(--glow-accent); animation: fx-glow-pulse 2.4s ease-in-out infinite; }` rule at `:716-719` (inside the `.typing-indicator .star` block). The box-shadow there is STATIC already (not in the keyframe); only the `animation` line referenced the box-shadow-pulsing keyframe. Move the star's glow onto a `::before` and let the opacity keyframe breathe it (2.4s ease-in-out infinite, from Step 0.1.1):
```scss
// W5/P0 #43: the eye-candy typing star breathes its glow via a static-glow
// ::before + opacity keyframe (was relying on fx-glow-pulse animating
// box-shadow). The star keeps its own steady presence; the ::before pulses.
.app.fx-max & {
	position: relative;
}
.app.fx-max &::before {
	content: "";
	position: absolute;
	inset: -2px;
	border-radius: 50%;
	pointer-events: none;
	box-shadow: 0 0 18px var(--glow-accent); // MAX glow, STATIC
	animation: fx-glow-pulse 2.4s ease-in-out infinite;
}
```
  (Confirm the `.star` selector already establishes a positioning context; if it sets `position: absolute` for its orbit layout, the `::before` `inset` anchors to it correctly — re-read `_content.scss` around the `.star` block before pasting, and adjust `inset` to match the star's size.)

- [ ] **Step 0.1.6: Verify the reduced-motion kill reaches both `::before`/`::after`.** The element-level kill does NOT reach pseudo-elements (see the existing note about the warp `::after` at `_motion.scss:222-225`). The current kill list (`:243-260`) already lists `.composer .send.sent` — add the pseudo-element selectors so their animations are also killed. Append to the reduced-motion block:
```scss
	.composer .send.sent::after,
	.typing-indicator .star::before,
```
  (Add these alongside the existing `.composer .send.sent,` / `.radial-menu,` entries inside the `@media (prefers-reduced-motion: reduce)` block at `_motion.scss:243-260`.)

- [ ] **Step 0.1.7: Document the doctrine.** Update the `_motion.scss` header (lines 1-2) to state the doctrine + pre-rendered-glow house technique:
```scss
// Motion library (W2; W5/P0 #43 doctrine). Shared keyframes + the
// reduced-motion kill switch. DOCTRINE: @keyframes may animate ONLY
// transform/translate/rotate/scale/opacity — never box-shadow,
// background-position, filter, width/height, top/left (all force layout or
// paint per frame; lethal on the POCO C3 floor). HOUSE TECHNIQUE
// "pre-rendered glow": carry the MAX box-shadow STATICALLY on a `::before`/
// `::after`, pulse only its opacity (see fx-glow-pulse + .composer
// .send.sent::after + .typing-indicator .star::before). Enforced by
// tests/style_lint.rs (authoritative, brace-aware) + .githooks/pre-commit
// (fast gate). .fx-max layers more in W5/W11 — every new keyframe is born
// composite-cheap by this rule.
```
- [ ] **Step 0.1.8: Run → PASS.** Run `cargo test --features ssr --test style_lint`. Expect PASS (no forbidden property in any keyframes body).

- [ ] **Step 0.1.9: Add the pre-commit grep gate (robust, multi-line aware).** Append to `.githooks/pre-commit` (after the existing body, before the final `exit 0`) a multi-line-tolerant grep that fails when a forbidden property appears inside a staged SCSS file's `@keyframes` neighborhood. This is the FAST gate; the brace-aware authority is `tests/style_lint.rs`:
```bash
# W5/P0 #43 motion doctrine: no paint/layout properties inside @keyframes.
# Fast gate (the authoritative brace-aware check is tests/style_lint.rs). We
# can't fully brace-parse in shell, so we scan every staged .scss file that
# contains an @keyframes and flag a forbidden property in the 100 lines after
# one. False positives are possible (a forbidden prop in a normal rule right
# after a keyframes block); the Rust test is precise, so a flagged commit
# should be confirmed against `cargo test --test style_lint`.
staged_scss=$(git diff --cached --name-only --diff-filter=ACM | grep '\.scss$' || true)
if [ -n "$staged_scss" ]; then
	if echo "$staged_scss" \
		| xargs -r grep -l '@keyframes' \
		| xargs -r grep -A100 '@keyframes' \
		| grep -E '^[[:space:]]+(box-shadow|background-position|filter:|width:|height:|top:|left:)' >/dev/null; then
		echo "pre-commit: motion doctrine (#43) violation near @keyframes — confirm with 'cargo test --features ssr --test style_lint'" >&2
		exit 1
	fi
fi
```
- [ ] **Step 0.1.10: Gate + commit.** Full gate (fmt + clippy ×3 + `cargo test --features ssr` 0 FAILED + `cargo leptos build --release`). Commit:
```
feat(style): #43 motion doctrine — keyframes animate transform/opacity only, fx-glow-pulse retrofit (W5/P0) (STD)

Codify the house rule that @keyframes may animate only
transform/translate/rotate/scale/opacity. Retrofit fx-glow-pulse (was
animating box-shadow — a paint property) to a pre-rendered-glow ::before/
::after that breathes opacity, fixing BOTH consumers (the send-confirm pulse
and the fx-max typing-star breathe). Add a brace-aware std::fs lint test + a
multi-line-tolerant pre-commit grep gate. Makes every future W5/W11 fx- effect
born floor-device-cheap.

Tests: no_keyframes_animate_paint_or_layout_properties

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 0.2: #54 Transform-free `.content` + body-level portal layer (Leptos/CSS, M)

**Files:** `src/ui/shell/channel/mod.rs` (the radial `RwSignal<Option<RadialState>>` mount, the lightbox `lightbox_view` mount, and the mobile emoji sheet mount — relocate all three to `<Portal>`), `style/_foundation.scss` (warp `::after` rebase off `.content`), `style/_content.scss` (move the `.fx-switching` dip transform from `.content` to the inner `.channel-view` wrapper), `style/_motion.scss` (`fx-warp` keyframe → transform; reduced-motion kill selector rebase). The three fixed overlays currently live inside `ChannelPane`, trapped by `.content`'s transform (documented in `_content.scss`'s CONSTRAINT note).

- [ ] **Step 0.2.1: Confirm the trap + the three overlays.** Re-read the `_content.scss` CONSTRAINT note (find via `grep -n "containing block\|stacking context\|CONSTRAINT\|fixed" style/_content.scss`): `.content` is the containing block + stacking context for `position: fixed` descendants while `.fx-switching`'s transform is non-none. Confirm the three fixed overlays mounted inside `ChannelPane` via `grep -n "radial\|lightbox\|emoji" src/ui/shell/channel/mod.rs` (radial `RwSignal<Option<RadialState>>`, lightbox `RwSignal<Option<LightboxState>>` driving `lightbox_view`, plus the mobile emoji sheet).

- [ ] **Step 0.2.2: (a) Relocate the three overlays to `<Portal>`.** Wrap each of the three fixed-overlay views in `ChannelPane` with Leptos `<Portal>` (which mounts to `document.body`). The Portal wraps the inner view, INSIDE its existing `<Show>` guard, so the overlay only mounts to the body while it is open. Full context per overlay:
```rust
use leptos::portal::Portal;

// --- radial menu (was rendered inline when `radial` is Some) ---
<Show when=move || radial.get().is_some()>
    <Portal>
        // existing radial-menu view, unchanged — same `.radial-menu` classes,
        // same act:: handlers (start_reply/copy_message_body/start_edit/ask_delete),
        // still driven by the `radial` signal.
        {radial_menu_view(radial)}
    </Portal>
</Show>

// --- lightbox (was rendered when `lightbox` is Some) ---
<Show when=move || lightbox.get().is_some()>
    <Portal>
        // existing lightbox view, unchanged — keeps z-100 layering + classes.
        {lightbox_view(lightbox, lb_transform)}
    </Portal>
</Show>

// --- mobile emoji sheet (was rendered when the emoji-open signal is set) ---
<Show when=move || emoji_open.get()>
    <Portal>
        // existing mobile emoji-sheet view, unchanged classes.
        {mobile_emoji_sheet_view(/* existing args */)}
    </Portal>
</Show>
```
  Match the EXACT existing function/closure names and signal names from Step 0.2.1's grep (the names above — `radial_menu_view`, `emoji_open`, `lb_transform`, `mobile_emoji_sheet_view` — are placeholders for whatever the inline render currently calls; do NOT introduce new names). Only the DOM mount point changes (now body, not inside `.content`). The reactive signals still drive `Show`/conditional rendering exactly as before; `<Portal>` re-renders reactively.

- [ ] **Step 0.2.3: Codify the body-level z-index + safe-area contract.** In `_foundation.scss` (a short shared block near the glass section), document that body-portal overlays own their z-index at the document level (radial above content chrome, lightbox at z-100 as today, emoji sheet above composer) and carry their own `env(safe-area-inset-*)` paddings since they no longer inherit `.content`'s box. Verify the lightbox keeps z-100 (`grep -n "z-index" style/_lightbox.scss`) and the radial keeps its layering (find the radial block via `grep -n "radial-menu" style/_content.scss`). No selector specificity change should be needed — only the mount point moved.

- [ ] **Step 0.2.4: (b)+(d) Rebase the warp dip off `.content` onto `.channel-view`.** In `_content.scss`, move the `.fx-switching` transform from `.content` to the inner `.channel-view` wrapper:
```scss
.content {
	display: flex;
	flex-direction: column;
	min-width: 0;
	min-height: 0;
	position: relative; // still anchors the streak ::after (now opacity-only)
	// W5/P0 #54: .content is NO LONGER transformed — the dip moved to the
	// inner .channel-view wrapper, so .content never becomes a containing
	// block for fixed descendants. The three former-fixed overlays are now
	// body-level <Portal>s; this comment supersedes the old CONSTRAINT note.

	.channel-view {
		display: flex;
		flex-direction: column;
		flex: 1;
		min-height: 0;
		position: relative;
		transition:
			opacity 180ms ease,
			transform 180ms ease;
		// W5/P0 #54 (d) directional warp: incoming pane slides from the
		// channel-list-index direction. The direction sign is read from a
		// `--warp-dir` custom property. FOUNDATION ships --warp-dir: 0
		// (neutral, non-directional dip). The directional sign (+1 / -1 from
		// the channel-list index) is DEFERRED to Phase 2 (Omloppsbana's swipe
		// strip), where the act layer sets it — see Open Question note below.
		&.fx-switching {
			opacity: 0.6;
			transform: translateX(calc(var(--warp-dir, 0) * 6%)) scale(0.985);
			transition-duration: 70ms;
		}
		@media (prefers-reduced-motion: reduce) {
			&.fx-switching {
				opacity: 1;
				transform: none;
			}
		}
	}
}
```
  **DEFERRAL (clearly marked):** `--warp-dir` ships with the `0` neutral default in Foundation; NO act-layer task sets the sign here. The directional upgrade (act sets `+1` / `-1` from the channel-list index sign) is **deferred to Phase 2 (Omloppsbana swipe-strip)**. The load-bearing Foundation change is removing the transform from `.content`; the `var(--warp-dir, 0)` fallback keeps the dip non-directional until Phase 2. This deferral is called out in the commit message.

- [ ] **Step 0.2.5: (c) Streak replacement — translateX, not background-position.** In `_foundation.scss`, replace the fx-max warp streak (currently animates `background-position` via `fx-warp` — a paint property the #43 lint now forbids; find it via `grep -n "fx-warp\|background-position" style/_foundation.scss`) with a 20%-wide gradient strip child swept via transform:
```scss
// W5/P0 #54(c)+#43: the fx-max warp streak is now a 20%-wide gradient strip
// swept across the pane via transform: translateX (composite-only), under
// overflow:hidden, instead of animating background-position. Lives on
// .channel-view (the transformed pane) now that .content is transform-free.
.app.fx-max .channel-view.fx-switching {
	overflow: hidden;
}
.app.fx-max .channel-view.fx-switching::after {
	content: "";
	position: absolute;
	top: 0;
	bottom: 0;
	left: 0;
	width: 20%;
	z-index: 6;
	pointer-events: none;
	background: linear-gradient(105deg, transparent 0%, var(--glow-accent) 50%, transparent 100%);
	transform: translateX(-100%);
	animation: fx-warp 180ms ease-out forwards;
}
```
  (Open Question #5: the streak deliberately tints with the generic `--glow-accent`; destination per-server accent awaits `guild.accent_color`, which does not exist in `schema.surql` today.)

- [ ] **Step 0.2.6: Rewrite the fx-warp keyframe to translateX.** In `_motion.scss`, replace the `fx-warp` keyframe so it sweeps via transform (now #43-compliant):
```scss
// W5/P0 #54(c): warp streak sweeps a 20%-wide strip across the pane via
// transform (composite-only), replacing the old background-position sweep.
@keyframes fx-warp {
	from {
		transform: translateX(-100%);
		opacity: 0;
	}
	20% {
		opacity: 1;
	}
	to {
		transform: translateX(500%);
		opacity: 0;
	}
}
```
- [ ] **Step 0.2.7: Update the reduced-motion kill list.** In `_motion.scss:243-260`, change the warp `::after` selector from `.app.fx-max .content.fx-switching::after` to `.app.fx-max .channel-view.fx-switching::after` (the streak moved to `.channel-view`). Keep `.radial-menu` in the list (it's now portaled but the selector is class-based, so it still matches).

- [ ] **Step 0.2.8: Re-run the #43 lint.** Run `cargo test --features ssr --test style_lint`. Expect PASS — `fx-warp` no longer contains `background-position`.

- [ ] **Step 0.2.9: Gate + commit.** Full gate (incl. `cargo build --bin authlyn-native --features freya`). Commit:
```
refactor(ui): #54 transform-free .content + body-level portal overlays (W5/P0) (STD)

Relocate the radial menu, lightbox, and mobile emoji sheet to document.body
via Leptos <Portal> (each inside its existing <Show>), so they stop depending
on .content's transform (.content was their containing block + stacking
context while warping). Move the warp dip onto the inner .channel-view wrapper
with a --warp-dir hook (ships neutral 0; the directional sign is DEFERRED to
Phase 2 / Omloppsbana). Convert the fx-max streak from background-position to a
transform-swept gradient strip (#43-compliant), and rebase the reduced-motion
kill selector. Unblocks all three W5 skeletons (which assume transformed panes
and fixed overlays coexisting).

Tests: no_keyframes_animate_paint_or_layout_properties

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 0.3: #20 Etched glass — split the glass mixin (CSS, M)

**Files:** `style/_foundation.scss` (the `glass()` mixin `:9-46`, the `.glass` consumer at the mixin's tail), `style/_tokens.scss` (add `--frost-top` / `--frost-bottom` precomputed tints + a noise data-URI token, in the `:root` block alongside the existing `--glass-*` tokens at `:52-57`), every blur consumer (topbar `_content.scss:70`, `.bottom-tabs`/`.channel-sheet` `_nav.scss`, `.glass` at `_foundation.scss:46`, toast `_toast.scss:53`). Each consumer already has an opaque fallback proving layout survives (`_foundation.scss:10-13`). **Open Question #2** (owner eyeball on the etched look).

- [ ] **Step 0.3.1: Inventory blur consumers.** Run `grep -rln "@include glass\|backdrop-filter\|class=\"glass\"" style/` to enumerate every consumer. Confirmed sites (current tree): `_foundation.scss` (the mixin + `.glass`), `_content.scss` (`.topbar` at `:70`, the fx-max ring drop-shadow), `_nav.scss`, `_toast.scss`.

- [ ] **Step 0.3.2: Add precomputed frost tokens.** In `_tokens.scss`'s `:root` block (after the `--glass-*` tokens at `:52-57`), add the two band tints that bake what `blur(14px)` of the STATIC aurora would produce at the chrome bands, plus the monochrome noise PNG for frost grain. The base64 below is a REAL 32×32 palette PNG (465 raw bytes) generated at plan time (`python3` + Pillow, deterministic seed) — it ships verbatim, no placeholder:
```scss
// W5/P0 #20 etched-glass tints: pre-computed colors approximating what
// blur(14px) of the STATIC aurora yields at the top/bottom chrome bands.
// (Aurora is deterministic in Standard, so the blur readback is wasted work
// on the POCO C3 GE8320 — bake it once instead.) SAMPLE VALUES — Open
// Question #2: owner to eyeball that Standard chrome reads ~90% like blur.
--frost-top: rgba(18, 24, 38, 0.82); // approximates aurora sample @ top band
--frost-bottom: rgba(12, 22, 30, 0.82); // approximates aurora sample @ bottom band
// 32x32 tiled monochrome noise for frost grain (465 bytes, compositor-cheap).
--frost-noise: url("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgBAMAAACBVGfHAAAAMFBMVEWUlJSIiIiAgIB4eHhsbGwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACYtSUTAAABXElEQVR42hWOCYEFIQhAQQ0wHgFEDYBiAFT6Z9q/Ad4B1eWYqb1PTSJchtI9HG67uU7n5QJUMnR7Rvy1+KgABpWK/t3wgas0AP8VFJ8eqnXVCBophnTbIKYW2MAp36y4yPlUuXq4qWTrYKNN1/MlOKPMFL9grev+ZoPCbU6xpBTCtiYQp9ThBc2l01gcOK+rdZaWFqt/EU6u3RkG3gm+GT9AS+JS0ugfeCoTokkBXi98CXSdBMOvwK44x/6kjQFiaT48qaEmON/6YKrP+V1Ps3whCPwq+2i7ujc8i4XAJMmyyr+GdIcCgdmGuq6O3H2HoBeVOO7tBcaHp8K8vk+GljyZIRLQXOuUQO4HtIEXLJ7c7hDwKHPdH8Ka5Sd/hVdFRMA9QItWS84V4wkyk9+tk127fXQH1eOPVug1SVSNMLFBmNMLp827PeCJoxr6EZvG3xn0Hl/LJyJ7J3jwD/CMQuaCdINuAAAAAElFTkSuQmCC");
```
  Generation command (recorded for reproducibility — re-run only to regenerate the grain tile, e.g. on a tone change):
```
python3 - <<'PY'
from PIL import Image
import random, io, base64
random.seed(1138)
levels = [108, 120, 128, 136, 148]
img = Image.new('L', (32, 32))
img.putdata([random.choice(levels) for _ in range(32 * 32)])
pimg = img.convert('P', palette=Image.ADAPTIVE, colors=8)
buf = io.BytesIO()
pimg.save(buf, format='PNG', optimize=True, bits=4)
print(base64.b64encode(buf.getvalue()).decode())
PY
```
  (ImageMagick `convert`/`magick` are absent on the dev Mac; Pillow 11.3 is present, so the PIL path above is the working generator. The `--frost-top`/`--frost-bottom` rgba values are sample values pending the Open Question #2 eyeball.)

- [ ] **Step 0.3.3: Split the mixin into `glass-etched` + `glass-live`.** In `_foundation.scss`, refactor the `glass()` mixin (`:9-46`):
```scss
// W5/P0 #20: the NEW Standard default. Compositor-only — NO backdrop-filter.
// Layered translucent gradient + tiled frost grain + 1px inset specular edge,
// using the pre-baked --frost-* tints. Rasterizes once, scrolls at composite
// cost. fx-max / "Materials: live refraction" escalates to glass-live.
@mixin glass-etched($border: true) {
	background:
		var(--frost-noise) repeat,
		linear-gradient(180deg, var(--frost-top), var(--frost-bottom));
	@if $border {
		border: 1px solid var(--glass-line);
	}
	box-shadow: inset 0 1px 0 var(--glass-highlight);
	// forced-colors still forces Canvas (parity with the old mixin guard).
	@media (forced-colors: active) {
		background: Canvas;
	}
}

// Live refraction — the old behavior, now an escalation (fx-max or the
// "Materials: live refraction" preference). Keeps every a11y guard intact
// (matches the original mixin's fallback-first / @supports / reduced-
// transparency / forced-colors structure at the old _foundation.scss:9-46).
@mixin glass-live($border: true) {
	background: var(--surface);
	@if $border {
		border: 1px solid var(--glass-line);
	}
	box-shadow: inset 0 1px 0 var(--glass-highlight);
	@supports (backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px)) {
		background: var(--glass-bg);
		-webkit-backdrop-filter: blur(14px) saturate(1.4);
		backdrop-filter: blur(14px) saturate(1.4);
	}
	@media (prefers-reduced-transparency: reduce) {
		background: var(--surface);
		-webkit-backdrop-filter: none;
		backdrop-filter: none;
	}
	@media (forced-colors: active) {
		background: Canvas;
		-webkit-backdrop-filter: none;
		backdrop-filter: none;
	}
}

// Back-compat shim: existing `@include glass` callers get etched by default.
@mixin glass($border: true) {
	@include glass-etched($border);
}
```
  (Before pasting `glass-live`, re-read the CURRENT `glass()` mixin at `_foundation.scss:9-46` and copy its EXACT `@supports`/`prefers-reduced-transparency`/`forced-colors` blur values — the `blur(14px) saturate(1.4)` above is the documented value but confirm it matches the live code so the escalation is a true round-trip of the old behavior.)

- [ ] **Step 0.3.4: Wire the fx-max live-refraction escalation.** Add the escalation so fx-max re-enables live blur on the chrome bands (live refraction survives as the toggled escalation, per #20). Place this AFTER the mixin definitions in `_foundation.scss`:
```scss
// W5/P0 #20: under fx-max, the chrome bars escalate back to live refraction.
.app.fx-max .topbar,
.app.fx-max .bottom-tabs,
.app.fx-max .channel-sheet {
	@include glass-live;
}
```
- [ ] **Step 0.3.5: Confirm `.glass` resolves to etched.** The `.glass` class at `_foundation.scss:46` uses `@include glass`, which now maps to `glass-etched` — Standard is compositor-only by default. No consumer-site edits required (the shim preserves call sites); confirm via `grep -n "@include glass" style/*.scss` (every hit still compiles through the shim).

- [ ] **Step 0.3.6: Gate + commit.** Full gate. Commit:
```
feat(style): #20 etched glass — compositor-only chrome default, live refraction as fx-max escalation (W5/P0) (STD)

Split glass() into glass-etched (new Standard default: pre-baked --frost-*
tints + a 465-byte tiled noise PNG + inset specular, NO backdrop-filter) and
glass-live (the old blur, kept for fx-max / Materials escalation). The
backdrop-filter readback over scrolling chat was the single heaviest GPU item
on the POCO C3 GE8320; etched glass rasterizes once and scrolls at composite
cost, ~90% identical (aurora is static/deterministic). A back-compat shim keeps
every @include glass call site working; the opaque/forced-colors fallbacks
survive on both recipes. Frost tints + grain are sample values pending an owner
eyeball (Open Question #2).

Tests: no_keyframes_animate_paint_or_layout_properties (regression: no new keyframe violations)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 0.4: #36 content-visibility rows (CSS, S)

**Files:** `style/_content.scss` (the `.messages` list + `.msg` rows), `style/_skeleton.scss` (confirm `.msg-skeleton` is exempt). The 3-point real-device verification is DEFERRED to the Phase-(7) gate (booked there explicitly — see Step 0.4.3); the CSS lands now.

- [ ] **Step 0.4.1: Add content-visibility to message rows.** In `_content.scss`, on the `.messages > .msg` selector (find via `grep -n "\.messages\b\|\.msg\b" style/_content.scss`):
```scss
// W5/P0 #36: off-screen messages stop laying out/painting. Long channels
// open faster and cost less memory on the POCO C3; browsers auto-pause
// animations in skipped subtrees (off-screen spell-spark loops stop costing
// frames). contain-intrinsic-size keeps the scrollbar stable. Prerequisite
// for affordable per-message ceremony (materialization, scene light) at STD.
.messages > .msg {
	content-visibility: auto;
	contain-intrinsic-size: auto 4.5rem;
}
```
- [ ] **Step 0.4.2: Exempt loading-skeleton rows.** Ensure `.msg-skeleton` rows (always in-viewport during initial load, `_skeleton.scss`) are NOT given `content-visibility: auto`. Confirm the selector `.messages > .msg` does not match `.msg-skeleton` (if skeleton rows carry the `.msg` class, add an explicit `.messages > .msg-skeleton { content-visibility: visible; }` override). Verify via `grep -n "msg-skeleton\|class.*skeleton" src/ui/shell/channel/skeleton.rs`.

- [ ] **Step 0.4.3: Document + BOOK the deferred real-device checks.** Add a comment naming the three Phase-(7) verification points so the gate plan knows what to exercise:
```scss
// W5/P0 #36 PHASE-7 GATE (real-device, booked): content-visibility:auto skips
// off-screen rows, so verify on the live app that (1) reply-quote
// scrollIntoView still forces a skipped target row to render and lands on it,
// (2) near-top history backfill keeps its scroll anchor (no jump), and
// (3) jump-to-unread-on-open scrolls to the right row. These three are part of
// the Phase-7 "W4 a11y backlog / geometry sweep" gate items.
```
  These three checks are formally listed in the Phase-7 gate (see the roadmap's Phase 7 → "content-visibility real-device 3-point check").

- [ ] **Step 0.4.4: Gate + commit.** Full gate. Commit:
```
perf(style): #36 content-visibility on message rows — off-screen messages stop existing (W5/P0) (STD)

.messages > .msg gets content-visibility:auto + contain-intrinsic-size, so
long channels lay out/paint only the viewport (faster open, lower memory on
the POCO C3) and off-screen fx- loops auto-pause. Loading-skeleton rows are
exempt (always in-viewport). The three real-device interaction checks
(reply-quote scrollIntoView, near-top backfill anchor, unread-jump) are booked
for the Phase-7 gate.

Tests: no_keyframes_animate_paint_or_layout_properties (regression)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 0.5: #49 HoloPanel — drag-summoned panel engine: skeleton + signals (Leptos primitive + unit tests, M)

**Files:** `src/ui/shell/holopanel.rs` (NEW — the `HoloPanel` component + the pure drag-math helpers + the `#[cfg(test)]` unit tests), `src/ui/shell/mod.rs` (`pub mod holopanel;`), `style/_holopanel.scss` (NEW), `style/main.scss` (`@use "holopanel";`). Build the ENGINE only; the first real consumer (the sheet rewrite) is Phase 3, the second (Holoterminal panels) is Phase 4. Extract-discipline (#49): do not over-abstract — the engine ships with NO consumers wired in Foundation beyond a smoke mount (Task 0.5b). This task lands the pure math + the component shell; Task 0.5b lands the hydrate pointer listeners + a smoke-mount test.

- [ ] **Step 0.5.1: Define the pure drag-math helpers FIRST (testable without a DOM).** In `holopanel.rs`, factor the gesture decisions into pure functions so they can be unit-tested:
```rust
//! W5/P0 #49 HoloPanel: one drag-summoned panel engine. Pointer drag maps to
//! a 0–1 progress (`--p` custom property; SCSS derives the per-edge
//! transform). Velocity-based commit, scrim coupled to --p, tap-vs-drag
//! disambiguation (7px slop), per-edge safe-area inset ownership, full a11y
//! (focus trap, Esc, restore, role=dialog, reduced-motion = instant snap).
//! Children render touch-clean; desktop opts IN via `desktop_chrome`.
//! Shared UI module — imports ZERO ssr/hydrate-only crates beyond leptos.

/// Which screen edge a panel is summoned from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Tap-vs-drag slop in CSS px: movement under this is a tap, not a drag.
pub const TAP_SLOP_PX: f64 = 7.0;
/// Velocity (progress units per ms) above which a flick commits regardless
/// of absolute progress (Koncept C's threshold).
pub const FLICK_COMMIT_PER_MS: f64 = 0.0015;

/// Map a pointer delta along the panel's open axis to drag progress 0..=1.
/// `delta_px` is signed toward "open"; `extent_px` is the panel's full travel.
pub fn progress_from_delta(delta_px: f64, extent_px: f64) -> f64 {
    if extent_px <= 0.0 {
        return 0.0;
    }
    (delta_px / extent_px).clamp(0.0, 1.0)
}

/// Decide whether a gesture commits to OPEN on release. Commits if past the
/// halfway detent OR if flicked open faster than the velocity threshold.
pub fn commits_open(progress: f64, velocity_per_ms: f64) -> bool {
    progress >= 0.5 || velocity_per_ms >= FLICK_COMMIT_PER_MS
}

/// Tap vs drag: total travel under the slop is a tap (passes through to the
/// child / triggers on_commit toggle), not a drag.
pub fn is_tap(total_travel_px: f64) -> bool {
    total_travel_px < TAP_SLOP_PX
}

/// Scrim opacity is coupled 1:1 to progress, capped at 0.85 (the prototype's
/// max backdrop darkness).
pub fn scrim_opacity(progress: f64) -> f64 {
    progress.clamp(0.0, 1.0) * 0.85
}
```
- [ ] **Step 0.5.2: Define the component shell (signals + view; pointer listeners land in Task 0.5b).** Below the helpers:
```rust
use leptos::prelude::*;

/// A snap detent: a named open fraction (e.g. D1=channels at 0.5, D2=galaxy
/// at 1.0). Kortdäck's sheet uses two; single-detent panels pass one.
#[derive(Clone, Copy, Debug)]
pub struct Detent {
    pub at: f64,
    pub key: &'static str,
}

#[component]
pub fn HoloPanel(
    /// Which edge the panel is summoned from.
    edge: Edge,
    /// One or more snap detents (sorted ascending by `at`).
    detents: Vec<Detent>,
    /// Called when a detent commits, with the committed detent key.
    #[prop(into)] on_commit: Callback<&'static str>,
    /// Desktop opts IN to drag-reorder / hover affordances; touch is clean.
    #[prop(optional)] desktop_chrome: bool,
    children: Children,
) -> impl IntoView {
    // Drag progress drives the `--p` custom property; SCSS derives the
    // per-edge transform. The hydrate pointer handlers (Task 0.5b) own writing
    // `progress`; the view binds it to --p and sets the a11y attributes.
    let progress = RwSignal::new(0.0_f64);
    let edge_class = match edge {
        Edge::Left => "holopanel--left",
        Edge::Right => "holopanel--right",
        Edge::Top => "holopanel--top",
        Edge::Bottom => "holopanel--bottom",
    };
    // `detents`/`on_commit`/`desktop_chrome` are consumed by the hydrate
    // pointer wiring in Task 0.5b; the shell holds them so the signature is
    // final now and 0.5b only fills the listener body.
    let _ = (&detents, &on_commit, desktop_chrome);
    view! {
        <div
            class=format!("holopanel {edge_class}")
            class:holopanel--desktop-chrome=desktop_chrome
            role="dialog"
            aria-modal="true"
            style:--p=move || progress.get().to_string()
        >
            {children()}
        </div>
    }
}
```
- [ ] **Step 0.5.3: SCSS for the engine.** Create `style/_holopanel.scss`:
```scss
// W5/P0 #49 HoloPanel engine styling. The component sets --p (0..1 drag
// progress); each edge derives its transform here. Scrim opacity tracks --p.
// Reduced-motion: instant snap (no transition). Per-edge safe-area inset
// ownership is declared here so consumers don't re-own it.
.holopanel {
	position: fixed;
	z-index: 60;
	transition: transform var(--dur-page) var(--ease-page);
	@media (prefers-reduced-motion: reduce) {
		transition: none; // instant snap
	}
}
.holopanel--left {
	top: 0;
	bottom: 0;
	left: 0;
	padding-left: env(safe-area-inset-left, 0px); // OWNS left edge
	transform: translateX(calc((var(--p) - 1) * 100%));
}
.holopanel--right {
	top: 0;
	bottom: 0;
	right: 0;
	padding-right: env(safe-area-inset-right, 0px); // OWNS right edge
	transform: translateX(calc((1 - var(--p)) * 100%));
}
.holopanel--top {
	left: 0;
	right: 0;
	top: 0;
	padding-top: env(safe-area-inset-top, 0px); // OWNS top edge
	transform: translateY(calc((var(--p) - 1) * 100%));
}
.holopanel--bottom {
	left: 0;
	right: 0;
	bottom: 0;
	padding-bottom: env(safe-area-inset-bottom, 0px); // OWNS bottom edge
	transform: translateY(calc((1 - var(--p)) * 100%));
}
// Scrim: a sibling the consumer renders; opacity driven by --p via the
// component (scrim_opacity()). Declared touch-action:none so dragging the
// panel never pans the page underneath.
.holopanel,
.holopanel-scrim {
	touch-action: none;
}
```
  Register it in `main.scss`: add `@use "holopanel";` after `@use "skeleton";` (chrome-layer position; cascade order preserved).

- [ ] **Step 0.5.4: Write the pure-math unit tests FIRST, run → FAIL.** Add a `#[cfg(test)] mod tests` inside `holopanel.rs` (pure, no DB, runs in any graph):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_clamps_and_scales() {
        assert_eq!(progress_from_delta(0.0, 300.0), 0.0);
        assert_eq!(progress_from_delta(150.0, 300.0), 0.5);
        assert_eq!(progress_from_delta(600.0, 300.0), 1.0); // clamps
        assert_eq!(progress_from_delta(50.0, 0.0), 0.0); // zero extent guard
    }

    #[test]
    fn commit_past_halfway_or_on_flick() {
        assert!(!commits_open(0.49, 0.0));
        assert!(commits_open(0.5, 0.0)); // detent
        assert!(commits_open(0.1, FLICK_COMMIT_PER_MS)); // flick wins low progress
        assert!(!commits_open(0.1, FLICK_COMMIT_PER_MS - 0.0001));
    }

    #[test]
    fn tap_slop_separates_tap_from_drag() {
        assert!(is_tap(0.0));
        assert!(is_tap(TAP_SLOP_PX - 0.01));
        assert!(!is_tap(TAP_SLOP_PX));
        assert!(!is_tap(40.0));
    }

    #[test]
    fn scrim_tracks_progress_capped() {
        assert_eq!(scrim_opacity(0.0), 0.0);
        assert!((scrim_opacity(1.0) - 0.85).abs() < 1e-9);
        assert!((scrim_opacity(2.0) - 0.85).abs() < 1e-9); // clamped
    }
}
```
  Run `cargo test --features ssr holopanel` → FAIL (module not declared yet — `pub mod holopanel;` is added in Step 0.5.5; confirm the red state before declaring done).

- [ ] **Step 0.5.5: Declare the module + run → PASS.** Add `pub mod holopanel;` to `src/ui/shell/mod.rs`'s top-of-file module list (match where shared shell components are declared — confirm via `grep -n "pub mod " src/ui/shell/mod.rs`). Run `cargo test --features ssr holopanel` → PASS (4 tests).

- [ ] **Step 0.5.6: Gate + commit.** Full gate (0 FAILED, incl. 4 holopanel). Commit:
```
feat(ui): #49 HoloPanel engine — drag-summoned panel primitive: math + shell (W5/P0) (STD)

Add the HoloPanel Leptos primitive's pure gesture math (progress_from_delta,
commits_open, is_tap, scrim_opacity — unit-tested), the Edge/Detent types, and
the component shell (signals + --p-bound view + role=dialog). The hydrate
pointer listeners land next (Task 0.5b). Engine only — the sheet rewrite is its
first real consumer (Phase 3), Holoterminal panels the second (Phase 4) per the
extract-at-second-consumer discipline.

Tests: progress_clamps_and_scales, commit_past_halfway_or_on_flick, tap_slop_separates_tap_from_drag, scrim_tracks_progress_capped

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 0.5b: #49 HoloPanel — hydrate pointer listeners + a11y + smoke mount (Leptos hydrate, M)

**Files:** `src/ui/shell/holopanel.rs` (extend the `HoloPanel` component with hydrate-gated pointer wiring + the a11y contract; add a smoke-mount test). **Open Question #6** (confirm web-sys binding names at execution).

- [ ] **Step 0.5b.1: Wire the hydrate pointer listeners.** Extend `HoloPanel` (inside the component body, after `progress` is defined) with a `#[cfg(feature = "hydrate")]` block that attaches the pointer handlers to the panel element via a `node_ref`, mirroring the proven `leptos::ev` / `leptos::web_sys` / `wasm_bindgen::JsCast` pattern in `lightbox.rs:283-291` + `:530` (`set_pointer_capture`). The handler set: `pointerdown` → capture + record start; `pointermove` → update `progress` from `progress_from_delta(delta, extent)`; `pointerup` → `commits_open(progress, velocity)` → snap `progress` to the committed detent's `at` and `on_commit.run(detent.key)`, else snap back to 0; tap (total travel `< TAP_SLOP_PX` via `is_tap`) passes through. Use a `NodeRef` so the listeners attach to the real DOM node:
```rust
use leptos::ev::PointerEvent;
use leptos::html;
let panel_ref = NodeRef::<html::Div>::new();
// ...attach panel_ref to the <div class="holopanel ...">...

#[cfg(feature = "hydrate")]
{
    use leptos::wasm_bindgen::JsCast;
    use leptos::web_sys;
    // Drag bookkeeping in a thread-local-free RwSignal closure (WASM is
    // single-threaded). `extent`/`start`/`last` track the gesture; `detents`
    // is moved in for the commit decision.
    let drag_start = RwSignal::new(None::<(f64, f64)>); // (coord, time_ms)
    let detents_for_commit = detents.clone();

    let on_down = move |ev: PointerEvent| {
        if let Some(el) = panel_ref.get() {
            // Open Question #6: confirm `set_pointer_capture` binding name at
            // execution (proven in lightbox.rs:530).
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        let coord = match edge {
            Edge::Left | Edge::Right => ev.client_x() as f64,
            Edge::Top | Edge::Bottom => ev.client_y() as f64,
        };
        drag_start.set(Some((coord, ev.time_stamp())));
    };

    let on_move = move |ev: PointerEvent| {
        if let Some((start_coord, _)) = drag_start.get() {
            let now = match edge {
                Edge::Left | Edge::Right => ev.client_x() as f64,
                Edge::Top | Edge::Bottom => ev.client_y() as f64,
            };
            // Sign the delta toward "open" per edge (right/bottom open by
            // moving toward the negative direction; left/top by positive).
            let raw = now - start_coord;
            let signed = match edge {
                Edge::Left | Edge::Top => raw,
                Edge::Right | Edge::Bottom => -raw,
            };
            let extent = match edge {
                Edge::Left | Edge::Right => {
                    web_sys::window().and_then(|w| w.inner_width().ok())
                        .and_then(|v| v.as_f64()).unwrap_or(360.0)
                }
                Edge::Top | Edge::Bottom => {
                    web_sys::window().and_then(|w| w.inner_height().ok())
                        .and_then(|v| v.as_f64()).unwrap_or(800.0)
                }
            };
            progress.set(progress_from_delta(signed, extent));
        }
    };

    let on_up = move |ev: PointerEvent| {
        if let Some((start_coord, start_t)) = drag_start.take() {
            let now = match edge {
                Edge::Left | Edge::Right => ev.client_x() as f64,
                Edge::Top | Edge::Bottom => ev.client_y() as f64,
            };
            let raw = now - start_coord;
            let total_travel = raw.abs();
            // Tap: passes through, no commit (the parent toggle handles taps).
            if is_tap(total_travel) {
                progress.set(0.0);
                return;
            }
            let dt = (ev.time_stamp() - start_t).max(1.0);
            let p = progress.get();
            // Velocity in progress-units/ms (rough; precise tuning is P2/P3).
            let velocity = (p / dt).abs();
            if commits_open(p, velocity) {
                // Snap to the nearest detent at/above the current progress.
                let target = detents_for_commit
                    .iter()
                    .copied()
                    .min_by(|a, b| (a.at - p).abs().partial_cmp(&(b.at - p).abs()).unwrap())
                    .unwrap_or(Detent { at: 1.0, key: "open" });
                progress.set(target.at);
                on_commit.run(target.key);
            } else {
                progress.set(0.0);
            }
        }
    };
    // bind on:pointerdown=on_down on:pointermove=on_move on:pointerup=on_up
    // on the panel <div> in the view! below.
}
```
  (Bind the three handlers on the panel `<div>` in the `view!`. The `velocity` here is a rough first cut — precise flick tuning is a Phase-2/3 real-device task, not a Foundation gate.)

- [ ] **Step 0.5b.2: Add the a11y contract (Esc, focus trap, reduced-motion instant snap).** Inside the same hydrate block, add an Esc-to-close `on:keydown` (snap `progress` to 0 on `Escape`), a focus trap (focus the panel on open; on `Tab` at the last focusable child, wrap to the first — mirror the dialog focus handling pattern; if no such helper exists in the tree, keep it minimal: focus the panel root on mount and trap `Tab`/`Shift+Tab` within `panel_ref`). Reduced-motion instant snap is handled by the SCSS (`transition: none` under `prefers-reduced-motion`, Step 0.5.3) — no JS branch needed; add a code comment pointing at that SCSS rule so a future reader does not add a redundant JS path.

- [ ] **Step 0.5b.3: Smoke-mount test (catch an early wiring error).** Add a `#[cfg(test)]` test that mounts `HoloPanel` in a minimal Leptos runtime to prove it renders without panicking (the pure-math tests don't exercise the component). Mirror how existing component tests construct a runtime (search via `grep -rln "create_runtime\|Owner::new\|leptos::ssr\|render_to_string" tests/ src/`); if the tree has no precedent for mounting a `#[component]` in a unit test, instead add a `#[test]` that constructs the `Detent`/`Edge` values the component consumes and asserts the commit-selection logic over a realistic detent set (a behavioral proxy for the listener's snap target):
```rust
#[test]
fn nearest_detent_selection_picks_closest() {
    let detents = [Detent { at: 0.5, key: "d1" }, Detent { at: 1.0, key: "d2" }];
    let pick = |p: f64| {
        detents
            .iter()
            .copied()
            .min_by(|a, b| (a.at - p).abs().partial_cmp(&(b.at - p).abs()).unwrap())
            .unwrap()
            .key
    };
    assert_eq!(pick(0.55), "d1");
    assert_eq!(pick(0.9), "d2");
}
```
  Run `cargo test --features ssr holopanel` → the new test FAILs first if the `nearest_detent` helper logic isn't extracted, then PASSes once the selection matches. (If a true mount test is feasible per the grep, prefer it; otherwise this behavioral proxy is the smoke check.)

- [ ] **Step 0.5b.4: Gate + commit.** Full gate (incl. `cargo clippy --features hydrate --target wasm32-unknown-unknown` for the pointer code). Commit:
```
feat(ui): #49 HoloPanel — hydrate pointer listeners + a11y contract (W5/P0) (STD)

Wire HoloPanel's hydrate pointer handlers (pointerdown→capture,
pointermove→--p progress via progress_from_delta, pointerup→commits_open→snap
to nearest detent + on_commit, tap passthrough via is_tap), the a11y contract
(role=dialog already set; Esc-to-close, focus trap), and rely on the SCSS
reduced-motion instant snap. Mirrors the proven leptos::ev/web_sys/JsCast +
set_pointer_capture pattern from lightbox.rs. Adds a behavioral smoke test for
the detent-selection logic. web-sys once-listener/vibrate surface confirmed at
execution (Open Question #6).

Tests: nearest_detent_selection_picks_closest

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 0.6: #19 Visual Haptics — feedback vocabulary (CSS + hydrate helper, S)

**Files:** `style/_motion.scss` (the `vh-*` keyframes + trigger classes + reduced-motion kill), `src/ui/shell/act/haptics.rs` (NEW — the `vh()` helper + the `KEY_HAPTIC_VIBRATE` toggle), `src/ui/shell/act/mod.rs` (`pub mod haptics;` + re-export), `src/ui/shell/state.rs` (`Prefs.haptic_vibrate`), `src/ui/shell/mod.rs` (init in the constructor + wire the demonstrative consumer), `src/ui/shell/account.rs` (the vibration toggle). Fallback (visual) is the PRIMARY language; `navigator.vibrate` is an enhancement behind a toggle (iOS PWAs have no vibrate). **Open Question #6** (confirm web-sys binding names at execution).

- [ ] **Step 0.6.1: Add the vh-* keyframe family (transform/opacity only — #43-clean).** In `_motion.scss`:
```scss
// W5/P0 #19 Visual Haptics — the app's sound-free, vibration-free feedback
// VOCABULARY. The visual form is PRIMARY (iOS PWAs have no navigator.vibrate);
// Android mirrors each to a designed vibration behind a user toggle, never
// exclusive content. UX-equality: identical feedback MEANING on POCO, iPhone,
// desktop. Every future feature (Initiative turns, Relay Baton, GM tension
// beats) speaks THIS instead of inventing ad hoc. All transform/opacity only.
//
//   vh-tick   — light acknowledge: 450ms radial threshold, effect-chip arm, copy
//   vh-thud   — weighty land: roll result landing, send commit
//   vh-shimmer— received glint: reaction / resonance received
@keyframes vh-tick {
	0% {
		transform: scale(1);
		opacity: 1;
	}
	40% {
		transform: scale(1.02);
		opacity: 0.7;
	}
	100% {
		transform: scale(1);
		opacity: 1;
	}
}
@keyframes vh-thud {
	0% {
		transform: translateY(0);
	}
	45% {
		transform: translateY(2px);
	}
	100% {
		transform: translateY(0);
	}
}
@keyframes vh-shimmer {
	from {
		opacity: 0;
		transform: translateX(-60%);
	}
	50% {
		opacity: 1;
	}
	to {
		opacity: 0;
		transform: translateX(160%);
	}
}
```
- [ ] **Step 0.6.1b: Add the glow-`::before` for vh-tick (pre-rendered glow, #43-clean).** vh-tick's design includes a 1-frame glow blip and vh-thud a shadow deepen; per the #43 house technique these are carried STATICALLY on a pseudo-element and the keyframe breathes opacity — NOT animated via box-shadow. Add the glow layer + an opacity pulse that is consistent with the keyframe (the description and the code must match — glow is static, opacity animates):
```scss
// W5/P0 #19 + #43: vh-tick's glow blip is a STATIC box-shadow on a `::before`
// whose OPACITY pulses (never animate box-shadow). The `.vh-tick` element must
// be position:relative for the `::before` to anchor (set in the class below).
.vh-tick::before {
	content: "";
	position: absolute;
	inset: 0;
	border-radius: inherit;
	pointer-events: none;
	box-shadow: 0 0 10px var(--glow-accent); // STATIC glow
	opacity: 0;
	animation: vh-tick-glow 140ms var(--ease-quick) 1;
}
@keyframes vh-tick-glow {
	0%,
	100% {
		opacity: 0;
	}
	40% {
		opacity: 1;
	}
}
```
- [ ] **Step 0.6.2: Add the trigger classes + the reduced-motion kill in ONE block.** In `_motion.scss`, the classes that consume the keyframes (named `vh-*` so they self-document) AND their `@media (prefers-reduced-motion: reduce)` kill rules, together so a future reader sees the whole contract at once:
```scss
// ---- W5/P0 #19 Visual Haptics trigger classes + reduced-motion kill ----
.vh-tick {
	position: relative; // anchors the glow ::before (Step 0.6.1b)
	animation: vh-tick 140ms var(--ease-quick) 1;
}
.vh-thud {
	animation: vh-thud 180ms var(--ease-quick) 1;
}
.vh-shimmer {
	position: relative;
}
.vh-shimmer::after {
	content: "";
	position: absolute;
	inset: 0;
	pointer-events: none;
	background: linear-gradient(105deg, transparent 40%, var(--glass-highlight) 50%, transparent 60%);
	animation: vh-shimmer 400ms ease-out 1;
}

// MARKED SECTION — reduced-motion kill for the vh-* family. The element-level
// kill does not reach pseudo-elements, so ::before/::after are listed
// explicitly (same reason as the warp ::after note at _motion.scss:222-225).
@media (prefers-reduced-motion: reduce) {
	.vh-tick,
	.vh-tick::before,
	.vh-thud,
	.vh-shimmer,
	.vh-shimmer::after {
		animation: none;
	}
}
```
  (This is a SEPARATE `@media (prefers-reduced-motion: reduce)` block from the existing one at `_motion.scss:243-260`; CSS merges multiple `@media` blocks fine. Alternatively, add the five selectors INTO the existing kill block — either is correct; keeping them together with the classes documents the vh- contract. Pick one and note the choice in the commit.)

- [ ] **Step 0.6.3: Add the `vh()` hydrate helper + vibration toggle pref.** Create `src/ui/shell/act/haptics.rs`:
```rust
//! W5/P0 #19 Visual Haptics helper. Adds a vh-* class to an element and
//! removes it on animationend (so it can re-fire). The visual form is the
//! primary feedback language; where navigator.vibrate exists AND the user
//! enabled the vibration enhancement, mirror to a designed pattern.

#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};

#[cfg(feature = "hydrate")]
const KEY_HAPTIC_VIBRATE: &str = "authlyn.haptic_vibrate";

/// The three feedback kinds in the app's haptic vocabulary.
#[derive(Clone, Copy)]
pub enum Vh {
    /// Light acknowledge (radial threshold, effect-chip arm, copy). 10ms.
    Tick,
    /// Weighty land (roll result, send commit). 20ms.
    Thud,
    /// Received glint (reaction/resonance received). No vibration.
    Shimmer,
}

#[cfg(feature = "hydrate")]
pub fn haptic_vibrate_enabled() -> bool {
    LocalStorage::get::<String>(KEY_HAPTIC_VIBRATE)
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[cfg(feature = "hydrate")]
pub fn set_haptic_vibrate(on: bool) {
    let _ = LocalStorage::set(KEY_HAPTIC_VIBRATE, if on { "1" } else { "0" });
}

/// Fire the visual haptic on `el`; mirror to navigator.vibrate when enabled.
#[cfg(feature = "hydrate")]
pub fn vh(el: &leptos::web_sys::Element, kind: Vh) {
    use leptos::wasm_bindgen::closure::Closure;
    use leptos::wasm_bindgen::JsCast;
    let class = match kind {
        Vh::Tick => "vh-tick",
        Vh::Thud => "vh-thud",
        Vh::Shimmer => "vh-shimmer",
    };
    let _ = el.class_list().add_1(class);
    // Remove on animationend so a repeat trigger re-fires the animation.
    let el2 = el.clone();
    let class_owned = class.to_string();
    let cb = Closure::<dyn FnMut()>::new(move || {
        let _ = el2.class_list().remove_1(&class_owned);
    });
    // Open Question #6: confirm the once-option listener binding against
    // web-sys 0.3.85 at execution. If `AddEventListenerOptions::new().once`
    // / `add_event_listener_with_callback_and_add_event_listener_options`
    // differ, fall back to a plain `add_event_listener_with_callback` + manual
    // removeEventListener inside the closure (the pattern radial.rs uses).
    let opts = leptos::web_sys::AddEventListenerOptions::new();
    opts.set_once(true);
    let _ = el.add_event_listener_with_callback_and_add_event_listener_options(
        "animationend",
        cb.as_ref().unchecked_ref(),
        &opts,
    );
    cb.forget();
    // Enhancement: mirror to a designed vibration pattern where supported.
    if haptic_vibrate_enabled() {
        if let Some(win) = leptos::web_sys::window() {
            let nav = win.navigator();
            let ms = match kind {
                Vh::Tick => 10,
                Vh::Thud => 20,
                Vh::Shimmer => return, // shimmer has no vibration
            };
            // Open Question #6: `vibrate_with_duration` is the documented
            // web-sys binding; confirm at execution (not yet used in-tree).
            let _ = nav.vibrate_with_duration(ms);
        }
    }
}

// ---- ssr stubs ----
#[cfg(not(feature = "hydrate"))]
pub fn haptic_vibrate_enabled() -> bool {
    false
}
#[cfg(not(feature = "hydrate"))]
pub fn set_haptic_vibrate(_on: bool) {}
```
  (`vibrate_with_duration` takes `u32` in web-sys; the `10`/`20` literals are `i32` by default, so they are written as integer literals here. Confirm the exact `AddEventListenerOptions` builder shape — newer web-sys uses `set_once(true)` on a value, older uses `.once(true)` returning `&mut Self`; match what 0.3.85 exposes, and if neither matches cleanly, use the plain `add_event_listener_with_callback` + in-closure `remove_event_listener_with_callback` fallback as the comment says.)

- [ ] **Step 0.6.4: Module + re-export.** Add `pub mod haptics;` to `act/mod.rs` (in the module list at `:34-48`) and re-export by adding a `pub use haptics::{haptic_vibrate_enabled, set_haptic_vibrate, vh, Vh};` line near the existing `pub use prefs::{…}` block (`:107-110`).

- [ ] **Step 0.6.5: Add the `Prefs.haptic_vibrate` field + init it BEFORE wiring the consumer.** Order matters — define the signal and its constructor init FIRST, then the account-modal toggle can reference it.
  - **(a)** In `state.rs`, add to the `Prefs` struct (after `ghost_quill`):
```rust
/// W5/P0 #19: whether to mirror visual haptics to navigator.vibrate where
/// supported (Android). Default OFF; visual feedback is always primary.
/// Persisted to localStorage as authlyn.haptic_vibrate.
pub(crate) haptic_vibrate: RwSignal<bool>,
```
  - **(b)** In `mod.rs`, in the `prefs = Prefs { … }` constructor block (`:219-224`), add the init line (mirroring `eyecandy`):
```rust
let prefs = Prefs {
    dialogue_style: RwSignal::new(act::rp_dialogue_style_enabled()),
    eyecandy: RwSignal::new(act::eyecandy_enabled()),
    ghost_quill: RwSignal::new(act::ghost_quill_enabled()),
    haptic_vibrate: RwSignal::new(act::haptic_vibrate_enabled()),
};
```
  - **(c)** Now (the signal exists) wire the account-modal toggle in `account.rs`, in the Preferences `<section class="account-section">` after the Ghost Quill `<label class="pref-row">` (`:208-216`), mirroring the eyecandy pattern at `:195-203`:
```rust
<label class="pref-row">
    <input type="checkbox" prop:checked=move || s.prefs.haptic_vibrate.get()
        on:change=move |ev| {
            let on = event_target_checked(&ev);
            s.prefs.haptic_vibrate.set(on);
            act::set_haptic_vibrate(on);
        }/>
    <span>"Vibration feedback (where supported)"</span>
</label>
```

- [ ] **Step 0.6.6: Wire one demonstrative consumer (send-commit → vh-thud).** Fire `vh-thud` on the existing send-confirm path so the vocabulary has a live first consumer. The send-confirm already flips `Composer.sent` for ~400ms (`act::send_message`, the `.send.sent` CSS pulse). In the hydrate send path, after the successful dispatch where `sent` is flipped, also fire the visual haptic on the Send button element (resolve it via the same `NodeRef`/`web_sys` lookup the send path already uses; if the Send button has no `NodeRef`, add one and pass its `Element` to `act::vh(&el, act::Vh::Thud)`). Keep it `#[cfg(feature = "hydrate")]`. (This is reinforcement of the existing pulse, not a replacement — the `.send.sent` glow stays.)

- [ ] **Step 0.6.7: Re-run the #43 lint.** `cargo test --features ssr --test style_lint` → PASS (vh-* + vh-tick-glow keyframes are transform/opacity only).

- [ ] **Step 0.6.8: Gate + commit.** Full gate. Commit:
```
feat(ui): #19 visual haptics — sound-free, vibration-free feedback vocabulary (W5/P0) (STD)

Add the vh-tick / vh-thud / vh-shimmer SCSS family (transform/opacity only,
#43-clean with a static-glow ::before for vh-tick, reduced-motion-killed in a
marked block) as the app's haptic CONTRACT, with a tiny vh(el, kind) hydrate
helper (add class + animationend cleanup) and a Prefs.haptic_vibrate toggle.
The visual form is PRIMARY (iOS PWAs have no navigator.vibrate); the toggle
mirrors tick=10ms / thud=20ms to a designed vibration on Android —
reinforcement, never exclusive content. Wire send-commit to vh-thud as the
first consumer. UX-equality: identical feedback meaning on POCO, iPhone,
desktop.

Tests: no_keyframes_animate_paint_or_layout_properties (regression: vh-* clean)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

## Phase 1 — Theme-switch Infrastructure + Ceremony Machinery

### Task 1.1: `authlyn.skeleton` client-local pref (hydrate + ssr stub) (data/pref, STD)

**Files:** `src/ui/shell/act/prefs.rs` (add `KEY_SKELETON` + helpers, mirroring the `KEY_EYECANDY` block at `:59-73` and the ssr stubs at `:120-125`), `src/ui/shell/act/mod.rs` (extend the `pub use prefs::{…}` re-export at `:107-110`), `tests/skeleton_switch.rs` (NEW — validates the id-validation helper).

- [ ] **Step 1.1.1: Define the valid skeleton ids as a const (shared hydrate + ssr).** In `prefs.rs`, before the hydrate helpers (un-gated so validation is reachable in the ssr test graph):
```rust
// W5/P1: the three structural UI skeletons (spec §1). `sk-`-prefixed in code
// (.app.sk-*, _sk_*.scss, sk_*/mod.rs); the stored pref value is the bare id
// WITHOUT the `sk-` prefix (orbit/deck/hud). NO silent default — a pref-less
// device gets the onboarding ceremony, except the localStorage-unavailable
// fallback which boots orbit for the session.
pub const SKELETON_IDS: &[&str] = &["orbit", "deck", "hud"];
/// The session fallback when localStorage cannot persist (private mode etc.).
pub const SKELETON_FALLBACK: &str = "orbit";

/// Validate a stored/selected skeleton id; unknown ids are rejected so a
/// stale or corrupt localStorage value can never apply a bogus root class.
pub fn is_valid_skeleton(id: &str) -> bool {
    SKELETON_IDS.contains(&id)
}
```
- [ ] **Step 1.1.2: Add the hydrate read/write helpers.** Mirror the eyecandy pattern (gloo-storage JSON String):
```rust
#[cfg(feature = "hydrate")]
const KEY_SKELETON: &str = "authlyn.skeleton";

/// The persisted skeleton id, if any AND valid. `None` means pref-less →
/// the caller runs the onboarding ceremony (no silent default).
#[cfg(feature = "hydrate")]
pub fn skeleton_pref() -> Option<String> {
    LocalStorage::get::<String>(KEY_SKELETON)
        .ok()
        .filter(|id| is_valid_skeleton(id))
}

/// Persist the chosen skeleton id. Returns false if the write failed
/// (localStorage unavailable) so the caller can fall back to a session-only
/// `orbit` without claiming it was saved.
#[cfg(feature = "hydrate")]
pub fn set_skeleton(id: &str) -> bool {
    if !is_valid_skeleton(id) {
        return false;
    }
    LocalStorage::set(KEY_SKELETON, id).is_ok()
}

/// Remove the stored skeleton pref (used by the ceremony's writability probe
/// to leave no committed value — see Task 1.3, "no silent default").
#[cfg(feature = "hydrate")]
pub fn clear_skeleton() {
    LocalStorage::delete(KEY_SKELETON);
}

/// The throwaway probe key the ceremony uses to detect localStorage
/// writability WITHOUT touching authlyn.skeleton (Open Question #3).
#[cfg(feature = "hydrate")]
const KEY_PREF_PROBE: &str = "_authlyn_pref_test";

/// True if localStorage can be written. Sets then deletes a throwaway key so
/// it never leaves a side effect on the real skeleton pref. A failed write
/// (private mode / quota / disabled) returns false → session fallback.
#[cfg(feature = "hydrate")]
pub fn local_storage_writable() -> bool {
    let ok = LocalStorage::set(KEY_PREF_PROBE, "1").is_ok();
    LocalStorage::delete(KEY_PREF_PROBE);
    ok
}
```
- [ ] **Step 1.1.3: Add the ssr stubs.** In the ssr-stub block (after the existing `set_ghost_quill` stub at `:132`):
```rust
#[cfg(not(feature = "hydrate"))]
pub fn skeleton_pref() -> Option<String> {
    None
}
#[cfg(not(feature = "hydrate"))]
pub fn set_skeleton(_id: &str) -> bool {
    false
}
#[cfg(not(feature = "hydrate"))]
pub fn clear_skeleton() {}
#[cfg(not(feature = "hydrate"))]
pub fn local_storage_writable() -> bool {
    false
}
```
- [ ] **Step 1.1.4: Re-export.** Extend the `pub use prefs::{…}` block at `act/mod.rs:107-110` to add: `clear_skeleton, is_valid_skeleton, local_storage_writable, set_skeleton, skeleton_pref, SKELETON_FALLBACK, SKELETON_IDS`.

- [ ] **Step 1.1.5: Write the validation test FIRST.** Create `tests/skeleton_switch.rs`. Reach the items through the `act` re-export (the integration test is an external crate consumer, so it must use a `pub` path — `act::prefs` is reachable because `ui`, `shell`, `act`, and `prefs` are all `pub mod`, and the items are `pub`; if any link in that chain is not `pub`, the Step 1.1.4 `pub use` on `act/mod.rs` exposes them as `act::is_valid_skeleton` etc., which the test then uses):
```rust
//! W5/P1 theme-switch guards. The skeleton-id validation runs in the ssr
//! graph (no DB needed). The switch-invariant assertions (SSE/composer/
//! selection preserved) are documented here as the Phase-7 gate contract.

// Reached via the act re-export (Step 1.1.4); this is the stable public path.
use authlyn_interactive::ui::shell::act::{
    is_valid_skeleton, SKELETON_FALLBACK, SKELETON_IDS,
};

#[test]
fn skeleton_ids_are_exactly_the_three_shells() {
    assert_eq!(SKELETON_IDS, &["orbit", "deck", "hud"]);
}

#[test]
fn valid_skeleton_accepts_known_rejects_unknown() {
    assert!(is_valid_skeleton("orbit"));
    assert!(is_valid_skeleton("deck"));
    assert!(is_valid_skeleton("hud"));
    assert!(!is_valid_skeleton("sk-orbit")); // stored value is bare id, no prefix
    assert!(!is_valid_skeleton("ritual")); // stale/legacy name rejected
    assert!(!is_valid_skeleton(""));
}

#[test]
fn fallback_is_a_valid_id() {
    assert!(is_valid_skeleton(SKELETON_FALLBACK));
    assert_eq!(SKELETON_FALLBACK, "orbit");
}
```
- [ ] **Step 1.1.5a: Confirm the test COMPILES (reachability gate).** Run `cargo test --features ssr --test skeleton_switch --no-run`. If it fails to compile because an item is unreachable, the fix is a concrete re-export line on `act/mod.rs` — add the exact line:
```rust
pub use prefs::{
    clear_skeleton, is_valid_skeleton, local_storage_writable, set_skeleton, skeleton_pref,
    SKELETON_FALLBACK, SKELETON_IDS,
};
```
  (append to / merge with the existing `pub use prefs::{…}` block at `:107-110`). The crate name is `authlyn-interactive`, so the import path is `authlyn_interactive::ui::shell::act::{…}`. Re-run `--no-run` until it compiles.

- [ ] **Step 1.1.6: Run → FAIL then PASS.** Before T1.1.1-1.1.4 land, `cargo test --features ssr --test skeleton_switch` FAILs (items not defined/reachable). After they land, run `cargo test --features ssr --test skeleton_switch` → PASS (3 tests).

- [ ] **Step 1.1.7: Gate + commit.** Full gate. Commit:
```
feat(ui): authlyn.skeleton client-local pref — three sk-* shell ids (W5/P1) (STD)

Add the authlyn.skeleton localStorage pref mirroring the fx-max pattern
(gloo-storage JSON String, hydrate-real + ssr stub). Stored value is the bare
id (orbit/deck/hud); is_valid_skeleton rejects stale/prefixed/unknown values so
a corrupt pref can never apply a bogus root class. set_skeleton returns false
when the write fails, letting the caller fall back to a session-only orbit.
Adds clear_skeleton + a local_storage_writable probe (throwaway key) for the
ceremony's no-silent-default detection. skeleton_pref()==None signals the
onboarding ceremony.

Tests: skeleton_ids_are_exactly_the_three_shells, valid_skeleton_accepts_known_rejects_unknown, fallback_is_a_valid_id

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 1.2: `Prefs.skeleton` signal + root-class application (state/render, STD)

**Files:** `src/ui/shell/state.rs` (`Prefs` struct — add `skeleton`), `src/ui/shell/mod.rs` (`AppShell` constructor `:219-224` — init; root render `:389` — apply `.app.sk-*`).

- [ ] **Step 1.2.1: Add the signal to `Prefs`.** In `state.rs`, after `haptic_vibrate` (from Task 0.6):
```rust
/// W5/P1: the selected structural UI skeleton id (orbit/deck/hud). Drives
/// the `.app.sk-*` root class. `None` until the ceremony resolves (pref-less
/// first run); the render treats `None` as "no sk-* class yet" while the
/// ceremony modal is up. Persisted to localStorage as authlyn.skeleton.
pub(crate) skeleton: RwSignal<Option<String>>,
```
- [ ] **Step 1.2.2: Init in the constructor.** In `mod.rs`, in the `prefs = Prefs { … }` block (`:219-224`):
```rust
let prefs = Prefs {
    dialogue_style: RwSignal::new(act::rp_dialogue_style_enabled()),
    eyecandy: RwSignal::new(act::eyecandy_enabled()),
    ghost_quill: RwSignal::new(act::ghost_quill_enabled()),
    haptic_vibrate: RwSignal::new(act::haptic_vibrate_enabled()), // from T0.6
    // W5/P1: read the stored pref; None ⇒ ceremony decides (T1.3). The
    // ceremony Effect (T1.3) sets this; we do NOT default here (no silent
    // default). The localStorage-unavailable fallback is applied by the
    // ceremony Effect, not here.
    skeleton: RwSignal::new(act::skeleton_pref()),
};
```
- [ ] **Step 1.2.3: Apply the root class — lifted ABOVE any skeleton branch.** At `mod.rs:389`, extend the root `<div class="app" …>` so the `sk-*` class is driven by the signal (same root that already carries `dialogue-style` + `fx-max`, so switching never remounts the shell — the invariant):
```rust
<div class="app"
    class:dialogue-style=move || s.prefs.dialogue_style.get()
    class:fx-max=move || s.prefs.eyecandy.get()
    class:sk-orbit=move || s.prefs.skeleton.get().as_deref() == Some("orbit")
    class:sk-deck=move || s.prefs.skeleton.get().as_deref() == Some("deck")
    class:sk-hud=move || s.prefs.skeleton.get().as_deref() == Some("hud")
>
```
  (While `skeleton` is `None` — ceremony up — NO `sk-*` class applies; the W3 scaffolding (Task 1.5) renders underneath the ceremony modal. After the ceremony or fallback resolves, exactly one `sk-*` class applies.)

- [ ] **Step 1.2.4: Verify it compiles + builds.** `cargo clippy --features hydrate --target wasm32-unknown-unknown`; `cargo leptos build --release`. (No new test here — the switch-invariant test lands in Task 1.6.)

- [ ] **Step 1.2.5: Gate + commit.** Full gate. Commit:
```
feat(ui): .app.sk-* root class driven by Prefs.skeleton signal (W5/P1) (STD)

Add Prefs.skeleton (RwSignal<Option<String>>) and apply exactly one of
.app.sk-orbit / .app.sk-deck / .app.sk-hud to the same root .app div that
carries fx-max — so switching the skeleton flips one class on a stable node
and never remounts AppShell (the SSE/composer/selection-preservation
invariant). None (ceremony pending) applies no sk-* class.

Tests: skeleton_ids_are_exactly_the_three_shells (regression)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 1.3: Onboarding ceremony machinery (pref-less three-way choice + fallback) (Leptos/CSS, STD)

**Files:** `src/ui/shell/ceremony.rs` (NEW — the `SkeletonCeremony` modal component), `src/ui/shell/mod.rs` (`pub mod ceremony;` + mount the ceremony in `AppShell` + the resolve Effect after `provide_context(s)` at `:245`), `style/_ceremony.scss` (NEW), `style/main.scss` (`@use "ceremony";`). The ceremony is the spec's "no silent default" mechanism: pref-less → three-way choice at first authenticated shell mount; covers new users AND existing users post-update. **Open Question #3** (writability-detection approach). **All ceremony prose is ENGLISH; the three skeleton THEME NAMES (Omloppsbana / Kortdäck / Holoterminal) stay as proper nouns.**

- [ ] **Step 1.3.1: Build the ceremony component.** Create `src/ui/shell/ceremony.rs`:
```rust
//! W5/P1 onboarding ceremony: the first-run three-way skeleton choice. NO
//! silent default — a pref-less device sees this at first authenticated shell
//! mount (new users AND existing users post-update). Selecting a skeleton
//! persists the pref and sets Prefs.skeleton; the ceremony then dismisses.
//! The localStorage-unavailable fallback is handled in AppShell (boots orbit
//! for the session WITHOUT showing this), per spec §1.

use crate::ui::shell::act;
use crate::ui::shell::state::Shell;
use leptos::prelude::*;

/// One selectable skeleton card in the ceremony. `title` is the canon theme
/// proper-noun (kept as-is); `blurb` is English UI copy.
struct SkChoice {
    id: &'static str,
    title: &'static str,
    blurb: &'static str,
}

const CHOICES: &[SkChoice] = &[
    SkChoice {
        id: "orbit",
        title: "Omloppsbana",
        blurb: "Spatial — channels orbit your server; swipe between worlds.",
    },
    SkChoice {
        id: "deck",
        title: "Kortdäck",
        blurb: "Layered — scrub through a deck of channels and servers.",
    },
    SkChoice {
        id: "hud",
        title: "Holoterminal",
        blurb: "Zero-chrome — the stream alone; summon panels from the edges.",
    },
];

#[component]
pub fn SkeletonCeremony(s: Shell) -> impl IntoView {
    let choose = move |id: &'static str| {
        // Persist + apply. If the write somehow fails here the session still
        // gets the chosen class (the signal is set regardless); the ceremony
        // only ever shows when local_storage_writable() returned true.
        let _saved = act::set_skeleton(id);
        s.prefs.skeleton.set(Some(id.to_string()));
    };
    view! {
        <div class="sk-ceremony-scrim" role="dialog" aria-modal="true" aria-label="Choose your interface">
            <div class="sk-ceremony">
                <h2>"Choose your interface"</h2>
                <p class="muted">"You can change this any time in Account \u{2192} Preferences."</p>
                <div class="sk-ceremony-cards">
                    {CHOICES.iter().map(|c| {
                        let id = c.id;
                        view! {
                            <button class=format!("sk-ceremony-card sk-pick-{id}")
                                on:click=move |_| choose(id)>
                                <span class="sk-ceremony-title">{c.title}</span>
                                <span class="sk-ceremony-blurb">{c.blurb}</span>
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>
        </div>
    }
}
```
- [ ] **Step 1.3.2: Mount the ceremony + resolve the fallback in `AppShell` (probe-and-clear, no silent default).** In `mod.rs`, after `provide_context(s)` (`:245`) add a hydrate-side resolve Effect and render the ceremony conditionally. The writability detection uses the dedicated throwaway probe (`local_storage_writable`, Task 1.1) — it NEVER touches `authlyn.skeleton`, so a working-storage device that has not yet chosen stays `None` (the ceremony shows; no silent default). Only a NON-writable device gets the session `orbit` fallback:
```rust
// W5/P1 ceremony resolve: on first authenticated mount, if there's no stored
// skeleton pref, decide between (a) showing the ceremony (localStorage works)
// or (b) the silent session-only `orbit` fallback (localStorage unavailable —
// spec §1). Writability is detected with a DEDICATED throwaway probe key
// (_authlyn_pref_test, set+delete) so we never write authlyn.skeleton here —
// keeping the "no silent default" promise: a writable device that hasn't
// chosen stays None and sees the ceremony; only a non-writable device falls
// back to a session-only orbit without ceremony. (Open Question #3: the owner
// may prefer a cleaner detection.)
Effect::new(move |_| {
    if s.prefs.skeleton.get_untracked().is_none() {
        if act::local_storage_writable() {
            // We CAN persist → genuine pref-less device. skeleton stays None,
            // so the ceremony renders. No value is written until a real choice.
        } else {
            // localStorage unavailable: session-only fallback, no ceremony.
            s.prefs.skeleton.set(Some(act::SKELETON_FALLBACK.to_string()));
        }
    }
});
```
  And in the `view!`, render the ceremony when `skeleton` is `None`:
```rust
<Show when=move || s.prefs.skeleton.get().is_none()>
    <crate::ui::shell::ceremony::SkeletonCeremony s=s/>
</Show>
```
- [ ] **Step 1.3.3: Module declaration.** Add `pub mod ceremony;` to `src/ui/shell/mod.rs`'s top-of-file module list (confirm placement via `grep -n "pub mod " src/ui/shell/mod.rs`).

- [ ] **Step 1.3.4: Ceremony styling — skeleton-neutral overlay.** Create `style/_ceremony.scss`. The first two lines pull the token vars + the `glass-etched` mixin (the partial must declare its own `@use`s — `@include glass-etched` won't resolve otherwise; the module names are `tokens` and `foundation`, matching the existing pattern in `_content.scss:1,5` / `_toast.scss:1,5`):
```scss
@use "tokens" as *;
@use "foundation" as *;
// W5/P1 onboarding ceremony — skeleton-NEUTRAL (it renders before any sk-*
// class applies). Full-viewport scrim + a centered card with the three
// choices. Mobile-first: cards stack on narrow, flow to a row on wide. No
// hardcoded 375-math (clamp/%/dvh per UX-equality).
.sk-ceremony-scrim {
	position: fixed;
	inset: 0;
	z-index: 80;
	display: flex;
	align-items: center;
	justify-content: center;
	padding: clamp(1rem, 5vw, 2.5rem);
	padding-top: calc(clamp(1rem, 5vw, 2.5rem) + env(safe-area-inset-top, 0px));
	padding-bottom: calc(clamp(1rem, 5vw, 2.5rem) + env(safe-area-inset-bottom, 0px));
	background: var(--scrim);
}
.sk-ceremony {
	@include glass-etched; // from T0.3
	border-radius: 14px;
	padding: clamp(1rem, 4vw, 2rem);
	max-width: min(92vw, 40rem);
	width: 100%;
}
.sk-ceremony-cards {
	display: flex;
	flex-direction: column; // mobile-first: stacked
	gap: 0.75rem;
	margin-top: 1rem;
}
@media (min-width: 600px) {
	.sk-ceremony-cards {
		flex-direction: row;
	}
}
.sk-ceremony-card {
	flex: 1;
	display: flex;
	flex-direction: column;
	gap: 0.35rem;
	text-align: left;
	padding: clamp(0.75rem, 3vw, 1.1rem);
	min-height: 44px; // touch target floor
	border: 1px solid var(--line-strong);
	border-radius: 10px;
	background: var(--card);
}
.sk-ceremony-title {
	font-family: var(--font-display);
	color: var(--text);
}
.sk-ceremony-blurb {
	font-family: var(--font-prose);
	color: var(--text-muted);
	font-size: 0.9rem;
}
```
  Register in `main.scss`: add `@use "ceremony";` near the modal-layer partials (after `@use "modal";` at `:21`).

- [ ] **Step 1.3.5: Add a test pinning the probe semantics (no silent default).** Extend `tests/skeleton_switch.rs` with an ssr-graph test that pins the probe contract via the stub behavior (the ssr stub of `local_storage_writable` returns `false`, and `skeleton_pref` returns `None` — proving the API shape the ceremony depends on). The "writable → stays None" and "non-writable → orbit fallback" behavior is hydrate-side; the ssr test pins that the surface is exactly these pure functions:
```rust
use authlyn_interactive::ui::shell::act::{local_storage_writable, skeleton_pref};

#[test]
fn ssr_stubs_signal_no_pref_and_no_storage() {
    // On the server there is no localStorage: skeleton_pref() is None (so the
    // ceremony would show on a writable client) and local_storage_writable()
    // is false (so the server never claims a writable store). This pins the
    // surface the ceremony's no-silent-default branch depends on; the live
    // writable→None / non-writable→orbit behavior is a Phase-7 device check.
    assert_eq!(skeleton_pref(), None);
    assert!(!local_storage_writable());
}
```
  (This is an ssr-graph assertion — the hydrate `local_storage_writable` probe-and-clear behavior is covered by the live ceremony smoke in Step 1.3.6 and booked into Phase 7.)

- [ ] **Step 1.3.6: Verify build + smoke.** `cargo clippy --features hydrate --target wasm32-unknown-unknown`; `cargo leptos build --release`; `cargo test --features ssr --test skeleton_switch` → PASS. Visual smoke (localhost:3000, dev DB, NEVER prod): clear `authlyn.skeleton` in devtools, reload an authenticated session → ceremony appears; pick each card → root gets the matching `.app.sk-*` (inspect element); reload → ceremony gone (pref persisted). Then verify private-mode/blocked-storage shows NO ceremony and boots orbit.

- [ ] **Step 1.3.7: Gate + commit.** Full gate. Commit:
```
feat(ui): onboarding skeleton ceremony — no-silent-default three-way choice (W5/P1) (STD)

Add SkeletonCeremony: a skeleton-neutral, mobile-first three-card chooser
(Omloppsbana/Kortdäck/Holoterminal — theme proper-nouns; all surrounding copy
English) shown at first authenticated mount when there is no stored
authlyn.skeleton pref, covering new users AND existing users post-update. A
DEDICATED throwaway probe key (_authlyn_pref_test, set+delete) detects
localStorage writability WITHOUT touching authlyn.skeleton, so a writable
pref-less device sees the ceremony (no silent default) and only a
localStorage-unavailable device takes the session-only orbit fallback (Open
Question #3). Selecting a card persists the pref and sets the root class.

Tests: ssr_stubs_signal_no_pref_and_no_storage, fallback_is_a_valid_id (regression)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 1.4: Account-modal skeleton picker (UI, STD)

**Files:** `src/ui/shell/account.rs` (Preferences section `:184-226` — add the radio group, after the haptic toggle from Task 0.6, mirroring the eyecandy `<label class="pref-row">` at `:195-203`).

- [ ] **Step 1.4.1: Add the radio group.** In `account.rs`, in the Preferences `<section class="account-section">`, after the haptic-vibrate label (Task 0.6) and before the version `<p>`, add a labeled radio group bound to `s.prefs.skeleton`. Selecting persists immediately + updates the signal (no ceremony — explicit change). All copy English; theme proper-nouns kept:
```rust
<h4 class="pref-subhead">"Interface skeleton"</h4>
<label class="pref-row">
    <input type="radio" name="skeleton" value="orbit"
        prop:checked=move || s.prefs.skeleton.get().as_deref() == Some("orbit")
        on:change=move |_| { act::set_skeleton("orbit"); s.prefs.skeleton.set(Some("orbit".to_string())); }/>
    <span>"Omloppsbana — spatial, swipe between worlds"</span>
</label>
<label class="pref-row">
    <input type="radio" name="skeleton" value="deck"
        prop:checked=move || s.prefs.skeleton.get().as_deref() == Some("deck")
        on:change=move |_| { act::set_skeleton("deck"); s.prefs.skeleton.set(Some("deck".to_string())); }/>
    <span>"Kortdäck — layered deck scrub"</span>
</label>
<label class="pref-row">
    <input type="radio" name="skeleton" value="hud"
        prop:checked=move || s.prefs.skeleton.get().as_deref() == Some("hud")
        on:change=move |_| { act::set_skeleton("hud"); s.prefs.skeleton.set(Some("hud".to_string())); }/>
    <span>"Holoterminal — zero-chrome, edge panels"</span>
</label>
```
- [ ] **Step 1.4.2: Verify build + smoke.** `cargo clippy --features hydrate --target wasm32-unknown-unknown`; `cargo leptos build --release`. Smoke (localhost): open Account modal → switch each radio → root `.app.sk-*` flips live without remount (the message stream, composer text, and selected channel must NOT reset — that's the Task 1.6 invariant, eyeballed here).

- [ ] **Step 1.4.3: Gate + commit.** Full gate. Commit:
```
feat(ui): account-modal skeleton picker — switch interface live (W5/P1) (STD)

Add a three-way radio group to the account modal Preferences section,
mirroring the eyecandy pref-row pattern (theme proper-nouns; English copy).
Selecting persists authlyn.skeleton and updates Prefs.skeleton, flipping the
.app.sk-* root class live (no ceremony — explicit change). Lets existing users
re-choose without waiting for a fresh first-run ceremony.

Tests: valid_skeleton_accepts_known_rejects_unknown (regression)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 1.5: W3-shell scaffolding retained (no-op verification, STD)

**Files:** `src/ui/shell/mod.rs` (a code comment at `:389`; no behavior change). Verifies the W3 shell still renders under a resolved skeleton class so the switch is provable mid-wave.

- [ ] **Step 1.5.1: Confirm W3 chrome survives.** With a skeleton selected (e.g. `orbit`), the W3 rail/sidebar/bottom-tabs still render — none of the `sk-*` SCSS exists yet, so the `.app.sk-orbit` class is currently a no-op selector and the W3 chrome shows through (confirm the W3 chrome render points via `grep -n "rail\|sidebar\|bottom-tabs" src/ui/shell/mod.rs`). This is the in-wave-only "no silent default" exception (spec §1, Phase 1): a pref-less dev build that took the localStorage-unavailable fallback boots W3 under `.app.sk-orbit`.

- [ ] **Step 1.5.2: Add the comment + commit (annotation only).** Add at `mod.rs:389` (above the root `<div class="app" …>`):
```rust
// W5/P1 NOTE: the W3 rail/sidebar/bottom-tabs below are SCAFFOLDING retained
// to prove the skeleton switch mid-wave. They render under any .app.sk-* class
// until that skeleton's _sk_*.scss + sk_*/mod.rs land (Phases 2-4). DELETE all
// W3 chrome + _layout/_rail/_sidebar/_nav/_mobile partials in Phase 6
// (retirement). This is the acknowledged in-wave-only "no silent default"
// exception (spec §1): a localStorage-unavailable dev build boots orbit→W3.
```
  Commit (annotation under `src/`, no behavior change):
```
docs(ui): mark W3 shell as W5 scaffolding pending Phase-6 retirement (W5/P1)

Annotate the root render so the retained W3 rail/sidebar/bottom-tabs are
unmistakably scaffolding (the in-wave-only no-silent-default exception), to be
deleted with _layout/_rail/_sidebar/_nav/_mobile in Phase 6.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

### Task 1.6: Theme-switch state invariants — structural guard + Phase-7 booking (test, STD)

**Files:** `tests/skeleton_switch.rs` (extend with the switch-invariant contract). The §13 invariant is "switch never drops SSE / composer draft / selection". The structural guarantee is: the switch flips one class on the stable `.app` root (Task 1.2.3) and never remounts `AppShell`. Since the integration harness is ssr/DB-side and cannot drive WASM signals, this task pins the STRUCTURAL invariant and books the live SSE/composer/selection check as a Phase-7 gate item. **Open Question #4** (whether to automate the live check via headed Playwright now).

- [ ] **Step 1.6.1: Add the contract assertions.** Extend `tests/skeleton_switch.rs`:
```rust
use authlyn_interactive::ui::shell::act::set_skeleton;

/// W5/P1 §13 invariant contract: switching the skeleton must NOT remount the
/// shell. The structural guarantee is that the skeleton lives on the same
/// stable Prefs aggregate / same .app root that carries fx-max (which we
/// already switch live without remount), so flipping it is a pure class
/// toggle. set_skeleton therefore must (a) accept only valid ids and (b)
/// never need to touch SSE / composer / selection state — it only persists a
/// string. These assertions pin that set_skeleton's surface is exactly that.
#[test]
fn set_skeleton_surface_is_pref_only() {
    // The ssr stub returns false (no localStorage on the server) and takes
    // ONLY an id — proving the API touches no shell state. (The hydrate impl
    // is the same shape; the live SSE/composer/selection preservation is a
    // Phase-7 real-device gate item, documented below.)
    assert!(!set_skeleton("orbit"));
    assert!(!set_skeleton("nonsense"));
}

// PHASE-7 GATE CONTRACT (real-device, per §13): after this lands, the wave
// gate MUST verify on the live app that switching the skeleton via the account
// picker preserves: (1) the SSE connection (no reconnect in the network
// panel), (2) the composer draft text, (3) the selected channel + scroll
// position. These cannot be asserted from the ssr harness (no WASM signals);
// they are booked as Phase-7 theme-machinery items. Open Question #4: the
// owner may want this automated sooner via headed Playwright on the MacBook.
```
- [ ] **Step 1.6.2: Run → PASS.** `cargo test --features ssr --test skeleton_switch` → PASS (now 5 tests: the three from T1.1, the probe-semantics from T1.3, and this one).

- [ ] **Step 1.6.3: Gate + commit.** Full gate. Commit:
```
test(ui): skeleton-switch invariant contract — pref-only API, no shell remount (W5/P1) (STD)

Pin the §13 invariant structurally: set_skeleton's entire surface is "persist
a validated id", touching no SSE/composer/selection state, and the root class
flips on the same stable .app node as fx-max (already switched live without
remount). The live SSE/composer/selection-preservation check is booked as a
Phase-7 real-device gate item (the ssr harness can't drive WASM signals; Open
Question #4: maybe automate via headed Playwright sooner).

Tests: set_skeleton_surface_is_pref_only

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
```

---

## Done = W5 Foundation exit criteria

1. **#43 motion doctrine** codified: keyframe lint test green, multi-line-tolerant pre-commit grep gate in place, `fx-glow-pulse` retrofitted to opacity-only with pre-rendered-glow pseudo-elements on BOTH consumers (`.send.sent::after` and the fx-max typing star `::before`); doctrine documented in `_motion.scss` header.
2. **#54 transform-free `.content`**: radial / lightbox / mobile-emoji relocated to body-level `<Portal>`s (each inside its `<Show>`); warp dip moved to `.channel-view` (with a neutral `--warp-dir: 0` hook; the directional sign deferred to Phase 2); fx-max streak is transform-swept (not background-position); reduced-motion kill rebased.
3. **#20 etched glass**: `glass-etched` is the Standard default (compositor-only, real 465-byte frost-noise PNG + pre-baked tints + inset specular), `glass-live` survives as the fx-max/Materials escalation; opaque + forced-colors fallbacks intact; back-compat shim keeps every call site. Frost look pending an owner eyeball (Open Question #2).
4. **#36 content-visibility** on `.messages > .msg`; loading-skeleton rows exempt; real-device 3-point check booked for Phase 7.
5. **#49 HoloPanel** engine landed with pure-math unit tests (4 green) + the component shell (Task 0.5), then hydrate pointer listeners + a11y + a smoke test (Task 0.5b); SCSS per-edge transform + scrim + safe-area ownership + reduced-motion instant-snap; NO consumer wired yet (sheet rewrite is the Phase-3 first consumer).
6. **#19 visual haptics**: `vh-tick`/`vh-thud`/`vh-shimmer` family (transform/opacity only with a static-glow `::before`, reduced-motion-killed in a marked block), `vh()` helper, `Prefs.haptic_vibrate` toggle; send-commit wired as first consumer.
7. **`authlyn.skeleton` pref** (fx-max pattern, hydrate + ssr stub, id-validated, + `local_storage_writable` probe + `clear_skeleton`); **`Prefs.skeleton` signal** drives exactly one `.app.sk-*` on the stable root.
8. **Onboarding ceremony**: pref-less → three-way choice at first authenticated mount (new + post-update; English copy, theme proper-nouns); localStorage-unavailable → session-only `orbit`, no ceremony, detected via a throwaway probe key (no silent default). **Account-modal picker** switches live.
9. **Switch invariant** pinned structurally (pref-only API, no remount); live SSE/composer/selection preservation booked for Phase 7 (Open Question #4).
10. **W3 shell** retained + annotated as Phase-6-retirement scaffolding.
11. **Full gate green**: `cargo fmt --all --check`; clippy ssr + hydrate(wasm32) + freya; `cargo test --features ssr` (0 FAILED, incl. `style_lint` + `skeleton_switch`); `cargo leptos build --release`; `cargo build --bin authlyn-native --features freya`. WASM baseline recorded (Task 0.0); owner signs the ceiling (Open Question #1).

**Next plan:** `2026-06-13-w5-skeleton-a-orbit.md` (Phase 2 — Omloppsbana: horizontal swipe strip, orbit-map picker, composer orb, first per-skeleton bindings of the nine effects + radial/swipe-to-reply, the directional `--warp-dir` sign, real-device axis-lock gate). Authored after Foundation lands, citing the portals (#54), etched glass (#20), HoloPanel (#49), and the switch infra from this plan. See the roadmap (`2026-06-13-w5-skelettvagen-roadmap.md`) for the full 6-doc decomposition and dependency order.
