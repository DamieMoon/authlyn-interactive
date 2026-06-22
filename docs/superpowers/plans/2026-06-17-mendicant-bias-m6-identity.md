# Mendicant Bias M6 (Identity) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL — use `superpowers:subagent-driven-development` (or `superpowers:executing-plans`) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. Commit per task with the `(M6/P#)` trailer + a `Tests:` line + the `Co-Authored-By` trailer; push only via the orchestrator, never to `main`/prod.

## Context

**Why now.** M5 (Skelettvägen) is release-verified — full gate green @ `43458c6`, 345 tests passing, touch-floor closeout `844fe5e`, iOS-sim signed off (ctx `019ed4de`). M6 is the next wave: spec §12 **M6 Identity** (`docs/superpowers/specs/2026-06-10-mendicant-bias-design.md:279`), detailed in **§3 Identity** (:94–99) and **effect G** per-server accent (:71).

**What M6 delivers (three deliverables):**
1. **Guild icons** — uploadable, manager-gated, riding the existing media pipeline; **per-server accent derived server-side at upload** via the `image` crate, stored as `guild.accent_color`; monogram fallback.
2. **Account profiles** — `display_name` (1–32, trimmed) + avatar, edited under Account; account avatar/identity **beside messages** (account resolves **live** at read; persona stays **snapshotted** at send — invariant untouched).
3. **Nova DOT** — Superintendent-inspired orb avatar + badge chip on system messages.

**The headline finding (de-risks the whole wave):** *most of the data substrate already exists and is `NONE`-safe — there is **no schema migration**.* `account.display_name` (backfilled), `account.avatar`, `guild.icon`, `guild.accent_color` are all defined (`option<record<media_blob>>` / `option<string>`), and the `nova_dot` system account is seeded and authors every `kind='system'` message. The media pipeline, accent validation/PATCH, the warp-jump accent consumer, persona `set_avatar`, `patch_guild`, and the `MSG_PROJECTION` live-resolution are all reusable. **M6 = wire the existing substrate + three small new surfaces.**

**Binding frame (owner rulings / spec — verified):**
- **Evolution, not port** (ctx `019ed36c`): design UI natively for the orbit shell; reuse only the data layer.
- **Owner is the visual oracle**: visual placement/look is decided on **live headed demos**, never pre-locked from text. This plan builds the *mechanism*; visual choices are flagged under *Owner decisions (demo-driven)*.
- **Orbit is the SOLE release shell** (ctx `019ed374`): the spec's "per-skeleton ×3" verification collapses to **orbit ×1**.
- **Server-trusted + privacy-404**, **SSE id-only**, **touch floor ≥44px** at each control's base SCSS def, **identity invariant** (account live / persona snapshot).

**Goal:** Land the three Identity surfaces in the orbit shell on top of the existing data substrate, preserving every security/identity invariant, with the per-server accent now *derived from the guild icon*.

**Architecture:** Reuse, don't invent. **Decided reconciliation:** the spec text names `guild.icon_media` / `account.avatar_media` as `option<string>`, but the implemented house pattern (already in schema) is `option<record<media_blob>>` record-links **projected to `*_id: Option<String>` in DTOs** (exactly like `persona.avatar → PersonaSummary.avatar_id`). M6 **reuses the existing `guild.icon` and `account.avatar` fields** — it does **not** add new string fields, and needs **no migration**. Icon/avatar upload = existing `POST /media` then a dedicated set-handler (mirrors `PUT /personas/{id}/avatar`). Accent derivation runs server-side in `spawn_blocking` (same `image`-crate calls as `make_thumb`) and maps to one of the 8 `ACCENT_PALETTE` names already canonical in `src/ui/accent.rs`.

**Tech Stack:** axum + Leptos 0.8 + SurrealDB 3.x; `image` v0.25 (already an ssr dep, `Cargo.toml:124`) for accent derivation; dart-sass via cargo-leptos.

**Spec:** §3 (:94–99), §1 effects A+G (:71), §12 M6 (:279), §13 verification (per-skeleton → orbit ×1).

**Gates (M6 exit):** `/check` (fmt + clippy ssr + clippy hydrate-wasm `-D warnings`) · `cargo test --features ssr` = **0 failed** (live SurrealDB) · `tests/style_lint` (motion-doctrine `@keyframes`) · `nova` builds (`cargo build --release --bin nova-mcp --features nova`, since `protocol.rs` changes) · **real-device iOS deck pass (orbit only)**.

**Branch:** `mendicant-bias` (already on it). Commit per task; push only via the orchestrator. **Never** to `main`/prod.

---

## Decided reconciliations (do not re-litigate)

1. **Record-link fields, not string fields.** Reuse `guild.icon` + `account.avatar` (`option<record<media_blob>>`); project to `icon_id` / `avatar_id` strings in DTOs (precedent: `PersonaSummary.avatar_id`, `MemberSummary.avatar_id`). No `*_media` string fields.
2. **No schema migration.** All M6 fields exist and are `NONE`-safe (`schema.surql:18,24,29,108,123`; nova_dot seed :38–42). Only a *guard test* in `tests/schema_apply.rs` is added (proves clean re-apply over a legacy row) — no `DEFINE FIELD` is added.
3. **Accent consumption is already wired.** Effect A (warp-jump tint) + effect G read `guild.accent_color` via `--glow-accent`/`--accent` on `.app` root (`shell/mod.rs:360-361`, `_foundation.scss:274-292`). M6 adds only the **derivation**; consumers light up unchanged.

## Owner decisions — demo-driven (resolve ON the headed demo; the plan ships the mechanism either way)

- **[headline] Account avatars in ORBIT chat.** Orbit chat is **name-only today** — `_sk_orbit_chat.scss:124` hides `.chat-avatar` under `.app.sk-orbit` (the sole shell). Spec §3 says "account avatar beside messages." So whether M6 brings avatars *into* the orbit chat row (override the name-only rule) or surfaces the account identity only via the subtle "· display_name" marker is a **demo call**. The server-side **live `author_avatar_id` projection lands regardless** (it is needed and harmless); only the render visibility is gated.
- **Accent overwrite policy.** Recommended default: icon upload **overwrites** `accent_color` with the derived value; the manual swatch picker (`ServerModal`) lets the manager re-override instantly. (Spec frames accent as *derived from* the icon, so a new icon re-deriving is least-surprising.) Owner may pick seed-when-unset (gate the accent UPDATE on `accent_color IN [NONE,'']`).
- **Icon clear path.** Whether to support removing an icon (→ monogram fallback) and whether clearing also clears the derived accent. Spec's monogram fallback implies a clear path; default = add a clear that reverts accent to default.
- **Nova orb** — art (rings/iris/gradient), ring animation (rotate vs breathe, period), size/placement, badge chip (orb-only vs orb+"SYSTEM"), accent-blue vs palette ring.
- **Guild icon render** — shape/mask on the orbit core nucleus + far discs; ✦ as overlay badge vs full replace; thumbnail width for hi-DPR.
- **"· display_name" marker** — separator glyph, opacity/weight, clickable vs plain.

---

## P1 — Guild icons + server-derived accent

### Task 1.1 — Project `icon_id` into guild DTOs + reads (TDD: round-trip test first)
**Files:** `src/protocol.rs`, `src/server/guilds/mod.rs`
- [ ] `protocol.rs`: add `#[serde(default)] pub icon_id: Option<String>` to `GuildSummary` (after `accent_color`, ~:153) and `GuildDetail` (~:188); document like `MemberSummary.avatar_id`.
- [ ] `load_my_guilds` (:71–122): add `(IF guild.icon != NONE THEN meta::id(guild.icon) ELSE NONE END) AS icon_id` to the SELECT + Row struct + the `GuildSummary` map.
- [ ] `load_guild_detail` (:353–398): same projection on `icon` + `GuildDetail` construction.
- [ ] Set `icon_id: None` at the other `GuildSummary`/`GuildDetail` literal sites (`create_guild` ~:266, `list_deleted_guilds` ~:491).
- **Tests:** `tests/guilds.rs` — upload+set an icon, then `GET /guilds` and `GET /guilds/{id}` both return `icon_id == media id`.
- **Commit:** `feat(guilds): project guild.icon as icon_id in summary + detail DTOs (M6/P1)`

### Task 1.2 — `set_guild_icon` handler + route (`PUT /guilds/{id}/icon`)
**Files:** `src/protocol.rs`, `src/server/guilds/icon.rs` (new), `src/server/guilds/mod.rs`, `src/server/mod.rs`
- [ ] `protocol.rs`: add `SetGuildIconRequest { pub media_id: String }` near `SetAvatarRequest` (plain Request, mirrors it).
- [ ] New `guilds/icon.rs` (cohesion with the derivation code): `set_guild_icon` — order: `json_rejection_response` → `require_manager` (404 non-member, 403 non-manager, rejects trashed guild) → media-exists privacy-404 (reuse/lift the `media_exists` probe from `personas/gallery.rs:387-428`) → `UPDATE guild SET icon = type::record('media_blob',$mid)` → **derive accent (Task 1.3) + UPDATE accent_color in the same round-trip** → `state.emit(SyncEvent::ListsChanged)` → 204. Mirrors `set_avatar` (`gallery.rs:35-66`) + `patch_guild` (`guilds/mod.rs:405-456`).
- [ ] Re-export from `guilds/mod.rs`; register `.route("/guilds/{id}/icon", put(guilds::set_guild_icon))` in `server/mod.rs` beside the `/personas/{id}/avatar` route.
- **Tests:** `tests/guilds.rs` — owner→204 + read reflects; non-member→404; plain member→403; unknown media→404.
- **Commit:** `feat(guilds): add PUT /guilds/{id}/icon — manager-gated icon set (M6/P1)`

### Task 1.3 — Deterministic server-side accent derivation
**Files:** `src/server/guilds/icon.rs`
- [ ] `derive_accent_from_image(bytes) -> Option<String>` per the algorithm below; wire it into `set_guild_icon` (read stored bytes via the `media_blob.storage_path`, same disk read `media.rs` uses; on decode failure keep the icon, skip accent, never 500).
- [ ] Define the 8 anchor RGBs mirroring `src/ui/accent.rs::accent_glow_css`; cross-reference comment so the two stay in lockstep.
- [ ] Honor the overwrite policy (default: overwrite; if owner picks stickiness, gate on `accent_color IN [NONE,'']`).

```
DETERMINISTIC ACCENT DERIVATION (runs in set_guild_icon, after the icon UPDATE)
1. decode+downscale (spawn_blocking, like make_thumb):
   image::load_from_memory(&bytes) → .thumbnail(64,64) → .to_rgb8()   // ≤4096 px, O(1)
2. saturation-weighted average (no clustering, no RNG): per pixel sRGB→HSV;
   weight w = s; accumulate sum_{r,g,b} += channel*w, sum_w += w, sat_total += s.
   (down-weights washed-out background so a logo on white derives from the logo)
3. grayscale gate then nearest-anchor:
   avg_s = sat_total / pixel_count;
   if sum_w==0 OR avg_s < SAT_THRESHOLD (~0.12) → "gray";
   else map avg to nearest of 8 FIXED anchors (= accent_glow_css triples) by
   squared sRGB distance, ties broken by ACCENT_PALETTE order. Result ∈ ACCENT_PALETTE.
4. persist: UPDATE guild SET accent_color = $derived (same handler/round-trip).
COLOR SPACE: sRGB u8 throughout; HSV only for the weight + grayscale gate. No k-means,
no random seed, fixed downscale filter, fixed tie-break ⇒ fully reproducible/testable.
```
- **Tests:** `tests/guilds.rs` — solid-red PNG→`red`, solid-green→`green`, flat-gray→`gray` (generate PNGs in-memory via `image`); unit test: each anchor RGB maps to its own name, low-saturation→`gray`.
- **Commit:** `feat(guilds): derive per-server accent from icon at upload via image crate (M6/P1)`

### Task 1.4 — Client api + act
**Files:** `src/client/api.rs`, `src/ui/shell/act/guild.rs`
- [ ] `api::set_guild_icon(gid, media_id)` via `put_json` (mirror `set_persona_avatar`, `api.rs:660`).
- [ ] `act::set_guild_icon(s, gid, file)`: `upload_media` → `api::set_guild_icon` → `refresh_guilds(s)` (re-renders icon AND rebinds the derived accent var). Add the `#[cfg(not(feature="hydrate"))]` stub.
- **Commit:** `feat(guilds): client upload+set guild icon, refresh rail for derived accent (M6/P1)`

### Task 1.5 — UI render: ServerModal upload control + orbit icon
**Files:** `src/ui/shell/server.rs`, `src/ui/shell/sk_orbit/mod.rs`
- [ ] `ServerModal`: add a "Server icon" section — live preview (`/media/{icon_id}?w=128`, ✦/monogram fallback) + a hidden `<input type=file accept="image/*">` behind a styled label calling `act::set_guild_icon`; derive `icon_id` live from `s.sel.guilds` like `accent_name`/`server_name` (`server.rs:38-62`). Note that uploading re-derives the accent.
- [ ] `sk_orbit/mod.rs`: render `<img src=/media/{id}?w=N>` when `icon_id` is `Some` in the **core nucleus** (~:599, ✦ fallback) and the **far-server discs** (~:745, monogram fallback). Use the `members.rs::avatar` Some/None match shape.
- **Tests:** manual headed demo (visual oracle).
- **Commit:** `feat(orbit): render guild icon in core nucleus, far discs, and ServerModal (M6/P1)`

### Task 1.6 — SCSS: icon styling + touch floor
**Files:** `style/_sk_orbit_chrome.scss`, `style/_modal.scss`
- [ ] `.sk-orbit-core-icon` (circular, `object-fit:cover`, fills the core disc) + far-disc icon img rule (sized to the mini-disc).
- [ ] Style the icon-upload control; **touch floor at its base def** (`min-height:2.75rem`, `+min-width` if square) — not an `.app.sk-orbit` override. No `@keyframes` added (style_lint stays green).
- **Commit:** `a11y(orbit): style guild icon render + 44px touch floor on icon upload (M6/P1)`

---

## P2 — Account profiles

### Task 2.1 — schema-apply guard (no migration; pin clean re-apply)
**Files:** `tests/schema_apply.rs`
- [ ] New `#[tokio::test]`: on a bare namespace, CREATE a legacy account row without `display_name`/`avatar`, apply `storage::SCHEMA`, assert `.check()` succeeds, backfill leaves `display_name=''`/`avatar=NONE`, and a subsequent `UPDATE … SET display_name='x'` succeeds (NONE-coercion guard works).
- **Commit:** `test(account): pin schema-apply clean over legacy account rows (M6/P2)`

### Task 2.2 — DTOs
**Files:** `src/protocol.rs`
- [ ] `MeResponse`: add `#[serde(default)] pub avatar_id: Option<String>` (doc like `is_admin`).
- [ ] Add `PatchAccountRequest { #[serde(default)] display_name: Option<String>, #[serde(default)] avatar: Option<String> }` — `#[derive(…, Default)]`, all-Option (PATCH-shaped); `avatar` is a `POST /media` id.
- [ ] `MessageEnvelope`: add `#[serde(default)] pub author_avatar_id: Option<String>` — **LIVE** account avatar (contrast frozen `persona_avatar_id`).
- **Commit:** `feat(protocol): add MeResponse.avatar_id, PatchAccountRequest, live author_avatar_id (M6/P2)`

### Task 2.3 — `/auth/me` projects `avatar_id`
**Files:** `src/server/auth/registration.rs`
- [ ] `account_profile()` (~:234): add `(IF avatar != NONE THEN meta::id(avatar) ELSE NONE END) AS avatar_id` to SELECT + Row + return tuple; `me` handler sets `MeResponse.avatar_id`.
- **Tests:** `tests/auth.rs` — me returns `avatar_id` (null before any avatar set).
- **Commit:** `feat(auth): project account avatar_id onto GET /auth/me (M6/P2)`

### Task 2.4 — `patch_account` endpoint (AuthAccount-gated, account-scoped)
**Files:** `src/server/auth/profile.rs` (new, re-export from `auth/mod.rs`), `src/server/validate.rs`, `src/server/mod.rs`
- [ ] `validate.rs`: `validate_display_name(&str)` — `chars().count() ∈ 1..=32` (trim is the caller's job; distinct from `validate_name`'s 1..=100, kept separate).
- [ ] `patch_account(State, AuthAccount, Json<PatchAccountRequest>)`: multi-field SET like `patch_guild`; `display_name` trimmed+validated (400 on out-of-range); `avatar` media-exists→404; **account-scoped** (UPDATE the caller's own row — no membership/manager gate, no privacy-404 surface); empty body→204; emit `SyncEvent::ListsChanged` on success.
- [ ] Route `.route("/account", patch(auth::patch_account))` (static, under the JSON body cap).
- **Tests:** `tests/auth.rs` (or new `tests/account.rs`) — display_name happy→204 + me reflects; empty/over-32/whitespace→400; avatar real→204, bogus→404; no cookie→401.
- **Commit:** `feat(auth): add PATCH /account for display_name + avatar (M6/P2)`

### Task 2.5 — LIVE `author_avatar_id` on `MSG_PROJECTION`
**Files:** `src/server/messages/reading.rs`
- [ ] `MSG_PROJECTION` (~:425, beside `author_display`): add `(IF author.avatar != NONE THEN meta::id(author.avatar) ELSE NONE END) AS author_avatar_id` — **read-time, never a snapshot**. Add the field to the row struct + `MessageEnvelope` mapping.
- [ ] Grep every `MessageEnvelope` construction site (system/roll/posting echo/trash) and set the field (compile error catches misses).
- **Tests:** `tests/messages.rs` — send msg, list→`author_avatar_id` null; then `PATCH /account` avatar, re-GET the **same old** message → `author_avatar_id` now reflects the new media id (LIVE).
- **Commit:** `feat(messages): project LIVE author_avatar_id onto messages (M6/P2)`

### Task 2.6 — Invariant test: live account identity vs frozen persona snapshot
**Files:** `tests/messages.rs`
- [ ] Send two messages (bare account + wearing a persona w/ snapshot avatar), `PATCH /account` display_name+avatar, re-GET: assert the **bare** message's `author_display`/`author_avatar_id` track the live account on an OLD row; the **persona** message's `persona_name`/`persona_avatar_id` stay **frozen** (its `author_*` still tracks the live account for the subtle marker).
- **Commit:** `test(messages): pin live account identity vs frozen persona snapshot (M6/P2)`

### Task 2.7 — SSE: broadcast on profile change
**Files:** `src/server/auth/profile.rs`, `tests/sync_events.rs`
- [ ] `patch_account` emits **broadcast** `SyncEvent::ListsChanged` (a rename/re-avatar alters `author_display`/`author_avatar_id` on this account's old messages in every shared channel → all clients must refetch; id-only contract preserved). Editing device also refetches `/auth/me` (client task 2.9).
- **Tests:** `tests/sync_events.rs` — a second member of a shared guild receives `ListsChanged` after member A `PATCH /account`.
- **Commit:** `feat(auth): broadcast ListsChanged on account profile change (M6/P2)`

### Task 2.8 — Client api
**Files:** `src/client/api.rs`
- [ ] `api::patch_account(display_name, avatar)` against `PATCH /account`; avatar upload reuses `upload_media`.
- **Commit:** `feat(client): add api::patch_account (M6/P2)`

### Task 2.9 — Client act + AccountModal Profile section
**Files:** `src/ui/shell/act/account.rs`, `src/ui/shell/account.rs`
- [ ] `act::save_display_name` + `act::set_account_avatar` (hydrate-gated + ssr stubs): mutate → refetch `current_user()` → `auth.user.set(Some(me))` so the editing device updates its own `AuthCtx` immediately (mirror `act/persona.rs::set_persona_avatar`).
- [ ] `AccountModal`: add a "Profile" section — `display_name` text input (`maxlength 32`, Save→`save_display_name`) + avatar slot with `<input type=file accept="image/*">` (reuse the wardrobe persona-avatar file-extraction block, `wardrobe.rs:534-550`). Read current values from the `AuthCtx` signal.
- **Tests:** manual headed demo.
- **Commit:** `feat(account): add Profile section (display_name + avatar) to AccountModal (M6/P2)`

### Task 2.10 — Render account avatar + subtle persona marker in message rows
**Files:** `src/ui/shell/channel/meta.rs`, `style/_msg_who.scss`
- [ ] `message_meta`: when **no** persona is worn, use the LIVE `author_avatar_id` for the row avatar (monogram fallback on `author_display`); when a persona **is** worn, keep the persona snapshot avatar and render a subtle `<span class="who-account"> · {author_display}</span>` after the name.
- [ ] `.who-account` SCSS: muted/lighter/small (non-interactive — no touch floor).
- [ ] **Note the orbit name-only collision** (`_sk_orbit_chat.scss:124` hides `.chat-avatar` under `.app.sk-orbit`): whether the avatar shows in orbit chat is the headline *Owner decision (demo-driven)* — server projection lands regardless; the render/visibility is set on the demo.
- **Tests:** manual headed demo.
- **Commit:** `feat(channel): account avatar + subtle persona marker in message rows (M6/P2)`

---

## P3 — Nova DOT orb

### Task 3.1 — Nova DOT orb inline-SVG component
**Files:** `src/ui/icons.rs` (or a sibling module)
- [ ] `NovaOrb` component following the `icons.rs` macro pattern (inline `<svg>`, fixed viewBox, `aria-hidden`, optional class prop). Keep the static orb body and the animatable ring as **distinct elements** so CSS can drive the ring without touching the body. Always-on (no ssr/hydrate-only crates; identical SSR/hydrate output). If the art is too rich to hand-author, `include_str!` `public/nova-dot.svg` into `inner_html`.
- **Commit:** `feat(ui): add Nova DOT orb inline-SVG component (M6/P3)`

### Task 3.2 — Render orb + badge chip in `system_message_meta`
**Files:** `src/ui/shell/channel/meta.rs`
- [ ] In `system_message_meta` (:130-145) replace `chat_avatar(…)` with the orb element using class **`.nova-orb`** (NOT `.chat-avatar` — see risk) and fold the "SYSTEM" badge into the orb's badge chip. Leave `message_meta` untouched; `kind='system'` keying already handled (`mod.rs:983-984`).
- **Tests:** existing `tests/system_messages.rs` round-trip stays green.
- **Commit:** `feat(ui): render Nova DOT orb as the system-message avatar + badge chip (M6/P3)`

### Task 3.3 — Ring keyframe + reduced-motion kill
**Files:** `style/_motion.scss`
- [ ] `@keyframes fx-nova-ring` animating **only** transform/rotate/scale/opacity (resting = element defaults). Any glow = **static** max `box-shadow` on a `::before`, pulse opacity only (never animate `box-shadow`/`filter`). Ensure reduced-motion coverage: give the ring an `fx-`-bearing class (auto-matched by the `[class*="fx-"] { animation:none }` kill) **or** add an explicit entry to the `_motion.scss` reduced-motion list.
- **Tests:** `cargo test --features ssr --test style_lint` green.
- **Commit:** `feat(a11y): animate the Nova DOT orb ring composite-cheap with reduced-motion kill (M6/P3)`

### Task 3.4 — Style the orb + orbit visibility exception
**Files:** `style/_content.scss`, `style/_sk_orbit_chat.scss`
- [ ] `.nova-orb` sizing/placement + badge-chip rules near `.msg.system`/`.system-badge` (reuse `.msg.system` accent tokens).
- [ ] Confirm the orbit `.chat-avatar { display:none }` (`_sk_orbit_chat.scss:124`) does **not** reach `.nova-orb` (different class) — the orb is the deliberate exception to orbit's name-only chat; verify it renders under `.app.sk-orbit`. Orb is decorative → no touch floor.
- **Tests:** visual check under orbit on the HTTPS deck.
- **Commit:** `feat(ui): style the Nova DOT orb and keep it visible under orbit (M6/P3)`

### Task 3.5 — Guard the renderer contract
**Files:** `tests/system_messages.rs`
- [ ] Assert the `kind='system'` + author `nova_dot` wire contract the orb keys on (largely pinned at :108-111, :200-204 — add an explicit assertion if a gap exists). *(Open: a true DOM-render smoke via `render_to_string` would be the suite's first component test — owner pick.)*
- **Commit:** `test(ui): guard the Nova DOT system-message orb render contract (M6/P3)`

---

## Verification (M6 gate)

- **Quality gate:** `/check` green; `cargo test --features ssr` = **0 failed** (live SurrealDB, per-worker namespaces); `tests/style_lint` green; `nova` builds (`protocol.rs` is always-on, must stay wasm-safe + nova-compatible).
- **New coverage:** guild icon set/round-trip + authz (404/403); accent derivation (red/green/gray + unit anchor map); `patch_account` (validation/authz/404/401) + `/auth/me` avatar_id; **live account identity vs frozen persona snapshot** (the core P2 invariant); profile-change SSE broadcast; schema-apply legacy-row guard; Nova orb contract + style_lint.
- **Pre-deploy DB gate:** `MSG_PROJECTION` gains a record-link point-read (`author.avatar`); run it through the throwaway-namespace prod-SurrealDB shape gate before any deploy (don't assume the prod binary plans it identically — `reading.rs:400-420`).
- **Real-device iOS deck pass (orbit only — sole shell):** on the novahome deck over HTTPS (WebKit photo-picker + Secure-cookie path), verify: guild-icon upload + monogram→photo on core/far nodes; derived accent visibly tints the warp-jump on channel switch; account avatar upload + (demo-decided) message-row render; Nova orb visible under orbit with reduced-motion respected; **every new control ≥44px**. Verify on the iPhone/WebKit deck, not Chromium (Chromium-green ≠ fixed).
- **Owner blessing:** walk the *Owner decisions (demo-driven)* list on the headed demo; lock the visual choices there.

## Critical files

- **Schema/DTO (always-on):** `src/storage/schema.surql` (no change — reuse :24,108,123), `src/protocol.rs` (GuildSummary/Detail, MeResponse, PatchAccountRequest, MessageEnvelope, SetGuildIconRequest).
- **Server:** `src/server/guilds/{mod.rs,icon.rs(new)}`, `src/server/accent.rs`, `src/server/media.rs`, `src/server/personas/gallery.rs` (pattern), `src/server/auth/{registration.rs,profile.rs(new),mod.rs}`, `src/server/validate.rs`, `src/server/messages/reading.rs`, `src/server/mod.rs` (routes).
- **Client/UI:** `src/client/api.rs`, `src/ui/accent.rs` (anchor source of truth), `src/ui/icons.rs`, `src/ui/shell/{server.rs,account.rs}`, `src/ui/shell/act/{guild.rs,account.rs}`, `src/ui/shell/sk_orbit/mod.rs`, `src/ui/shell/channel/{meta.rs,avatar.rs}`.
- **SCSS:** `style/{_sk_orbit_chrome,_sk_orbit_chat,_modal,_content,_msg_who,_motion}.scss`.
- **Tests:** `tests/{guilds,auth(or account),messages,sync_events,schema_apply,system_messages,style_lint}.rs`, harness `tests/common/mod.rs`.

## Provenance
Derived from spec §3/§1/§12/§13 + owner rulings in ctx `019ed4de` (M5 release-readiness), `019ed374` (orbit-only release scope), `019ed36c` (evolution-not-port), against a full 2026-06-17 code map (current-state of schema/media/guild/account/message-identity/orbit). Plan-readiness synthesis persisted to ctx (tags `milestone`/`m6`).
