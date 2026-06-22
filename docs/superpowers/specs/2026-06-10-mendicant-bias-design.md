# Mendicant Bias — full redesign + re-architecture (master spec)

**Date:** 2026-06-10 (catalogue locked same day after four creative passes)
**Amended:** 2026-06-12 — two owner rulings from the first live M4 device sessions written in as binding design laws (mobile-first, UX-equality; see "Design laws") plus a real-device iOS gate added to every wave's verification requirements (§13). Finding #54.
**Amended:** 2026-06-12 (2) — the owner's **Skelettvågen ruling** written in: M5 is redefined as the skeleton wave — THREE user-selectable UI skeletons (Omloppsbana, Kortdäck, Holoterminal) replace the M3 shell outright, with an onboarding theme ceremony and no silent default. The roadmap renumbers (Identity → M6, every later wave +1). Finding #48 and the M4 a11y backlog (both previously memory-only) are written in. See §1 Appearance, §2 Navigation, §12 Waves, §13 Verification; prototypes in `assets/2026-06-12-skelettvagen/`.
**Status:** Approved design — full catalogue locked by owner; implementation in flight (M1–M4 shipped; M5 Skelettvågen plan next)
**Codename:** `mendicant-bias` (bump `[package.metadata.release].codename` at release)
**Version:** **27.0.0** — this release retires CalVer (`YYYY.M.D`). The owner's call: a release of this scope deserves a major, 26.x ends here, and 27.0.0 ships the day the owner turns 27. SemVer from here on; update the project CLAUDE.md Versioning line at release.
**Assets:** `assets/2026-06-10-mendicant-bias/` — 13 mockup HTML fragments from the brainstorm session (`grand-catalogue.html` is the definitive 51-card catalogue; `final-design-v4.html` shows the visual system) plus `lens-concepts-48.json` (full Swedish pitches for the 48 lens concepts, from a 6-agent generation workflow). **Skelettvågen assets (added 2026-06-12):** `assets/2026-06-12-skelettvagen/` — the three interactive skeleton prototypes (`a-orbit.html`, `b-deck.html`, `c-hud.html`, all matrix-verified across the friend-group device geometry), the deck landing (`index.html`), the interactive evolution catalogue (`evolution.html`), and the ranked UX-evolution report (`ux-evolution-2026-06-11.md`: 64 ranked proposals, 3 kills, merge directives, feasibility warnings, 21 blind spots) — owner-approved input to M5 and later waves.

## Context

Two ambitions became one very large release. First: simplify and optimize the codebase with selective re-architecture (owner's choice over both a conservative refactor and a ground-up rewrite). Second: replace the warm-parchment "Grimoire" UI with a futuristic, high-tech, smartphone-first design. Across four creative passes the owner then locked a feature catalogue of ~76 concepts that turn authlyn from a chat app into a collaborative-fiction platform: world maps, tabletop mechanics groundwork, writing craft, community, cinematic delight, and mobile-native powers — plus one sanctioned AI feature (Hypebot) as a public test of NPC/CPU integration ahead of the next release ("Offensive Bias": CPU-driven NPCs/GM, D&D-like play).

Production simultaneously migrates from the Raspberry Pi (fenrir) back to a resurrected, upgraded **novahome** (§8), which removes the server-side performance ceiling and unlocks GPU inference for Hypebot.

The app remains what it is: a Discord-style roleplay chat (Leptos 0.8 full-stack, SurrealDB 3.x). Everything below preserves the seven security invariants in CLAUDE.md; new features inherit existing mechanisms rather than invent new ones.

## Language policy

- **All codebase documentation in English** (existing convention: CLAUDE.md, `//!` headers, doc-comments, commit messages, this spec).
- **All UI copy in English** (existing convention — current views use "username", "password", "your answer"). The Swedish concept names throughout the brainstorm were working aliases; the catalogue below fixes **canonical English product names** with the Swedish alias in parentheses for traceability to `lens-concepts-48.json` and the mockups.

## Design laws (binding — written into the spec 2026-06-12)

Two owner rulings from the first live M4 device sessions (2026-06-11) are design LAW for every wave, current and future. They are not preferences to weigh against other concerns; a design that violates them is wrong even if it ships. Their verification teeth live in §13.

### Mobile-first: PWA/touch is the meta experience
The installed PWA on a phone is the PRIMARY experience; desktop is the gracefully-degraded adaptation — never broken, but never the surface that dictates design. The real user group will never use the app on a computer. Every wave plan from M5 on is authored mobile-first: design the touch/PWA experience first, then adapt it to desktop. The evidence that sealed the ruling: every real bug found in the first live iPhone-PWA session was desktop furniture leaking into mobile (`draggable` rows hijacking taps, the text-selection callout beating the radial long-press) — none were missing mobile features. The truth lives on the phone.

### UX-equality: the same UX is a right, not a hardware perk
No device-class gating, ever. Visual/feature tiers (`.fx-max`) are USER toggles available to everyone — never device-detected, never auto-disabled; informing the user (a battery note) is fine, deciding for them is not. Performance work targets the FLOOR device (the friend group's POCO C3, Android 10): make the same beauty cheap enough to run everywhere, never give weaker devices less. Internal adaptive quality (e.g. render-resolution scaling) is acceptable only as an honest implementation detail that keeps the design and feature set identical for all. The law extends to GEOMETRY: nothing is iPhone-scoped — layouts and gestures scale fluidly to any screen size, aspect, and DPI (`clamp()`/percentages/`dvh`; no hardcoded 375-math), verified across the friend-group device matrix (§13), never against a single reference device.

## 1. Visual design: "Void Station × Liquid Glass"

### Material hierarchy (three layers)
1. **Background:** deep-space graphite blue (`#0b0e14` base) with subtle aurora tinting.
2. **Content:** opaque calm cards (`#10141d`, hairline borders `#1a2130`) — prose stays readable, scrolling stays cheap.
3. **Chrome:** frosted glass (backdrop-blur + saturate, specular top-edge highlight) on each skeleton's chrome surfaces — M3 interim: topbar/tab bar/bottom sheet/modals; post-M5: composer orb + channel pill + slide-over (Omloppsbana), command deck + flip-cards (Kortdäck), hologram panels (Holoterminal). The invariant is the principle: glass is for chrome, never for prose — except in Eye-candy mode.

### Tokens
- Accent: electric blue `#4d9fff` with glow halos; live/online mint `#8ee6c8`; desaturated red for destructive actions.
- The 8 persona tints (`.mk-*`) re-derived as luminous variants for the dark base; class names and stored values unchanged.
- Text ramp `#dde6f2` / `#aab8cc` / `#8a98ad` / `#5d6b80`. Motion: 120–180 ms, spring easings; `prefers-reduced-motion` disables decorative motion in both tiers.

### Typography ("Duo") & icons
- UI chrome + persona names: **Space Grotesk** (new, self-hosted woff2 400/600); names uppercase, letter-spaced.
- Prose: **Crimson Pro stays**. Metadata/timestamps: mono stack. EB Garamond retired.
- Text glyphs replaced by an inline-SVG icon set (~16 Leptos components).

### Appearance: skeleton × tier (amended 2026-06-12 — Skelettvågen)
Appearance has **two user-facing axes**, both under Account → Appearance, both UX-equality user choices (never device-gated):

**Axis 1 — Skeleton (the bones).** Three structural UI skeletons, each a complete interaction paradigm over the same shared core (content panes, message stream + effects renderer, composer internals, the `act/` layer, the `message_actions` predicate):

- **Omloppsbana** (`sk-orbit`) — spatial gesture-first: full-viewport channel panes in a horizontal swipe strip, a zoomable orbit map as guild/channel picker (entered via the channel pill — the pinch entry was judge-killed), floating composer orb with charge ring and effect blossom.
- **Kortdäck** (`sk-deck`) — layered z-stack: one continuous depth scrub from chat through the channel deck to the server galaxy, a persistent command deck as the only chrome, flip-cards instead of modals.
- **Holoterminal** (`sk-hud`) — zero chrome: the message stream alone over a parallax starfield; four edge-summoned hologram panels (channel rig / crew & personas / station HUD / console-composer); materialization sweep as arrival language.

The M3 hybrid shell (bottom tabs + glass sheet) is **retired by this wave** — it survives only as in-tree scaffolding during M5 development and is deleted before the wave closes. Prototype ground truth: `assets/2026-06-12-skelettvagen/`. Naming note: structural shells are always `sk-`-prefixed in code (`.app.sk-*`, `_sk_orbit.scss`, …); the existing loading-placeholder sense of "skeleton" (`_skeleton.scss`, `channel/skeleton.rs`) keeps its unprefixed names.

**Axis 2 — Tier (the garnish), within each skeleton:**
- **Standard (default):** the full rich design — glass chrome, spring entrances, directional bubbles, glow pulses, Nova DOT orb, and every [STD] effect below. Nothing is stripped down.
- **Eye-candy ("Ögongodis", opt-in):** adds MORE on top: glass message cards, specular sweeps over chrome, conic avatar rings, flowing gradient borders on own bubbles, multi-layer drifting aurora, rising sparks, shimmer on system messages, shader nebula, holographic depth. One root class (`.fx-max`); persisted via the existing client-prefs pattern.

**Selection & persistence.** The skeleton pref is client-local per device (`authlyn.skeleton` String id via the `.fx-max` client-prefs pattern; root class `.app.sk-*`) — each device boots its own shell. Cross-device roaming arrives with the prefs-roaming concept (ux-evolution rank 30), not in this wave; no schema change (§6 unaffected). **No silent default:** a device without a stored skeleton pref gets the **onboarding ceremony** — a first-run choice presenting all three skeletons (kin to the boarding-sequence concept, ux-evolution rank 64) — covering new users and existing users after the update alike. The ceremony runs at first authenticated shell mount; auth/login views are skeleton-neutral (shared `_auth` styling). If the pref store is unavailable (localStorage write fails), the device boots `sk-orbit` for the session without ceremony — the no-silent-default rule applies only where a pref can persist.

**Finding #48 (binding; written in 2026-06-12, previously memory-only).** The Standard↔Eye-candy delta must be **unmistakable** — "a wow tier that needs a diff tool is a failed wow tier" — delivered UX-equality-style (a user toggle for all, optimized to the POCO C3 floor) and exhibited by **every skeleton**. Backbone mechanisms from the ux-evolution catalogue: the ignition flip + settings diptych (rank 60 "The Power Surge", merged with rank 63's always-on steady-state deltas per the catalogue's 60+63 merge directive — the flip makes the toggle MOMENT unmistakable, the pulled-forward deltas keep the TIER unmistakable afterwards) and nebula plates (rank 37). M5 owns this delta; the Cinema wave completes the remaining ÖG set per skeleton.

### The nine wow effects (all approved)
A Warp jump (channel switch as FTL streak; tints toward destination accent) [STD light/ÖG full] · B Scene light (ambient lighting from active speakers' tints) [ÖG] · C Hologram materialization (scanline + particle message arrival) [ÖG] · D Constellation presence (typing indicator = orbiting stars per typist, replaces dots in both tiers) [STD] · E Charging send button (ring fills with message length; pulse on send) [STD] · F Holographic depth (gyro/pointer parallax + tracked specular) [ÖG] · G Per-server accent (derived server-side from guild icon at upload, stored as `guild.accent_color`) [STD] · H Shader nebula (small WebGL fragment shader; pauses on `document.hidden`) [ÖG] · I Radial long-press menu (reply/edit/copy/delete blossoms around the finger) [STD]. Refinement: Nova DOT's orb reacts to scene light.

**Skeleton note (2026-06-12):** the nine effects are shared-layer — they ride the message stream, composer, and transition grammar, so every skeleton inherits them. Per-skeleton **placement** (e.g. the warp-jump grammar vs Holoterminal's materialization sweep; the radial menu vs Kortdäck's lifted-card chips fan) is defined per skeleton in the M5 plan — affordances still flow from the shared predicates, never re-branched per skeleton.

### Directional bubbles
Messages authored by the **viewing account** (regardless of worn persona) align right — mirrored radius, blue-tinted card, avatar right, "· you" marker; others align left. Pure per-viewer view logic, no schema change; max-width ~88%; mobile and desktop.

## 2. Navigation & layout

**Amended 2026-06-12 (Skelettvågen):** the M3 hybrid nav below shipped as designed and was then judged by the owner as "the old UI with polish — same skeleton and DNA". **M5 replaces it outright with the three selectable skeletons (§1).** Each skeleton owns its navigation model (orbit map / card deck / edge holograms), its chrome placement, and its desktop adaptation — authored mobile-first per the design laws, with desktop as the gracefully-degraded adaptation *of that skeleton*. The M3 3-column desktop grid retires together with the M3 mobile shell. The Touch & mobile QoL workstream below survives as guaranteed **capabilities** (a reply gesture, message-action access, scroll restoration, keyboard handling, …) — but the concrete gesture binding of the items §1's skeleton note calls out as per-skeleton placements (the radial long-press menu; swipe-right-to-reply vs Omloppsbana's channel strip) is defined per skeleton in the M5 plan. The two subsections that follow are kept for the record as the M3 interim shell.

### Mobile (≤768 px): hybrid tabs + sheet (M3 interim — retired by M5)
- Bottom tab bar: **Chat / Servers / Friends / Personas**, glowing unread badges; account via topbar avatar chip.
- Chat tab owns the screen; tapping the channel name opens a glass **bottom sheet** (server icon row — with a fixed **✉ Direct** space first — above the channel list). One tap switches and dismisses; drag-down closes; spring physics.
- Servers tab: list → per-server management (channels CRUD/reorder, rename, emoji, lorebook, members, trash). Friends tab: list, requests, per-friend message button. Personas tab: wardrobe.
- The old edge-swipe drawer and `.nav-open` pattern are removed.

### Desktop (>768 px) (M3 interim — retired by M5)
3-column grid kept (rail + sidebar + content), fully re-skinned; fixed Direct entry atop the rail; live sync indicator in the topbar.

### Touch & mobile QoL workstream
`visualViewport` keyboard handling; swipe-right-to-reply; radial long-press menu; 44 px targets; pull-to-refresh; per-channel scroll restoration; jump-to-unread; optimistic send + retry queue + offline indicator; per-channel drafts; camera/photo upload + paste-image; a designed haptic language (Haptic Pulse, below) via `navigator.vibrate` with visual fallback; `content-visibility: auto` rows; PWA manifest colors updated; safe-area handling preserved.

## 3. Identity features

- **Guild icons:** uploadable (manager-gated), riding the full media pipeline; `guild.icon_media: option<string>` + `guild.accent_color: option<string>` (derived server-side at upload via the `image` crate). Monogram fallback.
- **Account profiles:** `account.display_name: option<string>` (1–32, trimmed) + `account.avatar_media: option<string>`; edited under Account. Account avatar beside messages; worn personas dominate with the account shown subtly ("· Damien"). **Account identity resolves live at read; persona identity stays snapshotted at send** (invariant untouched).
- **Nova DOT:** Superintendent-inspired orb avatar (bundled SVG in `public/`, CSS-animated ring) + badge chip on system messages.

## 4. DMs (with group support)

DM threads are channels without a guild: `channel.guild` → `option<record<guild>>`, `channel.kind` gains `'dm'`, new SCHEMAFULL `dm_member` table. Groups = 2+ members; creator invites friends; members can leave; optional title. `resolve_membership` branches on kind; non-members get the identical privacy-404. Everything else inherited: posting, cursors, soft-delete, attachments, push, SSE, personas (`channel_active_persona` works per channel). **DMs are NOT end-to-end encrypted** (server-readable like guild messages; the vault is the only zero-knowledge store) — the UI must not imply otherwise.

## 5. Encrypted personal toolbox

Per-user toolbox under Account with a zero-knowledge vault: dedicated passphrase (never leaves the client) → Argon2id (existing crate, WASM, OWASP params) → AES-256-GCM via WebCrypto; single JSON blob, envelope `{v, kdf, nonce, ciphertext}`, fresh nonce per save. Storage per user: device-only (localStorage) or synced as an opaque envelope to a new `vault` table (one row per account; version + 409-on-conflict). Auto-lock on inactivity; creation flow states plainly that a forgotten passphrase = unrecoverable by design. V1 tools: password/passphrase generator, secure notes, key/snippet storage, UUID/token generator, stateless hash/Base64/JSON utilities, TOTP authenticator (first cut if the workstream runs long).

**Hardware-backed unlock (WebAuthn PRF):** users may additionally enroll a passkey (platform authenticator — Secure Enclave, Android StrongBox — or a security key); the WebAuthn `prf` extension derives a device-bound secret that wraps the vault key, enabling biometric unlock backed by the user's own secure hardware. The passphrase remains the root credential; PRF wraps are per-device and revocable; the zero-knowledge property is unchanged (the server stores only opaque envelopes and WebAuthn public credentials). The server's TPM cannot strengthen client-side E2EE by design — hardware backing for the vault lives on the user's devices via PRF; the server TPM's role is in §8.

## 6. Codebase re-architecture & optimization

### Real-time: SSE replaces polling
`GET /events` (axum SSE, session-cookie auth, keep-alive) over a tokio broadcast hub in `AppState`; per-connection filtering against a cached membership set invalidated by membership events. **Notify-and-fetch:** events carry ids/kind only (message created/edited/deleted + channel, typing, unread bump, meta changed, dm created, plus new feature events: atmosphere/mood change, initiative turn, clock tick, presence move, sheet update…); clients refetch through existing permission-checked read endpoints — the push path carries no content and adds **no new authorization surface**. Typing stays in-memory, broadcast on POST. Reconnect = full re-sync (lists + batched unread). Persistent failure degrades to the old polling behind a small sync-layer abstraction. Idle clients drop from ~150–200 req/min to keep-alive only.

### Server & client optimization
Batched `GET /unread` (one round-trip for all visible channels); lazy guild-channel loading; attachment MIME folded into `MSG_PROJECTION`; `Cache-Control` on media (thumbnails immutable 1y); web-sys feature audit; WASM bundle watched (shader + argon2 additions must stay reasonable; `wasm-release` already aggressive).

### Simplification
Shared typed API layer with gloo-net/reqwest transport backends replacing the duplicated client/native layers (~300 lines removed; freya keeps building); permission-gate helper collapsing repeated authorization boilerplate; macro-generated SSR no-op action stubs; centralized error-response helpers.

### Server-side rendering unlocked (novahome)
With §8's hardware, heavier server-side work is permitted where it beats client cost: poster/keepsake/quote-card PNG composition, EPUB/PDF typesetting for Story Book exports, atlas tile/thumbnail pyramids. Client-side remains the default for per-user cosmetic rendering (crests, leitmotifs, filters).

### Schema changes (NONE-coercion discipline)
All new fields on populated tables are `option<>`: `account.display_name`, `account.avatar_media`, `guild.icon_media`, `guild.accent_color`, persona fields for mood portraits/entrances/leitmotif config, `message` extensions for effects/kind variants. `channel.kind` gains `'dm'`. Relaxing `channel.guild` to `option<>` must be guarded by a dedicated `tests/schema_apply.rs` case. New tables (e.g. `dm_member`, `vault`, `scene`, `world_calendar`, `atlas_*`, `inventory_item`, `sheet_*`, `quest`, `clock`, `quote`, `event_session`, `bot_config`…) are unconstrained by existing rows; exact set defined per wave in the implementation plan.

## 7. Hypebot & the bot gateway

The sole AI feature of this release, and the deliberate public test for Offensive Bias NPC/CPU integration.

- **App-side bot gateway (brain-agnostic):** generalizes the Nova DOT pattern into a bot framework — bot accounts (login-disabled, badged), **per-channel opt-in**, triggers (on-demand command, scene-close hook, "Previously On" recap fill, capped cadence), a server-side context assembler (recent message window + scene titles + lore summaries, bot-gated), and posting through the existing system-message pipeline with a clear COMMENTATOR badge. Rate-limited; per-guild kill switch. Bot output never impersonates user personas.
- **Brains, pluggable:** default = **local llama.cpp server on novahome's RTX 4080 Super (16 GB VRAM)** — candidate shapes: ~27B at Q4 with partial CPU offload (DDR5-6000 makes offload viable) vs a 12–14B fully VRAM-resident at higher speed/context; final model chosen at implementation time against current benchmarks. Async job queue, single-flight, latency-tolerant by design. The **nova-mcp bridge remains the external-brain path** (user-supplied stronger LLM). The app is fully functional with no brain attached (feature hidden). fenrir-class CPU inference remains a documented fallback.
- **Offensive Bias seam:** the same gateway later drives CPU-controlled NPC personas and GM assistance; nothing in this release hardcodes "commentary" as the only bot capability.

## 8. Production host migration: novahome

Prod moves from fenrir (Pi 4B) to the resurrected **novahome**: Ryzen 9 7900X (12c/24t), RTX 4080 Super 16 GB, 32 GB DDR5-6000 CL30 (EXPO I on ASUS TUF X670E-Plus), Kingston Fury Renegade 2 TB + WD Blue SN580 2 TB NVMe.

- **Effects:** release builds drop from ~90 min to minutes; the wasm-opt Cortex-A72 gotcha becomes irrelevant; SSE concurrency, search indexing, and server-side rendering get real headroom; Hypebot gets GPU inference.
- **Provisioning split:** Claude prepares a bootable USB on the dev MacBook — OS image + fully automated install configuration + post-install provisioning scripts (users/SSH keys, Caddy, SurrealDB, GitHub Actions runner with a new label, CUDA + llama.cpp service, firewall, unattended upgrades). The owner's only manual step is BIOS (disabling Secure Boot — accepted: it simplifies the proprietary NVIDIA/CUDA stack for Hypebot; TPM 2.0 functions independently of Secure Boot, and SB with MOK-signed drivers can be revisited later). OS: Debian 13 for fleet consistency, NVIDIA driver + CUDA from NVIDIA's repository.
- **TPM 2.0 plan (fTPM on the X670E):** (1) LUKS2 full-disk encryption on both NVMe drives with TPM-sealed keys (`systemd-cryptenroll`, PCR-bound) — unattended boot, but a stolen or removed disk is unreadable; an offline recovery passphrase is printed and stored physically. (2) Server secrets as TPM-encrypted systemd credentials (`systemd-creds encrypt --with-key=tpm2` + `LoadCredentialEncrypted=` in the authlyn/nova units) — VAPID keys, `NOVA_PASSWORD`, and admin configuration never sit in plaintext on disk. (3) TPM RNG as an entropy source. (4) Optional later: tpm2-pkcs11 for TLS/SSH keys. Client-side vault hardware backing is WebAuthn PRF (§5), not the server TPM.
- **Migration tasks (infra, outside this repo):** the USB provisioning above; migrate prod data with a restore-verified backup (per the established `/data/prod_backups/` discipline); deploy-workflow retarget (the fenrir migration provides the playbook in reverse); demote fenrir to fallback/xray duty; update CLAUDE.md (Deploy section, gotchas) and project memory when cut over.
- **Timing** (before vs parallel to implementation) is decided in the implementation plan; development proceeds locally regardless. The no-prod-experiments guardrail applies to novahome exactly as it did to fenrir.

## 9. Feature catalogue (locked — everything approved)

Canonical English names; Swedish brainstorm alias in parentheses. Sizes: S=days, M≈week, L=multi-week. Full pitches: `lens-concepts-48.json`. ♻ = built as one with its partner.

### 9.1 Application features (9)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| Story Book | Berättelseboken | L | Mark scenes/chapters; render channels as a typeset novel; export HTML/EPUB |
| Fate Engine | Ödesmotorn | S | Server-validated `/roll`, coins, oracles as animated chips — no cheating |
| Living Lore | Levande lore | M | Lorebook names auto-link in prose; tap opens floating glass lore card |
| Constellation Map | Konstellationskartan | M | Starmap of persona relationships derived from interaction history |
| Memory Core | Minneskärnan | M | Full-text search across messages (SurrealDB search indexes), hit highlighting |
| Message Effects | Meddelandeeffekter | S | Send-as whisper (blur until tapped), shout (shake), spell (particles) |
| Ghost Quill | Spökpennan | M | Opt-in live preview of a co-writer's in-progress text via SSE |
| Session Calendar | Sessionskalendern | M | Schedule play sessions, RSVP, push reminders, Nova DOT announces |
| Council Polls | Rådslaget | S | In-chat polls — the group votes on the story's path |

### 9.2 Pioneer features (7)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| World Atlas | Världsatlasen | L | Uploadable world map; channels/lore/scenes pinned; pan/zoom home for the world |
| Atmospheres | Atmosfärer | M | GM-set channel mood (storm/night/battle/feast) synced live to everyone + procedural WebAudio ambience; Media Session lock-screen track (♻ The Aether) |
| Living Portraits | Levande porträtt | M | Mood-variant persona avatars, auto-detected from \*emotes\* or chosen per message |
| Director Mode | Regissörsläget | L | Scene slates, narrator voice, secret whispers to chosen players, NPC carousel; intertitle time cards (♻ Intertitles) |
| Performance Mode | Föreställningsläget | M | Replay a scene as timed theater with effects and scene light |
| Backdrops | Kulisserna | S/M | Scene art dimmed behind the chat, crossfading on scene changes |
| Time Capsule | Tidskapseln | S | Sealed letters arriving at a future time; dead-man-switch option |

### 9.3 Owner features (3)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| Presence Map | Närvarokartan | M | Personas live ON the atlas as glowing avatars; drag to move; channels know who's present |
| Inventory | Packningen | L | Persona items with art + lore, gifts/trades, GM grants; absorbs Relics provenance & The Cache burial (♻) |
| Hypebot | — | L | Nova DOT as prose-rich commentator; pluggable brain (novahome GPU default, nova-mcp external); §7 |

### 9.4 Lens: The World (8)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| World Calendar | Tideräkningen | M | Custom in-fiction calendar, moon phases, seasons; scenes stamped in story time |
| Weather Almanac | Väderleken | M | Deterministic seeded weather per region/world-date, zero server cost; GM can override; feeds Atmospheres |
| Realms | Markerna | L | Painted map regions carrying channels, default atmosphere, weather seed, lore card; per-pin Place Memories (♻) |
| Journeys | Färdvägen | M | Persona travel: route drawn, world-day travel time, SSE arrival event, travel journal |
| Fog of War | Dimhöljet | M | GM reveals the map progressively, live for everyone; discovery log |
| The Post | Postgången | M | Sealed persona-to-persona letters; delivery time from map distance (♻ Time Capsule) |
| Place Memories | Platsminnen | S | Derived per-pin chronicle: scenes, visitors, items left behind |
| The Cache | Gömman | S | Bury inventory items at exact map points; privacy-404 hides them (♻ Inventory) |

### 9.5 Lens: The Table (8)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| Character Sheets | Arket | L | GM-defined sheet templates per guild; `/roll 1d20+STR` reads the sheet; glass card pop-out |
| Initiative | Initiativet | M | Turn tracker pinned to the channel; active turn glows for all; "your turn" push |
| Quest Board | Uppdragstavlan | M | Pinned quests with lore links and status flow (rumor→active→done); system-message updates |
| Relics | Klenoden | M | Item rarity lustre, lore links, automatic provenance timeline (♻ Inventory) |
| The Forge | Smedjan | M | Player crafting proposals; GM approval consumes components, mints the item with provenance |
| Campfire | Lägerelden | M | Session-close ceremony: XP, milestones, scene/line-of-the-night nominations |
| Progress Clocks | Urtavlan | S | BitD-style segmented clocks; secret behind the GM screen until unveiled |
| Status Marks | Märkena | M | GM-applied condition runes (poisoned, blessed, hunted) glowing on messages, snapshotted |

### 9.6 Lens: The Writing Room (8)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| Marginalia | Marginalen | M | Margin notes on a line of prose without breaking the scene; export keeps or filters them |
| Relay Quill | Stafettpennan | S | Glowing whose-turn-to-write marker; composer lights up; your-turn push + app badge (♻ Relay Baton) |
| Writing Sprints | Skrivstugan | M | Booked focus sessions: countdown, word goals, ambience, closing session card |
| Golden Lines | Guldkornen | S | Guild quote book; typeset gold-framed cards; native-share PNG (♻ Quote Cards) |
| The Chronicle | Krönikan | M | The whole saga ordered in story time across channels; braided plotlines |
| Palimpsest | Palimpsesten | L | Edits preserved as layers; canon revisions need co-author approval with a visible seam |
| The Serial | Följetongen | M | Publish chapters; "New chapter!" push to subscribed readers; secret read link (♻ The Gallery) |
| Previously On | Sedan sist | M | Recap card at your unread line — rotating human narrator or Hypebot fill (♻ Hypebot) |

### 9.7 Lens: The Fellowship (8)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| The Threshold | Tröskeln | M | Newcomers read GM-marked key chapters before the quill unlocks; ceremonial entrance |
| The Gallery | Läktaren | M | Silent spectators via secret link; reader lanterns above the channel; applause waves |
| Guest Cameos | Gästspel | M | Friend cameos: own persona, one channel, one scene, guest-badged; access dies with the scene |
| Laurels | Lagerkransen | S | Three weekly awards per member; procedural SVG emblems land in the inventory |
| Crests | Vapenskölden | M | Heraldic crests derived from name + writing stats; pure client SVG algorithmics |
| The Hearth | Härden | M | Guild home flame fed by anyone's writing — a collective streak no one carries alone |
| The Salon | Salongen | M | Museum plaques per persona: portraits, debut, nominated quotes |
| Anniversaries | Minnesdagen | S | Nova DOT memory cards on story anniversaries; custom guild feast days |

### 9.8 Lens: The Cinema (8)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| End Credits | Eftertexterna | M | Chapter-close credit roll generated from real data; shareable |
| Keepsake Cards | Reliken | M | Holographic foil moment cards; gyro-driven shimmer; guild trophy cabinet |
| Leitmotifs | Ledmotivet | M | Per-persona signature melody synthesized deterministically from the name (♻ Entrances) |
| Soundboard | Ljudpulten | M | GM glass panel of synthesized stings (thunder, bell, blade) — bytes over SSE, sound on every phone |
| Entrances | Entrén | S | Persona entry flourishes — smoke, sparks, frost — played for everyone on enter/switch |
| Intertitles | Mellantexten | S | Silent-film time cards ("Three days later…"); chapter breaks in Story Book export (♻ Director Mode) |
| Haptic Pulse | Pulsen | S | A designed haptic language: dice rumble, whispers tick, GM tension beats; per-channel off switch |
| The Poster | Affischen | M | Compose the campaign's film poster (epic/noir/folk-tale templates) → PNG into the channel |

### 9.9 Lens: The Pocket World (8)
| Feature | Alias | Sz | One-line |
|---|---|---|---|
| Share Target | Inkastet | M | Share images/text from any app straight into a scene; channel + persona picker sheet |
| The Tintype | Plåten | M | In-composer camera with period filters (daguerreotype, polaroid), persona seal; client-side only |
| Table Beacon | Bordsbålet | M | Wake-locked second screen for table play: atmosphere, giant dice, writing embers |
| The Aether | Etern | S | Ambience as a Media Session track: scene art on the lock screen, headphone controls (♻ Atmospheres) |
| Relay Baton | Stafettpinnen | M | Your-turn push + glowing app badge; one tap lands you in the composer, persona worn (♻ Relay Quill) |
| Quote Cards | Citatkortet | S | Long-press → typeset glass card with persona signature → native share sheet (♻ Golden Lines) |
| The Herald | Härolden | M | Notifications voiced by the character, bundled per scene; per-persona rules; quiet hours |
| The Knapsack | Ränseln | M | Offline chapter packs via service worker; read position syncs home |

## 10. Invariants (unchanged; new compliance points)

All seven CLAUDE.md invariants stand. New surfaces comply by inheritance: SSE/vault/bot/atlas endpoints use the same `AuthAccount` extractor; DM/Cache/secret-clock/whisper visibility collapses to the identical privacy-404; all new queries are parameterized; media-derived features (icons, backdrops, posters, tintypes) ride the existing media pipeline; persona snapshots extend to mood, marks, and guest badges at send-time; account identity is the only live-resolved display data (deliberate, §3); bot posts are badged and never impersonate user personas.

## 11. Out of scope (this release)

Native Freya client redesign (must keep compiling; benefits from the shared API layer only) · light theme (meaning **color-scheme** theming — distinct from the structural skeleton themes of §1, which ARE in scope as of the 2026-06-12 amendment) · E2EE for DMs (vault-only) · CPU-driven NPCs / AI GM / any AI beyond Hypebot (→ Offensive Bias) · federation · the deleted `deploy/`/`scripts/`/`end2end/` tooling stays deleted.

## 12. Waves (for the implementation plan)

Foundations first — most of the catalogue rides four substrates: the SSE bus, the scene system, the atlas, and the inventory.

- **M1 Realtime backbone:** SSE bus + `/unread` + perf fixes (cache headers, MIME projection, lazy channels)
- **M2 Design system:** tokens, fonts, icons, SCSS foundation, appearance-tier scaffolding (`.fx-max`)
- **M3 Shell & navigation:** hybrid mobile nav, desktop reskin, directional bubbles
- **M4 Chat experience:** composer, STD wow effects, Message Effects, Constellation presence, Fate Engine, Ghost Quill
- **M5 Skelettvågen (amended 2026-06-12 — supersedes both the original "M5 Identity" here and the M4 plan footer's "M5 = Eye-candy tier" note):** the skeleton wave, one all-in-one mega-wave by owner ruling. Internal phase order:
  - **(0) Prerequisite cluster** — the ux-evolution prerequisites, mandatory before any skeleton work: motion doctrine + lint (rank 43), transform-free `.content` + body portal layer (54), the etched-glass material recipe (20), `content-visibility` rows (36), the HoloPanel gesture engine (49 — one engine; Omloppsbana's slide-over, Kortdäck's depth scrub, and Holoterminal's four panels are its consumer families), visual-haptics vocabulary (19).
  - **(1) Theme-switch infrastructure + ceremony machinery** — state construction and the mount Effect lifted above the skeleton branch in `AppShell`: a skeleton switch must never drop the SSE connection, composer draft, or selection state. This phase delivers the MACHINERY only (pref key, branch point, pref-less detection); the user-facing three-way ceremony content lands after phase (4) and is a phase-(7) gate item. The M3 shell remains in-tree as scaffolding to prove the switch, and mid-wave a pref-less dev build boots that scaffolding — an acknowledged in-wave-only exception to "no silent default" (the `sk-orbit` fallback becomes meaningful from phase (2) on).
  - **(2) Skeleton A — Omloppsbana** (built first by owner ruling). The swipe strip's feasibility tax is acknowledged up front: peek-never-marks-read discipline, SSE open-channel semantics for peeked neighbors, axis-lock arbitration (including swipe-right-to-reply vs the horizontal strip) tuned on real devices. Orbit-map entry by pill tap only (the pinch entry was judge-killed).
  - **(3) Skeleton B — Kortdäck.** The depth scrub's blur-recede is re-engineered compositor-cheap (no per-frame full-layer `filter`) BEFORE the aesthetic locks — the floor device cannot pay for live blur.
  - **(4) Skeleton C — Holoterminal.** The message-row parallax is ÖG-tier-only, scroll/CSS-var-driven, and paused on `document.hidden`; edge gestures negotiated against iOS back-swipe.
  - **(5) The #48 ÖG delta** (§1): ignition flip + settings diptych (60+63 merged) + nebula plates (37), exhibited per skeleton.
  - **(6) M3-shell retirement** — delete the M3 composition and its layout partials (`_layout`/`_rail`/`_sidebar`/`_nav`/`_mobile`); shared partials that consume insets today (`_content`, `_toast` — the toast rides `_nav`'s `--tabbar-h` contract) are rebound: each skeleton publishes its own composer/chrome anchor var and claims or cedes each inset edge explicitly. Safe-area rule, defined: for each inset edge (top/bottom/left/right) under each `.app.sk-*` root, exactly ONE rule in the computed layout chain applies the `env(safe-area-inset-*)` padding — verified by (a) a static audit mapping every `env(safe-area-inset-*)` site to one owner per skeleton and (b) the notched-device check (composer/toast/topbar flush, no double gap, no underlap) ×3 on the iPhone 13 mini.
  - **(7) The wave gate** (§13, per-skeleton).

  The M4 a11y backlog (sheet a11y, touch-AT message actions, radial focus management — previously memory-only) folds in as **per-skeleton requirements**: every dialog-like overlay that returns (slide-overs, flip-cards, hologram panels) reaches Modal-parity focus behavior (trap, Esc, restore-to-trigger), while continuous navigation states (Kortdäck's depth scrub) instead guarantee a keyboard/AT-operable equivalent path; and message actions are AT-reachable on touch with a testable shape — with VoiceOver on the real-device iOS pass (and TalkBack in the matrix sweep), focusing a message row exposes exactly the actions `message_actions(kind, mine)` grants, each activatable end-to-end, per skeleton; AT exposure derives from the predicate, never re-branched. Shared-layer ux-evolution items (whisper rework 8+13, dice console 12+15+31, fx-max ceremony 60+63, persona chip 1/21, Outbox 6 — schema landmine verified first, etc.) ride M5's shared phases where they fit, later waves otherwise. The catalogue's directives bind — the three kills by name: pinch orbit-entry (accidental-pinch hazard), the deck-plate transit stamp (ceremony tax on the highest-frequency action — no skeleton's arrival language may block the channel switch), and the Persona Orrery (leaks draft-derived magnitude to non-opted-in receivers, breaching the Ghost Quill BOTH-ways gate — binding on every skeleton's typing surface); plus the merge and feasibility directives.
- **M6 Identity:** guild icons + accent, account profiles, Nova DOT orb (the M4 warp-tint refinement awaits `guild.accent_color` landing here)
- **M7 DMs & fellowship:** DMs/groups, Guest Cameos, The Gallery, The Threshold, Laurels, Crests, The Hearth, The Salon, Anniversaries, Constellation Map
- **M8 Scenes & the Book:** scene/chapter system, Story Book + exports, Director Mode, Time Capsule, Performance Mode, End Credits, Intertitles, The Chronicle, Marginalia, Previously On (human), The Serial, Golden Lines/Quote Cards, Writing Sprints, Relay Quill/Baton, Palimpsest
- **M9 The World:** World Atlas, Presence Map, Realms + Place Memories, Fog of War, World Calendar, Weather Almanac, Journeys, The Post
- **M10 The Table:** Inventory + Relics + The Cache + The Forge, Character Sheets, Initiative, Progress Clocks, Status Marks, Quest Board, Campfire
- **M11 Vault & toolbox**
- **M12 Cinema & senses:** Atmospheres + ambience + The Aether, Soundboard, Leitmotifs + Entrances, Backdrops, Living Portraits, Haptic Pulse, Eye-candy tier completion per skeleton (shader nebula [H], holographic depth [F], keepsakes — distinct from M5's nebula plates and Holoterminal's scroll parallax, which already landed as the #48 delta backbone), The Poster
- **M13 Pocket world & QoL:** Share Target, The Herald, The Knapsack, The Tintype, Table Beacon, full mobile QoL pass
- **M14 Hypebot:** bot gateway, context assembler, llama.cpp integration on novahome, Previously-On fill, commentary triggers
- **M15 Consolidation & polish:** shared API layer, permission-gate helper, stub macros, Memory Core search, Living Lore, Session Calendar + Council Polls if not landed earlier, final a11y/perf audit

> Renumbering note (2026-06-12): waves after M4 shifted by one when Skelettvågen took the M5 slot. Memory/handoff references to "M13 Hypebot" and the like predate this amendment.

Dependency edges, test-first details, and per-wave schema definitions belong to the implementation plan. Wave order may interleave; M1+M2 must land first, and from the 2026-06-12 amendment **M5 is a hard predecessor of any later wave that touches UI** (the per-skeleton gates of §13 presuppose the three skeletons exist) — interleave freedom applies only to non-UI substrates (schema, server endpoints).

## 13. Verification

- The full existing integration suite passes (`cargo test --features ssr`, live local SurrealDB, per-worker namespaces; 144 at spec time, growing per wave — 293 as of the 2026-06-12 amendment); new integration tests per wave named in commit `Tests:` lines.
- Priority new suites: SSE delivery + membership filtering; `/unread`; DM privacy-404 + groups; vault opacity/authz/version conflict; `channel.guild` relaxation schema-apply guard; guild-icon permission gating; bot gateway rate limits + badge integrity; dice-server validation; scene/export correctness.
- `cargo fmt --all`; clippy clean on `ssr`, `hydrate` (wasm32), `freya`; `cargo build --bin authlyn-native --features freya` still succeeds.
- Playwright (headed, M2 dev machine): mobile-viewport flows (login → send → channel switch via the active skeleton's picker [orbit map / depth scrub / channel rig; M3 interim: the bottom sheet] → DM → vault unlock → atlas → roll) + desktop screenshots; WebKit needs the documented cookie injection. Headless/emulated smoke is NECESSARY but NOT SUFFICIENT — it never closes a wave on its own (next bullet).
- **Real-device iOS gate (binding, added 2026-06-12):** every wave's final verification task (the "Wn verification gate + visual smoke" convention the wave plans follow) must include a real-device iOS PWA pass on the owner's iPhone 13 mini (iOS 26.5) whenever the wave touches touch interaction. Origin (finding #54): the M4 headless smoke ran a Chromium Android profile and missed EVERY real iOS touch bug — the `-webkit-touch-callout` selection beating the radial long-press, `draggable` DnD hijacking taps, the `user-scalable` lightbox zoom. Headless browsers lie about exactly these surfaces. Geometry/DPI verification sweeps the friend-group device matrix per the UX-equality law: POCO C3 360×800 (performance floor), iPhone SE 2022 375×667 (shortest), iPhone 13 mini 375×812 (notch reference), Nothing Phone 2 412×892 (widest).
- **Per-skeleton gates (binding, added 2026-06-12):** from M5 on, every wave-gate item that touches the UI runs **per selectable skeleton** — visual smoke ×3, the real-device iOS pass ×3 when touch surfaces changed, the geometry/device-matrix sweep ×3. M5's own exit additionally proves the theme machinery itself: a skeleton switch never drops SSE/composer/selection state; the onboarding ceremony lands correctly on a pref-less device (new install AND post-update legacy device); exactly-once safe-area-inset ownership re-verified per skeleton on notched hardware; and the WASM bundle stays within a size budget (three shells compile into the single hydrate cdylib — the budget guards the production build, and a budget breach is a finding, not a footnote). Budget procedure: at phase-(0) start, `cargo leptos build --release` and record raw + gzip bytes of the `.wasm` under `target/site/pkg/` as the baseline; the M5 plan header records baseline and budget; the budget number itself is an owner sign-off at plan review; the phase-(7) gate re-measures with the same command and compares.
- **No prod experiments** — all work on branch `mendicant-bias`; push to `main` only after explicit owner approval. Post-migration, CLAUDE.md's deploy section must be retargeted to novahome before the first deploy lands there.
