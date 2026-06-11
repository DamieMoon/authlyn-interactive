# Mendicant Bias W2: Design System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Grimoire palette with the Void Station × Liquid Glass design system — semantic tokens, Duo typography (Space Grotesk chrome + Crimson Pro prose), glass/motion/aurora foundations, the `.fx-max` appearance-tier scaffolding, and the inline-SVG icon module — leaving the app fully usable and visually coherent for W3/W4 to build on.

**Architecture:** Token-level swap with a SEMANTIC RENAME (`--parchment`→`--void`, `--gold`→`--accent`, …) applied mechanically across all partials, plus tokenization of the 9 hardcoded rgba escape vectors the audit found. New `_foundation.scss` (glass/aurora utilities) and `_motion.scss` (keyframes + reduced-motion kill) partials. Appearance tier = one localStorage pref + one root class. Icons = always-on Leptos components (`src/ui/icons.rs`), one proof-site swap now, mass adoption in W3/W4.

**Tech Stack:** dart-sass via cargo-leptos (`style-file = "style/main.scss"`), CSS custom properties, fontsource woff2 (SIL OFL), Leptos view! SVG.

**Spec:** `docs/superpowers/specs/2026-06-10-mendicant-bias-design.md` §1, §12 W2. Reference mockups: `docs/superpowers/specs/assets/2026-06-10-mendicant-bias/final-design-v4.html`.

**Gates (no wasm test harness for SCSS/UI):** `cargo leptos build` (dart-sass compiles), grep-gates (zero old-token references; every consumed `var(--x)` defined), clippy ssr+hydrate+freya, full ssr suite green, visual smoke screenshots via the node-playwright harness in `/tmp/nova-gif/` (NEVER against prod).

**Branch:** `mendicant-bias` (already on it). Commit per task. Push only via the orchestrator.

---

### Task 1: Void Station tokens + semantic rename + escape-vector tokenization

**Files:**
- Rewrite: `style/_tokens.scss`
- Mechanical rename in: every `style/_*.scss` that consumes old token names
- Targeted edits: `style/_markup.scss:131`, `style/_wave_b.scss:52,252,375`, `style/_rail.scss:40`, `style/_sidebar.scss:77,86`, `style/_lightbox.scss:39,54,88`, `style/_mobile.scss:41`, `style/_trash.scss:106`

- [ ] **Step 1.1: Write the new `style/_tokens.scss`** (full replacement):

```scss
// Design tokens — the only file in `style/` that carries hex literals.
//
// W2 (mendicant-bias): values + NAMES are now the **Void Station palette**
// (deep-space graphite, electric-blue accent with restrained glow, Duo
// typography). Token names are semantic (--void/--surface/--accent…) so the
// next redesign is again a value-only edit of THIS FILE.
//
// SCSS `$vars` are kept dual-tracked: they're consumed by the few
// `rgba($var, alpha)` sites where SCSS-time substitution is still simpler.
// UPDATE THEM TOGETHER with the `:root` tokens — they must stay consistent.

$bg: #0b0e14;
$panel: #0e121a;
$panel-2: #131927;
$line: #1a2130;
$text: #dde6f2;
$muted: #8a98ad;
$accent: #4d9fff;

:root {
	// Core surfaces (Void Station: layered deep-space graphite)
	--void-deep: #07090f; // darkest base, page edges / rail bg
	--void: #0b0e14; // page surface
	--surface: #0e121a; // raised chrome panels (rail / sidebar / modal)
	--card: #10141d; // content cards (.msg) — calm, opaque, NOT glass
	--surface-2: #131927; // hover / active surface
	--line: #1a2130; // hairline borders
	--line-strong: #222b3c; // emphasized borders (inputs, frames)
	--input-bg: #0a0d13; // form-input background (darker than the page)

	// Ink
	--text: #dde6f2; // primary text
	--text-soft: #aab8cc; // secondary text / chat author name
	--text-muted: #8a98ad; // tertiary text
	--text-faint: #5d6b80; // hints, timestamps, placeholders

	// Electric-blue accent family — three steps for hover/disabled
	--accent: #4d9fff; // primary accent
	--accent-bright: #7fb6ff; // hover / highlight
	--accent-soft: #2a4a8a; // subdued accent for borders / disabled
	--live: #8ee6c8; // online / live / success (mint)

	// Danger — desaturated red tuned for the dark base
	--danger: #e8707b;
	--danger-soft: #c45a64;
	--danger-hover: #d4626d;
	--danger-border: #7a3038;

	// Role / status colors
	--role-admin: #7fb6ff;

	// Attachment + backdrop surfaces
	--attachment-bg: #0a0d13;
	--backdrop-deep: #05070b; // floating-button bg (gallery-remove, lightbox-close)
	--avatar-tile: #1c2433; // monogram fallback frame in `chat-avatar`

	// Glass material (chrome only — never on prose cards in Standard)
	--glass-bg: rgba(20, 26, 40, 0.55);
	--glass-line: rgba(255, 255, 255, 0.08);
	--glass-highlight: rgba(255, 255, 255, 0.09);

	// Glow + overlay tokens (replace the audited hardcoded rgba escape vectors)
	--glow-accent: rgba(77, 159, 255, 0.55);
	--unread-glow: rgba(77, 159, 255, 0.7); // rail/channel unread markers
	--pinged-glow: rgba(255, 180, 127, 0.8); // @-mention warmth stays warm
	--mention-bg: rgba(77, 159, 255, 0.14);
	--spoiler-bg: rgba(127, 182, 255, 0.14);
	--emoji-fav-bg: rgba(255, 154, 213, 0.15);
	--overlay: rgba(14, 18, 26, 0.9); // lightbox/floating control chips
	--scrim: rgba(4, 6, 10, 0.6); // mobile drawer / modal backdrops

	// Aurora ambience (W2: static tint; .fx-max animates it)
	--aurora-1: rgba(22, 33, 58, 0.85); // blue nebula tint (top-right)
	--aurora-2: rgba(13, 37, 48, 0.7); // teal nebula tint (bottom-left)

	// Persona-tint palette — luminous against --card; same 8 roles as before.
	--tint-red: #ff8a96;
	--tint-orange: #ffb47f;
	--tint-yellow: #ffd47f;
	--tint-green: #8ee6c8;
	--tint-blue: #7fb6ff;
	--tint-purple: #c4a8ff;
	--tint-pink: #ff9ad5;
	--tint-gray: #9aa7bd;

	// Duo typography (spec §1): tech sans chrome, serif prose, mono meta.
	--font-ui:
		"Space Grotesk", -apple-system, "SF Pro Display", "Segoe UI", system-ui, sans-serif;
	--font-prose:
		"Crimson Pro", Georgia, "Iowan Old Style", Cambria, "Times New Roman", serif;
	--font-display: var(--font-ui);
	--font-mono: "JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas, monospace;

	// Motion — quick & precise; spring for entrances. Decorative motion dies
	// under prefers-reduced-motion (see _motion.scss).
	--ease-page: cubic-bezier(0.4, 0, 0.2, 1);
	--ease-quick: cubic-bezier(0.4, 0, 0.6, 1);
	--ease-spring: cubic-bezier(0.2, 0.9, 0.3, 1.15);
	--dur-quick: 80ms;
	--dur-base: 150ms;
	--dur-page: 200ms;
	--dur-entrance: 450ms;
}
```

- [ ] **Step 1.2: Mechanical rename across partials.** Apply this exact mapping in every `style/_*.scss` EXCEPT `_tokens.scss` (old name → new name):

| old | new | | old | new |
|---|---|---|---|---|
| `--parchment-deep` | `--void-deep` | | `--gold-warm` | `--accent-bright` |
| `--parchment` | `--void` | | `--gold-soft` | `--accent-soft` |
| `--vellum-2` | `--surface-2` | | `--gold` | `--accent` |
| `--vellum` | `--surface` | | `--ink-danger-hover` | `--danger-hover` |
| `--rule-line` | `--line` | | `--ink-danger-border` | `--danger-border` |
| `--ink-soft` | `--text-soft` | | `--ink-danger-soft` | `--danger-soft` |
| `--ink-muted` | `--text-muted` | | `--ink-danger` | `--danger` |
| `--ink` | `--text` | | `--font-body` | `--font-ui` |

ORDER MATTERS (longest-prefix first within each family — `--parchment-deep` before `--parchment`, `--ink-danger-*` before `--ink-danger` before `--ink-soft`/`--ink-muted` before `--ink`, `--gold-*` before `--gold`). Run as one sed pass:

```bash
cd /Users/damien/Developer/authlyn-interactive/style
for f in _*.scss; do [ "$f" = "_tokens.scss" ] && continue; sed -i '' \
  -e 's/--parchment-deep/--void-deep/g' -e 's/--parchment/--void/g' \
  -e 's/--vellum-2/--surface-2/g' -e 's/--vellum/--surface/g' \
  -e 's/--rule-line/--line/g' \
  -e 's/--ink-danger-hover/--danger-hover/g' -e 's/--ink-danger-border/--danger-border/g' \
  -e 's/--ink-danger-soft/--danger-soft/g' -e 's/--ink-danger/--danger/g' \
  -e 's/--ink-soft/--text-soft/g' -e 's/--ink-muted/--text-muted/g' -e 's/--ink/--text/g' \
  -e 's/--gold-warm/--accent-bright/g' -e 's/--gold-soft/--accent-soft/g' -e 's/--gold/--accent/g' \
  -e 's/--font-body/--font-ui/g' "$f"; done
```

CAUTION: `-e 's/--ink/--text/g'` runs LAST in the ink family; verify no other token begins with `--ink` (`grep -rn '\-\-ink' style/` must be empty afterwards). `--card`, `--line-strong`, `--text-faint` are NEW names — nothing maps to them mechanically; they get adopted by later waves (and the targeted edits below).

- [ ] **Step 1.3: Targeted escape-vector edits** (exact replacements):

| File:line | old value | new |
|---|---|---|
| `_markup.scss:131` | `rgba(99, 132, 255, 0.14)` | `var(--mention-bg)` |
| `_wave_b.scss:52` | `rgba(110, 168, 230, 0.14)` | `var(--spoiler-bg)` |
| `_wave_b.scss:252` | `rgba(180, 86, 106, 0.15)` | `var(--emoji-fav-bg)` |
| `_wave_b.scss:375` | `rgba(180, 86, 106, 0.15)` | `var(--emoji-fav-bg)` |
| `_rail.scss:40` | `rgba(238, 161, 96, 0.7)` | `var(--unread-glow)` |
| `_sidebar.scss:77` | `rgba(245, 245, 245, 0.7)` | `var(--unread-glow)` |
| `_sidebar.scss:86` | `rgba(238, 161, 96, 0.8)` | `var(--pinged-glow)` |
| `_lightbox.scss:39,54,88` | `rgba(28, 26, 43, 0.9)` | `var(--overlay)` |
| `_mobile.scss:41` | `rgba(0, 0, 0, 0.5)` | `var(--scrim)` |

Leave generic `rgba(0,0,0,…)` shadows elsewhere (design-independent). In `_trash.scss:106` keep `rgba($accent, 0.12)` but the `$accent` SCSS var is now `#4d9fff` (dual-track updated in Step 1.1) — add the warning comment: `// $accent is the SCSS twin of --accent (_tokens.scss): update BOTH.`

- [ ] **Step 1.4: Grep-gates (these are this task's "tests")**

```bash
cd /Users/damien/Developer/authlyn-interactive
# 1) No old token name survives anywhere (style + rust + public):
grep -rnE -- '--(parchment|vellum|rule-line|ink|gold|font-body)' style/ src/ public/ && echo "GATE-1 FAIL" || echo "GATE-1 OK"
# 2) Every consumed var(--x) is defined in _tokens.scss:
comm -23 <(grep -rhoE 'var\(--[a-z0-9-]+' style/ src/ | sed 's/var(//' | sort -u) \
         <(grep -roE -- '--[a-z0-9-]+' style/_tokens.scss | cut -d: -f2- | sort -u) \
  | grep -v '^$' && echo "GATE-2 FAIL (undefined tokens above)" || echo "GATE-2 OK"
```

Expected: both OK. (GATE-2's comm finds consumed-but-undefined tokens; defined-but-unconsumed is fine — new tokens await later waves.)

- [ ] **Step 1.5: Build + suite + commit**

```bash
cargo leptos build 2>&1 | tail -2   # dart-sass must compile the renamed partials
cargo test --features ssr 2>&1 | grep -c "test result: ok"   # 22, no FAILED
git add style/ && git commit -m "feat(design): Void Station tokens — semantic rename + escape-vector tokenization

The Grimoire palette retires: deep-space surfaces, electric-blue accent
with restrained glow, luminous persona tints, glass/glow/aurora token
families. Token NAMES are now semantic (--void/--surface/--accent…),
renamed mechanically across every partial; the 9 audited hardcoded rgba
escape vectors now ride tokens (--mention-bg, --unread-glow, --overlay…).
SCSS \$vars dual-track updated.

Tests: grep-gates (no stale token names; all consumed tokens defined); cargo leptos build; ssr suite green"
```

---

### Task 2: Duo typography — Space Grotesk in, EB Garamond out, prose stays serif

**Files:**
- Modify: `style/_typography.scss`, `style/_tokens.scss` (font tokens landed in Task 1)
- Add: `public/fonts/space-grotesk-400.woff2`, `public/fonts/space-grotesk-600.woff2`
- Delete: `public/fonts/eb-garamond-400.woff2`, `public/fonts/eb-garamond-600.woff2`
- Targeted prose-site edits: `style/_content.scss`, `style/_markup.scss`, `style/_lorebook.scss`, `style/_wardrobe.scss`

- [ ] **Step 2.1: Fetch the fonts** (SIL OFL, fontsource jsdelivr mirror — same source as the existing fonts):

```bash
cd /Users/damien/Developer/authlyn-interactive/public/fonts
curl -fsSL -o space-grotesk-400.woff2 https://cdn.jsdelivr.net/npm/@fontsource/space-grotesk@5/files/space-grotesk-latin-400-normal.woff2
curl -fsSL -o space-grotesk-600.woff2 https://cdn.jsdelivr.net/npm/@fontsource/space-grotesk@5/files/space-grotesk-latin-600-normal.woff2
ls -la space-grotesk-*.woff2   # expect ~15-25 KB each; non-empty
rm eb-garamond-400.woff2 eb-garamond-600.woff2
```

- [ ] **Step 2.2: Rewrite `style/_typography.scss`:** replace both EB Garamond `@font-face` blocks with Space Grotesk 400/600 (`src: url("/fonts/space-grotesk-400.woff2")` etc., `font-display: swap` kept); update the header comment: Duo typography — Space Grotesk (UI chrome, 400+600) + Crimson Pro (prose, 400+600), both SIL OFL, fontsource mirror.

- [ ] **Step 2.3: Prose sites keep the serif.** Task 1's rename made `--font-ui` the body default (old `--font-body` consumers). Flip the PROSE surfaces to `var(--font-prose)` — add `font-family: var(--font-prose);` to exactly these selectors (locate by grep; line numbers drift):
  - `_content.scss`: the message body text rule (`.msg .text` — grep `\.text` in the messages section) and the composer `textarea` rule
  - `_markup.scss`: `.mk-dialogue` (if it sets a font) — verify; prose markup inherits from `.text` otherwise
  - `_lorebook.scss`: the lore entry body/description rule
  - `_wardrobe.scss`: the persona description text rule
  Read each partial first; if a site already inherits the body font with no explicit font-family, ADD the prose declaration to the listed selectors only.

- [ ] **Step 2.4: Gate + commit**

```bash
grep -rn "EB Garamond\|eb-garamond" style/ src/ public/ && echo FAIL || echo OK   # zero references
cargo leptos build 2>&1 | tail -2
git add style/ public/fonts/ && git commit -m "feat(design): Duo typography — Space Grotesk chrome, Crimson Pro prose

EB Garamond retires with the Grimoire look. UI chrome (body default)
is now Space Grotesk 400/600; the story itself — message text, composer,
lorebook, persona descriptions — stays Crimson Pro via --font-prose.

Tests: zero eb-garamond references; cargo leptos build; visual smoke in W2 verification"
```

---

### Task 3: Foundation partials — glass, aurora, motion

**Files:**
- Create: `style/_foundation.scss`, `style/_motion.scss`
- Modify: `style/main.scss` (two `@use` lines after `base`)

- [ ] **Step 3.1: Create `style/_foundation.scss`:**

```scss
// Void Station material foundations (W2). Glass is for CHROME (topbar,
// tabbar, sheets, modals) — prose cards stay opaque (--card) for legibility
// and battery; see spec §1. Consumers opt in via @include glass / .glass.
@use "sass:meta";

@mixin glass {
	background: var(--glass-bg);
	-webkit-backdrop-filter: blur(14px) saturate(1.4);
	backdrop-filter: blur(14px) saturate(1.4);
	border: 1px solid var(--glass-line);
	box-shadow: inset 0 1px 0 var(--glass-highlight);
}

.glass {
	@include glass;
}

// Aurora ambience: two soft nebula tints behind everything. Static in
// Standard; .fx-max (appearance tier) animates a slow drift. Painted on a
// fixed pseudo-layer so the grid layout above is untouched.
.app::before {
	content: "";
	position: fixed;
	inset: 0;
	z-index: -1;
	pointer-events: none;
	background:
		radial-gradient(ellipse 90% 55% at 80% -10%, var(--aurora-1) 0%, transparent 60%),
		radial-gradient(ellipse 65% 45% at 5% 110%, var(--aurora-2) 0%, transparent 55%),
		var(--void);
}

// Appearance tier: .fx-max escalates ambience. W2 ships the scaffolding +
// the first demonstrative effects; W5/W11 add the full Eye-candy set here.
.app.fx-max::before {
	animation: aurora-drift 9s ease-in-out infinite alternate;
}
.app.fx-max .rail-guild.active,
.app.fx-max .channel.active {
	box-shadow: 0 0 14px var(--glow-accent);
}
```

- [ ] **Step 3.2: Create `style/_motion.scss`:**

```scss
// Motion library (W2): shared keyframes + the reduced-motion kill switch.
// Components reference these names; .fx-max layers more in W5/W11.

@keyframes aurora-drift {
	from {
		transform: translate3d(0, 0, 0) scale(1);
		opacity: 0.85;
	}
	to {
		transform: translate3d(12px, -10px, 0) scale(1.06);
		opacity: 1;
	}
}

@keyframes fx-slide-in {
	from {
		opacity: 0;
		transform: translateY(14px) scale(0.98);
	}
	to {
		opacity: 1;
		transform: translateY(0) scale(1);
	}
}

@keyframes fx-glow-pulse {
	0%,
	100% {
		box-shadow: 0 0 6px var(--glow-accent);
	}
	50% {
		box-shadow: 0 0 16px var(--glow-accent);
	}
}

@keyframes fx-sweep {
	from {
		transform: translateX(-130%) skewX(-16deg);
	}
	to {
		transform: translateX(330%) skewX(-16deg);
	}
}

// Decorative motion dies wholesale under reduced-motion. Scoped to the
// fx- keyframe consumers + the aurora layer — NOT a global animation kill
// (the skeleton shimmer already handles its own reduced-motion rule).
@media (prefers-reduced-motion: reduce) {
	.app::before,
	.app.fx-max::before,
	[class*="fx-"] {
		animation: none !important;
	}
}
```

- [ ] **Step 3.3:** `style/main.scss`: add `@use "foundation";` and `@use "motion";` immediately after `@use "base";`. Run the GATE-2 token check from Task 1 again (the new partials consume tokens). `cargo leptos build` → ok.

- [ ] **Step 3.4: Commit**

```bash
git add style/ && git commit -m "feat(design): glass, aurora, and motion foundations

@include glass for chrome surfaces; a fixed aurora pseudo-layer behind
the app (static in Standard, drifting under .fx-max); shared fx-
keyframes with a scoped prefers-reduced-motion kill.

Tests: token grep-gate; cargo leptos build; visual smoke in W2 verification"
```

---

### Task 4: `.fx-max` appearance-tier scaffolding (pref + root class + toggle)

**Files:**
- Modify: `src/ui/shell/state.rs` (Prefs), `src/ui/shell/act/prefs.rs`, `src/ui/shell/mod.rs` (root class + Prefs init), `src/ui/shell/account.rs` (toggle row)

- [ ] **Step 4.1:** `state.rs` `Prefs` gains (matching the `dialogue_style` field's doc style):

```rust
    /// When on, the `.fx-max` root class unlocks the Eye-candy appearance
    /// tier (animated aurora, stronger glows; W5/W11 add the full set).
    /// Standard is the default. Persisted to localStorage.
    pub(crate) eyecandy: RwSignal<bool>,
```

- [ ] **Step 4.2:** `act/prefs.rs` gains the pair, EXACTLY mirroring `rp_dialogue_style_enabled`/`set_rp_dialogue_style` (hydrate-real + ssr no-op, localStorage key `"authlyn.eyecandy"`, absent = OFF):

```rust
/// Eye-candy appearance tier (`.fx-max`). Default OFF (Standard).
pub fn eyecandy_enabled() -> bool { /* mirror rp_dialogue_style_enabled with the new key */ }
pub fn set_eyecandy(on: bool) { /* mirror set_rp_dialogue_style */ }
```

(Copy the existing fns' bodies verbatim, swapping the key — read the file first; do not invent a new storage idiom.)

- [ ] **Step 4.3:** `mod.rs`: initialize `eyecandy: RwSignal::new(act::eyecandy_enabled())` wherever `Prefs { dialogue_style: … }` is constructed (find it; mirror the init pattern), and extend the root div (line ~450):

```rust
class:fx-max=move || s.prefs.eyecandy.get()
```

- [ ] **Step 4.4:** `account.rs` (after the dialogue-style `pref-row`, same structure):

```rust
<label class="pref-row">
    <input type="checkbox" prop:checked=move || s.prefs.eyecandy.get()
        on:change=move |ev| {
            let on = event_target_checked(&ev);
            s.prefs.eyecandy.set(on);
            act::set_eyecandy(on);
        }/>
    <span>"Eye-candy appearance (extra glow & motion)"</span>
</label>
```

(Export `eyecandy_enabled`/`set_eyecandy` through `act/mod.rs` like the other prefs fns.)

- [ ] **Step 4.5: Gates + commit:** clippy hydrate(wasm32)+ssr+freya clean; ssr suite green; `cargo leptos build` ok.

```bash
git add src/ui/ && git commit -m "feat(client): .fx-max appearance-tier scaffolding

authlyn.eyecandy pref (localStorage, default Standard), root-class hook,
and the Account toggle. The tier's full effect set lands in W5/W11; W2
ships drifting aurora + stronger active-glows as proof.

Tests: clippy clean on all three graphs; ssr suite green (stub parity)"
```

---

### Task 5: Icon component module + one proof site

**Files:**
- Create: `src/ui/icons.rs`
- Modify: `src/ui/mod.rs` (module decl), ONE proof site: the account-modal close button in `src/ui/shell/account.rs` (a `"✕"` text glyph → `<IconClose/>`)

- [ ] **Step 5.1: Create `src/ui/icons.rs`** — always-on (ssr+hydrate; NO ssr-only imports), stroke-based 24-viewBox icons inheriting `currentColor`. Component skeleton (repeat for each icon with its path data):

```rust
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
        pub fn $name() -> impl IntoView {
            view! {
                <svg class="icon" viewBox="0 0 24 24" fill="none"
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
```

CAVEAT: if Leptos's `view!` rejects `inner_html` on `<svg>` in this version, fall back to writing each component's `view!` with literal `<path d="…"/>` children instead of the macro (same path data, more boilerplate) — semantics identical; report which form compiled.

- [ ] **Step 5.2:** `src/ui/mod.rs`: add `pub mod icons;` matching the existing module list style. Add a base rule to `style/_base.scss`: `.icon { width: 1.1em; height: 1.1em; vertical-align: -0.18em; }`.

- [ ] **Step 5.3: Proof site:** in `src/ui/shell/account.rs`, replace the modal close button's `"✕"` text child with `<crate::ui::icons::IconClose/>` (import per file style). EXACTLY ONE site — the rest are W3/W4.

- [ ] **Step 5.4: Gates + commit:** clippy ×3 clean (icons are always-on: the freya graph compiles `ui`? CHECK — if `src/ui/` is NOT in the freya graph (native has its own ui), freya clippy is unaffected; verify and note). ssr suite green; `cargo leptos build` ok.

```bash
git add src/ui/ style/_base.scss && git commit -m "feat(design): inline-SVG icon component module

18 stroke-based currentColor icons (24-viewBox) as always-on Leptos
components; the account modal's close glyph is the integration proof.
W3/W4 retire the remaining text glyphs.

Tests: clippy clean (all graphs); ssr suite green; visual smoke in W2 verification"
```

---

### Task 6: PWA chrome colors

**Files:**
- Modify: `src/app.rs:28` (`theme-color` meta), `public/manifest.webmanifest` (`background_color`, `theme_color`)

- [ ] **Step 6.1:** Both files: `#221c16` → `#0b0e14`. (Two values in the manifest, one meta in app.rs.)
- [ ] **Step 6.2:** `grep -rn "221c16" src/ public/ style/` → empty. Commit:

```bash
git add src/app.rs public/manifest.webmanifest && git commit -m "feat(design): PWA chrome adopts the Void Station base color

Tests: zero #221c16 references remain"
```

---

### Task 7: W2 verification gate + visual smoke

**Files:** none (verification; screenshots as deliverables)

- [ ] **Step 7.1: Full gate**

```bash
cargo fmt --all --check && cargo clippy --features ssr 2>&1 | tail -1 \
  && cargo clippy --features hydrate --target wasm32-unknown-unknown 2>&1 | tail -1 \
  && cargo clippy --features freya 2>&1 | tail -1 \
  && cargo test --features ssr 2>&1 | grep -cE "test result: ok" \
  && cargo leptos build --release 2>&1 | tail -2
```

Expected: clean, 22 ok suites, release build completes (wasm-opt included — first --release of the branch).

- [ ] **Step 7.2: Visual smoke** (local dev server + the node-playwright harness at `/tmp/nova-gif/` — pattern from W1's smoke; NEVER prod): `cargo leptos watch` in background → screenshot `/login` (Void Station auth card), register a throwaway account → screenshot the shell (rail/sidebar/content in new palette + aurora), toggle Eye-candy in the account modal → screenshot with `.fx-max` active, and confirm in the DOM that `.app.fx-max` is set and Space Grotesk loaded (`document.fonts.check('1em "Space Grotesk"')`). Save PNGs to `/tmp/w2-smoke-*.png`. Eyeball: prose (message text) still serif; no unreadable contrast; no leftover parchment-brown surfaces.

- [ ] **Step 7.3: Wrap-up:** kill the dev server; update CLAUDE.md's identity/conventions ONLY if something it states became false (the palette isn't described there — likely no edit); final commit if needed; the orchestrator pushes and runs the final wave review.

---

## Done = W2 exit criteria

1. App fully usable in the Void Station palette: dark graphite surfaces, electric-blue accent, luminous tints, aurora layer, glass+motion foundations available.
2. Duo typography live: Space Grotesk chrome, Crimson Pro prose, EB Garamond gone.
3. `.fx-max` toggle works end-to-end (pref → root class → visible aurora drift).
4. Icon module compiles in all graphs with one proven integration site.
5. Grep-gates pass (no stale tokens; all consumed tokens defined; no #221c16).
6. Full gate green incl. a `--release` build (wasm-opt path exercised).
7. Visual smoke screenshots delivered to the owner.

**Next plan:** W3 (shell & navigation — hybrid mobile nav, desktop reskin, directional bubbles) written against the post-W2 tree.
