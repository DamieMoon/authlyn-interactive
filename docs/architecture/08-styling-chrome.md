# Styling & chrome

The visual layer is a single dart-sass cascade. One entry stylesheet (`style/main.scss`) `@use`s ~25 partials in a fixed load order; cargo-leptos compiles it to one `authlyn-interactive.css`. There is no runtime CSS-in-Rust: components couple to styling **only** through class-name and CSS-custom-property contracts. A redesign is, by construction, a value-only edit of `style/_tokens.scss`.

This doc covers the orchestration, design tokens, typography, the motion doctrine, the three glass materials, the `sk-orbit` cosmic chrome, the modal/slide-over system, and the product-wide 44px touch-target floor. Everything behavioral is pinned by **`tests/style_lint.rs`** — a pure static file scan (no DB, no browser) that runs in every feature graph and is part of `.githooks/pre-commit` (it is **not** in `/check`; see [09-testing.md](./09-testing.md)). The fidelity gate that `style_lint` cannot replace — real WebKit/touch defects on hardware — is the owner deck-pass; see **CLAUDE.md → "UI fidelity"** (the canonical statement; not restated here).

The SCSS is graph-agnostic: it ships identically under `ssr` (SSR'd HTML carries the class names) and `hydrate` (the WASM client toggles the same classes). See [01-overview.md](./01-overview.md) for the graph split and [07-ui-shell.md](./07-ui-shell.md) for the component side.

---

## 1. Orchestration: the cascade-order contract

`style/main.scss` is 32 lines: a header comment plus `@use` lines. **Load order is the cascade** — partials emit in source order, so equal-specificity ties resolve by position. Adding a partial means adding one `@use` line *at the right position*.

`@use` order (load-bearing orderings annotated):

| # | Partial | Role | Order constraint |
|---|---------|------|------------------|
| 1 | `tokens` (`as *`) | `:root` design tokens | must be first — every later partial reads `var(--…)` |
| 2 | `typography` | `@font-face` (Duo fonts) | — |
| 3 | `base` | resets, base `button`/`input`, global reduced-motion freeze | early — base element styles |
| 4 | `foundation` | glass mixins, aurora layer, accent-swatch | before any glass consumer |
| 5 | `motion` | keyframe library + reduced-motion kill | before consumers reference keyframes |
| 6 | `auth` | login/register surface | — |
| 7 | `content` | base (non-orbit) chat surface | base chrome — orbit overrides it |
| 8 | `wardrobe` | persona cards + detail editor | before `crest` |
| 9 | `crest` | M7 crests | **after `wardrobe`** — leans on the portrait-slot box |
| 10 | `markup` | message markup tints | — |
| 11 | `modal` | base modal + orbit slide-over rebuild | — |
| 12 | `msg_who` | author-name styling | — |
| 13 | `mobile` | narrow-screen `@media` (incl. `.modal { max-width: 92vw }`) | — |
| 14 | `sk_orbit_chrome` | orbit shell chrome (pill/orb/map/composer/station) | **after `content`/`mobile`** — equal-specificity `.app.sk-orbit` rules must win the cascade over base + the `_mobile.scss` narrow rule |
| 15 | `sk_orbit_chat` | orbit chat-card look | **right after `sk_orbit_chrome`** to preserve its cascade |
| 16 | `attachments` | attachment thumbs / upload progress | — |
| 17 | `skeleton` | loading shimmer placeholders | — |
| 18 | `holopanel` | right-edge HoloPanel slide-over | — |
| 19 | `lorebook` | lorebook pane | — |
| 20 | `lightbox` | image lightbox (body-portal) | — |
| 21 | `toast` | toast notifications | — |
| 22 | `trash` | trashed-channels disclosure | — |
| 23 | `wave_b` | markup cluster + Friends/Members panes | — |
| 24 | `mobile_emoji` | mobile emoji-picker `@media` | **last** — its `@media` must win |

Two orderings are explicitly load-bearing and annotated in `main.scss`: `sk_orbit_*` loads **after** the base content/modal partials (so `.app.sk-orbit` rules win at equal specificity), and `mobile`/`mobile_emoji` belong **last** (so their `@media` overrides win). `crest` after `wardrobe` is a structural dependency, not a cascade tie.

Partial grouping (the mental model the flat `@use` list doesn't make explicit): **tokens → type → base → foundation → motion → base chrome (content/modal) → orbit chrome → panes → body-portal overlays → mobile media**.

---

## 2. Design tokens (`style/_tokens.scss`)

`_tokens.scss` is the **only** file in `style/` carrying hex literals. Everything else reads tokens via `var()`. Token names are **semantic** (`--void`/`--surface`/`--accent`), so a redesign re-skins by editing values here. The current palette is **Void Station** (deep-space graphite + electric-blue accent + Duo typography).

### Surfaces — the elevation ladder

Ordered darkest → lightest (`_tokens.scss:10`): `--void-deep` `#07090f` < `--void` `#0b0e14` < `--surface` `#0e121a` < `--card` `#121724` < `--surface-2` `#131927`. Plus `--line`/`--line-strong` (hairlines), `--input-bg` (darker than the page).

`--card` was lifted a notch (live finding #53: other-author bubbles melted into the void on crushed-black panels) but stays strictly below `--surface-2` so the ladder keeps its order.

### Ink, accent, status

`--text` / `--text-soft` / `--text-muted` / `--text-faint` (4-step ink). Accent family `--accent` `#4d9fff` / `--accent-bright` / `--accent-soft` (hover/disabled steps). `--live` (mint, online/success). Danger family `--danger*`. `--role-admin`.

### Glass + frost

- `--glass-bg` `rgba(20,26,40,.55)`, `--glass-line`, `--glass-highlight` — the translucent material + edges.
- `--frost-top` / `--frost-bottom` — pre-computed colors approximating `blur(14px)` of the *static* aurora at the top/bottom chrome bands, baked once (cheaper than a live readback on the POCO C3 GE8320 floor). These are the opaque fallback for `glass-etched` and the no-`backdrop-filter` branch of the glass mixins.
- `--frost-noise` — a 32×32 tiled data-URI PNG. **Still declared but unused by `glass-holo`** (owner ruling 2026-06-15 removed the grain layer — it read as "TV static"). `style_lint` forbids `glass-holo` from re-layering it (§5).

### Glow / overlay / scrim

`--glow-accent` `rgba(77,159,255,.55)`, `--unread-glow`, `--pinged-glow` (@-mention warmth stays warm), `--mention-bg`, `--spoiler-bg`, `--overlay`, **`--scrim`** `rgba(4,6,10,.6)` (modal dim — `style_lint`-restricted to true modal backdrops, §6).

### Aurora + persona tints

`--aurora-1` (blue nebula tint), `--aurora-2` (teal). 8 persona tints `--tint-{red,orange,yellow,green,blue,purple,pink,gray}` (luminous against `--card`; the chat author column reads its color from these, §9).

### Typography + motion tokens

`--font-ui` (Space Grotesk → system sans), `--font-prose` (Crimson Pro → Georgia), `--font-display` (= `--font-ui`), `--font-mono` (JetBrains Mono). Eases `--ease-page`/`--ease-quick`/`--ease-spring` and durations `--dur-quick` (80ms) / `--dur-base` (150ms) / `--dur-page` (200ms) / `--dur-entrance` (450ms).

> **Pinning:** token *values* are unpinned by design (they are the redesign surface). The *contracts that depend on tokens* are pinned: `--scrim` placement (`scrim_only_on_modal_backdrops`), `--frost-noise` absence from `glass-holo` (`glass_holo_is_liquid_glass_not_frost_noise`).

---

## 3. Typography (`style/_typography.scss`)

Ten `@font-face` rules for the **Duo** type system, all self-hosted subset woff2 in `public/fonts/` (~13–19 KB each, ≈64 KB total), all `font-display: swap` (no FOIT — first paint uses the system fallback chain, re-renders when the woff2 arrives):

| Family | Weights / styles | Role |
|--------|------------------|------|
| Space Grotesk | 400 / 500 / 600 / 700 | UI chrome (`--font-ui`); orbit uses 700 for core name / node hashes / dock, 500 for node labels |
| Crimson Pro | 400 / 600, + italic 400 / 600 | prose — message text, composer, lorebook, persona descriptions (`--font-prose`); italics are real faces, not faux-oblique |
| JetBrains Mono | 400 / 700 | orbit chrome / meta — timestamps, tags, mono eyebrows (`--font-mono`) |

Caching is the **service worker's** job, not HTTP headers: the static-file fallback sends no `Cache-Control`, and `public/sw.js` serves `/fonts/` cache-first from its versioned runtime cache, busted per release via `CACHE_VERSION`. See [11-build-deploy-pwa.md](./11-build-deploy-pwa.md). Re-hosting is permitted under the SIL OFL.

> **Pinning:** font wiring is **(unpinned)** by `style_lint` (no `@font-face` guard). The font→SW relationship is `public/sw.js` `CACHE_VERSION`.

---

## 4. Motion doctrine (`style/_motion.scss`)

The single hardest rule in the visual system, machine-enforced.

### The composite-only allowlist

> `@keyframes` bodies may animate **only** `transform` / `translate` / `rotate` / `scale` / `opacity`. **Never** `box-shadow`, `background-position`, `filter:`, `width:`, `height:`, `top:`, `left:` — each forces layout or paint per frame and is lethal on the POCO C3 floor device.

- **Pinned by `tests/style_lint.rs::no_keyframes_animate_paint_or_layout_properties`** — brace-matched scan of every `@keyframes` body across `style/*.scss` against the `FORBIDDEN` const. `top:`/`left:`/`filter:` carry the colon so a `transform: translate(...)` substring can't false-positive.
- **Exempt by name** (`EXEMPT_KEYFRAMES`): `shimmer`, `gallery-skeleton-shimmer` — the brief loading-placeholder sweeps (textbook `background-position` over a `200%` gradient), short-lived, not perpetual `fx-` effects on the interactive hot path. `fx-warp` was a *time-boxed* exemption, removed in M5/P0 once rewritten to a `transform: translateX` sweep.

### The "pre-rendered glow" house technique

A glow **pulse** must never animate `box-shadow` (a paint prop). Instead: carry the **MAX** glow as a *static* `box-shadow` on a `::before`/`::after`, and pulse only its **opacity** via a transform/opacity keyframe. Reference consumers: `fx-glow-pulse` + `.composer .send.sent::after` (`_content.scss:1170`), `.sk-orbit-orb::before/::after` (`_sk_orbit_chrome.scss:1179,1202`), `.msg.effect-spell .text::before`. This is why the glow survives reduced-motion (it is *state*, not motion) while the breathing stops.

### `backdrop-filter` is permitted outside keyframes

Since the 2026-06-15 "Real Liquid Glass as the default" ruling, `backdrop-filter` is the orbit-chrome default (`glass-holo`, §5) — it is **not** in `FORBIDDEN` and is allowed *outside* keyframes. The remaining guardrail is WebKit 1:1 (§5). `box-shadow`/`filter` *inside* a keyframe stay forbidden.

### Two-layer reduced-motion kill

| Layer | Where | What it does |
|-------|-------|--------------|
| Global freeze | `_base.scss:86-95` | `*`, `*::before`, `*::after` → `animation-duration: 0.01ms !important` + `transition-duration: 0.01ms` + `animation-iteration-count: 1`. `0.01ms` (not `0`) keeps animation/transition **events firing** for JS listeners. Covers e.g. the skeleton shimmer. |
| Decorative `animation: none` | `_motion.scss:341-349` (vh-* block) + `_motion.scss:400-422` (big decorative block) | Higher-specificity selectors that remove decorative animation *entirely*. Matches `[class*="fx-"]` for fx-named consumers, and **lists pseudo-elements + non-fx-named classes EXPLICITLY** — the attribute/element-level kill can't reach `::before`/`::after` or classes without `fx-` in the string. |

The explicit list (the contract that the element/attribute kill cannot reach): `.app::before`/`.app.fx-max::before` (aurora), `.typing-indicator .star`/`.star-core` (fx-orbit/fx-twinkle), `.msg.msg-ghost` (fx-ghost-shimmer), `.composer .send.sent` + `::after`, `.radial-menu` (fx-blossom), `.app.fx-max .channel-view.fx-switching::after` (fx-warp), `.msg.effect-shout`, `.msg.effect-spell` + its `::before`/`::after`/`.text::before`/`.text::after`, `.msg.roll` + `.roll-die`, `.nova-orb-ring` (fx-nova-ring). Per-component `@media (prefers-reduced-motion: reduce)` blocks in `_foundation`/`_sk_orbit_chrome`/`_modal`/`_holopanel`/`_wardrobe` extend this for their pseudo-element consumers (e.g. `.sk-orbit-orb::after`'s `fx-orb-breath`, `.sk-orbit-map`'s open keyframe).

**Rule of effect:** a killed decorative effect must rest at its **static state** — the glow stays, the motion stops. Keyframes whose opacity rises from a 0 *base* (e.g. `fx-spark`) leave nothing behind when killed.

> **Pinning:** the doctrine is `no_keyframes_animate_paint_or_layout_properties`. The reduced-motion two-layer is **code-pinned** at `style/_base.scss:86-95` + `style/_motion.scss:341-422` (no dedicated `style_lint` guard scans the kill lists).

### Keyframe inventory (35 declarations across the tree)

Composite-cheap by the doctrine. Families:

| Family | Count | Members (consumers) |
|--------|------:|---------------------|
| `fx-*` decorative | 17 | `fx-slide-in`, `fx-glow-pulse`, `fx-ghost-shimmer`, `fx-sweep`, `fx-orbit`, `fx-twinkle`, `fx-blossom` (radial menu), `fx-warp` (channel-switch streak), `fx-shout`, `fx-spark`, `fx-roll-in`, `fx-die-tumble`, `fx-nova-ring`, `fx-aurora-pulse` (`_foundation`), `fx-orb-breath` (`_sk_orbit_chrome`), `fx-sk-orbit-twinkle-a`/`-b` (`_sk_orbit_chrome` starfield) |
| `vh-*` Visual-Haptics | 4 | `vh-tick`, `vh-tick-glow`, `vh-thud`, `vh-shimmer` |
| `sk-orbit-*` | 7 | `sk-orbit-twinkle-a`/`-b`† , `sk-orbit-hints-in`, `sk-orbit-hints-card-in`, `sk-orbit-map-in`, `sk-orbit-spin`, `sk-orbit-spin-rev`‡, `sk-orbit-chip-in`, `sk-orbit-slideover-in` (`_modal`) |
| loading / misc | — | `aurora-drift` (fx-max aurora), `modal-backdrop-enter`, `modal-enter` (`_modal`), `shimmer`§ (`_skeleton`), `gallery-skeleton-shimmer`§ (`_wardrobe`), `att-progress-sweep` (`_attachments`), `toast-drain` (`_toast`) |

† the two starfield twinkles carry the `fx-` token *and* the `sk-orbit` token (`fx-sk-orbit-twinkle-a/-b`) so the global `[class*="fx-"]` kill auto-covers them. ‡ `sk-orbit-spin-rev` is defined but `.sk-orbit-orbit.retro` reverses via `animation-direction: reverse` on `sk-orbit-spin`, so `-rev` is currently unreferenced. § `shimmer` + `gallery-skeleton-shimmer` are the two `EXEMPT_KEYFRAMES`.

### Visual Haptics (`vh-*`)

The app's **sound-free, vibration-free feedback vocabulary** (`_motion.scss:243`). Visual form is primary (iOS PWAs have no `navigator.vibrate`); Android mirrors each to a designed vibration behind a user toggle. UX-equality across POCO / iPhone / desktop. Three meanings: `vh-tick` (light acknowledge — 450ms radial threshold, effect-chip arm, copy), `vh-thud` (weighty land — roll result, send commit), `vh-shimmer` (received glint — reaction/resonance). Every future realtime feedback speaks this rather than inventing ad hoc.

---

## 5. Glass materials (`style/_foundation.scss`)

Glass is for **chrome only** — bars, sheets, modals, orbit controls. **Prose cards stay opaque** (`--card`) for legibility and battery: a `backdrop-filter` inside a scroll list re-blurs every frame (§9). Three mixins, plus a back-compat shim:

| Mixin | Default for | `backdrop-filter`? | Background | Use when |
|-------|-------------|--------------------|------------|----------|
| `glass-etched($border)` | **Standard non-orbit chrome** | **No** (compositor-only) | smooth `--frost-*` gradient + 1px inset specular | bars/sheets in the base shell; rasterizes once, scrolls at composite cost |
| `glass-holo($border)` | **Standard orbit chrome** (since 2026-06-15) | **Yes** — `blur(16px) saturate(1.4)` | translucent `--glass-bg` (on supporting engines) under an accent sheen | every orbit control + dispatch-pane control; **real Apple Liquid Glass** |
| `glass-live($border)` | **fx-max escalation** | **Yes** — `blur(14px) saturate(1.4)` | `--surface` fallback → `--glass-bg` on supporting engines | the "Materials: live refraction" / fx-max tier (`.app.fx-max …` rules) |
| `glass($border)` | back-compat shim | — | → `glass-etched` | legacy callers; `.glass` class |

`$border: false` skips the 4-side `--glass-line` border for consumers that draw a single edge after the include (bars/sheets, the slide-over head's `border-bottom`).

### `glass-holo` structure (the subtlest material contract — `_foundation.scss:69-117`)

Four-branch, fallback-first:

1. **Fallback first** — smooth glass (no noise, no blur): the accent-sheen gradient over the opaque `--frost-*` gradient, for devices without `backdrop-filter` (POCO C3 floor) or with Reduce Transparency on. A pre-baked **static `box-shadow` stack** stated once: faint outer accent halo + accent-soft edge ring (the "holographic rim") + inset specular.
2. **`@supports (backdrop-filter…)`** — go translucent (`--glass-bg`) so the aurora refracts for real; emit **both** `backdrop-filter` and the **mandatory `-webkit-backdrop-filter` sibling**. `box-shadow` is deliberately **not** restated here (it's static and orthogonal to blur support).
3. **`@media (prefers-reduced-transparency: reduce)`** — strip back to `glass-etched` (no sheen, no blur), placed *after* the upgrade so it wins at equal specificity. (WebKit doesn't support this query, so enriching-by-default + stripping only on `reduce` keeps the holo skin alive on iOS — footgun F1.)
4. **`@media (forced-colors: active)`** — Canvas + no blow/blur, independent of the transparency preference.

**The box-shadow-owns-the-cascade convention:** the mixin states `box-shadow` once in the base; a consumer that re-states `box-shadow` *after* `@include glass-holo` (e.g. `.sk-orbit-pill` at `_sk_orbit_chrome.scss:279`, `.composer` at `:570`) wins the cascade as a single shorthand — the established sk-orbit pattern.

### The "let the mixin own the background" rule

`glass-holo`/`glass-live` emit their `background` *inside* the `@supports` block, which sits *after* the rule body. So on a `backdrop-filter` engine (WebKit/iOS, the primary target) a consumer that **restates a top-level `background:` after the `@include` is writing a DEAD declaration** — the control ships the blue accent-glass fill, not the authored one, while reading correctly on Chromium (the invisible UI-fidelity class). A nested `background:` inside `&:hover`/`@supports`/`@media` is fine (depth ≥ 2).

> **Pinning:**
> - `glass-holo` is real Liquid Glass + no frost-noise — **`tests/style_lint.rs::glass_holo_is_liquid_glass_not_frost_noise`** (brace-isolates the mixin body; asserts `backdrop-filter` + `-webkit-` present, `--frost-noise` absent).
> - Every standard `backdrop-filter` has a `-webkit-` sibling, per file by count-equality — **`tests/style_lint.rs::backdrop_filter_always_has_webkit_sibling`**.
> - Orbit + dispatch controls inherit the material — **`tests/style_lint.rs::orbit_chrome_controls_inherit_glass_material`** (registry of 9) + **`::dispatch_pane_controls_inherit_glass_material`** (registry of 8: `.server-icon-upload`, `.add-row button`, `.flist button`/`label`, `.member-role-btn`, `.member-kick`, `.card-actions`, `.detail-actions`).
> - Mixin owns the background — **`tests/style_lint.rs::glass_holo_consumers_let_the_mixin_own_the_background`** (same 8 consumers; `has_top_level_background` depth-1 scan).

### Aurora layer (`.app::before`)

Two soft nebula ellipses (blue top-right `--aurora-1`, teal bottom-left `--aurora-2`) over a top-anchored `--void → --void-deep` vignette, on a `position: fixed`, `z-index: -1` pseudo-layer overscanned `inset: -15%`. Standard tier: an **opacity-only** breath (`fx-aurora-pulse`, no transform → no blur re-eval cost for the chrome's backdrop sampling). fx-max escalates to `aurora-drift` (`_foundation.scss:246`) at `steps(36)` over 9s — quantized, not smooth, because this layer is the literal backdrop of the always-blurred glass bars; a smooth drift would force a `blur` re-evaluation **every compositor frame, forever, even idle** (battery rule). Each step is sub-perceptual (≤0.34px translate at a desktop edge).

**Aurora visibility contract:** the layer is `z-index: -1`, so it paints below in-flow content but above the canvas. `html` sets **no** background and must never gain one — an `html` background stops the `body`→canvas background propagation and paints over the aurora. `.app`/`.content` paint no background of their own.

---

## 6. The `sk-orbit` cosmic chrome (`style/_sk_orbit_chrome.scss`, ~1530 lines)

The largest partial: the `.app.sk-orbit` shell built **1:1 against the `a-orbit.html` prototype** (the visual oracle, §10). **Every rule is `.app.sk-orbit`-prefixed** so it can never match a deck/hud session or the retained base scaffolding — **except** the body-portal overlays (`.sk-orbit-map`, `.sk-orbit-hints`), which are `<Portal>`'d to `document.body` as siblings of `.app` and so are *unprefixed* (their `sk-orbit-*` class names are self-scoping — only `SkOrbitShell` emits them). The shell loads after the base partials so equal-specificity rules win.

### Shell shape — `svh` and clip-both-axes

- `.app.sk-orbit` is `min-height: 100svh` (**not** `dvh`): on the iOS 26.5 standalone PWA `100dvh` resolves to the *visual* viewport (taller than the document's layout/client height), producing a thin phantom vertical scroll. `svh` matches the layout viewport exactly; the fixed bottom chrme leaves no gap.
- `.app.sk-orbit` sets `-webkit-user-select: none` + `-webkit-touch-callout: none` so iOS WebKit doesn't hijack the long-press radial menu into a full-screen blue selection (the textarea/inputs opt selection back **in**).
- `.content.sk-orbit-content` is the `100svh` flex column and must `overflow: clip` **both axes** — never a per-axis split. `clip` (not `hidden`) forbids scrolling and creates **no** scroll container, so the sticky pill and per-pane `.messages` auto-scroll are untouched. The split (`overflow-x: clip` + implicit `overflow-y: visible`) was the iOS-hardware-only phantom side/vertical scroll (both the sim and pw-webkit reported green). `contain: paint` is also forbidden — it traps the fixed orb/help/composer.

> **Pinned by `tests/style_lint.rs::sk_orbit_content_clips_both_axes_no_paint_containment`** (asserts `overflow: clip`, no `overflow-x`/`overflow-y`, no `contain: paint`).

### The chrome components (each ↔ its `a-orbit.html` id)

| Component | Class(es) | Prototype id | Notes |
|-----------|-----------|--------------|-------|
| Channel pill | `.sk-orbit-pill` (+ `-name`/`-hash`/`-server`/`-dots`/`-dot`/`-live`) | `#pill` | **`position: sticky` in-flow header** — *not* fixed (see footgun below). `glass-holo` + re-stated drop/halo box-shadow. `fx-max` → `glass-live`. |
| Help disc | `.sk-orbit-help` | `#helpBtn` | bottom-left `?` glass disc, `glass-holo`, hidden while composing |
| Compose orb | `.sk-orbit-orb` (+ `-wrap`/`-glyph`/`-ring`/`-ring-arc`) | `#orb` | a **compose trigger**, not a send button; `glass-holo`; idle `fx-orb-breath` aura on `::after`; `data-armed` recolors per effect |
| Charge ring | `.sk-orbit-ring-arc` | `#sendBtn .arc` | the in-composer send arc; `stroke-dashoffset: var(--dash)`; `.full` brightens (static `drop-shadow`) |
| Effect blossom | `.sk-orbit-chip` | effect chips | `glass-holo`; `sk-orbit-chip-in` entrance |
| Orbit map | `.sk-orbit-map` (+ `-scrim`/`-core`/`-core-glyph`/`-core-icon`/`-core-name`/`-core-sub`/`-node`/`-far`/`-dock`) | `#orbitMap` | **body-portal, unprefixed**; full cosmic backdrop; `sk-orbit-map-in` warp; node-dive `.diving` exit |
| Kepler orbits | `.sk-orbit-ring`/`.sk-orbit-orbit`/`.sk-orbit-nodepos` | `.orbit`/`.nodePos` | per-node revolution at seeded `--orbit-period`/`--orbit-r`/`--orbit-a`; `.retro` reverses ~17% |
| Swipe strip | `.sk-orbit-strip` / panes | swipe panes | transformed pane (the stacking-context boundary several footguns reference) |
| Pane wrapper | `.sk-orbit-panes` | — | non-channel dispatch panes; sticky `.account-head`; swipe-right-to-back |
| Gesture hints | `.sk-orbit-hints` (+ `-card`/`-row`/`-ok`) | `#hints`/`.hintCard` | **body-portal, unprefixed**; `sk-orbit-hints-in` |
| Station slide-over | `.sk-orbit-station-close` | `#slideOver` | full-screen management surface |

The map **core** (`.sk-orbit-core`) is a luminous body, **not** `glass-holo` — a 90px radial-gradient disc with a static `box-shadow` glow. Likewise `.sk-orbit-core`/`.sk-orbit-far` are excluded from the material registry (by design no frosted glass). The **starfield** (`.sk-orbit-stars` + `.fx-sk-orbit-stars-a`/`-b`) is ~80 stars baked as a static multi-value `box-shadow` across two counter-phased 1px fixed layers, opacity-only twinkle, rendered via `:is(.app.sk-orbit, .sk-orbit-map)` so it reaches both the chat and the portaled map.

### Hotspots (the rules with the deepest WebKit reasoning)

- **Channel pill — sticky, not fixed** (`:255`). A `position: fixed` pill living over the scroll list inside the `transform`ed `.sk-orbit-strip` cannot frost the chat: that transform is a stacking-context/containing-block boundary the fixed pill's `backdrop-filter` can't cross, so crisp text bled *through and above* the capsule. Rebuilt as an in-flow sticky header (first child of the `.sk-orbit-content` column).
- **Composer hidden-at-rest triple-guard** (`:558`). `translateY(120%)` alone did not reliably hide the composer on real iOS WebKit (percentage translate resolves against the transformed `.channel-view`/`.sk-orbit-strip` ancestor and can compute short of off-screen, or paint one frame at `0`). The rest state therefore also sets `opacity: 0` **and** `visibility: hidden` — a hard hide no percentage-transform quirk can defeat. `.composing` flips all three. `visibility` steps with the class (not animated); motion stays transform/opacity only.
- **Charge-ring send button** (`_content.scss:1113-1184`). A `conic-gradient(var(--accent) calc(var(--charge)*360deg) …)` fill masked to a thin rounded-rect rim via `mask-composite: exclude` (+ `-webkit-mask-composite: xor`). A radial-gradient mask was wrong — its last stop extends past 100%, leaving corner spandrels opaque (solid-block bug). The fill is *state* (survives reduced-motion); the `.sent` pulse is *motion* (pre-rendered glow on `::after`, killed in `_motion.scss`).
- **Orbit map overlay touch-action** (`:628`). The map is a full-cover `position: fixed` body-portal. The app `<body>` stays pannable (`_base.scss touch-action: manipulation`, shared with non-orbit routes — it can't be globally locked like the prototype's `body{position:fixed}`), so without `touch-action: none` on the map root, iOS WebKit routes a vertical drag on the overlay to the document and rubber-bands the chat behind it. `touch-action` intersects along the hit-path, so the root declaration covers the whole subtree.

> **Pinned by `tests/style_lint.rs::orbit_map_overlay_blocks_touch_scroll`** (asserts `touch-action: none` in the base `.sk-orbit-map {` rule).

---

## 7. Modal & slide-over system (`style/_modal.scss`, ~1000 lines)

Two presentations sharing one Rust `Modal` (focus-trap / Esc / focus-restore / backdrop — see [07-ui-shell.md](./07-ui-shell.md)):

### Base modal (desktop / deck / hud)

`.modal-backdrop` — `position: fixed`, `inset: 0`, centering flex, `var(--scrim)` dim, `touch-action: none` (swallow pinch), `@supports` blur(4px). `.modal` — opaque `--surface` card, `max-width: 28rem`, `max-height: 85vh` with `overflow-y: auto` and the scrollbar **hidden** (`scrollbar-width: none` + `::-webkit-scrollbar { display: none }`) to kill the iOS overlay scroll indicator hard against the right edge. `:focus-visible` ring on focusable children. Entrance keyframes `modal-backdrop-enter` / `modal-enter`.

### Orbit slide-over (`.app.sk-orbit`)

Management surfaces (`.account-modal` / `.server-modal` / `.wardrobe-modal` / `.channel-manager` / `.persona-detail`) are rebuilt into the prototype's `#slideOver` (`a-orbit.html:293`): a **full-viewport panel** gliding from the right edge (`translateX(102%)` → `0`, `sk-orbit-slideover-in`), with an integrated **sticky head** (`.account-head` — back-arrow disc + "swipe → close" grip via `::after`) and **one scroll container**. This is a **pure-presentation rebuild** — the Rust a11y machinery and form logic are unchanged; the swipe-to-close drag is opt-in (`.modal--swipe-close` + an inline `--drag-x` the JS writes, `src/ui/modal.rs`).

iOS-sticky invariants baked into `_modal.scss:506-624`:
- The panel is the **SOLE** scroll container; the sticky head is its **direct child**. No `overflow != visible` / `transform` / `filter` / `backdrop-filter` ancestor may sit between the scroller and the sticky head, or iOS WebKit `position: sticky` silently breaks.
- The rest state must stay **transform-free** — a resting `transform` would become the containing block for a nested confirm's `position: fixed` backdrop and re-anchor it off-viewport. During drag, `.dragging` kills the spring-back transition; on release the inline transform eases home and is cleared on `transitionend`.
- The head carries `transform: translateZ(0)` to isolate its glass repaint from momentum-scroll judder, and `@include glass-holo($border: false)` for the integrated Liquid-Glass look.

**Compact orbit dialogs** (`.confirm-modal` / `.accent-modal` / `.persona-info`) are *re-centered* (`:has()`-keyed backdrop) and re-skinned to the void glass — without their own orbit rule they fell through to the base centered `.modal` while the shared backdrop had already dropped its centering flex, rendering as an orphaned top-left card (a real owner-reported iPhone bug).

> **Pinning:** modal *styling* is largely **(unpinned)** by `style_lint`. The *behaviors* are pinned: leaving a management surface pops one step back to its origin (Station or the map) and never into a channel — **`tests/style_lint.rs::management_modal_dismiss_returns_to_origin`** (exactly 3 `swipe_close=true` modals, each routing both `close=` and `<ModalHead on_close=…>` through `act::modal_back`, with an explicit no-channel-entry scan); the swipe engines bail pointer-capture on controls — **`::swipe_engines_bail_pointer_capture_on_controls`** (incl. `src/ui/modal.rs`); each pref toggle renders once — **`::each_pref_toggle_is_rendered_exactly_once`**.

---

## 8. The 44px touch-target floor (product-wide)

> Every registered interactive control declares a **≥ 44px (`2.75rem`)** height floor (`min-height`, plus `min-width` for square/icon buttons), applied at the control's **base/shared definition** — **never** as an `.app.sk-orbit` override.

This is the hard rule from **CLAUDE.md → "UI fidelity"** (owner ruling 2026-06-17): Mendicant Bias is touch-first across the **whole product**, not scoped to `sk-orbit`. Compact desktop-density controls are exactly the regression the product exists to retire.

There is **no blanket `button { min-height }`** — it would distort the bespoke chrome (composer orb, map nodes, swipe strip). Instead the floor is applied per-control at its base definition, and a **curated registry** in `style_lint` is the allowlist *by construction*: a new floored control joins it **by hand**. Deferred controls (`.persona-reorder` / `.channel-reorder` / `.gallery-remove`, pending the wardrobe rebuild) and bespoke geometry (composer orb, map nodes, `.member-avatar` `<img>`) are simply **not members**, which is how they are exempt. Square icon buttons floor via `height` (e.g. the `.row-edit` back-arrow disc, the `.manager-grip`/`.persona-grip`/`.lore-grip` finger-drag grips at `min-width`+`min-height: 2.75rem`).

> **Pinned by `tests/style_lint.rs::registered_interactive_controls_declare_44px_touch_floor`** — a **23-entry `FLOOR_CONTROLS` registry**. For each, `all_bodies` collects every rule whose head is `.<ctrl> {` and `declares_touch_floor` checks at least one declares `min-height`/`height` ≥ `2.75rem` (`len_to_rem` parses `rem`/`px`, **skips** `%`/`vh`/`calc`/`auto` rather than reading them as 0). Card-/detail-action buttons floor via a nested `button {}`, so the registry holds the **parent block** selector. The registry deliberately grows by hand — never an auto button-scan (which false-positives on image tiles, inline spans, list rows).

Registry members (current): `sk-orbit-pill`, `sk-orbit-sat`, `sk-orbit-chip`, `sk-orbit-orb`, `sk-orbit-station-close`, `sk-orbit-persona-card`, `sk-orbit-account-btn`, `accent-swatch`, `account-logout`, `account-save`, `row-edit`, `trash-toggle`, `trash-restore`, `member-role-btn`, `member-kick`, `add-row button`, `flist button`, `flist label`, `card-actions`, `detail-actions`, `toast-action`, `persona-grip`, `lore-grip`.

---

## 9. Base chat surface & the legibility/battery line (`style/_content.scss`)

The non-orbit chat surface: `.content` / `.channel-view` / `.messages` flex column, the `.msg` card with directional bubbles (`.own` / `.system` / `.roll` / `.msg-draft` / `.msg-ghost`), message effects, composer + toolbar + charge-ring send, the typing-indicator constellation, `.chat-avatar`, `.nova-orb`, the radial long-press menu, and `.jump-bottom`/`.unread-pill`.

**Glass is for chrome only — the load-bearing legibility/battery rule:**
- `.msg` is **opaque `--card`**, never glass (`_content.scss:103`) — a `backdrop-filter` inside a scroll list re-blurs every frame. `.roll-chip` is a glass *wash* over the same opaque base (`:491`).
- The per-persona name tint must use the **full selector depth** `.content .messages .msg .who.mk-*` (specificity 0,4,0) to beat the base `.who` color — a bare `.msg .who.mk-*` (0,3,0) loses. **Do not simplify** (`_markup.scss:54-67`).

**Message effects** (M4/T5–T6, all `transform`/`opacity` per the doctrine): `effect-whisper` (blur+reveal — *pure state*, must work under reduced-motion), `effect-shout` (`fx-shout` shake, keeps its warm tint when killed), `effect-spell` (`fx-spark` sparks with an fx-max breathing halo — spark s1 is relocated to `.text::before` to free the row `::before` for the halo, because a `.text`-anchored glow inherits `border-radius: 0` and paints a sharp "codeblock box"), `.msg.roll` (`fx-roll-in` + fx-max `fx-die-tumble`).

**Body-portal overlay contract** (`_foundation.scss:168`): the radial menu, lightbox, and mobile emoji picker are Leptos `<Portal>`s to `document.body` (each in its own `<Show>`), so `.content` stays transform-free and never traps them as a containing block. They own their z-index at the document level (`.radial-backdrop` z 60 — **`background: transparent`**, the B4-safe twin of the blossom scrim; `.radial-menu` arc z 61; lightbox z 50) and carry their own safe-area paddings.

> **Pinning:** the opaque-card / selector-depth rules are **code-pinned** at `style/_content.scss:103,491` + `style/_markup.scss:54-67` (no `style_lint` guard). The transparent-popover rule **is** pinned — `::scrim_only_on_modal_backdrops` (§6/below).

---

## 10. The visual oracle & the deck-bug-class guards

### Prototypes (`docs/superpowers/specs/assets/2026-06-12-skelettvagen/`)

Three skeleton HTML mockups are the **source-of-truth visual oracle**. `_sk_orbit_chrome.scss` and `_modal.scss` cite exact prototype line numbers (`a-orbit.html:NNN`):

| File | Concept | Status |
|------|---------|--------|
| `a-orbit.html` ("Omloppsbana") | the orbit chrome — `#pill`, `#orb`, `#composer`, `#orbitMap`, `#slideOver`, constellation, effects | **the chosen direction**; sk-orbit is built 1:1 against it |
| `b-deck.html` ("Kortdäck") | z-stacked holographic cards | reference only |
| `c-hud.html` ("Holoterminal") | zero-chrome edge-summoned holograms | reference only |

These are read-only references, not imported. The deleted `visual-gate/` Playwright tooling stays deleted (commit `68b65bd`); the fidelity gate is the **owner deck-pass** (see CLAUDE.md). `style_lint` is the per-class *external* signal, **not** a substitute for it.

### Deck-bug-class regression guards

One defect class kept slipping through: orbit chrome shipped compile/clippy/ssr/Chromium-green yet was a real WebKit/touch/visual defect caught only on the owner's physical iPhone — and a fixed property did not survive the next rewrite. Each guard pins one property by a pure static scan and was validated to turn **RED** on the pre-fix state of a named commit:

| Guard (`tests/style_lint.rs::`) | Bug class | RED-at | Pins |
|---|---|---|---|
| `scrim_only_on_modal_backdrops` | **B4** full-app blackout | `6c90d20^` | `var(--scrim)` only on `modal-backdrop`/`holopanel-scrim`/`sk-orbit-hints`; a non-modal catcher (`.radial-backdrop`, `.sk-orbit-blossom-scrim`) must be `background: transparent` (fx-max is unconditional, so an opaque scrim blacks out the whole app behind a tiny menu) |
| `sk_orbit_content_clips_both_axes_no_paint_containment` | iOS side/vertical scroll | `66270c9^` | `.sk-orbit-content` `overflow: clip` both axes; no `contain: paint` |
| `orbit_map_overlay_blocks_touch_scroll` | overlay rubber-band | fidelity gate | `.sk-orbit-map` `touch-action: none` |
| `backdrop_filter_always_has_webkit_sibling` | silent WebKit glass loss | F1 | one `-webkit-backdrop-filter` per standard decl, per file |
| `glass_holo_is_liquid_glass_not_frost_noise` | "TV static" / dropped blur | 2026-06-15 | `glass-holo` carries blur + `-webkit-`, no `--frost-noise` |
| `orbit_chrome_controls_inherit_glass_material` | **B2** flat chrome | `6c90d20^` | 9 orbit controls `@include glass-holo`/`glass-live` |
| `dispatch_pane_controls_inherit_glass_material` | **B2** flat dispatch | round-4 deck | 8 dispatch controls carry the material at base |
| `glass_holo_consumers_let_the_mixin_own_the_background` | dead background on WebKit | — | no top-level `background:` after the include |
| `no_dead_fx_max_negation` | permanently-dead CSS | (purged) | no `:not(.fx-max)` (fx-max is unconditional, `mod.rs:372`) |
| `no_html5_drag_and_drop_in_ui` | iOS-dead reorder | `bacbcf4^` | no `on:drag*`; `draggable` only literal `"false"` (reorder uses the pointer-capture grip) |
| `swipe_engines_bail_pointer_capture_on_controls` | desktop dead buttons | `569be68^` | `drag.rs`/`holopanel.rs`/`modal.rs` bail before `set_pointer_capture` on interactive controls |
| `each_pref_toggle_is_rendered_exactly_once` | duplicate toggle | `66f5e84^` | `dialogue_style`/`ghost_quill`/`haptic_vibrate` each one checkbox |
| `management_modal_dismiss_returns_to_origin` | settings-exit into a channel | `66f5e84^` | exactly 3 `swipe_close=true` modals dismiss via `act::modal_back` (origin-aware), none entering a channel |

**`fx-max` is rendered unconditionally** (`class="app fx-max"`, `src/ui/shell/mod.rs:372`) — it is the *appearance/eye-candy tier*, present on every session, so any `:not(.fx-max)` fallback is dead and any opaque scrim on a non-modal catcher is a permanent blackout. This single fact underlies `no_dead_fx_max_negation` and `scrim_only_on_modal_backdrops`.

---

## 11. The JS ↔ CSS custom-property contract

Component→CSS coupling is class-name + custom-property only. Beyond the `:root` tokens, Rust components write **inline** custom props and toggle **state classes** that the SCSS reads to drive composite-cheap transforms/opacity. The inline-written vars:

| Var | Written by | Read by | Meaning / range |
|-----|-----------|---------|-----------------|
| `--composer-h` | `src/ui/shell/channel/mod.rs` (ResizeObserver on `<html>`) | jump-bottom / unread-pill / toast / orbit messages | live composer height (px) |
| `--accent`, `--glow-accent` | `src/ui/accent.rs` (inline style on `.app`) | everything reading the accent | per-guild rebind from `guild.accent_color`; default = token |
| `--strip-x` | `src/ui/shell/sk_orbit/drag.rs` | swipe strip | horizontal swipe offset |
| `--p`, `--drag-x` | `drag.rs` / `holopanel.rs` / `modal.rs` | panel/slide-over transforms | drag progress / pixel offset |
| `--charge`, `--dash` | `shell/channel/mod.rs` (compose signal) | `.send` conic ring / `.sk-orbit-ring-arc` | 0..1 fill / `stroke-dashoffset` |
| `--orbit-r`, `--orbit-a`, `--orbit-period`, `--orbit-d` | `src/ui/shell/sk_orbit/orbit_map.rs` | Kepler node placement | seeded radius / start angle / period |
| `--scene-tint` | shell (active speaker) | `.sk-orbit-content::before` (fx-max scene-light) | active-speaker tint |
| `--warp-dir` | act layer (deferred) | `.channel-view` directional warp | +1/-1/0 |
| `--chip-i` | effect-blossom render | `.sk-orbit-chip` stagger | chip index |
| `--drag-x` (modal) | `src/ui/modal.rs` | slide-over `transform` | swipe-to-close px (cleared on release) |

State classes the SCSS keys on (per-component naming contracts): `.composing` / `.armed` / `.charging` / `.full` / `.sent` / `[data-armed="whisper|shout|spell"]` / `.revealed` / `.dragging` / `.drag-over` / `.worn` / `.diving` / `.fx-switching`, plus the modifier classes `--snap` / `--single`.

> **Pinning:** the var contract is **(unpinned)** as a registry — it is enforced at each consumer pair (the Rust writer + the SCSS reader). The pointer-gesture writers are partly pinned via `swipe_engines_bail_pointer_capture_on_controls`.

---

## Source map

**Stylesheets**
- `style/main.scss` — entry; the `@use` cascade-order contract.
- `style/_tokens.scss` — design tokens (the only hex-literal file); Void Station palette, Duo font stacks, motion eases/durations.
- `style/_typography.scss` — 10 `@font-face` (Duo self-hosted woff2).
- `style/_base.scss` — resets, base `button`/`input`, the global `0.01ms` reduced-motion freeze (`:86-95`).
- `style/_foundation.scss` — the three glass mixins (`glass-etched`/`glass-holo`/`glass-live`), aurora layer, accent-swatch, body-portal overlay contract.
- `style/_motion.scss` — keyframe library + the two-layer reduced-motion kill; the composite-only doctrine header.
- `style/_content.scss` — base chat surface; opaque `.msg` card, charge-ring send, message effects, radial menu, typing constellation.
- `style/_markup.scss` — per-persona name-tint selector-depth contract (`:54-67`).
- `style/_modal.scss` — base modal + orbit full-screen slide-over rebuild; iOS-sticky invariants.
- `style/_sk_orbit_chrome.scss` — the `.app.sk-orbit` cosmic shell (pill/orb/map/composer/station/starfield).
- `style/_sk_orbit_chat.scss` — orbit chat-card look (loads right after the chrome).
- `style/_wave_b.scss` / `style/_wardrobe.scss` — dispatch panes + persona cards/editor (the non-orbit `glass-holo` consumers).
- `style/_skeleton.scss` / `style/_attachments.scss` / `style/_toast.scss` / `style/_holopanel.scss` / `style/_lorebook.scss` / `style/_lightbox.scss` / `style/_trash.scss` / `style/_crest.scss` / `style/_auth.scss` / `style/_msg_who.scss` / `style/_mobile.scss` / `style/_mobile_emoji.scss` — the remaining partials.

**Components that write the contract**
- `src/ui/accent.rs` — rebinds `--accent`/`--glow-accent` per guild.
- `src/ui/shell/mod.rs` — renders `class="app fx-max"` unconditionally (`:372`).
- `src/ui/shell/channel/mod.rs` — `--composer-h` (ResizeObserver), `--charge`/`--dash`, state classes.
- `src/ui/shell/sk_orbit/{drag,holopanel,orbit_map}.rs`, `src/ui/modal.rs` — pointer-gesture engines writing `--strip-x`/`--p`/`--drag-x`/`--orbit-*`.

**Visual oracle (read-only)**
- `docs/superpowers/specs/assets/2026-06-12-skelettvagen/{a-orbit,b-deck,c-hud}.html` — the prototypes the SCSS is built 1:1 against and cites by line number.

**The canonical pin — `tests/style_lint.rs`** (pure file scan, every graph; in `.githooks/pre-commit`, not `/check`)
- Motion doctrine: `no_keyframes_animate_paint_or_layout_properties`.
- WebKit Liquid Glass: `backdrop_filter_always_has_webkit_sibling`, `glass_holo_is_liquid_glass_not_frost_noise`, `glass_holo_consumers_let_the_mixin_own_the_background`.
- Material inheritance: `orbit_chrome_controls_inherit_glass_material`, `dispatch_pane_controls_inherit_glass_material`.
- iOS/WebKit footguns: `sk_orbit_content_clips_both_axes_no_paint_containment`, `orbit_map_overlay_blocks_touch_scroll`, `scrim_only_on_modal_backdrops`, `no_dead_fx_max_negation`, `no_html5_drag_and_drop_in_ui`, `swipe_engines_bail_pointer_capture_on_controls`.
- Modal/pref behavior: `management_modal_dismiss_returns_to_origin`, `each_pref_toggle_is_rendered_exactly_once`.
- Touch floor: `registered_interactive_controls_declare_44px_touch_floor` (23-entry registry).

**Cross-links:** [01-overview.md](./01-overview.md) (graph split) · [07-ui-shell.md](./07-ui-shell.md) (component side) · [09-testing.md](./09-testing.md) (style_lint vs pre-commit vs /check) · [11-build-deploy-pwa.md](./11-build-deploy-pwa.md) (SCSS→CSS build, SW font cache) · CLAUDE.md → "UI fidelity" (the canonical touch-floor + deck-pass statement).
