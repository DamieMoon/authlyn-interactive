# Mendicant Bias W5 (Skelettvågen) — Wave Roadmap & Master Index

> **This is the W5 master index.** It holds the full scope map (Phases 0–7), the 6-document decomposition with its dependency rationale, and a status table tracking which docs are authored. Each phase is authored into its own executable plan as the wave progresses; this index points to them and explains the order. The canonical executable doc for Phase 0 + Phase 1 is `docs/superpowers/plans/2026-06-13-w5-skelettvagen-foundation.md`.

**Spec:** `docs/superpowers/specs/2026-06-10-mendicant-bias-design.md` §1 (appearance axes — skeleton × tier, nine effects), §2 (navigation — W3 retired by W5), §12 (waves), §13 (verification gates). Prototypes: `assets/2026-06-12-skelettvagen/` (`a-orbit.html`, `b-deck.html`, `c-hud.html`, all matrix-verified across the friend-group device geometry; `index.html` deck landing; `evolution.html` catalogue) + `ux-evolution-2026-06-11.md` (64 ranked proposals, 3 binding kills, merge directives).

**Branch convention:** `mendicant-bias` work continues; commits tagged `(W5/Pn)`. Push to `main` is a live fenrir deploy — owner sign-off only.

---

## What W5 is

Three user-selectable structural UI skeletons (`sk-orbit` / `sk-deck` / `sk-hud`) replace the W3 hybrid shell outright, over one shared core (message stream + W4 effects + composer + `act/` layer + `message_actions(kind, mine)` predicate). Two orthogonal appearance axes: **Skeleton** (structure, `.app.sk-*`) and **Tier** (Standard default / Eye-candy `.fx-max` opt-in). Onboarding ceremony, no silent default. Mobile-first (PWA primary); desktop is the gracefully-degraded adaptation. `sk-` prefix for structural shells only — loading placeholders (`_skeleton.scss`, `channel/skeleton.rs`) keep unprefixed names (no collision).

---

## Full Scope Map (Phases 0–7)

### Phase 0 — Prerequisite Cluster (6 items, ALL land before any skeleton work) — **AUTHORED in the Foundation plan**
- **T0.1 #43 Motion doctrine** (eye-candy, S): `@keyframes` may animate ONLY transform/translate/rotate/scale/opacity; retrofit `fx-glow-pulse` (animates box-shadow) → static `::before`/`::after` carrying the MAX box-shadow, opacity-pulsed (BOTH consumers: `.send.sent` and the fx-max typing star); add a `tests/style_lint.rs` `std::fs` brace-aware guard + a `.githooks/pre-commit` grep; document the pre-rendered-glow house technique. Acceptance: lint test red→green, hook fails on a planted violation, no box-shadow/filter/width/height/top/left in any `@keyframes`.
- **T0.2 #54 Transform-free `.content` + body portal layer** (warp, M): (a) relocate radial menu + lightbox + mobile emoji sheet to `<Portal>` on document.body with owned z-index + safe-area contract; (b) rebase warp dip OFF `.content` (move the transform to an inner `.channel-view` wrapper); (c) replace the fx-max streak `background-position` with a 20%-wide gradient strip swept via `translateX`; (d) directional pane translate from the channel-list index via `--warp-dir` (ships neutral `0`; the directional sign is deferred to Phase 2). Acceptance: `.content` carries no transform; overlays render over a transformed pane; clippy ×3 + leptos build.
- **T0.3 #20 Etched Glass** (void-station, M): split `glass()` into `glass-etched` (new Standard default, compositor-only: layered gradient + a real 465-byte base64 noise PNG + 1px inset specular + `--frost-top`/`--frost-bottom` precomputed tints) and `glass-live` (keeps backdrop-filter, becomes the fx-max/Materials escalation); a back-compat shim keeps every blur consumer working. Acceptance: no backdrop-filter at Standard tier; opaque/forced-colors fallback still proven; visual ~90% identical (owner eyeball — open question).
- **T0.4 #36 content-visibility rows** (mobile-qol, S): `.messages > .msg { content-visibility: auto; contain-intrinsic-size: auto 4.5rem; }`; exempt `.msg-skeleton`; the reply-quote scrollIntoView / near-top backfill anchor / unread-jump checks are booked into the Phase-7 gate. Acceptance: CSS lands now; real-device 3-point interaction pass deferred to P7.
- **T0.5 / T0.5b #49 HoloPanel engine** (desktop-furniture-leak, M): Leptos primitive `HoloPanel { edge, detents, on_commit, children, desktop_chrome }` — pointer→`--p` 0–1 drag progress, velocity commit, scrim coupled to `--p`, 7px tap-vs-drag slop, per-edge safe-area ownership, full a11y (focus trap, Esc, restore, role=dialog, reduced-motion=instant). Foundation lands the **pure math + component shell + unit tests** (T0.5) then the **hydrate pointer listeners + a11y + smoke** (T0.5b). Children render touch-clean; desktop opts IN. First real consumer is Phase 3 (the sheet rewrite); extract/generalize at the second consumer (Phase 4 Holoterminal panels).
- **T0.6 #19 Visual Haptics** (new, S): SCSS family `vh-tick`/`vh-thud`/`vh-shimmer` (transform/opacity only, static-glow `::before`, reduced-motion-killed); `fn vh(el, kind)` hydrate helper adds class + cleans on animationend; `navigator.vibrate` mirror behind the `Prefs.haptic_vibrate` toggle (tick=10ms, thud=20ms) as enhancement, never exclusive content; vocabulary documented in the SCSS header as the app's haptic contract. Acceptance: fallback-first; clippy ×3; helper compiles; send-commit wired as first consumer.

### Phase 1 — Theme-switch Infrastructure + Ceremony Machinery — **AUTHORED in the Foundation plan**
- **T1.1 `authlyn.skeleton` client-local pref** (fx-max pattern): `KEY_SKELETON` + `skeleton_pref()` / `set_skeleton()` / `clear_skeleton()` / `local_storage_writable()` + `SKELETON_IDS` / `SKELETON_FALLBACK` / `is_valid_skeleton()` in `act/prefs.rs` (hydrate-real + ssr stubs), re-exported in `act/mod.rs`.
- **T1.2 `Prefs.skeleton: RwSignal<Option<String>>`** in `state.rs` + init in the `AppShell` constructor; root-class application `.app.sk-orbit|sk-deck|sk-hud` at `mod.rs:389`, lifted above any skeleton branch (same stable node as `fx-max`).
- **T1.3 Onboarding ceremony machinery**: pref-less → three-way choice modal at first authenticated shell mount; `sk-orbit` session fallback when localStorage write fails (detected via a throwaway probe key, no silent default); covers new users AND existing users post-update. English copy; theme proper-nouns kept.
- **T1.4 Account-modal picker**: radio group in the Preferences section (eyecandy pattern), switches live.
- **T1.5 In-tree W3-shell scaffolding retained** to prove the switch (the acknowledged in-wave-only "no silent default" exception; deleted in Phase 6); annotated in the root render.
- **T1.6 Theme-switch state invariants (§13)**: switch must never drop SSE / composer draft / selection — pinned structurally (pref-only API, no remount); the live behavioral check is booked into the Phase-7 gate.

### Phase 2 — Skeleton A: Omloppsbana (`sk-orbit`) — task-level outline
- **Tasks:** `src/ui/shell/sk_orbit/mod.rs` + `style/_sk_orbit.scss` registered in `main.scss`. Horizontal swipe strip (3 mounted panes: prev/current/next, `content-visibility:auto` from #36). Holographic channel pill (top-center) → orbit-map picker (pill-tap entry only; pinch entry judge-killed — binding). Floating composer orb with charge ring (#E) + effect blossom on long-hold. Right-edge slide-over (personas + station settings) via HoloPanel. Per-skeleton placement of radial long-press (#I), swipe-to-reply (rank 14), warp-jump (#A — sets the directional `--warp-dir` sign deferred from Foundation T0.2), scene-light (#B).
- **Feasibility tax:** peek-never-marks-read discipline; SSE open-channel semantics for peeked neighbors + memory mgmt; axis-lock arbitration (swipe-reply vs horizontal strip) tuned on real iOS.
- **Acceptance:** picker channel-switch test; axis-lock real-device gate (#54); SSE preserved across strip swipe; safe-area edges owned exactly-once; clippy ×3 + leptos build.

### Phase 3 — Skeleton B: Kortdäck (`sk-deck`) — task-level outline
- **Tasks:** `src/ui/shell/sk_deck/mod.rs` + `style/_sk_deck.scss`. Depth scrub (one continuous z-stack: chat L0 → channel deck L1 → server galaxy L2) driven by `--t0`/`--t1` vars (compositor-cheap, NO per-frame `filter` — blur-recede re-engineered FIRST, gating aesthetic lock). Persistent command deck = only chrome. Flip-cards replace modals (HoloPanel **first real consumer**: two detents D1=channels, D2=galaxy). Action chips fan (lifted-card chips instead of radial). 3D `rotateY` flip with slide fallback for old Safari. Reuses A's gesture/axis-lock patterns.
- **Acceptance:** depth-scrub picker test; blur GPU cost verified on POCO C3 floor; HoloPanel detent commit test; continuous-nav AT path (keyboard/AT-operable, not modal-trapped); real-device gate; clippy ×3.

### Phase 4 — Skeleton C: Holoterminal (`sk-hud`) — task-level outline
- **Tasks:** `src/ui/shell/sk_hud/mod.rs` + `style/_sk_hud.scss`. Message stream alone over parallax starfield (CSS-var-driven, ÖG-tier-only, paused on `document.hidden`). Four edge-summoned hologram panels (channel rig / crew & personas / station HUD / console-composer) — **HoloPanel second consumer → extract/generalize the engine here**. Materialization sweep (#C) as arrival language. CRT grain + vignette overlay. Edge-swipe arbitration vs iOS back-swipe. Reuses A+B gesture vocabulary.
- **Acceptance:** channel-rig picker test; edge-swipe vs iOS back-swipe real-device gate (#54); panel pointer-capture crosstalk resolution; four panels reach Modal-parity focus; parallax paused on hidden; clippy ×3.

### Phase 5 — The #48 ÖG Delta (Eye-candy Wow Tier) — task-level outline
- **Tasks:** Ignition flip (rank 60 "The Power Surge", merged with rank 63): flipping `.fx-max` feels like a master switch — settings diptych (steady-state deltas + moment-of-toggle unmistakability). Nebula plates (rank 37): replace the planned WebGL shader with 2–3 pre-rendered transparent AVIF/WebP plates (~1280px, tens of KB), stacked as fixed layers behind the app, each on an infinite transform keyframe (translate3d+scale, 60–120s, opposing dirs); `document.hidden` → `.app.bg-idle { animation-play-state: paused }`. Holographic depth (#F): rAF-throttled gyro/pointer nudges plate translate3d at different ratios. Standard = static aurora + slowest plate; fx-max = all plates + faster drift + parallax on. Per-skeleton exhibition of the unmistakable delta (shared backend, per-skeleton visual placement).
- **Acceptance:** the delta is unmistakable ("a wow tier that needs a diff tool is a failed wow tier"); WASM bundle budget respected (owner-signed ceiling from Foundation T0.0); per-skeleton ÖG smoke.

### Phase 6 — W3-shell Retirement — task-level outline
- **Tasks:** Delete the W3 composition/layout partials (`_layout.scss`, `_rail.scss`, `_sidebar.scss`, `_nav.scss`, `_mobile.scss`) + the bottom-tabs/sheet/`.nav-open` Rust in `mod.rs`. Remove the in-tree scaffolding from Phase 1 (Task 1.5). Rebind shared inset consumers (`_content.scss`, `_toast.scss` — toast rode `_nav`'s `--tabbar-h`) so each skeleton publishes its own chrome anchor var and claims/cedes each inset edge explicitly.
- **Safe-area rule (binding):** for each inset edge (top/bottom/left/right) under each `.app.sk-*` root, EXACTLY ONE rule in the computed layout chain applies `env(safe-area-inset-*)` padding.
- **Acceptance:** (a) static audit mapping every `env(safe-area-inset-*)` site to one owner per skeleton; (b) notched-device check ×3 skeletons on iPhone 13 mini — composer/toast/topbar flush, no double gap, no underlap.

### Phase 7 — The Wave Gate (per-skeleton ×3) — task-level outline
- **Gate 1 Visual smoke ×3:** channel list + channel switch via the active picker (orbit map / depth scrub / channel rig), DM, vault unlock, atlas nav, roll — each skeleton independently.
- **Gate 2 Real-device iOS PWA ×3:** owner's iPhone 13 mini (iOS 26.5), not emulated — channel switch, radial 450ms threshold, swipe-to-reply, touch-AT message actions, notch safe-area. (Origin: W4 headless missed `-webkit-touch-callout` / `draggable` / `user-scalable` bugs.)
- **Gate 3 Geometry/device-matrix sweep ×3:** POCO C3 360×800 (floor), iPhone SE 375×667 (shortest), iPhone 13 mini 375×812 (notch), Nothing Phone 2 412×892 (widest) — no hardcoded 375-math; `clamp()`/`%`/`dvh`.
- **content-visibility real-device 3-point check (booked from Foundation T0.4):** reply-quote scrollIntoView lands on a skipped row; near-top backfill keeps its anchor (no jump); jump-to-unread-on-open scrolls correctly.
- **Theme machinery verification (booked from Foundation T1.6, Open Question #4):** switch never drops SSE/composer/selection (network panel shows no SSE reconnect; composer draft + selected channel + scroll survive); ceremony lands on a pref-less device (new install + post-update legacy); localStorage-unavailable boots orbit with no ceremony; exactly-once inset ownership.
- **WASM bundle budget (Open Question #1):** re-measure `target/site/pkg/authlyn-interactive.wasm` (raw+gzip) vs the owner-signed Foundation T0.0 baseline.
- **W4 a11y backlog (per-skeleton):** sheet a11y, touch-AT message actions, radial focus mgmt, Modal-parity overlays, continuous-nav AT path, message-actions AT-reachable (derived from `message_actions`, never re-branched), iOS VoiceOver pass.
- **Full gate:** `cargo fmt --all --check`, clippy ssr+hydrate(wasm32)+freya, `cargo test --features ssr` (0 FAILED), `cargo leptos build --release`, `cargo build --bin authlyn-native --features freya`.
- **Three binding kills enforced:** pinch orbit-entry, deck-plate transit stamp, Persona Orrery.

---

## Cross-cutting / parallel (no W5 prerequisite)
Shared-layer ux-evolution items ride W5's shared phases where they fit, else later: whisper #8/#13, dice console #12/#15/#31, persona chip #1/#21, outbox #6 (verify the UNIQUE `(channel, author, client_nonce)` schema landmine first). **Per-server accent #G needs a `guild.accent_color` schema field that does NOT exist in `schema.surql` today** — NOT in Foundation scope; it's a non-UI substrate that can land in parallel under the SCHEMAFULL NONE-coercion + enum-OVERWRITE invariants (CLAUDE.md). Someone must author it before any skeleton renders the accent (Foundation Open Question #5); the warp directional tint uses the generic `--glow-accent` until then.

---

## The 6-Document Decomposition

W5 is authored as six executable plan docs, written incrementally. The Foundation doc exists now; the rest are authored as each phase begins (so each plan is grounded against the code the prior plan actually landed, not against a guess).

| # | Document | Covers | Status |
|---|----------|--------|--------|
| 1 | `2026-06-13-w5-skelettvagen-foundation.md` | **Phase 0** (the 6 prerequisites) + **Phase 1** (switch infra + ceremony) | **Authored** ✅ |
| 2 | `2026-06-13-w5-skeleton-a-orbit.md` | **Phase 2** — Skeleton A: Omloppsbana (`sk-orbit`) | To author (after Foundation lands) |
| 3 | `2026-06-13-w5-skeleton-b-deck.md` | **Phase 3** — Skeleton B: Kortdäck (`sk-deck`); HoloPanel's first real consumer | To author (after A) |
| 4 | `2026-06-13-w5-skeleton-c-hud.md` | **Phase 4** — Skeleton C: Holoterminal (`sk-hud`); HoloPanel extract at second consumer | To author (after B) |
| 5 | `2026-06-13-w5-og-delta.md` | **Phase 5** — the #48 Eye-candy ÖG delta (ignition flip, nebula plates, holographic depth) per skeleton | To author (after C) |
| 6 | `2026-06-13-w5-retirement-and-gate.md` | **Phase 6** (W3-shell retirement) + **Phase 7** (the per-skeleton ×3 wave gate) | To author (last) |

### Dependency rationale (why this order)
- **A first.** Omloppsbana is the simplest mounted-pane model and establishes the gesture/axis-lock vocabulary the other two reuse. It also consumes the Foundation portals (#54), etched glass (#20), and visual haptics (#19) directly, validating them under a real skeleton before B/C depend on the same substrate. A is where the deferred `--warp-dir` directional sign (Foundation T0.2) is finally set.
- **B = HoloPanel's first real consumer, reusing A.** Kortdäck's flip-cards are the first place the Foundation HoloPanel engine (#49) is wired to real content (two detents D1=channels, D2=galaxy), so B both reuses A's gesture/axis-lock patterns and proves the HoloPanel engine end-to-end — but it does NOT yet generalize the engine (extract-at-second-consumer discipline).
- **C = HoloPanel extract, reusing A+B.** Holoterminal's four edge panels are the **second** HoloPanel consumer, which is where the engine is extracted/generalized (per #49's "extract at the second consumer" rule). C reuses A's gesture vocabulary and B's HoloPanel wiring, so it must come after both.
- **ÖG needs all three.** The #48 delta must be exhibited by EVERY skeleton (spec finding #48), so the ÖG-Delta plan can only be authored once all three skeletons exist to hang the per-skeleton placements on. The shared backbone (nebula plates, ignition flip) is built once; the per-skeleton exhibition is the tail.
- **Retirement + Gate last.** The W3 shell can only be deleted (Phase 6) once all three skeletons render standalone (the scaffolding from Foundation T1.5 was retained precisely to keep the switch provable until then). The per-skeleton ×3 wave gate (Phase 7) is the final verification across all three, and it cashes the checks booked throughout Foundation (WASM budget T0.0, content-visibility T0.4, switch invariant T1.6).

---

## Open Questions (owner decisions) — carried from the Foundation plan
These are surfaced in the Foundation plan's "Open Questions" section and re-listed here for the wave-level view. They are owner decisions, not blockers to authoring.

1. **WASM bundle-budget ceiling** — owner signs the allowed growth over the Foundation T0.0 baseline before Phase 7 re-measures.
2. **Standard-tier etched-glass look** — owner eyeballs the `--frost-noise` PNG + `--frost-top`/`--frost-bottom` tints (~90% identical to blur?).
3. **Ceremony localStorage-writability detection** — probe-and-clear (current) vs a cleaner alternative.
4. **§13 switch-invariant automation** — automated headed-Playwright guard now vs deferred to the Phase-7 device gate.
5. **`guild.accent_color` schema field** — does not exist today; must be authored (SCHEMAFULL NONE-coercion + enum-OVERWRITE invariants) before any skeleton renders per-server accent (#G).
6. **`web_sys` API surface** — confirm the once-listener (`AddEventListenerOptions`) + `navigator.vibrate` bindings against web-sys 0.3.85 at execution; `set_pointer_capture` is already confirmed in-tree.
