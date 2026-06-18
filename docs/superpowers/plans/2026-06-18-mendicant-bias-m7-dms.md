# M7 / P1 — Direct Messages (core)

## Context

M7 = the spec's **W7 "DMs & Fellowship"** wave (identity-map W#≡M#; W6 Identity shipped as M6). The full wave is DMs/groups + the 8-feature Fellowship lens (§9.7) + Constellation Map — too large for one plan. **Owner ruling: this plan is scoped to DMs-core as P1**, the hard architectural predecessor; the Fellowship features get their own later plans.

Why DMs first: they are the one piece that changes the channel/membership *substrate* — `channel.guild` stops being mandatory and a second membership model appears. Everything the Fellowship features touch (channels, messages, presence, push) sits on top of that substrate, and the spec makes **W5 + the DM substrate** the precondition for later UI waves.

**The decisive finding from exploration:** message/read-state/active-persona routes are already **channel-scoped** (`/channels/{cid}/messages`, `/channels/{cid}/mark-read`, `/channels/{cid}/active-persona` — `src/server/mod.rs:135-149`), *not* guild-scoped. A DM thread is just a `channel` with `kind='dm'` and `guild=NONE`. So the entire message/compose/cursor/soft-delete/attachment/persona/SSE/unread/push stack is **inherited verbatim** once two shared resolvers branch on `kind`. DMs need only a small *lifecycle* surface (`/dms` + a `dm_member` table) — there is **no parallel message API**.

Outcome: a working, demoable DM feature (1:1 + groups) on the deck, so the owner can make the demo-driven UX calls (orbit placement, visuals) at deck-pass — the M6 pattern. (See memory `ux-choices-demo-driven`.)

---

## Design

### T1 — Schema + migration guards (`src/storage/schema.surql`, `tests/schema_apply.rs`)

1. **Widen `channel.guild`** `record<guild>` → `option<record<guild>>` using **`DEFINE FIELD OVERWRITE`** (not `IF NOT EXISTS`). This is the M6 `message.kind` lesson exactly: the field already exists, so `IF NOT EXISTS` is a no-op that keeps the strict `record<guild>` and makes `CREATE channel SET guild = NONE` (every DM) fail. Existing rows all hold a guild → valid for `option<record>`, no backfill, no coercion crash.
2. **Widen `channel.kind`** ASSERT `['text','lorebook']` → `['text','lorebook','dm']`, also via **`DEFINE FIELD OVERWRITE`** (widening an enum ASSERT needs OVERWRITE — `IF NOT EXISTS` silently keeps the narrow ASSERT and rejects `'dm'`). Keep `DEFAULT 'text'`.
3. **New `dm_member` table** (parallel to `guild_member`, `schema.surql:144-159`):
   ```
   DEFINE TABLE IF NOT EXISTS dm_member SCHEMAFULL;
   DEFINE FIELD  ... channel   ON dm_member TYPE record<channel>;
   DEFINE FIELD  ... account   ON dm_member TYPE record<account>;
   DEFINE FIELD  ... joined_at ON dm_member TYPE datetime DEFAULT time::now();
   DEFINE INDEX  ... dm_member_pair    ON dm_member FIELDS channel, account UNIQUE;
   DEFINE INDEX  ... dm_member_account ON dm_member FIELDS account;   -- M-37 lesson: account-only lookup on every /events connect + /unread
   ```
   No `active_persona` field — DM persona-wear rides the existing `channel_active_persona` table like every channel. The optional group **title lives in `channel.name`** (empty for 1:1).
4. **Guards** (mirror the existing over-populated patterns in `tests/schema_apply.rs`):
   - `channel.guild` widening over a populated channel table: old schema with strict `guild record<guild>` + narrow kind ASSERT, seed a populated `kind='text'` channel with a guild, apply real `storage::SCHEMA`, assert: no crash; legacy channel keeps its guild + kind; **a new `CREATE channel SET guild=NONE, kind='dm'` is now accepted** (this is the assertion that fails if OVERWRITE is missing — the same bite-proof discipline as the M6 security-purge guard `56376e7`).
   - `dm_member_account` index applies over populated `dm_member` rows (mirror `new_guild_member_account_index_applies_over_populated_rows`).

### T2 — Shared resolvers branch on kind (`src/server/access.rs`)

- **`resolve_membership`** (`access.rs:47-99`): the channel-resolve query selects `meta::id(guild)` — NONE for a DM, which won't deserialize into the current `ChanRow.guild_key: String`. Change to **`guild_key: Option<String>`** and branch the membership query on `kind`:
  - `kind == 'dm'` → `SELECT true AS member FROM dm_member WHERE channel = type::record('channel',$cid) AND account = ...`
  - else → existing `guild_member` check (unchanged).
  - Soft-delete: for DMs the `guild.deleted_at = NONE` clause is vacuously true (NONE), but resolve DMs on `channel.deleted_at` only — branch the resolve SQL by kind to keep it explicit. The `Membership` enum contract (3 outcomes, all → privacy-404) is **unchanged**, so guild callers don't change.
- **`visible_channels`** (`access.rs:116-134`): add a DM arm — channels where `kind='dm' AND deleted_at=NONE AND id IN (dm_member of $account)`. `VisibleChannel.guild_id` becomes **`Option<String>`** (DMs have no guild). Consumers to update: `events.rs` SSE filter (keys on `channel_id` only — guild_id unused, safe), `unread.rs` aggregation, and the wire DTO `ChannelUnread.guild_id` → `Option<String>` (client routes a None-guild unread row to the DM list badge).

### T3 — Protocol DTOs (`src/protocol.rs`, always-on / wasm-safe, serde-only)

- `CreateDmRequest { members: Vec<String>, title: Option<String> }`, `DmSummary { id, title: Option<String>, kind:"dm", members: Vec<DmMemberSummary> }`, `DmMemberSummary { account_id, username, display_name, avatar_id }`, `ListDmsResponse { dms: Vec<DmSummary> }`, `InviteToDmRequest { account_id }`. Follow the suffix conventions; reuse `ChannelSummary.kind` (already a string → `'dm'` is wire-compatible).
- `ChannelUnread.guild_id: String → Option<String>` (T2 ripple).
- No new `SyncEvent` variant required: account-targeted **`ListsChanged`** already forces the DM-list refetch + `/events` visibility reload (`events.rs` reloads `visible` on `ListsChanged`). (A dedicated `DmCreated` is optional, not needed for correctness — keep minimal.)

### T4 — DM lifecycle module (`src/server/dms/`, new — mirror `src/server/guilds/`)

Routes (registered in `src/server/mod.rs`; `verb_noun`, static-over-dynamic):
- `POST /dms` → **`create_dm`**: `AuthAccount` creator; validate members non-empty + each an **accepted friend** of the creator (friend-gate, see §friendship query in `friends.rs`); member cap (define, e.g. ≤ 16). Create `channel{guild:NONE, kind:'dm', name:title??''}` + a `dm_member` row per member (creator + invitees), all under **`with_write_conflict_retry`** (idempotent 409). **1:1 dedup:** if exactly 2 total members, return the existing 1:1 DM (a `kind='dm'` channel whose `dm_member` set is exactly those two) instead of creating a duplicate; groups are never deduped. `emit_for(member_accounts, ListsChanged)`. Returns `DmSummary`.
- `GET /dms` → **`list_dms`**: all `kind='dm'` channels where caller ∈ `dm_member`, with members + title.
- `POST /dms/{tid}/members` → **`invite_to_dm`**: any member invites an accepted friend *of the inviter*; add `dm_member`; `emit_for(all, ListsChanged)`. Non-member caller → privacy-404 (via `resolve_membership`).
- `DELETE /dms/{tid}/members/me` → **`leave_dm`**: remove caller's `dm_member`; `emit_for(remaining+leaver, ListsChanged)`. **When membership hits 0, soft-delete the channel** (`deleted_at`) so the existing purge sweep reclaims it.
- **Messages / read-state / active-persona: no new routes** — `/channels/{cid}/messages` etc. work the moment T2 lands.
- **Push** (`src/server/push.rs`): branch the recipient query on channel kind — for `kind='dm'`, recipients = `push_subscription` of `dm_member` (not `guild_member`), minus author; `load_notification_info`'s `channel.guild` is NONE for DMs (make `guild_key` optional; title falls back to sender display name).
- **Mentions** (`resolve_mentions`, posting path): branch to the `dm_member` set for `kind='dm'` so `@ping` resolves in groups (currently guild-members-only).

### T5 — Client wiring (`src/client/api.rs`, `src/ui/shell/`)

- `api.rs`: `list_dms`, `create_dm(members, title)`, `invite_to_dm(tid, account)`, `leave_dm(tid)` (reuse `get`/`post_json`/`delete` transport). **Messages reuse the existing channel-scoped client fns verbatim** — a DM thread id *is* a channel id (the payoff of channel-scoped routes).
- `state.rs`: add `dms: RwSignal<Vec<DmSummary>>`; opening a DM sets the active channel through the **existing `sel_channel` + `ChannelPane`** path (a DM thread is a channel), minimizing new state.
- `act/dm.rs` (new, mirror `act/guild.rs`): `open_dm`, `create_dm_thread`, `invite_to_dm`, `leave_dm`, `refresh_dms`; re-export from `act/mod.rs`. ssr no-op stubs per the disjoint-graph convention.
- `act/sync.rs`: on `ListsChanged`, also `refresh_dms`; DM unread (guild_id None) routes to the DM-list badge.

### T6 — Orbit UI: demo-grade placement (`src/ui/shell/sk_orbit/`, `src/ui/shell/`)

Build the **cheapest demoable placement** so the owner can judge on the deck: a **"Direktmeddelanden" entry in the right-edge station slide-over** opening a DM thread list; tapping a thread opens it in the shared `ChannelPane`. Group create/invite uses a **friend-picker modal** (reuse `src/ui/modal.rs::Modal` + the friends list already in `src/ui/shell/friends.rs`). Every new control meets the **≥44px** floor at its base definition. **No encryption affordance** anywhere on DM UI (spec: DMs are server-readable, the UI must not imply E2EE).

> This placement is the **demo vehicle, not a locked decision.** The three structural directions (station list / DM-node-in-map / separate DM layer) remain open for the owner's deck-pass; the function is placement-agnostic because threads render through `ChannelPane` regardless.

### T7 — Tests (`tests/dms.rs` new; `tests/schema_apply.rs` from T1)

`#[tokio::test]`, full-sentence names, `tests/common/mod.rs` harness. Cover: create 1:1 + group; friend-gate rejection (inviting a non-friend); **privacy-404 byte-parity** for non-member list/post/invite/leave (pin against the guild privacy-404 body, as `tests/messages.rs` does); 1:1 dedup (same pair twice → same thread); message round-trip through `/channels/{cid}/messages` on a DM; persona-per-channel in a DM; DM unread; `ListsChanged` emitted on create/invite/leave; **leave-last-member soft-deletes** the thread. Existing `guilds.rs`/`messages.rs`/`auth.rs` suites must stay green (the `access.rs` branch must not regress guild membership).

---

## Critical files

- **Schema:** `src/storage/schema.surql` (channel.guild/kind OVERWRITE, dm_member); guards in `tests/schema_apply.rs`.
- **Resolvers (load-bearing):** `src/server/access.rs` (`resolve_membership`, `visible_channels`) — the single branch point that makes the whole inherited stack work for DMs.
- **New module:** `src/server/dms/` + route registration in `src/server/mod.rs`.
- **Inherited, branch-only:** `src/server/push.rs` (recipient query), the mention resolver in the `src/server/messages/` posting path.
- **DTOs:** `src/protocol.rs` (must compile under all three graphs — serde-only).
- **Client:** `src/client/api.rs`, `src/ui/shell/state.rs`, `src/ui/shell/act/{mod,dm,sync}.rs`, `src/ui/shell/sk_orbit/`, reuse `src/ui/modal.rs` + `src/ui/shell/friends.rs` + `src/ui/shell/channel/`.

## Reuse (do not reinvent)

`MSG_PROJECTION` + `post_message`/`persist_message` + soft-delete + `with_write_conflict_retry` + `GET /unread` + `channel_active_persona` + the SSE bus (`emit_for`/`ListsChanged`/per-frame session recheck) + the `friendship` accepted-friends query (`src/server/friends.rs`) + `ChannelPane`/composer + `Modal` + `FriendsPane`'s friend list — all inherited or reused; the net-new code is the `dm_member` model, the `/dms` lifecycle handlers, and the demo-grade DM list/picker UI.

## Demo-driven / owner-oracle decisions (deferred to deck-pass — not blocking P1)

- Orbit placement of DMs (the three directions).
- DM list/thread visual treatment, group-avatar/title rendering, the friend-picker modal styling.
- Policy revisitable at demo: friend-gate strictness (default: accepted-friend required for 1:1 + invites); who may invite to a group (default: any member, of their own friend); member cap.

## Conventions / invariants upheld

Commit-per-task, trailer `(M7/P1)`, gate per task. Server-trusted + **privacy-404** (non-membership → 404), SSE **id-only**, write-conflict **409-never-500**, schema **OVERWRITE** for widenings, `protocol.rs` compiles to wasm, disjoint feature graphs (dms = ssr-only), **44px** touch floor, DMs **not E2EE** (no encryption UI).

## Verification

1. `cargo test --features ssr` (live SurrealDB) — **0 failed**, incl. new `tests/dms.rs` + the T1 `schema_apply` guards + all existing suites.
2. `/check` — `cargo fmt --all --check` → clippy ssr → clippy hydrate-wasm (`-D warnings`).
3. `cargo build --release --bin nova-mcp --features nova` (protocol.rs touched → all three graphs compile).
4. Throwaway-namespace prod-SurrealDB shape gate before any deploy (the `channel.guild` widening over the populated prod table is the highest-risk migration — verify boot-apply on prod-shaped data; the boot-log watch is the live net).
5. **Demo on the deck** (`/test-deploy` → novahome `https://192.168.0.239:3434`, WebKit/iPhone over HTTPS with the dev root CA — **not** Chromium) for the owner's deck-pass on the demo-driven UX decisions above.
