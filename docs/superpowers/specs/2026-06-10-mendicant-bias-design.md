# Mendicant Bias — full redesign + selective re-architecture

**Date:** 2026-06-10
**Status:** Approved design, pending implementation plan
**Codename:** `mendicant-bias` (bump `[package.metadata.release].codename` at release)
**Mockups:** `assets/2026-06-10-mendicant-bias/` (self-contained HTML fragments from the brainstorm session; `wow-concepts.html` and `final-design-v4.html` are the most complete references)

## Context

Two ambitions, one release. First: simplify and optimize the codebase, with selective re-architecture where it pays (the owner chose "re-architecture of selected parts" over both a conservative refactor and a ground-up rewrite). Second: replace the current warm-parchment "Grimoire" UI with a futuristic, high-tech design that is smartphone-first and touch-friendly. During brainstorming the scope grew deliberately to include identity features (guild icons, account nicknames/avatars), DMs with group support, an end-to-end-encrypted personal toolbox, and a mobile QoL pass.

The app remains what it is: a Discord-style roleplay chat (Leptos 0.8 full-stack, SurrealDB 3.x, prod on a Raspberry Pi 4B). Everything below preserves the seven security invariants in CLAUDE.md verbatim; where new features touch authorization, they inherit the existing mechanisms rather than invent new ones.

## 1. Visual design: "Void Station × Liquid Glass"

Chosen over neon-cyberpunk, tactical-HUD, and pure-glassmorphism directions (see `visual-style.html`); the owner hesitated between Void Station and Neon Pulse, so restrained neon glow is deliberately part of the accent language.

### Material hierarchy (three layers)
1. **Background:** deep-space graphite blue (`#0b0e14` base) with subtle aurora tinting.
2. **Content:** opaque calm cards (`#10141d`, hairline borders `#1a2130`) — prose stays readable, scrolling stays cheap.
3. **Chrome:** frosted glass (translucent panels, `backdrop-filter: blur+saturate`, specular top-edge highlight) on topbar, tab bar, bottom sheet, modals. Glass is for chrome, never for prose (except in Ögongodis mode).

### Tokens
- Accent: electric blue `#4d9fff` with glow (`box-shadow` halos); live/online mint `#8ee6c8`; desaturated red for destructive actions.
- The 8 persona tints (`.mk-*` classes) are re-derived as luminous variants tuned for the dark-blue base. Class names and stored color values are unchanged — only the resolved CSS colors change.
- Text ramp: `#dde6f2` / `#aab8cc` / `#8a98ad` / `#5d6b80`.
- Motion: 120–180 ms precise transitions, spring easings (`cubic-bezier(0.2, 0.9, 0.3, 1.15)`-family) for entrances; `prefers-reduced-motion` disables all decorative motion in both appearance modes.

### Typography ("Duo")
- **UI chrome + persona names:** Space Grotesk (new, self-hosted woff2 400/600), names uppercase with letter-spacing.
- **Prose (message bodies):** Crimson Pro stays (already self-hosted) — the story keeps its literary soul.
- **Metadata/timestamps:** monospace stack (`JetBrains Mono`/system mono).
- EB Garamond is retired (delete the woff2 files and `@font-face`).

### Iconography
Text glyphs (↑↓⤒⤓ etc.) are replaced by a small inline-SVG icon set (~16 icons) implemented as Leptos components — required anyway for the tab bar.

### Appearance modes
- **Standard (default):** the full approved design — glass chrome, spring entrances, directional bubbles, glow pulses, Nova DOT orb, and the [STD]-tier wow concepts below.
- **Ögongodis (opt-in, Konto → Utseende):** everything in Standard *plus* the [ÖG]-tier effects. One root class (`.fx-max`) gates the entire tier. Persisted with the existing client-prefs pattern (localStorage, like the dialogue-formatting toggle). `prefers-reduced-motion` always wins.

### Wow concepts (all nine approved)
| | Concept | Tier |
|---|---|---|
| A | **Warp jump** — channel/server switch as an FTL streak transition; light 200 ms version in Standard, full warp field in Ögongodis. Refinement: the warp tints toward the *destination's* accent color (see G). | STD light / ÖG full |
| B | **Scene light** — ambient channel lighting blends the active speakers' persona tints, shifting as the conversation moves. | ÖG |
| C | **Hologram materialization** — incoming messages form via scanline sweep + coalescing particles (Standard keeps spring slide-in). | ÖG |
| D | **Constellation presence** — the typing indicator is replaced (both tiers) by orbiting star-points, one per typist, in their persona color. | STD |
| E | **Charging send button** — ring fills proportionally to message length; on send a pulse wave follows the message into the flow. | STD |
| F | **Holographic depth** — glass layers tilt with gyroscope (mobile) / pointer (desktop); specular highlight tracks the implied light source. | ÖG |
| G | **Per-server accent** — each guild's accent is derived from its icon's dominant color **server-side at upload time** (the `image` crate is already in the ssr graph) and stored as `guild.accent_color`, so all clients agree at zero client cost. Chrome, glows, and markers re-tint per server. | STD |
| H | **Shader nebula** — a small WebGL fragment shader renders a slowly evolving nebula/starfield background. Pauses on `document.hidden`. The heaviest effect; Ögongodis only. | ÖG |
| I | **Radial long-press menu** — long-press on a message blossoms a radial glass menu (reply/edit/copy/delete) around the finger; replaces hover-only actions on touch. | STD |
| — | Refinement: Nova DOT's orb reacts to scene light (B) where both are active. | ÖG |

### Directional bubbles
Messages authored by the *viewing account* (regardless of worn persona) align right with mirrored corner radius, blue-tinted card, avatar on the right, and a subtle "· du" marker; everyone else aligns left. Pure view-layer logic per reader — no schema or server change. Bubbles cap at ~88% width so long prose breathes. Applies on mobile and desktop.

## 2. Navigation & layout

### Mobile (≤ 768 px): hybrid tabs + sheet
- **Bottom tab bar:** Chatt / Servrar / Vänner / Personas, glowing unread badges. Account reached via avatar chip in the topbar.
- **Chatt tab:** the channel owns the screen. Tapping the topbar channel name (or swiping up on it) opens a **glass bottom sheet**: a horizontal server-icon row (with a fixed "✉ Direkt" space first — see DMs) above the channel list with unread markers. One tap switches channel and dismisses; drag-down closes. Spring physics on the sheet.
- **Servrar tab:** server list → per-server management (channels CRUD/reorder, rename, emoji manager, lorebook, members, trash).
- **Vänner tab:** friends list, requests, and per-friend "message" button (opens/creates the DM thread).
- **Personas tab:** the wardrobe (gallery, editor, sharing).
- The old edge-swipe drawer and `.nav-open` pattern are removed.

### Desktop (> 768 px)
Keeps the efficient 3-column grid (rail + channel sidebar + content), fully re-skinned. A fixed DM home entry sits at the top of the rail. Live sync status indicator in the topbar.

### Touch & mobile QoL workstream
- Keyboard handling via `visualViewport` so the composer is never obscured by the iOS keyboard.
- Swipe-right-on-message to reply; long-press radial menu (concept I); 44 px minimum touch targets everywhere.
- Pull-to-refresh; per-channel scroll position restoration; jump-to-unread pill.
- Optimistic send with a retry queue and an offline indicator; per-channel draft persistence (extend the existing compose auto-save).
- Camera/photo-library upload and paste-image support; haptics via `navigator.vibrate` where available.
- `content-visibility: auto` on message rows for 60 fps scrolling in long histories.
- PWA niceties: manifest colors updated to the new palette; safe-area/notch handling preserved exactly as today (`viewport-fit=cover` + `env(safe-area-inset-*)`).

## 3. Identity features

- **Guild icons:** uploadable per guild (manager permission), reusing the entire media pipeline (server-minted random id, image-only allowlist, thumbnails, nosniff). New `guild.icon_media: option<string>` + `guild.accent_color: option<string>` (server-derived at upload, validated hex). Shown in rail, sheet, and server lists; letter monogram remains the fallback.
- **Account profiles:** `account.display_name: option<string>` (1–32 chars, trimmed) and `account.avatar_media: option<string>`, edited under Konto. Account avatar shows beside messages; when a persona is worn the persona identity dominates and the account shows subtly ("· Damien"). **Resolution semantics:** account identity is resolved live at read time (rename/avatar changes apply to history); persona identity remains snapshotted at send (invariant untouched).
- **Nova DOT:** the system messenger gets a Superintendent-inspired orb avatar — a bundled SVG asset in `public/` (CSS-animated ring where rendered), special-cased for `kind='system'` messages, plus a `SYSTEM` badge chip. No DB change.

## 4. DMs (with group support)

**Model:** DM threads are channels without a guild. `channel.guild` becomes `option<record<guild>>`; `channel.kind` gains `'dm'`. A new `dm_member` SCHEMAFULL table (channel, account, joined_at) carries membership. Group threads are the same thing with 2+ members; the creator invites friends; members can leave. Thread title optional (defaults to member names).

**Access:** `resolve_membership` branches on kind — guild channels resolve via `guild_member` as today; DM channels resolve via `dm_member`. Non-members get the identical privacy-404. Everything else is inherited unchanged: posting, cursor pagination, soft-delete, attachments, push, SSE, and **personas** (in-character DMs are first-class; `channel_active_persona` already works per channel). Starting/inviting requires friendship.

**UX:** "✉ Direkt" space first in the mobile sheet's server row and at the top of the desktop rail; message buttons in the Vänner tab. DM pings push-notify by default.

**Explicit boundary:** DMs are *not* end-to-end encrypted — they are server-readable like guild messages. The vault (below) is the only zero-knowledge store. The UI must not imply otherwise.

## 5. Encrypted personal toolbox ("Verktygslådan")

A per-user client-side toolbox under Konto, with a zero-knowledge vault.

- **Crypto:** vault contents are always encrypted client-side with a dedicated passphrase (separate from the login password; never leaves the client). Argon2id (the `argon2` crate already in the workspace, compiled to WASM; OWASP-recommended parameters) derives a 256-bit key; AES-256-GCM via the browser's native WebCrypto encrypts the vault as a single JSON blob. Envelope: `{v, kdf: {algo, m, t, p, salt}, nonce, ciphertext}`. Fresh random nonce per save. No homegrown primitives.
- **Storage choice per user:** device-only (localStorage) or synced to the server. The server stores only the opaque envelope in a new `vault` table (one row per account: blob, version, updated_at) behind normal session auth, with optimistic concurrency (version check → 409 on conflict, surfaced as a merge prompt). The Pi provably cannot read contents.
- **Unlocking:** passphrase prompt per session; auto-lock after inactivity. Creation flow states plainly: **a forgotten passphrase means unrecoverable data, by design.** Passphrase strength meter at setup.
- **V1 tools:** password/passphrase generator, secure notes, key/snippet storage, UUID/token generator, and stateless utilities (hash, Base64, JSON format). TOTP authenticator is approved for v1 but is the first thing to cut if the workstream runs long.

## 6. Codebase re-architecture & optimization

### Real-time: SSE replaces polling
- New `GET /events` (axum SSE, session-cookie auth — EventSource sends cookies same-origin; keep-alive pings). A broadcast hub in `AppState` (tokio broadcast); per-connection filtering against a cached membership set, invalidated by membership-change events.
- **Notify-and-fetch:** events carry only ids/kind (message_created/edited/deleted + channel id, typing, unread bump, meta changed, dm created). The client refetches through the existing read endpoints with their tested permission checks — the push path carries no content, so it adds **no new authorization surface**.
- Typing state stays in-memory (`AppState.typing`) but is broadcast on POST instead of piggybacked on polls.
- Reconnect: EventSource auto-reconnects; on open the client re-syncs (list calls + batched unread). Persistent SSE failure degrades to the current polling, kept behind a small sync-layer abstraction.
- Effect: idle clients drop from ~150–200 req/min to keep-alive only — the single biggest Pi win.

### Server & client optimization
- **`GET /unread`:** one batched endpoint returning `{channel_id, unread_count, pinged}` for all visible channels (replaces N per-channel `list_messages` probes at boot/reconnect).
- Lazy guild-channel loading (fetch channels for the opened guild only, cached).
- Attachment MIME folded into `MSG_PROJECTION` (removes the second per-page query).
- `Cache-Control` headers on media: thumbnails `public, max-age=31536000, immutable` (deterministic by id), originals long-lived.
- web-sys feature audit; WASM bundle watched (the shader + argon2-in-WASM additions must stay within reason; `wasm-release` profile already aggressive).

### Simplification
- **API transport consolidation:** the duplicated request/decode/error layers in `src/client/api.rs` (gloo-net) and `src/native/api.rs` (reqwest) merge into one shared, always-on typed endpoint layer with two thin transport backends (~300 lines removed). The freya graph keeps building; it simply consumes the shared layer.
- Permission-gate helper collapsing the repeated authorization match boilerplate across server handlers.
- SSR no-op action stubs generated by macro instead of hand-written pairs.
- Error-response helper centralizing repeated status/body literals.

### Schema changes (NONE-coercion discipline)
Every new field on a populated table is `option<>` (no backfill needed): `account.display_name`, `account.avatar_media`, `guild.icon_media`, `guild.accent_color`. The `channel.kind` value domain is extended with `'dm'` (existing rows unaffected). Relaxing `channel.guild` from required to `option<>` must be verified against the schema-apply crash-loop gotcha with a dedicated `tests/schema_apply.rs` case before merge. New tables (`dm_member`, `vault`) are unconstrained by existing rows.

## 7. Invariants (unchanged, with new compliance points)

All seven CLAUDE.md invariants stand. New features comply by inheritance:
1. Session-cookie-only identity — SSE and vault endpoints use the same `AuthAccount` extractor.
2. Authorization re-derived per mutate — DM sends re-check `dm_member`; guild icon uploads re-check manager role.
3. Privacy-404 — DM threads return the identical body to non-members.
4. Parameterized SQL only — all new queries use `.bind()`/`type::record`.
5. Media invariants — guild icons and account avatars ride the existing pipeline untouched.
6. Persona snapshot at send — unchanged; account identity is the only live-resolved display data (deliberate, documented above).
7. Fail-closed admin + soft-delete semantics — unchanged; DM threads honor soft-delete hiding.

## 8. Out of scope

- Native Freya client redesign (it must keep compiling and benefits from the shared API layer, nothing more).
- Light theme; message reactions; full-text search; federation.
- E2EE for DMs (vault-only; stated boundary).
- The deleted `deploy/`/`scripts/`/`end2end/` tooling stays deleted.

## 9. Workstreams (for the implementation plan)

W1 Realtime backbone (SSE + `/unread` + perf fixes) · W2 Design system (tokens, fonts, icons, SCSS foundation) · W3 Shell & navigation (hybrid mobile nav, desktop reskin) · W4 Chat experience (bubbles, composer, STD-tier concepts) · W5 Ögongodis tier (`.fx-max` effects, shader) · W6 Identity (guild icons, profiles, Nova DOT) · W7 DMs · W8 Vault/toolbox · W9 Mobile QoL & a11y polish · W10 API consolidation & simplifications.

Sequencing, dependencies, and test-first details belong to the implementation plan, not this spec.

## 10. Verification

- All 144 existing integration tests pass (`cargo test --features ssr` against live local SurrealDB; per-worker namespace isolation as today).
- New integration tests per workstream, named in commit `Tests:` lines per convention: SSE delivery + membership filtering, `/unread` correctness, DM privacy-404 + group membership flows, vault endpoint opacity/authz/version-conflict, schema-apply safety for `channel.guild` relaxation, guild-icon permission gating.
- `cargo fmt --all`; clippy clean on all three graphs (`ssr`, `hydrate` on wasm32, `freya`); `cargo build --bin authlyn-native --features freya` still succeeds.
- Playwright (headed, M2 dev machine): mobile-viewport flows (login → send → sheet switch → DM → vault unlock) + desktop screenshots; WebKit caveat: inject the session cookie per the known Secure-cookie gotcha.
- **Nothing touches prod or `SURREAL_DB=prod`** during development; all work on branch `mendicant-bias`. Push to `main` only after explicit owner approval (push = live deploy to fenrir; Pi cold build ~90 min; wasm-opt A72 gotcha may need re-applying if the cargo-leptos cache was wiped).
