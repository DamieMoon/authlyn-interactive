# 01 — System at a glance

`authlyn-interactive` is a self-hosted, server-trusted roleplay chat platform — the
Discord shape (guilds → channels, membership, friends, DMs) fused with the
SillyTavern shape (personas, lorebooks, dice). It is **one Rust crate** that
compiles into three mutually-exclusive artifacts from a single source tree:

- the **axum + Leptos-SSR server binary** (`authlyn-interactive`),
- the **Leptos-hydrate WASM bundle** that runs in the browser,
- the **`nova-mcp` MCP bridge binary** (optional).

Stack, dependency rationale, toolchain, and conventions are *not* restated here —
they live where they are authoritative:

- Stack + directory tree: [`../../README.md`](../../README.md).
- Every dependency's purpose + the per-graph feature wiring: the `#`-comments in
  [`../../Cargo.toml`](../../Cargo.toml) (dense and canonical).
- Toolchain probe, Bash allowlist, hooks: [`../../.claude/settings.json`](../../.claude/settings.json).
- Build / run / test / deploy invariants + footguns: [`../../CLAUDE.md`](../../CLAUDE.md).

This document covers only the cross-cutting shape: **the three feature graphs**,
**the crate map**, **the domain glossary**, and **the boot sequence**.

---

## 1. The three feature graphs

The crate has three Cargo features that are **disjoint at the binary level** —
each produces a different artifact, and code from one graph must **never**
cross-import code from another. The split is enforced by `cfg`-gating in
`src/lib.rs` and by the `[features]` table in `Cargo.toml`.

| Graph | Cargo feature | Artifact | Target | Owns (top-level modules) |
|-------|---------------|----------|--------|--------------------------|
| **ssr** | `ssr` | server bin `authlyn-interactive` | native | `db`, `server`, `storage` |
| **hydrate** | `hydrate` | WASM lib (`cdylib`) | `wasm32-unknown-unknown` | the browser side of `client`, `ui` |
| **nova** | `nova` | bin `nova-mcp` | native | `src/bin/nova-mcp.rs` only |
| **always-on** | (none) | linked into all three | must compile to wasm32 | `protocol`, `markup`, + the SSR/hydrate-shared shapes of `app`/`ui` |

### What compiles where — the `lib.rs` evidence

The gating is literal. `src/lib.rs` declares the always-on spine **ungated** and
the server graph **behind `cfg(feature = "ssr")`**:

```rust
pub mod app;          // shared root (renders under ssr, hydrates under hydrate)
pub mod client;       // browser REST client (hydrate-real, ssr-stub)
pub mod markup;       // always-on
pub mod protocol;     // always-on
pub mod ui;           // Leptos views (shared shapes; behavior hydrate-gated)

#[cfg(feature = "ssr")] pub mod db;       // SurrealDB connect/schema
#[cfg(feature = "ssr")] pub mod server;   // axum routes + state
#[cfg(feature = "ssr")] pub mod storage;  // schema.surql

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() { /* console_error_panic_hook + mount_body(App) */ }
```

`protocol` and `markup` carry **no** `cfg` — that absence is the proof they are
always-on. `db`/`server`/`storage` are each `#[cfg(feature = "ssr")]`, so the
WASM build never even parses them. The `hydrate()` entrypoint is the only
`#[wasm_bindgen]` export and exists only under `hydrate`.

`src/bin/nova-mcp.rs` is registered in `Cargo.toml` with
`required-features = ["nova"]`, so the default build and `cargo leptos` **never**
pull it into the app graph. The `nova` feature deliberately enables **zero**
leptos/surreal/app dependencies — it is a standalone HTTP→MCP bridge that talks
to the *running* authlyn API as the `Nova` account.

### Why cross-import is forbidden (not stylistic — it breaks the build)

Each graph pulls a dependency set the others cannot tolerate:

- **ssr** pulls `surrealdb`, `tokio` (multi-thread), `axum`, `argon2`, `image`,
  `web-push` — all native-only. None can compile to `wasm32`.
- **hydrate** pulls `gloo-net`, `gloo-storage`, `gloo-timers`, `web-sys`,
  `js-sys`, `wasm-bindgen` — all wasm-only. `getrandom` is forced to its `js`
  feature *only* on `cfg(target_arch = "wasm32")` so it can borrow the browser's
  `crypto.getRandomValues`.
- **nova** pulls `rmcp` + `reqwest` + a fresh tokio — a separate native process.

A single `use crate::server::…` from a `ui` module would drag `surrealdb`/`tokio`
into the WASM build and fail to compile for `wasm32`. The reverse (a `use
crate::client::…` reaching for `gloo-net` from the server) fails the native
build. The disjointness is therefore a **hard compile invariant**, not a
convention. Anything touching the always-on spine (`protocol.rs`, `markup/`)
must compile under **all three** graphs — "compiles under all three" is part of
"done" for those files.

The hydrate/ssr-shared modules (`app`, `ui`, `client`) resolve the seam with the
**dual-definition pattern**: an action/effect is defined *hydrate-real* and
*ssr-no-op* under opposite `cfg` arms, so a view can call it ungated while
`gloo-net` never reaches the ssr graph (see
[`07-ui-shell.md`](07-ui-shell.md) for the `act/` dispatch layer).

### The always-on spine

Only two modules are unconditionally in every graph, and both are **serde-only**
(no axum/surrealdb/tokio/gloo), so they compile to `wasm32`:

| Module | Role | Pin |
|--------|------|-----|
| `src/protocol.rs` (1165 LOC) | The single wire contract: ~84 request/response/summary/detail/envelope DTOs + the id-only `SyncEvent` SSE enum + `ErrorBody`. Shared **verbatim** by the server (emits) and the hydrate client (decodes). | `src/protocol.rs::tests::message_envelope_deserializes_without_persona_description_or_color` (wire back-compat); `tests/sync_events.rs::sync_event_serializes_with_snake_case_type_tags` |
| `src/markup/` | The chat markup engine: tokenize → tree → blocks, plus deterministic crest derivation. Same parse on server and client. | `tests/mentions.rs` (markup-driven mention extraction); see [`06-markup-engine.md`](06-markup-engine.md) |

`SyncEvent` is the load-bearing always-on shape for realtime: it carries **ids
only, never content**, and its wire tags are pinned by
`tests/sync_events.rs::sync_event_serializes_with_snake_case_type_tags`,
`::targeted_sync_events_pin_their_wire_shape`, and
`::reload_sync_event_is_a_bare_global_tag`.

See [`02-request-lifecycle.md`](02-request-lifecycle.md) for how a request
crosses the seam, and [`reference/rest-api.md`](../reference/rest-api.md) for the
full DTO/route matrix.

---

## 2. Crate map

The full per-module purpose lives in the sibling docs and in each file's `//!`
header. This is the orientation index (one line per node; counts from the
17-subsystem map are reliable).

```
src/
  lib.rs            module wiring + cfg-gating + the wasm `hydrate()` entrypoint
  main.rs           ssr-only axum boot (connect → schema → state → router → serve)
  app.rs            Leptos root: the HTML `shell()` + the routed `App` component
  protocol.rs       ALWAYS-ON wire DTOs + SyncEvent (serde-only, wasm-safe)
  db.rs             [ssr] SurrealDB connect + retry + apply_schema
  markup/           ALWAYS-ON chat markup engine
    mod.rs            module surface
    tokenize.rs       lexer
    tree.rs           AST
    blocks.rs         block-level assembly
    crest.rs          deterministic heraldic-crest derivation
  client/           [hydrate-real / ssr-stub] browser REST client
    mod.rs
    api.rs            ~60 DTO calls over gloo-net Fetch; lifts ErrorBody → ApiError
  storage/          [ssr] SurrealDB schema
    mod.rs            `pub const SCHEMA = include_str!("schema.surql")`
    (schema.surql)    the table/field/index/event definitions
  server/           [ssr] the whole axum server
    mod.rs            router assembly (api_router/make_router) + purge sweep
    state.rs          AppState (DB handle, media dir, SSE bus, ephemeral maps)
    errors.rs         typed 4xx/5xx → Json(ErrorBody) wrapper
    retry.rs          with_write_conflict_retry (racy CREATE → idempotent 409)
    access.rs         membership/authorization re-derivation
    permissions.rs    role/permission checks
    validate.rs       input validation
    db_helpers.rs     shared query helpers
    datetime.rs       fixed-9-digit RFC3339 row formatting (lex-orderable)
    accent.rs         deterministic accent-color derivation from icons
    auth/             session cookie identity (argon2id, SHA-256 token rows)
    guilds/           guilds + channels + membership + deletion + icon
    dms.rs            guild-less DM threads
    friends.rs        account-to-account friendships
    messages/         post/read/edit/delete/restore + rolling + typing + unread + read_state
    personas/         personas core + gallery + editors + per-channel wear
    cameos.rs         ephemeral guest channel access
    lorebook.rs       collaborative lorebook entries
    media.rs          encrypted blob store + on-the-fly cached JPEG thumbnails
    emoji.rs          per-guild custom emoji
    events.rs         GET /events SSE stream (id-only, per-frame session re-check)
    push.rs           VAPID Web Push sender
    system_messages.rs admin "Nova DOT" broadcasts
    feedback.rs       feedback intake
    dev_reload.rs     dev hot-reload SSE nudge (test deck auto-refresh)
  ui/               [shared shapes; behavior hydrate-gated] Leptos UI
    mod.rs, modal.rs, avatar.rs, crest.rs, icons.rs, accent.rs,
    clipboard.rs, inline_rename.rs, markup_view.rs, auth.rs   shared primitives
    emoji/{mod,data}.rs   custom + unicode emoji resolver/dataset
    shell/            the Discord-style authed app shell
      mod.rs, state.rs, account.rs, members.rs, friends.rs,
      lorebook.rs, wardrobe.rs, holopanel.rs, server.rs, toast.rs,
      emoji_manager.rs
      act/            the 28-fn action/dispatch layer (compose/edit/sync/…)
      channel/        message pane (list, composer, lightbox, radial, …)
      sk_orbit/       the sk-orbit spatial shell (orbit-map, strip, orb, warp)
  bin/
    nova-mcp.rs       [nova] standalone HTTP→MCP bridge (no app deps)
```

Per-area depth lives in the siblings:
[`03-data-model.md`](03-data-model.md) (storage),
[`04-realtime-sse.md`](04-realtime-sse.md) (events/push),
[`05-auth-privacy.md`](05-auth-privacy.md) (auth),
[`06-markup-engine.md`](06-markup-engine.md) (markup),
[`07-ui-shell.md`](07-ui-shell.md) (ui shell),
[`08-styling-chrome.md`](08-styling-chrome.md) (SCSS),
[`09-testing.md`](09-testing.md) (test harness),
[`10-nova-mcp.md`](10-nova-mcp.md) (nova),
[`11-build-deploy-pwa.md`](11-build-deploy-pwa.md) (build/deploy/PWA).

---

## 3. Domain glossary

The vocabulary a reader needs before any other doc makes sense. (Authoritative
shapes are the `protocol.rs` DTOs and the SurrealDB tables; see
[`03-data-model.md`](03-data-model.md).)

| Term | Meaning |
|------|---------|
| **Guild** | A server/community — the top-level container, Discord-style. Owns channels, roles, membership, custom emoji, and an icon. Soft-deletable with a 30-day rollback window. |
| **Channel** | A message thread inside a guild. Also the shape used for a **DM** (a `kind='dm'` channel with no guild). Soft-deletable with a 1-day rollback window. |
| **DM** | A guild-less direct-message thread between accounts — modeled as a channel whose `guild` is `NONE`, with `dm_member` rows and a `dm_pair` uniqueness lock for the 1:1 case. |
| **Friend** | An account-to-account friendship edge, independent of any guild. |
| **Persona** | An account-global roleplay identity (name, avatar, description, accent, gallery) the user can **wear** per-channel. The worn persona's identity is stamped onto each message at post time (SillyTavern model). |
| **Wear** | The per-channel binding of a persona to a user (`channel_active_persona`). The currently-worn persona determines the author identity on new messages in that channel. |
| **Cameo / guest** | An **ephemeral, scoped** grant of channel access to a guest (`channel_guest`), optionally time-boxed (`expires_at`). Lets a non-member appear in one channel without joining the guild. |
| **Lorebook** | Collaborative SillyTavern-style world/lore entries scoped to a channel (`lorebook_entry`), editable by permitted members. |
| **Accent** | A deterministic accent-color **derived server-side** from a guild icon or persona avatar (e.g. a red icon → `"red"`), mapped to CSS tokens client-side. Not user-picked. |
| **Nova** | Two things sharing a name. (1) The **`Nova` system account** — seeded into the DB, cannot log in, and is the author of admin "DOT" broadcast system messages. (2) The **`nova-mcp` bridge** — a standalone binary (`nova` feature) that authenticates to the running HTTP API *as* the Nova account and exposes the platform over MCP. |
| **Cameo vs. friend vs. member** | Three distinct access grades: a **member** belongs to a guild; a **friend** is a peer edge; a **guest/cameo** is a per-channel, possibly-expiring grant. Authorization is re-derived from these per mutation — see below. |
| **sk-orbit** | The shipping spatial UI shell (orbit-map picker, swipe strip, compose orb). The product's actual chrome; see [`07-ui-shell.md`](07-ui-shell.md). |

**Server-trusted identity (the platform's defining rule).** Identity comes from
the **session cookie only** — never from a client-supplied id. Authorization is
**re-derived per mutation** from membership/cameo/friend state, and
**non-membership is a 404, not a 403** (privacy: a non-member cannot even learn a
resource exists). Pinned by `tests/guilds.rs::nonmember_get_guild_is_404`,
`tests/personas.rs::batch_gallery_non_owner_non_editor_is_privacy_404`, and
`tests/soft_delete.rs::restore_collapses_to_privacy_404_for_non_members_and_unknown_channels`.
Full treatment in [`05-auth-privacy.md`](05-auth-privacy.md).

---

## 4. Boot sequence

The server entrypoint is `src/main.rs`, gated `#[cfg(feature = "ssr")]` with a
`#[tokio::main]` async `main` (the `#[cfg(not(feature = "ssr"))]` `main` is an
empty stub so the lib target links). The non-obvious ordering — **connect and
apply schema before binding the listener** — is deliberate: a schema-apply
failure must crash *before* the process accepts traffic.

Step by step, as written in `main.rs`:

1. **Init tracing.** `tracing_subscriber::fmt()` with env filter
   (`info,authlyn_interactive=debug` default) → stdout/journald. Without it,
   500s would have no server-side cause.
2. **Read Leptos config** via `get_configuration(None)`; extract `site_addr` and
   `leptos_options`.
3. **Connect to SurrealDB with retries** — `db::connect_with_retries()`
   (`src/db.rs:83`): 10 attempts, 500 ms backoff, wrapping `db::connect()` which
   signs in as root and selects ns/db from env (`SURREAL_*`, defaulting
   `authlyn`/`dev`). Failure `expect()`-panics with the start-the-DB hint. The
   retry helper itself is unit-pinned by
   `src/db.rs::tests::retry_succeeds_after_transient_failures` and
   `::retry_returns_last_error_on_exhaustion`.
4. **Apply the schema** — `db::apply_schema()` (`src/db.rs:53`) runs
   `storage::SCHEMA` (= `include_str!("schema.surql")`) and `.check()`s it.
   **This is the crash-loop surface:** a non-idempotent migration over a
   populated DB fails here and the boot dies. The schema's idempotency over a
   prod-shaped populated DB is pinned by
   `tests/schema_apply.rs::applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent`
   (plus the per-field backfill tests in that file). Migration footguns
   (`option<>`-or-backfill, `DEFINE FIELD OVERWRITE` for widened ASSERTs) are in
   [`03-data-model.md`](03-data-model.md).
5. **Ensure the media dir.** `MEDIA_STORAGE_DIR` (default `./media`) is
   `create_dir_all`'d; `AppState` later **canonicalizes** it (the GET
   path-traversal guard depends on a canonical root — see `state.rs:74`).
6. **Build the Web Push sender** — `server::push::PushSender::from_env()`
   (`src/server/push.rs:74`): `None` when VAPID env is unset, which makes every
   push path a silent no-op (the app runs fine without push). Logged either way.
7. **Construct `AppState`** — `AppState::with_leptos(surreal, leptos_options,
   media_dir, push)` (`src/server/state.rs:185`). `AppState` (`state.rs:60`) is
   the single `Clone`-cheap object every handler receives:

   | Field | Purpose |
   |-------|---------|
   | `leptos: LeptosOptions` | render config (`FromRef` lets Leptos route helpers work) |
   | `db: Arc<Surreal<Client>>` | shared DB connection (clone = refcount bump) |
   | `media_dir: Arc<PathBuf>` | **canonicalized** attachment root |
   | `push: Option<Arc<PushSender>>` | VAPID sender, or `None` = push off |
   | `typing: Arc<Mutex<…>>` | ephemeral "is typing" map (never the DB; never held across `.await`) |
   | `typing_drafts: Arc<Mutex<…>>` | Ghost-Quill live-draft text (never the DB; never the SSE bus) |
   | `draft_ttl`, `sse_recheck_period` | `Copy` timings, test-injectable before the router clone |
   | `events: broadcast::Sender<BusEvent>` | the **process-wide id-only SSE bus** (capacity 256; laggards resync) |

   Mutation handlers fan out through `AppState::emit` / `AppState::emit_for`
   (`state.rs:165`/`176`) — best-effort, never failing the request. See
   [`04-realtime-sse.md`](04-realtime-sse.md).
8. **Spawn the purge sweep** — `server::spawn_purge_sweep(state.clone())`
   (`src/server/mod.rs:445`): a detached tokio task running
   `purge_soft_deleted` (`mod.rs:366`) on a 3600 s interval (first tick is
   immediate). It hard-deletes soft-deleted rows past their window — **message
   1h, channel 1d, guild 30d** — cascading a purged channel/guild to its child
   tables (messages, lorebook entries, active-persona, read-state, dm_member,
   dm_pair, channel_guest, custom_emoji, guild_member). Idempotent, safe on an
   interval. Pinned by `tests/soft_delete.rs::purge_hard_deletes_message_past_window_only`,
   `::purge_cascades_guild_to_all_child_tables`,
   `::purge_should_cascade_guild_member_rows`, and
   `::purge_should_cascade_dm_member_rows`. (The cascade deletes use inline
   subqueries instead of `LET` vars to dodge a SurrealDB 3.1.0-beta.3
   composite-index DELETE mis-plan — see the comments in `mod.rs`.)
9. **Build the merged router.** `server::api_router()` (the app's HTTP API,
   `mod.rs:358`) is `.leptos_routes(…)`'d with the SSR route list from
   `generate_route_list(App)`, given a `file_and_error_handler` fallback (serves
   the `shell`), and `.with_state(state)`. The app routes are assembled by
   `api_routes()` (`mod.rs`), which `Router::new()` + `.merge(small_body_routes())`
   + `.merge(media_routes())` (media is a separate router so its body-size limit
   differs). Tests drive the same routes via `make_router(state)` →
   `tower::ServiceExt::oneshot` with **no port bind** (`mod.rs:352`); see
   [`09-testing.md`](09-testing.md).
10. **Bind + serve.** `TcpListener::bind(site_addr)` then `axum::serve`.

The Leptos side of the boot is `src/app.rs`: `shell(LeptosOptions)` emits the
full `<html>` document (PWA manifest, theme-color, apple-touch icons,
viewport zoom-lock with `viewport-fit=cover`, hydration scripts, the
`register-sw.js` service-worker bootstrap), and the `App` component provides
`AuthCtx` (resolved once on mount via `/auth/me` under
`#[cfg(feature = "hydrate")]`) and routes `/login`, `/register`, `/` (→ `Home`).
PWA/service-worker detail is in [`11-build-deploy-pwa.md`](11-build-deploy-pwa.md).

---

## Source map

Key files:

- `src/lib.rs` — module wiring; the `cfg`-gating that defines the three graphs + the wasm `hydrate()` export.
- `src/main.rs` — ssr-only axum entrypoint; the connect → schema → state → sweep → merged-router → serve boot order.
- `src/app.rs` — Leptos root: the HTML `shell()` document + the routed `App` component (`AuthCtx`, `/login` `/register` `/`).
- `src/db.rs` — SurrealDB `connect`/`connect_with_retries`/`apply_schema`.
- `src/protocol.rs` — the always-on serde-only wire contract (~84 DTOs + `SyncEvent` + `ErrorBody`).
- `src/storage/mod.rs` — `SCHEMA = include_str!("schema.surql")`.
- `src/server/state.rs` — `AppState` (the per-handler state object) + `emit`/`emit_for`.
- `src/server/mod.rs` — `api_router`/`make_router`/`api_routes` router assembly + `purge_soft_deleted`/`spawn_purge_sweep`.
- `Cargo.toml` `[features]` + `#`-comments — the canonical per-graph crate membership and dependency rationale.
- `README.md` — stack + directory tree.

Tests that pin this doc's claims:

- `src/protocol.rs::tests::message_envelope_deserializes_without_persona_description_or_color` — always-on wire back-compat.
- `tests/sync_events.rs::sync_event_serializes_with_snake_case_type_tags`, `::targeted_sync_events_pin_their_wire_shape`, `::reload_sync_event_is_a_bare_global_tag` — the id-only `SyncEvent` wire shape.
- `src/db.rs::tests::retry_succeeds_after_transient_failures`, `::retry_returns_last_error_on_exhaustion` — the connect-retry helper.
- `tests/schema_apply.rs::applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent` — schema-apply is crash-free + idempotent on boot.
- `tests/schema_apply.rs::nova_dot_system_account_is_seeded_and_cannot_log_in` — the Nova system account seed.
- `tests/soft_delete.rs::purge_hard_deletes_message_past_window_only`, `::purge_cascades_guild_to_all_child_tables`, `::purge_should_cascade_guild_member_rows`, `::purge_should_cascade_dm_member_rows` — the purge sweep windows + cascade.
- `tests/guilds.rs::nonmember_get_guild_is_404`, `tests/personas.rs::batch_gallery_non_owner_non_editor_is_privacy_404`, `tests/soft_delete.rs::restore_collapses_to_privacy_404_for_non_members_and_unknown_channels` — server-trusted identity + privacy-404.
