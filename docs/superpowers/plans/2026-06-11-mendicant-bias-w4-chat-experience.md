# Mendicant Bias W4: Chat Experience Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Bring the message flow to life — Standard-tier wow effects that live in the chat (constellation typing indicator, charging send button, warp channel-switch, radial long-press action menu), Message Effects (whisper / shout / spell send-modes), the server-validated Fate Engine (`/roll`), and opt-in Ghost Quill (live co-writer draft preview) — all on the W1 SSE bus + W2/W3 design system, preserving the message-schema, markup-panic-free, and id-only-bus invariants.

**Architecture:** CSS/signal-only effects first (no schema/server risk): constellation typing (render swap), charging send (signal off compose length), warp (transition class on channel switch), radial menu (touch long-press over the existing actions). Then the schema/server features: Message Effects add ONE `option<string>` field to `message` (NONE-safe, schema_apply-guarded) + a composer mode picker + render branches; the Fate Engine is a client slash-command intercept → a server endpoint that validates dice syntax + does the RNG server-side + persists a `kind='roll'` message (cheating-proof). Ghost Quill is the riskiest — it keeps the id-only bus intact via a hybrid: the existing `Typing` event nudges the client to fetch a new ephemeral in-memory typing-draft endpoint; opt-in, last, and cuttable if the wave runs long.

**Tech Stack:** Leptos 0.8, the W2/W3 design system (`fx-` keyframes, icons, glass), SurrealDB (one `option<>` field + a new `kind` value), axum (one roll endpoint + one typing-draft endpoint), the markup parser (UNCHANGED — effects are a message field, not markup, to keep the panic-free invariant untouched).

**Spec:** `docs/superpowers/specs/2026-06-10-mendicant-bias-design.md` §1 (wow concepts D/E/A/I), §9.1 (Fate Engine, Message Effects, Ghost Quill), §12 W4. Mockups: `assets/2026-06-10-mendicant-bias/wow-concepts.html` (D constellation, E charging-send, A warp, I radial), `app-concepts.html` (B Ödesmotorn, F effects).

**Gates:** `cargo leptos build`, clippy ssr+hydrate(wasm32)+freya, full ssr suite (currently 23 suite lines; W4 adds tests for effects/roll/typing-drafts — count grows, 0 FAILED), visual smoke (`/tmp/nova-gif/` node-playwright, NEVER prod). New server features get TDD integration tests in `tests/`.

**Branch:** `mendicant-bias`. Commit per task. Orchestrator pushes.

**Invariant watch:** message schema NONE-coercion (effects field MUST be `option<>` or backfilled; add a `tests/schema_apply.rs` case); markup parser panic-free (NOT touched — effects are a field); SSE id-only/no-content (Ghost Quill must NOT put draft text in a SyncEvent — it rides a fetch endpoint); session-cookie auth + privacy-404 (roll + typing-draft endpoints inherit `AuthAccount` + `channel_access`); server-authoritative RNG (the client never computes a roll result); persona send-path double-check (the roll endpoint accepts a persona and MUST re-check `can_edit_persona` for BOTH the suggested and the stored persona, exactly like `posting.rs` — test it); roll immutability is ENFORCED, not assumed (`kind='roll'` rows reject author edit AND delete server-side — system-message immutability is an authorship side-effect that does NOT transfer to roller-authored rows; see T6).

---

### Task 1: Constellation typing indicator (CSS/signal, STD)

**Files:** `src/ui/shell/channel/mod.rs` (the `.typing-indicator` render ~634-645), `style/_content.scss` (`.typing-indicator`), `style/_motion.scss` (a typing keyframe)

Replace the plain text line with orbiting star-points (one per typist) + the names. Per spec §1 concept D, both tiers. The typing payload is `Vec<String>` of display names (no per-persona color available) — so stars share the accent/mint hue; note the per-persona-color enrichment as a future step (would need the typing payload to carry colors).

- [ ] **Step 1.1:** In `style/_motion.scss` add `@keyframes fx-orbit` (a small star orbiting a center: `transform: rotate(0) translateX(6px) rotate(0)` → `rotate(360deg) translateX(6px) rotate(-360deg)`) and `@keyframes fx-twinkle` (opacity 0.5↔1 scale 0.8↔1.2). Both are `fx-` prefixed so the reduced-motion kill catches them.
- [ ] **Step 1.2:** Rewrite the typing render in `mod.rs`: when `s.msg.typing` is non-empty, render `<div class="typing-indicator"><span class="constellation">` with N `<span class="star">` (N = min(typists, 3), each with a staggered `animation-delay`) `</span>` + the existing name text. Keep the 1/2/several name logic. When empty, render nothing (unchanged).
- [ ] **Step 1.3:** `style/_content.scss` `.typing-indicator`: style `.constellation` (a small fixed-size relative box) + `.star` (absolutely-centered tiny dots, `background: var(--accent)`/`var(--live)` alternating, `box-shadow` glow, `animation: fx-orbit ... , fx-twinkle ...`). Keep it subtle and small (it sits above the composer). Under `.app.fx-max` make the glow stronger.
- [ ] **Step 1.4: Build + commit.** `cargo leptos build` ok; ssr suite unchanged-green. Commit `feat(ui): constellation typing indicator — orbiting stars per typist (STD)`. Visual smoke deferred to T8.

---

### Task 2: Charging send button (CSS/signal, STD)

**Files:** `src/ui/shell/channel/mod.rs` (the Send button ~1143-1152), `style/_content.scss`, `style/_motion.scss`

Per spec §1 concept E: a ring around Send fills with composed-message length; a pulse fires on send. Keep it cheap — no canvas; a conic-gradient ring driven by a CSS custom property bound to a signal.

- [ ] **Step 2.1:** Add a derived signal/`Memo` `charge` = `(compose.len() clamped 0..=N) / N` (N ~ 280 chars — a "full" feel, not a hard limit). The Send button gets `style:--charge=move || format!("{}", charge.get())` and a `.charging` class when compose is non-empty.
- [ ] **Step 2.2:** `style/_content.scss`: the Send button gets a `::before` conic-gradient ring masked to a thin annulus (`mask: radial-gradient(...)`), `background: conic-gradient(var(--accent) calc(var(--charge) * 360deg), transparent 0)`. The pulse on send: add a transient `.sent` class (set true for ~400ms in `send_message`, then false) driving `animation: fx-glow-pulse` once. Under fx-max, brighter.
- [ ] **Step 2.3:** In `act/message.rs::send_message`, after a successful dispatch, briefly flip a `s.composer.sent` signal (spawn a gloo-timers 400ms reset) — OR keep it purely CSS via `:active` if the signal plumbing is heavy (judge; a signal is cleaner for the post-send pulse). Note the choice.
- [ ] **Step 2.4: Build + commit.** `feat(ui): charging send button — ring fills with message length, pulse on send (STD)`.

---

### Task 3: Warp channel-switch transition (CSS/signal, STD light / ÖG full)

**Files:** `src/ui/shell/mod.rs` (the `.content`/pane wrapper), `src/ui/shell/state.rs` (a `switching` signal), `src/ui/shell/act/channel.rs` (flip it on open), `style/_content.scss` + `style/_motion.scss`

Per spec §1 concept A: channel/server switch is a quick FTL streak. STD = a ~180ms fade+scale; ÖG = a fuller warp-streak overlay. Refinement (destination-accent tint) AWAITS per-server `accent_color` (spec §1 wow-effect G / §3 Identity, lands in W5 — not built) — use the generic accent now; note it.

- [ ] **Step 3.1:** `state.rs`: add `switching: RwSignal<bool>` to SyncState (doc it). `act/channel.rs::open_channel_at`: set `switching.set(true)` at entry; after the pane/messages are set, schedule `switching.set(false)` on a short gloo-timers timeout (~180ms) so the in-animation plays. (Keep it simple — no animationend listener; a fixed short timer matches the CSS duration.)
- [ ] **Step 3.2:** `mod.rs`: bind `class:fx-switching=move || s.sync.switching.get()` on the pane/content wrapper.
- [ ] **Step 3.3:** `style/_content.scss`: `.content.fx-switching` (STD) → brief `opacity: 0.6; transform: scale(0.985)` with a `transition` on the way back in (the class-removal animates the return). `style/_motion.scss`: add `@keyframes fx-warp` for the ÖG streak; `.app.fx-max .content.fx-switching` plays a light-streak `::after` overlay. Reduced-motion: the `fx-` selectors are already killed; ALSO ensure the STD transition is disabled under reduced-motion (the global freeze handles `transition`? verify — `_base.scss` freezes animations; add a transition guard if needed).
- [ ] **Step 3.4: Build + commit.** `feat(ui): warp channel-switch transition (STD fade, ÖG streak)`.

---

### Task 4: Radial long-press action menu (touch, STD)

**Files:** `src/ui/shell/channel/meta.rs` (the `.msg-actions`), a new small component or inline, `style/_content.scss`/`_nav.scss`, uses `src/ui/icons.rs`

Per spec §1 concept I: long-press a message → a radial glass menu of reply/copy/edit/delete blossoms around the touch point; replaces the always-visible hover buttons on touch. Desktop keeps the hover `.msg-actions` row. Touch detection via `(pointer: coarse)`.

- [ ] **Step 4.1:** Detect touch (reuse the `enter_inserts_newline`/`(pointer: coarse)` pattern — extract a shared `is_touch()` helper in channel/mod.rs if not already). On a touch device, attach a long-press handler to each `.msg` (pointerdown → 450ms timer → if not cancelled by pointermove/up, open the radial menu at the press coords; pointerup/move before the timer cancels). Use web-sys pointer events (hydrate-only). Track an open-menu signal `radial: RwSignal<Option<(message_id, x, y)>>`.
- [ ] **Step 4.2:** Render the radial menu (hydrate, when `radial` is Some): a fixed-position glass element at (x,y) with the action buttons (IconReply/IconCopy + IconEdit/IconTrash for own) arranged in an arc (CSS `translate` per slice with a blossom keyframe). A backdrop captures the dismiss tap. Reuse the same act:: handlers the hover buttons call (start_reply/copy_message_body/start_edit/ask_delete).
- [ ] **Step 4.3:** CSS: `.radial-menu` glass (use `@include glass($border:false)`), the arc layout, `@keyframes fx-blossom` (scale 0→1 + rotate settle), backdrop `var(--scrim)`. Honor reduced-motion. Hide the hover `.msg-actions` on `(pointer: coarse)` (touch uses the radial instead); keep them on desktop.
- [ ] **Step 4.4:** Gates: clippy ×3 (hydrate pointer code), ssr suite green, build ok. Mobile smoke can wait for T8, but quick-verify the long-press opens the menu (note in report). Commit `feat(ui): radial long-press action menu on touch (STD)`.

---

### Task 5: Message Effects — whisper / shout / spell (schema + composer + render)

**Files:** `src/storage/schema.surql` (message field), `tests/schema_apply.rs` (guard), `src/protocol.rs` (DTO + send request), `src/server/messages/posting.rs` (accept + validate), `src/native/api.rs` (the `SendMessageRequest` struct literal MUST name the new field or the freya graph stops compiling), `src/ui/shell/channel/mod.rs` (composer mode picker), `src/ui/shell/channel/meta.rs` or the `.msg` render (effect class), `src/ui/shell/act/message.rs` (send the effect), `style/_content.scss` + `_motion.scss`, `tests/messages.rs` (effect round-trip)

Per spec §9.1 (the letter "F" belongs to the `app-concepts.html` mockup, NOT spec §1 — §1's F is Holographic depth). **Effects are a message FIELD, not markup** (keeps the parser panic-free invariant untouched). whisper = blurred until tapped (a berättarverktyg — spoiler-like); shout = shake + warm color; spell = particles **at Standard tier too** (spec §9.1 gives no tier split and §1 says STD strips nothing — keep STD particles lightweight CSS sparks; `.fx-max` adds density/glow on top). Reduced-motion kills all three animations.

- [ ] **Step 5.1: TDD schema-apply guard FIRST.** Add a `tests/schema_apply.rs` test mirroring `applying_kind_over_populated_messages…`: seed a pre-effect `message` row (populated), apply the real schema (which now adds `effect`), assert the legacy row survives with `effect = NONE` and other fields intact. Run → FAIL (field doesn't exist yet).
- [ ] **Step 5.2:** `schema.surql`: add `DEFINE FIELD effect ON message TYPE option<string> ...` with an ASSERT restricting to the allowed set (`$value = NONE OR $value IN ['whisper','shout','spell']`). It's `option<>` so NO backfill needed (NONE-safe per the invariant). Re-run 5.1's test → PASS.
- [ ] **Step 5.3:** `protocol.rs`: add `effect: Option<String>` to `MessageEnvelope` (`#[serde(default)]`) and to the post-message request DTO. `posting.rs`: accept the optional effect, VALIDATE it server-side (reject unknown values → 400, mirroring body validation), persist it in the CREATE. Add it to `MSG_PROJECTION` (reading.rs) so it rides the page response. `tests/messages.rs`: a round-trip test (post with effect=whisper → list → envelope.effect == "whisper"; post with garbage effect → 400).
- [ ] **Step 5.4:** Composer: a small effect-mode picker (a button cycling None→Whisper→Shout→Spell, or a popover) near Send; the chosen mode is sent with the message and then resets. `act/message.rs::send_message` passes the effect through `api::post_message`. The send request/ api wrapper gains the optional effect arg.
- [ ] **Step 5.5:** Render: the `.msg` gets an `effect-{name}` class when `envelope.effect` is set. CSS: `.msg.effect-whisper .text` blurred (`filter: blur(4px)`) until a `.revealed` class (tap toggles a per-message signal — reuse the spoiler reveal pattern from markup_view if applicable); `.msg.effect-shout` shake (`@keyframes fx-shout`) + `--tint-orange` text; `.msg.effect-spell` particles AT STD (lightweight CSS sparks — e.g. 2-3 glowing pseudo-element dots on `fx-` keyframes) with a glow base, and `.app.fx-max` layering more density on top. Reduced-motion safe (every new `fx-` keyframe either matches the `[class*="fx-"]` kill or gets an explicit kill-list entry).
- [ ] **Step 5.6:** Gates: clippy ×3, full ssr suite (schema_apply + messages effect tests green), build ok. Commit `feat(messages): whisper/shout/spell message effects (option<> field, server-validated)`. Cite the schema_apply + round-trip tests in the `Tests:` line.

---

### Task 6: Fate Engine — `/roll` (server-validated dice)

**Files:** `src/server/messages/` (a roll handler — new `roll.rs` or in posting), `src/server/mod.rs` (route), `src/storage/schema.surql` (extend `kind` ASSERT to allow `'roll'`), `tests/schema_apply.rs` (kind-value guard), `src/protocol.rs` (roll request/response or reuse message create), `src/ui/shell/act/message.rs` (slash-command intercept), `src/ui/shell/channel/` (animated result chip render), `tests/` (roll validation + RNG-is-server-side)

Per spec §9.1 Fate Engine. `/roll 2d20+3`, `/coin`, `/oracle` — **server does the RNG** (cheating-proof). The result becomes a `kind='roll'` message (immutable like system, but authored by the roller's persona/account).

- [ ] **Step 6.1: Decide the data shape.** A roll is a message with `kind='roll'` whose `body` carries the structured result (e.g. `"2d20+3 → [14,8]+3 = 25"`), authored by the caller (persona-aware like a normal send). Extend the `message.kind` ASSERT to include `'roll'` — **edit the ASSERT IN PLACE; do NOT touch or re-split the `kind` backfill** (it is materialised inside the first `message` backfill on purpose; re-splitting crash-loops schema apply, per CLAUDE.md + `tests/schema_apply.rs`). Add a schema_apply test for the new enum value, and extend the existing kind-guard's accept-loop to include `'roll'`. The roll never goes through markup as a command — it's parsed at send.
- [ ] **Step 6.2: Server endpoint + RNG.** `POST /channels/{cid}/roll` (AuthAccount + channel_access → privacy-404), body `{ expr: String, persona: Option<String> }`. Parse a constrained dice grammar (`NdM(+/-K)?`, plus `coin`, `oracle`) — REJECT anything outside it (400; bounded N≤100, M≤1000 to avoid abuse). Server RNG (`rand` — already a server dep). **Persona check is MANDATORY:** route the persist through `posting.rs`'s validated path (or replicate its double-check exactly): re-check `can_edit_persona` for BOTH the suggested and the stored persona — a roll must not let a caller impersonate a persona they cannot wear. Persist a `kind='roll'` message with the formatted result as body, persona snapshot like normal posting, emit `MessageCreated` on the bus. Return the created id. TDD: `tests/roll.rs` — valid expr → roll message created with a plausible result; invalid expr → 400; the result is within the dice's possible range (statistical sanity, not exact); privacy-404 for non-members; **rolling with a persona the caller cannot edit → rejected (mirror posting's test)**.
- [ ] **Step 6.2b: Enforce roll immutability (the "cheating-proof" claim).** System-message immutability is an authorship side-effect (`account:nova_dot` cannot log in) — it does NOT transfer to roller-authored `kind='roll'` rows: without a guard the roller can PATCH the body and forge the result, or delete an unfavorable roll. Add an explicit `kind == 'roll'` rejection (403) in the message EDIT path AND the author DELETE path (decision: rolls are fully immutable like system — the body is server-generated from a constrained grammar, so there is no offensive-content moderation need). TDD first: edit-own-roll → 403, delete-own-roll → 403, body unchanged after the attempts.
- [ ] **Step 6.3: Client intercept.** `act::send_message`: if `body.trim().starts_with("/roll ")` / `/coin` / `/oracle`, route to `api::roll(cid, expr, persona)` instead of `post_message` (clear compose as usual). A bad expr surfaces the 400 message in the composer status line. Non-command sends are unchanged.
- [ ] **Step 6.4: Render the result chip.** A `kind='roll'` message renders as an animated glass chip (a die glyph + the result, `fx-` entrance) rather than a plain bubble — distinct from `kind='system'` (Nova DOT) and `kind='user'`. Read the existing kind-branch in the `.msg` render and add the roll branch. Under fx-max, a die-tumble animation.
- [ ] **Step 6.5:** Client gating: the radial/hover action sets must NOT offer edit/delete on `kind='roll'` rows (extend the shared kind→actions predicate from the T4 close-out rather than re-branching `is_system` in multiple places).
- [ ] **Step 6.6:** Gates: clippy ×3, full ssr suite (roll + schema_apply green), build ok. Commit `feat(messages): Fate Engine /roll — server-validated dice, animated result chips`.

---

### Task 7: Ghost Quill — opt-in live co-writer draft (hybrid, CUTTABLE)

**Files:** `src/server/state.rs` (an ephemeral typing-draft map), `src/server/messages/typing.rs` (store draft on ping), a new `GET /channels/{cid}/typing-drafts` handler + route, `src/protocol.rs` (the draft DTO), `src/ui/shell/act/` (a pref toggle + fetch-on-Typing), `src/ui/shell/account.rs` (the opt-in toggle), the channel render (ghost rows), `tests/`

ARCHITECTURAL CONSTRAINT (hard): the SSE bus stays id-only — draft TEXT never rides a SyncEvent. Instead the existing `Typing` event nudges the client to fetch the ephemeral draft. Opt-in (default OFF), per the spec. This task is the wave's riskiest and runs LAST — but note the spec's locked catalogue grants a cut-if-long license only to the vault's TOTP, not to Ghost Quill: **actually cutting it requires owner sign-off**; absent that, a miss defers it to a later wave with an explicit note.

- [ ] **Step 7.1: Server ephemeral store.** `AppState` gains `typing_drafts: Arc<Mutex<HashMap<(cid,account), (String, Instant)>>>` (mirror the `typing` map's TTL+prune discipline, 8s TTL, NEVER held across await). `typing.rs`: the typing POST gains an optional `draft: Option<String>` body (capped length, e.g. 2000 chars); when present, store it; prune stale on read. The bare typing ping (no draft) still works (Ghost Quill is opt-in on the SENDER side too — only sends draft text when both parties opted in? — simplest: the sender always may send draft text; the RECEIVER only fetches/renders it when THEY enabled Ghost Quill. Decide + document; recommend: sender sends draft only when the sender's own Ghost Quill pref is on, receiver renders only when receiver's pref is on — both-opt-in, privacy-respecting).
- [ ] **Step 7.1b: Clear-on-send.** A successful message post REMOVES the author's entry from `typing_drafts` for that channel (in `posting.rs`, same place the typing entry clears) — otherwise a ghost row lingers beside the just-landed real message for up to the TTL.
- [ ] **Step 7.2: Endpoint.** `GET /channels/{cid}/typing-drafts` (AuthAccount + channel_access → privacy-404) returns `[{ account_id, display_name, draft }]` for OTHER members with a live draft. The TTL must be TESTABLE without an 8s sleep: make it an injectable `Duration` on the store (prod default 8s; the test constructs/overrides a short one, or the prune fn takes `now` as a param). TDD: `tests/typing_drafts.rs` — a posted draft is readable by a member, excluded for the caller themselves, privacy-404 for non-members, prunes after the (short, injected) TTL, and is gone after the author sends the message.
- [ ] **Step 7.3: Client.** A `ghost_quill` pref (localStorage, default OFF — mirror the eyecandy/dialogue prefs exactly) + Account toggle. When ON: the composer's typing-ping includes the current draft; and on a `Typing` SyncEvent for the open channel, fetch `/typing-drafts` and render ghost rows (dashed, italic, the in-progress text) below the messages. When OFF: bare typing pings (no draft sent), no ghost rows.
- [ ] **Step 7.4:** Gates: clippy ×3, full ssr suite (typing-drafts tests green), build ok. Commit `feat(chat): Ghost Quill — opt-in live co-writer draft preview (hybrid, id-only bus intact)`. If cut: note the cut in the W4 verification + leave the spec line for a later wave.

---

### Task 8: W4 verification gate + visual smoke

**Files:** none (verification; screenshots as deliverables)

- [ ] **Step 8.1: Full gate.** `cargo fmt --all --check`, clippy ssr+hydrate(wasm32)+freya, full ssr suite (0 FAILED), `cargo leptos build --release`.
- [ ] **Step 8.2: Visual smoke** (dev server + `/tmp/nova-gif/`, NEVER prod): two accounts in a channel. Capture: constellation typing (account B types → account A sees orbiting stars), charging send (compose grows → ring fills), a whisper message (blurred until tapped) + a shout (shake/color), a `/roll 2d20+3` result chip, a channel switch (warp), and (if Ghost Quill landed) a ghost row. Mobile viewport: long-press a message → radial menu. Save `/tmp/w4-*.png`. Eyeball the W2/W3 carry-forward (bubbles, glass, tints) intact.
- [ ] **Step 8.3:** Update CLAUDE.md if a new invariant landed (the `effect` field + `kind='roll'` are worth a one-line note in the message-schema invariant section; the typing-drafts ephemeral store + its no-content-on-bus property worth a gotcha line). Commit if changed.

---

## Done = W4 exit criteria

1. Constellation typing, charging send, warp switch, radial long-press menu all live (Standard tier; fx-max enhancements where specced); reduced-motion safe.
2. Message Effects (whisper/shout/spell) round-trip through an `option<>` schema field, server-validated, schema_apply-guarded; render correctly.
3. Fate Engine `/roll` is server-authoritative (client cannot forge a result — edit AND delete of `kind='roll'` rejected server-side with tests), persona-checked like posting, validated, rendered as an animated chip; `kind='roll'` schema_apply-guarded.
4. Ghost Quill either landed (opt-in, id-only bus intact via the fetch endpoint) or deferred to a later wave with an explicit note + owner sign-off.
5. Full gate green incl. `--release`; visual smoke delivered; markup parser UNTOUCHED (panic-free invariant intact); SSE bus still carries no content.

**Next plan:** ~~W5 (Eye-candy tier)~~ — superseded 2026-06-12 by the owner's Skelettvågen ruling: **W5 = Skelettvågen** (three user-selectable UI skeletons replace the W3 shell; spec §12 as amended 2026-06-12 is authoritative). The eye-candy material moved to W5's #48 delta backbone + W12 Cinema & senses.
