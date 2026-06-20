# Mendicant Bias M3: Shell & Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Replace the mobile edge-swipe drawer with the hybrid navigation (bottom tab bar Chat/Servers/Friends/Personas + a glass channel-switch bottom-sheet), reskin the desktop 3-column shell into Void Station chrome (glass topbar, mono timestamps, uppercase persona names), and add directional message bubbles (the viewer's own messages align right). Desktop keeps its efficient 3-column grid; mobile becomes thumb-first.

**Architecture:** Mobile/desktop split at the existing 768px breakpoint. Desktop: the 3-column grid stays, reskinned. Mobile: the grid collapses to single-column content; the rail+sidebar stop being swipe-drawers and instead surface as (a) a Servers tab pane and (b) a tap-triggered glass bottom-sheet for fast channel switching (the proven `wardrobe_open` modal-signal pattern). Directional bubbles are a per-message `mine` class already computed at render time — CSS-only flip, scoped to avoid the `.who.mk-*` persona-tint cascade. The `wire_swipe_drawer` JS and `.nav-open` drawer transforms are deleted.

**Tech Stack:** Leptos 0.8 view!, the M2 design system (tokens/glass mixin/icons/`.fx-max`), dart-sass, `matchMedia("(pointer: coarse)")` for touch detection (existing pattern).

**Spec:** `docs/superpowers/specs/2026-06-10-mendicant-bias-design.md` §1 (directional bubbles, mono meta, uppercase names, glass chrome), §2 (hybrid nav). Mockups: `assets/2026-06-10-mendicant-bias/final-design-v4.html`, `mobile-nav-hybrid.html`.

**Gates:** `cargo leptos build`, clippy ssr+hydrate(wasm32)+freya, full ssr suite (23 ok), visual smoke (desktop + mobile-viewport screenshots via `/tmp/nova-gif/` node-playwright — NEVER prod). Use the M2 icon components (`src/ui/icons.rs`) for tab/topbar glyphs; they take an optional `class` prop.

**Branch:** `mendicant-bias`. Commit per task. Orchestrator pushes.

**M2 carry-forward to verify as you touch these surfaces:** member roster role pills (two blues), rail unread dots, Nova DOT system wash, skeleton contrast — eyeball during the smoke.

---

### Task 1: Directional message bubbles (CSS-only, desktop+mobile)

**Files:**
- Modify: `src/ui/shell/channel/mod.rs` (add `.own` to the `.msg` li class when `mine`), `style/_content.scss` (`.msg.own` rules), `style/_msg_who.scss` (verify no conflict)

The `mine` boolean already exists at the message-map site (`channel/mod.rs:~475` — `me.as_deref() == Some(m.author_id.as_str())`). It's passed to `message_meta` but the `<li>` wrapper doesn't carry it yet.

- [ ] **Step 1.1:** In `channel/mod.rs`, find `li_class` (the computed class string for the message `<li>`, ~line 480-494). Append `" own"` when `mine` (and NOT for system messages — system is `kind=='system'`, never "own"; verify the existing `li_class` logic and compose cleanly, e.g. a `mine.then_some(" own")`). The class must remain a single space-joined string.

- [ ] **Step 1.2:** In `style/_content.scss`, add own-message rules INSIDE the `.messages` block, AFTER the base `.msg` rules (so they override). Directional bubble per spec §1: own messages align right, mirrored radius, blue-tinted card, max-width so prose breathes:

```scss
		// Directional bubbles (spec §1): the viewer's own messages lean right
		// with a mirrored corner + blue-tinted card; everyone else stays left.
		// Cap width so long prose still breathes. Pure presentation — the
		// `.own` class is set per-message from the `mine` flag at render.
		.msg {
			max-width: 88%;
			margin-right: auto; // others: hug the left
		}
		.msg.own {
			margin-left: auto;
			margin-right: 0;
			background: color-mix(in srgb, var(--accent) 9%, var(--card));
			border-color: var(--accent-soft);
			border-radius: 0.5rem 0.15rem 0.5rem 0.5rem; // mirrored top-right
		}
		.msg.own .meta {
			flex-direction: row-reverse;
		}
		.msg.own .msg-actions {
			margin-left: 0;
			margin-right: auto;
		}
```

NOTE: `.msg + .msg { margin-top }` stays. The `max-width` + `margin-right: auto` on base `.msg` must not break the system-message full-width look — if `.msg.system` should stay full-width, add `.msg.system { max-width: 100%; margin-right: 0; }`. Verify against the system-message rules already in the file and the draft row (`.msg-draft`).

- [ ] **Step 1.3:** Verify the persona-tint cascade is untouched: `.msg.own .who.mk-red` etc. must still tint (the `_msg_who.scss` / `_markup.scss` `.content .messages .msg .who.mk-*` selectors are specificity 0,4,0 and `.own` doesn't change depth). Read `_msg_who.scss`; confirm no rule assumes `.meta` is `row` (the reverse only flips visual order). The avatar (`chat_avatar`) sits first in `.meta` — under `row-reverse` it moves to the right edge for own messages, which is correct (your avatar on your side).

- [ ] **Step 1.4: Build + suite + commit**

```bash
cargo leptos build 2>&1 | tail -2
cargo test --features ssr 2>&1 | grep -c "test result: ok"   # 23, no FAILED
git add src/ui/shell/channel/mod.rs style/_content.scss && git commit -m "feat(ui): directional message bubbles — own messages lean right

The viewer's own messages (mine) get a right-aligned, blue-tinted card
with a mirrored corner and reversed meta row; others stay left. Pure
presentation off the existing per-message mine flag; the persona-tint
cascade and system/draft rows are unaffected.

Tests: cargo leptos build; ssr suite green; visual smoke in M3 verification"
```

---

### Task 2: Mono timestamps + uppercase persona names (spec §1 Duo polish)

**Files:**
- Modify: `style/_content.scss` (`.when`, `.who`), `style/_msg_who.scss`

- [ ] **Step 2.1:** `.when` (timestamp) → `font-family: var(--font-mono); font-size: 0.72rem; letter-spacing: 0.02em;` (it's currently sans 0.75rem). Find `.when` in `_content.scss`.

- [ ] **Step 2.2:** `.who` (persona name) → uppercase, letter-spaced, slightly smaller bold per spec §1 "names uppercase with letter-spacing":

```scss
			.who {
				font-weight: 600;
				font-size: 0.82rem;
				letter-spacing: 0.06em;
				text-transform: uppercase;
				color: var(--text-soft);
			}
```

CAUTION: the persona name can be long/multilingual — `text-transform: uppercase` is purely visual (the stored/snapshotted name is unchanged) and safe. Verify the reply-quote author (`.reply-quote-who`) — decide if it ALSO goes uppercase (recommend yes for consistency, but smaller) and apply if so. The system-message author (`.system-author`) already has its own rule — leave it or align it; note your choice.

- [ ] **Step 2.3:** Build + commit:

```bash
cargo leptos build 2>&1 | tail -2
git add style/ && git commit -m "feat(design): mono timestamps + uppercase persona names (Duo polish)

Timestamps now ride --font-mono; persona names are uppercase + letter-
spaced per spec §1. Visual only — snapshotted names are untouched.

Tests: cargo leptos build; visual smoke in M3 verification"
```

---

### Task 3: Glass chrome on topbar + modals (desktop+mobile)

**Files:**
- Modify: `style/_content.scss` (`.topbar`), `style/_modal.scss` (`.modal`)

- [ ] **Step 3.1:** `.topbar` adopts the glass foundation. Find the `.topbar` rule in `_content.scss`; replace its opaque `background` with `@use "foundation" as *;`'s glass — BUT partials can't easily `@include` across files unless foundation's mixin is importable. Check: `_foundation.scss` defines `@mixin glass`. To use it in `_content.scss`, add `@use "foundation" as *;` at the top of `_content.scss` (dart-sass module system) and `@include glass;` in `.topbar`. VERIFY this compiles (module `@use` order — foundation only defines a mixin + the `.glass`/`.app::before` rules; `@use`-ing it from _content is fine and won't double-emit the rules because `@use` is idempotent per compilation... ACTUALLY dart-sass `@use` of a partial that has top-level rules WILL emit them once at first use — since main.scss already `@use "foundation"`, a second `@use` from _content is deduplicated; verify the compiled CSS has the aurora/`.glass` rules exactly once). If the cross-file mixin import is fiddly, the FALLBACK is to inline the glass declarations directly in `.topbar` (background var(--glass-bg) + backdrop-filter + the @supports opaque fallback) with a comment pointing at the foundation mixin as the canonical source — report which path you took. Keep the topbar's bottom border (single-edge: `border-bottom: 1px solid var(--line)` — the glass mixin's 4-side border is wrong for a bar, so override to bottom-only after the include).

- [ ] **Step 3.2:** `.topbar` needs `position: relative; z-index: 5;` (above the message scroll) so the blur samples content beneath, and `backdrop-filter` requires the element not be the scroll container (it isn't — `.messages` scrolls). Verify the topbar sits above `.messages` in the flex column and the glass reads over scrolling content.

- [ ] **Step 3.3:** `.modal` (the card, not `.modal-backdrop`) adopts glass similarly — OR keep it opaque `--surface` if glass-over-backdrop looks muddy (the backdrop is already `--scrim`). RECOMMEND: modal card stays opaque `--surface` (legibility), but the `.modal-backdrop` gets a subtle blur: `backdrop-filter: blur(4px)` inside the same `@supports` guard so the app behind softens. Apply that to `.modal-backdrop`. Report the choice.

- [ ] **Step 3.4:** Build, eyeball compiled CSS for the glass + @supports, commit:

```bash
cargo leptos build 2>&1 | tail -2
grep -c "backdrop-filter" target/site/pkg/*.css   # ≥2 (foundation + topbar/backdrop)
git add style/ && git commit -m "feat(design): glass topbar + modal backdrop blur

The topbar is frosted glass (with the @supports opaque fallback for
older WebKit); the modal backdrop softens the app behind it. Modal cards
stay opaque --surface for legibility.

Tests: cargo leptos build; backdrop-filter present; visual smoke in M3 verification"
```

---

### Task 4: Delete the swipe drawer + `.nav-open` machinery

**Files:**
- Modify: `src/ui/shell/mod.rs` (delete `wire_swipe_drawer` + its call + the `class:nav-open` binding + the `.scrim` element if unused after), `style/_mobile.scss` (delete the drawer transforms), `src/ui/shell/state.rs` (`nav_open` — keep or remove?)

DO THIS BEFORE Task 5 builds the replacement, so the new nav isn't fighting the old drawer.

- [ ] **Step 4.1:** In `src/ui/shell/mod.rs`: delete the `wire_swipe_drawer` fn (the ~90-line pointer-event hydrate fn) and its mount call. Delete the `class:nav-open=...` binding on the `.app` div (keep `class:dialogue-style` and `class:fx-max`). Delete the `.scrim` `<div>` and its click handler (the bottom-sheet in Task 5 brings its own backdrop). Find every other `s.sync.nav_open` reader/writer (`grep -rn nav_open src/ui/`) — the topbar hamburger toggles it, the rail-home/wardrobe/channel-select handlers `.set(false)` it. Those all change in Task 5; for THIS task, neutralize them minimally: remove the `nav_open.set(false)` side-effects (they become no-ops once the drawer is gone) and the hamburger's toggle (the hamburger becomes the sheet trigger in Task 5 — for now, leave the button but make its handler a TODO no-op or remove the button; cleanest: remove the `.nav-toggle` button here and let Task 5 add the real trigger).

- [ ] **Step 4.2:** `state.rs`: `nav_open: RwSignal<bool>` in SyncState. Task 5 needs a sheet-open signal — RENAME `nav_open` → `sheet_open` now (or add `sheet_open` and delete `nav_open`). Recommend: rename `nav_open` → `sheet_open` (one signal, repurposed) and update all references. Document the rename.

- [ ] **Step 4.3:** `style/_mobile.scss`: delete the `.rail`/`.sidebar` `position:fixed` + `transform: translateX()` drawer block, the `.app.nav-open` rules, and the `.scrim` rule. KEEP the mobile composer/textarea sizing bumps and any safe-area bits not tied to the drawer. After deletion the mobile `.app` is just `grid-template-columns: 1fr` (content-only) — Task 5 adds the tab bar. Leave a comment marking where the bottom-tabs CSS lands.

- [ ] **Step 4.4:** Gates — this is a DELETION task; the app must still compile and the DESKTOP must be unaffected (the drawer was mobile-only). clippy hydrate+ssr+freya clean; ssr suite 23 ok; `cargo leptos build` ok. Mobile is intentionally half-broken between T4 and T5 (no nav) — that's fine within the wave; do NOT smoke mobile until T5.

```bash
git add src/ui/shell/ style/_mobile.scss && git commit -m "refactor(ui): delete the edge-swipe drawer ahead of the hybrid nav

wire_swipe_drawer, the .nav-open transforms, and the .scrim are gone;
the nav_open signal is renamed sheet_open for the M3 bottom-sheet. Mobile
has no nav between this commit and the next (bottom-tabs); desktop's
3-column grid is unaffected.

Tests: clippy clean (all graphs); ssr suite green; cargo leptos build"
```

---

### Task 5: Mobile bottom-tab bar + glass channel-switch sheet

**Files:**
- Modify: `src/ui/shell/mod.rs` (tab bar element + sheet element + triggers), `style/_mobile.scss` (tab bar + sheet CSS), maybe a new `style/_nav.scss`
- Use: `src/ui/icons.rs` (IconChat/IconServers/IconFriends/IconPersonas + the optional class prop)

- [ ] **Step 5.1: Tab bar markup.** In `mod.rs`'s `AppShell` view, after `.content` (and before the modals), add a mobile-only bottom tab bar. Four tabs mapping to existing state:
  - **Chat** → `s.sync.pane.set(Pane::Channel)` (active when pane==Channel)
  - **Servers** → opens the channel-switch sheet (`s.sync.sheet_open.set(true)`) — the sheet IS the server+channel switcher
  - **Friends** → `act::show_friends(s)` (pane==Friends)
  - **Personas** → `act::show_wardrobe(s)` (the wardrobe modal)

  Each tab: `<button class="tab" class:active=...>` with an icon (`<IconChat class="tab-icon"/>`) + a label span. Glowing unread badge on a tab when relevant (Chat tab shows aggregate unread? — keep simple in M3: a dot on Chat when any channel is unread, reading `s.notify.unread_guilds` non-empty; or defer badges — note the choice). The bar is `<nav class="bottom-tabs">`.

- [ ] **Step 5.2: Channel-switch sheet markup.** A glass bottom-sheet (the proven modal-signal pattern), shown when `s.sync.sheet_open.get()`. It contains: a horizontal server-icon row (reuse the rail's guild rendering — extract a helper or inline) with a fixed "✉ Direct" placeholder slot first (DM lands in M6; M3 just reserves the visual slot OR omits it — recommend omit in M3, add in M6, to avoid a dead button; note the choice), above the channel list (reuse the sidebar's channel rendering). Tapping a channel: switch + `sheet_open.set(false)`. The sheet has a grab handle, a backdrop (its own scrim, click-to-dismiss), and drag-down-to-close is NICE-TO-HAVE (a tap backdrop dismiss is the M3 floor; drag physics can be a later polish — note it).

  REUSE STRATEGY: the rail guild list and sidebar channel list are currently inline in `AppShell`. Extracting them into small `#[component]` fns (`RailGuilds`, `ChannelList`) that both the desktop columns AND the mobile sheet render is the clean path — do this refactor as part of T5 (it also shrinks the 400-line AppShell). If extraction is too invasive, duplicate the channel-list markup into the sheet and note the debt.

- [ ] **Step 5.3: The topbar channel-name trigger.** On mobile, tapping the channel name in the topbar opens the sheet (`sheet_open.set(true)`) — this is the primary fast-switch gesture (spec §2). Add a `▾` affordance (an icon) next to the channel name on mobile. Desktop ignores it (the sidebar is always visible).

- [ ] **Step 5.4: CSS** (`style/_mobile.scss` or new `_nav.scss`, `@use`-d in main.scss after mobile):
  - Desktop (>768px): `.bottom-tabs { display: none; }`, the channel-sheet is never shown (`.channel-sheet` hidden; the sidebar column is the switcher).
  - Mobile (≤768px): `.bottom-tabs` is a fixed bottom bar, glass (`@include glass` or inline), `padding-bottom: calc(0.5rem + env(safe-area-inset-bottom, 0px))`, 4 evenly-spaced tabs, active tab in `--accent` with text-shadow glow, unread dot. The `.content`/`.composer` must reserve space so the tab bar doesn't cover the composer: add `padding-bottom` to `.content` equal to the tab bar height (or make `.app` a grid row for the bar). CRITICAL safe-area math: composer bottom inset + tab bar height + the tab bar's own safe-area inset must not double-count — the tab bar owns `env(safe-area-inset-bottom)`, the composer sits above the tab bar. Lay it out so the composer's bottom is the tab bar's top.
  - `.channel-sheet`: fixed bottom sheet, glass, `border-radius: 18px 18px 0 0`, slides up (`transform: translateY(0)` from `translateY(100%)`, `transition` honoring reduced-motion), `z-index` above content (50) below global popovers; its backdrop `--scrim` at z-index 49. `max-height: 70vh; overflow-y: auto`. Honor `env(safe-area-inset-bottom)`.

- [ ] **Step 5.5: Gates + mobile smoke.** clippy ×3, ssr suite 23 ok, `cargo leptos build`. Then the node-playwright smoke at a MOBILE viewport (390×844, deviceScaleFactor 3, the iPhone-ish size): screenshot the chat with bottom-tabs, tap the channel name → screenshot the open glass sheet, tap a tab → confirm pane switch. Inject the session cookie with `secure:false` per the WebKit-cookie gotcha if using a WebKit context (Chromium is fine over http). Save `/tmp/m3-mobile-*.png`. Verify safe-area: the composer isn't hidden behind the tab bar.

```bash
git add src/ui/shell/ style/ && git commit -m "feat(ui): hybrid mobile nav — bottom tabs + glass channel sheet

Chat/Servers/Friends/Personas bottom tab bar (M2 icons) replaces the
swipe drawer; the channel name (or Servers tab) opens a glass bottom-
sheet with the server row + channel list for one-tap switching. Rail
guilds and channel list extracted into shared components so desktop
columns and the mobile sheet render the same source. Safe-area honored
(tab bar owns the home-indicator inset; composer sits above it).

Tests: clippy clean (all graphs); ssr suite green; mobile-viewport smoke in M3 verification"
```

---

### Task 6: Desktop 3-column reskin polish

**Files:**
- Modify: `style/_rail.scss`, `style/_sidebar.scss`, `style/_content.scss`, `style/_layout.scss`

The grid stays; this is spacing/chrome polish to match the Void Station desktop mock (`final-design-v4.html` desktop panel): darker rail, hairline separators, glowing active markers, a live-sync status hint in the topbar.

- [ ] **Step 6.1:** Rail: confirm `--void-deep` bg, the active-guild ring + (under fx-max) glow already land from M2. Tighten guild spacing/size to match the mock if needed (46px circles are fine). Add a subtle top/bottom gradient mask if the mock shows it (optional).

- [ ] **Step 6.2:** Sidebar: server header gets a small mono "channel count / status" subline? (optional, matches mock). Channel rows: the active channel gets the left accent border (`border-left: 2px solid var(--accent)`) per the mock; verify against current active style.

- [ ] **Step 6.3:** Topbar live-sync hint: a small right-aligned mono chip showing sync state (e.g. `● LIVE` in `--live` mint when the SSE stream is connected, dimmed when on polling fallback). The client knows: `s.sync` has the driver state — there's no explicit "connected" signal today; the cheapest honest signal is `s.sync.polling` (true once a driver runs) — but that doesn't distinguish SSE-vs-fallback. OPTION: add a `sse_live: RwSignal<bool>` to SyncState set true on EventSource `onopen` and false on the fallback handoff (act/sync.rs has both hooks) — a ~4-line addition that makes the chip honest. Implement that and render the chip (mono, mint when live). Desktop-only or both — show on both. Note: keep it subtle (the mock shows `● SYNC SSE LIVE · 12ms`; the latency number is fake-precision — show just `● LIVE` / `● POLLING`).

- [ ] **Step 6.4: Gates + commit.** clippy ×3 (the sse_live signal touches hydrate+ssr stub), ssr suite 23 ok, build ok.

```bash
git add src/ui/ style/ && git commit -m "feat(design): desktop 3-column Void Station reskin + live-sync chip

Rail/sidebar/topbar chrome polished to the Void Station desktop mock:
hairline separators, accent active markers, and an honest live-sync chip
(mint ● LIVE on the SSE stream, ● POLLING on the fallback) driven by a
new sse_live signal set from the EventSource onopen / fallback hooks.

Tests: clippy clean (all graphs); ssr suite green; visual smoke in M3 verification"
```

---

### Task 7: M3 verification gate + visual smoke

**Files:** none (verification; screenshots as deliverables)

- [ ] **Step 7.1: Full gate**

```bash
cargo fmt --all --check && cargo clippy --features ssr 2>&1 | tail -1 \
  && cargo clippy --features hydrate --target wasm32-unknown-unknown 2>&1 | tail -1 \
  && cargo clippy --features freya 2>&1 | tail -1 \
  && cargo test --features ssr 2>&1 | grep -cE "test result: ok" \
  && cargo leptos build --release 2>&1 | tail -2
```

Expected: clean, 23 ok suites, release build completes.

- [ ] **Step 7.2: Dual-viewport smoke** (dev server + `/tmp/nova-gif/`; NEVER prod). DESKTOP (1280×800): register a throwaway account, seed a guild + 2 personas + a few messages from two different accounts (so directional bubbles show both sides), screenshot the reskinned shell. MOBILE (390×844): same account, screenshot chat-with-bottom-tabs, the open channel sheet, the Friends tab, the Personas (wardrobe) modal. Verify: directional bubbles (own right / other left), mono timestamps, uppercase names, glass topbar, live-sync chip, no composer/tab-bar overlap, persona tints intact. Eyeball the M2 carry-forward surfaces (role pills, system message). Save `/tmp/m3-desktop-*.png` + `/tmp/m3-mobile-*.png`. Kill the dev server after.

- [ ] **Step 7.3:** Update CLAUDE.md's mobile-PWA-safe-area gotcha if the selectors changed (the drawer `.rail`/`.sidebar` fixed-position insets are gone; the new `.bottom-tabs` owns `env(safe-area-inset-bottom)` — update the gotcha's named selectors to reflect reality). Commit if changed.

---

## Done = M3 exit criteria

1. Mobile: bottom-tab nav + glass channel sheet work end-to-end; no swipe drawer; composer never hidden by the tab bar; safe-area honored.
2. Desktop: 3-column grid reskinned to Void Station; live-sync chip honest.
3. Directional bubbles (own right / other left) on both viewports; persona tints + system/draft rows intact.
4. Mono timestamps, uppercase persona names, glass topbar live.
5. Full gate green incl. `--release`; dual-viewport smoke delivered.

**Next plan:** M4 (chat experience — composer, STD wow effects, message effects, constellation presence, fate engine, ghost quill) against the post-M3 tree.
