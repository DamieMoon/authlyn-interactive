# 09 — Testing: the canonical source of truth

`docs/ARCHITECTURE.md` was deleted because its numbered invariant catalogue rotted into stale, unanchored prose. The surviving source of truth for this crate's behaviour is **the integration tests in `tests/*.rs` and the code they drive**. This document is the *index* to that source of truth — it does not restate the invariants in prose that can drift; it points at the executable pin for each one. When this doc and a test disagree, the test wins. Read the test.

> Cross-links: [request lifecycle](02-request-lifecycle.md) · [data model & schema](03-data-model.md) · [realtime SSE](04-realtime-sse.md) · [auth & privacy](05-auth-privacy.md) · [styling & chrome](08-styling-chrome.md) · [build/deploy/PWA](11-build-deploy-pwa.md) · [REST reference](../reference/rest-api.md) · [conventions](../reference/conventions.md). For the exact toolchain probe, the Bash allowlist, and the `PostToolUse` rustfmt hook, see [`.claude/settings.json`](../../.claude/settings.json); for the dev-DB / run / test invocations, see [`CLAUDE.md`](../../CLAUDE.md) and [`README.md`](../../README.md).

## The test contract

| Property | Value |
|---|---|
| Invocation | `cargo test --features ssr` |
| Hard prerequisite | a **live SurrealDB** on `ws://127.0.0.1:8000` (ns `authlyn` / db `dev`) — start with `surreal start --user root --pass root --bind 127.0.0.1:8000 memory` or `/dev-db` |
| Suites | **28** files in `tests/*.rs` (`tests/common/` is a shared module, not a suite) |
| Test functions | **313** `#[test]` / `#[tokio::test]` (verified: `grep -rhE '#\[(tokio::)?test' tests/*.rs \| wc -l`) |
| `#[ignore]` | **0** — there is no quarantined test; green means every test ran |
| Gate | **0 failed**. Any failure fails the gate. |
| SurrealDB pin | SDK `=3.1.0-beta.3`; the on-machine `surreal` CLI must share the **3.x** major (write-conflict / unique error texts diverge otherwise — see [retry_canary](#retry_canary--live-db-error-text)) |

### Why **both** `--features ssr` AND a live DB are mandatory

- **`--features ssr`** — every suite except the three static/wire suites carries `#![cfg(feature = "ssr")]` (or per-fn `#[cfg(feature = "ssr")]`). Without the flag those suites compile to *nothing* and silently pass. The crate's server surface (`make_router`, `AppState`, `server::retry`, …) only exists in the ssr graph. See [disjoint feature graphs](01-overview.md) and the `[features]` `#`-comments in [`Cargo.toml`](../../Cargo.toml).
- **A live DB** — the harness opens a real `Surreal<Client>` over WebSocket (`tests/common/mod.rs:145`). There is no embedded/mock engine. The DB is load-bearing not just for storage assertions but for the two *canary* classes that exist precisely to pin **live** SurrealDB behaviour: the error-string matchers ([retry_canary](#retry_canary--live-db-error-text)) and the schema migration path ([schema_apply](#schema_apply--prod-shaped-migration--idempotent-boot)).

The pre-commit hook (`.githooks/pre-commit`) runs the fmt+clippy subset (`/check`) **plus** the one static suite that needs no DB: `cargo test --features ssr --test style_lint`. The full correctness gate (`cargo test --features ssr`) is the owner/CI step and requires the DB.

## The shared harness — `tests/common/mod.rs`

`tests/common/mod.rs` is brought into each suite with `mod common;`. It is a **module, not a test binary**: a `mod.rs` inside a sub-directory is not compiled as its own test target, so nothing double-runs (`tests/common/mod.rs:5-8`). It carries `#![cfg(feature = "ssr")]` and `#![allow(dead_code)]` (a given suite uses only a subset of the exports).

### `Arena` — one isolated test world

```rust
pub struct Arena {
    pub router: Router,        // axum Router wired against `state`
    pub db: Surreal<Client>,   // direct handle for post-condition queries
    pub media_dir: PathBuf,    // per-arena media tempdir
    pub state: AppState,       // the SAME AppState the router was built from
}
```

Construction (`common::arena()`, `tests/common/mod.rs:62`):

1. `test_db()` opens a fresh connection on a **globally unique namespace** (below) and applies `storage::SCHEMA` + `.check()` (`tests/common/mod.rs:122-130`).
2. `test_media_dir()` makes a tempdir `authlyn-test-media-<random_id()>` (`tests/common/mod.rs:116-120`).
3. `AppState::new(db, media_dir)` is built, **then** `make_router(state.clone())` wires the router.

**No socket is bound.** Requests go through `router.clone().oneshot(req)` via `tower::ServiceExt` — the router is exercised as a `Service`, in-process. This is why there is no port, no base URL, and no flakiness from a real listener.

#### The build-state-before-`make_router` rule (load-bearing)

`AppState` carries `Copy` tuning fields (`draft_ttl`, `sse_recheck_period`) consumed by the SSE / typing-draft tasks. They MUST be set **before** `make_router` clones the state — once cloned, the router holds its own copy. Two dedicated constructors exist solely to honour that ordering rather than mutating `Arena::state` afterwards:

| Constructor | Overrides | Why | Source |
|---|---|---|---|
| `arena_with_draft_ttl(Duration)` | Ghost-Quill typing-draft TTL (prod 8 s) | run prune tests in ms, not by sleeping | `tests/common/mod.rs:80` |
| `arena_with_sse_recheck_period(Duration)` | SSE quiet-stream session re-check (prod 30 s) | run the quiet-revocation tests in ms | `tests/common/mod.rs:98` |

#### `Arena::state` vs a fresh `AppState` (load-bearing)

Some invariants live on ssr core fns that no HTTP route can reach (a stray bus frame, an admin broadcast, a dev-reload nudge). Those tests call the core fn **with `Arena::state`** and assert on SSE delivery. A freshly constructed `AppState::new(...)` carries its **own broadcast channel**, so an emission on it would never reach the router's `GET /events` subscribers — the assertion would pass vacuously (`tests/common/mod.rs:54-59`). Always emit on `Arena::state`, never on a new `AppState`.

### Per-worker isolation: namespace + media tempdir

`cargo test` runs suites and tests in parallel. Each `arena()` must not see another worker's rows. Two isolation axes:

- **Namespace** = `test_{pid}_{seq}_{random_id()}` (`tests/common/mod.rs:155-167`), database name equal to the namespace.
- **Media tempdir** = `authlyn-test-media-{random_id()}` (`tests/common/mod.rs:116-120`).

`random_id()` (`tests/common/mod.rs:174`) is 16 bytes from `rand::thread_rng()`, hex-encoded.

**Why `pid+seq` alone is not collision-proof** (documented at `tests/common/mod.rs:157-164`): the dev SurrealDB is a *long-lived in-memory* server that accumulates every test namespace across runs, and the OS recycles PIDs. A later run that reuses an old PID with the same low `seq` lands on a **prior** run's namespace and inherits its rows — observed as a flaky `409 "username already taken"` on fixed-username tests (e.g. lorebook's `"Owner"`). Mixing in `random_id()` (the same 16-byte entropy source the media tempdir already uses) makes each namespace globally unique regardless of PID reuse or DB accumulation. *This is the subtlest correctness dependency in the whole harness — pinned by the comment + rationale at the cited lines, not by an assertion.*

### `test_db()` vs `raw_db()`

| Helper | Schema applied? | Use | Source |
|---|---|---|---|
| `test_db()` | yes — `storage::SCHEMA` + `.check()` | every normal suite (via `arena()`) | `tests/common/mod.rs:122` |
| `raw_db()` | **no** — bare isolated namespace | migration tests that define an *old* schema, seed prod-shaped rows, then re-apply `storage::SCHEMA` exactly as boot does | `tests/common/mod.rs:136` |

Connection env overrides: `SURREAL_URL` / `SURREAL_USER` / `SURREAL_PASS` (default `127.0.0.1:8000` / `root` / `root`), `tests/common/mod.rs:137-153`.

### Request helpers

| Helper | Returns | Notes | Source |
|---|---|---|---|
| `register_account(router, user, pass)` | `authlyn_session=<token>` cookie | POSTs `/auth/register`, asserts 201, harvests the `Set-Cookie`; **this token is the identity replayed by every other call** | `tests/common/mod.rs:285` |
| `send(router, method, path, cookie, body)` | `(StatusCode, Option<Set-Cookie name=value>, Value)` | low-level; surfaces the first `Set-Cookie` as a bare `name=value` to replay | `tests/common/mod.rs:246` |
| `post_json` / `get_json` | `(StatusCode, Value)` | JSON-shaped convenience | `tests/common/mod.rs:209` / `:381` |
| `build_json_request` + `status_of` | a prebuilt `Request` + a spawn-shaped driver | for concurrency tests firing on cloned routers via `tokio::spawn` | `tests/common/mod.rs:182` / `:204` |
| `open_sse` / `next_sse_data` | streaming `Body` + `SseRead` | SSE; see below | `tests/common/mod.rs:305` / `:348` |

Response bodies are read to bytes with a **1 MiB cap** (`1 << 20`) and parsed to `serde_json::Value`. Server-trusted identity is structural here: identity is passed *only* as the harvested session cookie, never in a request body — see [auth & privacy](05-auth-privacy.md).

### SSE discipline: `SseRead{Data, Timeout, Closed}` (load-bearing)

`next_sse_data(body, within)` (`tests/common/mod.rs:348`) drains frames until one `data: <json>` line arrives, the window elapses, or the stream ends — returning one of:

| Variant | Meaning | A test asserting it is testing… |
|---|---|---|
| `Data(Value)` | a `data:` line arrived and parsed | …that an event *was* delivered |
| `Timeout` | window elapsed, **stream still open and silent** | …a **privacy filter** (the event was withheld but the connection lives) |
| `Closed` | the body stream ended (server dropped it) | …**fail-closed kill** (e.g. logout terminates the stream) |

`Timeout` vs `Closed` is the whole point: a privacy test MUST assert `Timeout` — `Closed` would only prove the server dropped the stream, not that an event was withheld (`tests/common/mod.rs:326-337`). Parser assumptions (`tests/common/mod.rs:341-347`): axum's serializer output — one single-line `data:` per event plus `: ` keep-alive comments; per-frame lossy UTF-8 (fine while `SyncEvent` payloads are ASCII ids only; a multi-byte char split across frames would mangle); an unparseable `data:` line **panics** rather than being silently skipped.

## The 28 suites → what each pins

Milestones use the project's `M#` namespace (see [`CLAUDE.md`](../../CLAUDE.md)); `Wave-1` = the safety-net characterization suites (audit `019e6c08`); `L-`/`F-`/review-`M-##` are review-finding ids. Owning milestone is taken from each suite's `//!` header.

| Suite | Tests¹ | Owning | Canonical pin(s) |
|---|---:|---|---|
| `auth.rs` | 16 | Step-1 | registration/login/session/logout; `me_*` 401 without/with-garbage cookie; `concurrent_register_same_username_never_500s` (409-not-500) |
| `guilds.rs` | 17 | Step-2/M5 | guild CRUD, membership privacy-404, role gates, rail reorder; `concurrent_invite_yields_one_member_row` (UNIQUE race) |
| `messages.rs` | 34 | Step-3 | **largest suite**; composite `(sent_at,id)` cursor, strict tie-break both directions, the `truncate`-before-flip mutation guard, `MAX_BODY_CHARS`/attachment caps, markup verbatim, `nonmember_post_probes_collapse_to_the_identical_privacy_404` (M-26 validation order) |
| `personas.rs` | 22 | Step-4 | persona CRUD owner-scoping, avatar/gallery + batch caps, key-redeem/friend-share editor lifecycle, snapshot-name-on-send, `concurrent_channel_wear_converges_to_one_row` |
| `lorebook.rs` | 5 | Step-5 | lorebook-entry CRUD, position ordering, non-lorebook-channel 400, membership privacy-404 |
| `friends.rs` | 5 | Step-6 | request→pending→accept, reverse-request auto-accept, duplicate 409, unfriend, self/unknown guards |
| `dms.rs` | 21 | M7/P1 | DM = `channel kind='dm'`, no guild; friend-gate, 1:1 dedup, leave→last-member soft-delete, unfriend read-lock; `dm_privacy_404_body_is_byte_identical_to_the_guild_channel_404`; `concurrent_one_to_one_creates_converge_on_one_thread` |
| `cameos.rs` | 14 | M7/P2 | guest-cameo lifecycle, friend-gate, send-time badge survives revoke; `nonmembers_cannot_invite_or_revoke` |
| `events.rs` | 19 | M1 | the SSE suite — see [canary families](#3-the-sse-revocation--quiet-death-family) |
| `sync_events.rs` | 3 | M1 | `SyncEvent` **wire shapes** (serde-only, no server/DB): snake_case `type` tags, targeted-event shapes, bare global reload tag |
| `typing_drafts.rs` | 13 | M4/T7 | Ghost-Quill ephemeral drafts; `typing_drafts_returns_privacy_404_with_identical_body_for_non_members`; `whisper_armed_draft_is_masked_to_the_fixed_placeholder_for_other_members` |
| `roll.rs` | 11 | M4/T6 | Fate-Engine dice grammar + server RNG; `editing_own_roll_is_403_and_the_body_is_unchanged`, `deleting_own_roll_is_403_and_the_roll_survives` (roll fully immutable) |
| `read_state.rs` | 4 | L-1 | cross-device `mark-read` / `read-state` `(sent_at,id)` cursor |
| `unread.rs` | 9 | M1 | batched `/unread` cursor math, strict tie-break (M-11), ping flag, soft-delete exclusions (M-16), privacy (only visible channels) |
| `mentions.rs` | 6 | L-4 | `@member` resolves to a guild-member account → `pinged_users` → per-reader `is_pinged`; non-member/unknown resolves to nobody |
| `accent.rs` | 1 | M5/P2 | `guild.accent_color` manager-gated write, palette validation (400 on junk), round-trip |
| `emoji.rs` | 7 | Wave-1 | custom-emoji CRUD; `non_member_is_404_on_all_emoji_routes` (gate precedes name validation) |
| `media.rs` | 19 | Wave-1 | media blob store safety: `stored_path_outside_media_dir_is_refused`, `upload_rejects_script_capable_mimes`, nosniff/attachment for non-images, `no_media_response_is_cache_control_public`, `thumbnails_revalidate_via_pipeline_version_etag_instead_of_immutable` (M-29) |
| `cache_control.rs` | 3 | Wave-1 | JSON no-store vs media-immutable split (`server/mod.rs` map_response layer) |
| `feedback.rs` | 8 | Wave-1 | feedback submit + admin-gate (403 / write-never-ran) |
| `push.rs` | 7 | Wave-1 | Web-Push subscription CRUD + `load_notification_info` row read; `concurrent_subscribe_same_endpoint_converges_to_one_row` (full send is out of scope — needs a live service) |
| `soft_delete.rs` | 17 | Wave-1 | soft-delete→trash→restore round-trips, restore authz matrix, `restore_collapses_to_privacy_404_for_non_members_and_unknown_channels`, windowed `purge_soft_deleted` + cascade to child tables |
| `system_messages.rs` | 8 | — | Nova-DOT admin broadcast; `system_broadcast_is_403_for_non_admin_and_writes_nothing` + core-fn fan-out tests |
| `dev_reload.rs` | 4 | — | dev hot-reload over SSE; `reload_frame_is_payload_free`, `dev_reload_is_403_for_non_admin` + core-fn test |
| `skeleton_switch.rs` | 5 | M5/P2 | skeleton-id validation (ssr, **no DB**) pinning the `prefs.rs` persistence surface surviving the M3 retirement (Orbit is the sole shell) |
| **`retry_canary.rs`** | 3 | — | **canary** — live error-text matchers, [below](#1-retry_canary--live-db-error-text) |
| **`schema_apply.rs`** | 17 | Step-0 | **canary** — prod-shaped migration + idempotent boot, [below](#2-schema_apply--prod-shaped-migration--idempotent-boot) |
| **`style_lint.rs`** | 15 | — | **static scan, all graphs** — motion + deck-bug + 44px registries, [below](#4-style_lint--the-lone-all-graphs-static-scan) |

¹ Test-function count per suite (`#[test]` / `#[tokio::test]`), distinct from helper fns. Sum = **313**.

## The behavioural invariants and their pins

Each row is a behaviour the project guarantees, with the executable pin. Verify against the test before trusting the prose.

| Invariant | Pinned by |
|---|---|
| **Server-trusted identity** — identity is the session cookie only (never the request body); authorization is re-derived per mutate | `tests/auth.rs::me_without_cookie_is_401`, `::me_with_garbage_cookie_is_401`, `::logout_invalidates_the_session`; structurally, every suite passes identity solely via `register_account`'s `Set-Cookie` |
| **Privacy-404** — non-membership returns **404 not 403**, body **byte-identical** to the canonical channel/guild 404 (no existence oracle); the membership gate runs **before** any DB probe (reply-target/attachment/name) | `tests/messages.rs::nonmember_post_probes_collapse_to_the_identical_privacy_404` (M-26); `tests/typing_drafts.rs::typing_drafts_returns_privacy_404_with_identical_body_for_non_members`; `tests/dms.rs::dm_privacy_404_body_is_byte_identical_to_the_guild_channel_404`; `tests/emoji.rs::non_member_is_404_on_all_emoji_routes`; `tests/cameos.rs::nonmembers_cannot_invite_or_revoke`; `tests/soft_delete.rs::restore_collapses_to_privacy_404_for_non_members_and_unknown_channels` |
| **SSE bus is id-only & per-connection-filtered** — frames carry `type`+ids, never content (typing drafts especially); identity holds for the stream lifetime; logout fail-closed **kills** the stream (`Closed`); a quiet revoked stream dies within ~one `sse_recheck_period`, bound to a **deadline**, not to bus activity | `tests/events.rs::logging_out_a_session_ends_its_live_events_stream`, `::a_quiet_stream_dies_after_revocation_without_any_event`, `::a_revoked_stream_dies_even_while_invisible_bus_traffic_keeps_arriving`, `::outsider_never_receives_events_for_a_channel_they_cannot_see`, `::kicked_member_stops_receiving_channel_events_mid_stream`; `tests/typing_drafts.rs::whisper_armed_draft_is_masked_to_the_fixed_placeholder_for_other_members`; `tests/dev_reload.rs::reload_frame_is_payload_free`; `tests/sync_events.rs` (wire shapes) |
| **Write-conflict retry** — racy `CREATE`/`UPSERT` on a UNIQUE index → idempotent **409, never 500**; the SurrealDB error-string matchers in `src/server/retry.rs` are load-bearing | `tests/retry_canary.rs` (all 3, against **real** DB errors incl. the accepted aborted-transaction false-positive + M-33 tightening); `tests/auth.rs::concurrent_register_same_username_never_500s`; `tests/guilds.rs::concurrent_invite_yields_one_member_row`; `tests/personas.rs::concurrent_channel_wear_converges_to_one_row`; `tests/push.rs::concurrent_subscribe_same_endpoint_converges_to_one_row`; `tests/dms.rs::concurrent_one_to_one_creates_converge_on_one_thread` |
| **Schema migration over a populated prod-shaped table** — a new non-`option` field is materialized in the **single** existing first backfill (a separate revalidating UPDATE over still-`NONE` arrays crash-loops boot); `option<>` needs no backfill; widening an enum ASSERT or a field type needs `DEFINE FIELD OVERWRITE` (not `IF NOT EXISTS`, which silently keeps the narrower def); apply is idempotent across boot-restart | `tests/schema_apply.rs::applying_kind_over_populated_messages_materialises_without_wiping_attachments`, `::applying_effect_over_populated_messages_keeps_legacy_rows_with_effect_none`, `::applying_guest_cameo_over_populated_messages_materialises_without_wiping_attachments`, `::widened_kind_assert_reaches_a_db_where_kind_already_exists`, `::widening_channel_guild_to_option_over_populated_channels_admits_guildless_dms`, `::applying_schema_over_account_with_security_fields_purges_them_without_crashing`, `::applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent` |
| **Test isolation** — each worker gets a globally-unique namespace **and** media tempdir; `pid+seq` alone is not collision-proof (long-lived in-memory DB + PID reuse); `random_id()` is the fix | `tests/common/mod.rs:155-167` (ns format + flaky-409 rationale); `tests/common/mod.rs:116-120` (media tempdir) — *comment/code-pinned, not an assertion* |
| **Admin-gated endpoints are fail-closed** — with no `AUTHLYN_ADMIN_USERNAMES` in the test env the admin set is empty → every caller is non-admin → **403**, fired **before** any write. The admin-**allowed** path is intentionally **not** exercised through HTTP (a process-env read would race parallel workers); the fan-out core fn is driven directly instead | `tests/feedback.rs` (list/delete 403, write-never-ran); `tests/system_messages.rs::system_broadcast_is_403_for_non_admin_and_writes_nothing` + `broadcast_*` core-fn tests; `tests/dev_reload.rs::dev_reload_is_403_for_non_admin` + `broadcast_reload` core-fn tests |
| **Roll immutability + media safety** — a `kind='roll'` message is fully immutable (author's own edit **and** delete are explicit 403); `/media` is image-or-curated-download only (script-capable MIMEs → 415), non-images served octet-stream + nosniff + attachment, stored path must canonicalize **inside** `media_dir` (escape → refuse, never serve), thumbnails private + pipeline-version ETag (never `public` on the session-gated route, M-29) | `tests/roll.rs::editing_own_roll_is_403_and_the_body_is_unchanged`, `::deleting_own_roll_is_403_and_the_roll_survives`; `tests/media.rs::stored_path_outside_media_dir_is_refused`, `::upload_rejects_script_capable_mimes`, `::pdf_upload_is_served_as_nosniff_attachment`, `::no_media_response_is_cache_control_public`, `::thumbnails_revalidate_via_pipeline_version_etag_instead_of_immutable`; `tests/cache_control.rs` |

## The four canary families

A *canary* pins something the compiler cannot: live external behaviour, or a static doctrine. A renamed string or a dropped property degrades silently in production with **no compile signal** — the canary is the only signal.

### 1. `retry_canary` — live DB error text

3 tests (`tests/retry_canary.rs`). SurrealDB `=3.1.0-beta.3` exposes **no typed error variant** for write-conflict / unique-violation, so `server::retry::{is_write_conflict, is_unique_violation}` **substring-match the SDK's `Display` text**. A future text rename would silently disable the retry loop and degrade every UNIQUE-violation 409 into a 500 — invisibly. Each canary synthesizes the real error against a throwaway table and asserts the matcher still fires, echoing the live `Display` string in the failure message so a rename is immediately legible.

- `is_write_conflict_matches_real_surrealdb_conflict` — synthesizes a genuine MVCC conflict: 10-way parallel `UPDATE` fan-out on one row under a `multi_thread` runtime, 50-attempt cap. If no conflict ever materializes that *itself* fails (the synth or MVCC behaviour changed).
- `aborted_transaction_sibling_text_is_indistinguishable_from_a_write_conflict` — pins the **accepted false-positive class** plus the **M-33** tightening. On 3.1.x, any aborted multi-statement transaction rewrites its non-failing statements' result rows to the generic sibling text *"The query was not executed due to a failed transaction"*; `Response::check()` surfaces that first sibling, so at the matcher's layer a permanently-failing transaction is byte-identical to a genuine commit-time conflict — `is_write_conflict` **must keep matching it** or the 3.1.3 genuine-conflict path loses its retry. M-33 simultaneously forbids over-matching a loose *"failed transaction"* substring (e.g. a thrown message echoing user data): the third marker must be the full generic sentence. **Narrowing this matcher is a two-way trap** (`tests/retry_canary.rs:141-233`).
- `is_unique_violation_matches_real_surrealdb_violation` — synthesizes a real UNIQUE violation (two `CREATE`s, same key) and asserts `is_unique_violation` fires **and** `is_write_conflict` does **not** (disjointness — else the retry loop would retry an unretryable error and 500 instead of mapping to 409).

### 2. `schema_apply` — prod-shaped migration + idempotent boot

17 tests (`tests/schema_apply.rs`). The boot path is `db::apply_schema` → `main.rs` `.expect(...)`; a schema edit that crash-loops boot is the single most expensive failure mode. These tests apply `storage::SCHEMA` **exactly as boot does** over **prod-shaped, populated** databases (defined via `raw_db()` with an "old" schema first) and assert `.check()` is `Ok` + the migration landed:

- **single-first-backfill folds** — `kind` / `effect` / `accent_color` / `guest_cameo` materialize new fields inside the *one* existing backfill UPDATE; a separate revalidating UPDATE over still-`NONE` arrays crash-loops boot (the 2026-06-01 attachments-wipe lesson) — `applying_kind_over_populated_messages_materialises_without_wiping_attachments`, `applying_effect_over_populated_messages_keeps_legacy_rows_with_effect_none`, `applying_guest_cameo_over_populated_messages_materialises_without_wiping_attachments`, `applying_accent_color_over_populated_guilds_keeps_legacy_rows_with_accent_none`.
- **`DEFINE FIELD OVERWRITE` widenings** — `widened_kind_assert_reaches_a_db_where_kind_already_exists` (message.kind +`'roll'`); `widening_channel_guild_to_option_over_populated_channels_admits_guildless_dms` (channel.guild → `option`, kind +`'dm'`).
- **security-field purge ordering** — `applying_schema_over_legacy_account_backfills_display_name_and_keeps_avatar_none`, `applying_schema_over_account_with_security_fields_purges_them_without_crashing` (backfill `display_name` before the full-table UPDATE).
- **new account-only indexes over populated rows** — `new_guild_member_account_index_applies_over_populated_rows`, `new_dm_member_account_index_applies_over_populated_rows`, `dm_pair_unique_index_rejects_duplicate_pair_and_reapplies`, `new_channel_guest_account_index_applies_over_populated_rows`.
- **the system seed** — `nova_dot_system_account_is_seeded_and_cannot_log_in` (the un-loggable sentinel hash).
- **the prod-deploy gate** — `applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent` (`tests/schema_apply.rs:1215`): the full M7 surface over 100+ prod-shaped rows, **then re-applied** — the single test standing between a schema edit and a boot crash-loop.

See [data model & schema](03-data-model.md) for the schema itself (`src/storage/schema.surql`).

### 3. The SSE revocation / quiet-death family

Inside `events.rs` (19 tests). The id-only / per-connection-filter / fail-closed direction of the bus — see [realtime SSE](04-realtime-sse.md). The non-obvious pins:

- `outsider_never_receives_events_for_a_channel_they_cannot_see` — filter (`Timeout`, stream alive).
- `kicked_member_stops_receiving_channel_events_mid_stream`, `guild_soft_delete_silences_open_member_streams` — mid-stream revocation: the connection drains its `lists_changed` then goes **silent-but-alive**.
- `logging_out_a_session_ends_its_live_events_stream` — fail-closed **kill** (`Closed`; a reconnect then 401).
- `a_quiet_stream_dies_after_revocation_without_any_event` — the deadline-bound recheck (`arena_with_sse_recheck_period(100ms)`): a revoked session whose stream is silent still dies within one period.
- `a_revoked_stream_dies_even_while_invisible_bus_traffic_keeps_arriving` — the M-05 leak guard: the recheck deadline must **not** re-arm on filtered `continue`s, or a revoked stream lives as long as a neighbour keeps typing.

These drive `a.state.emit` / `emit_for` directly for paths HTTP cannot reach — using `Arena::state` (see [the harness](#arenastate-vs-a-fresh-appstate-load-bearing)).

### 4. `style_lint` — the lone all-graphs static scan

15 `#[test]` (`tests/style_lint.rs`). **No feature gate** — the *only* suite that compiles and runs in **all three** graphs (ssr + hydrate + nova). Pure filesystem scan of `style/*.scss` + `src/**/*.rs` from `CARGO_MANIFEST_DIR`: no DB, no browser. It is the **external per-class signal** for the UI-fidelity defects that compile/clippy/Chromium-green cannot catch and that only surface on the owner's physical iPhone (see [styling & chrome](08-styling-chrome.md) and the *UI fidelity* section of [`CLAUDE.md`](../../CLAUDE.md)). It runs in `.githooks/pre-commit` via `cargo test --features ssr --test style_lint`, on top of `/check`.

**It is not a substitute for the owner deck-pass** — it is the static floor under it.

Doctrine guards:

| Guard | Pins | Mechanism |
|---|---|---|
| `no_keyframes_animate_paint_or_layout_properties` | motion doctrine #43 — `@keyframes` may animate only transform/opacity; `box-shadow`/`background-position`/`filter:`/`width:`/`height:`/`top:`/`left:` forbidden (per-frame repaint on the POCO C3 floor) | brace-matched keyframe bodies vs the `FORBIDDEN` list, minus the `EXEMPT_KEYFRAMES` allowlist (`shimmer`, `gallery-skeleton-shimmer`) |
| `backdrop_filter_always_has_webkit_sibling` | WebKit 1:1 (owner 2026-06-15, Liquid-Glass-default) — every standard `backdrop-filter` decl has a `-webkit-` sibling so Safari never silently loses the glass | per-file **count-equality** of the two declarations (comments stripped; order/gap-independent) |
| `glass_holo_is_liquid_glass_not_frost_noise` | the `glass-holo` mixin emits the `backdrop-filter` upgrade + its `-webkit-` sibling and **does not** layer `--frost-noise` (the rejected "TV static") | brace-matched mixin body in `_foundation.scss` |
| `orbit_map_overlay_blocks_touch_scroll` | iOS scroll-lock (F1) — `.sk-orbit-map` carries `touch-action: none` so the fixed full-cover overlay swallows touch and WebKit can't rubber-band the chat behind it | brace-matched base `.sk-orbit-map` rule |

Deck-bug-class regression guards (M5 → M7) — each validated to turn **red** on the pre-fix state of a named commit:

| Guard | Class it pins |
|---|---|
| `scrim_only_on_modal_backdrops` | `var(--scrim)` (opaque modal dim) only on a true modal backdrop (`MODAL_SCRIM_ALLOWLIST`); under unconditional `fx-max` an opaque scrim on a popover blacks out the whole chat (B4) |
| `sk_orbit_content_clips_both_axes_no_paint_containment` | `.sk-orbit-content` must `overflow: clip` **both** axes (no per-axis split → iOS hardware-only side-scroll) and no `contain: paint` (traps the fixed orb/composer) |
| `no_dead_fx_max_negation` | no permanently-dead `:not(.fx-max)` selector (`fx-max` is rendered unconditionally) |
| `no_html5_drag_and_drop_in_ui` | no HTML5 DnD handlers (iOS WebKit has none); `draggable=` only the literal `"false"` |
| `swipe_engines_bail_pointer_capture_on_controls` | the three pointer-gesture engines (`drag.rs`/`holopanel.rs`/`modal.rs`) bail before `set_pointer_capture` on interactive controls (else desktop click-dead) |
| `each_pref_toggle_is_rendered_exactly_once` | each bool pref toggle has exactly one checkbox render site (the Ghost-Quill dup class) |
| `management_modal_dismiss_returns_to_origin` | each of the **3** `swipe_close=true` management modals routes both its `close=` and its `<ModalHead>` dismiss through `act::modal_back` (origin-aware one-step-back, Bug 3) and never enters a channel |
| `orbit_chrome_controls_inherit_glass_material` | every `MATERIAL_CONTROLS` member includes `glass-holo`/`glass-live` in some rule (B2 flat-chrome) |
| `dispatch_pane_controls_inherit_glass_material` | the `PANE_CONTROLS` (wave-b / wardrobe / server-icon) include the glass material at their base (B2) |
| `glass_holo_consumers_let_the_mixin_own_the_background` | a glass consumer must **not** restate a top-level `background:` after the include (dead on backdrop-filter engines) |
| `registered_interactive_controls_declare_44px_touch_floor` | every `FLOOR_CONTROLS` member declares a `≥ 2.75rem` (44px) height floor — the owner 2026-06-17 product-wide tap floor |

**Curated-registry maintenance protocol** (the hard rule): these guards drive **curated allowlists / registries** (`EXEMPT_KEYFRAMES`, `MODAL_SCRIM_ALLOWLIST`, `MATERIAL_CONTROLS`, `PANE_CONTROLS`, `FLOOR_CONTROLS`, the `PREF_TOGGLES`/engine lists). When adding or altering chrome you **extend the registry by hand** — you do **not** disable a guard. A new floored/material control joins its registry as a deliberate edit; the registries are the static encoding of UI doctrine, intentionally not an auto button-scan (which false-positives on image tiles, inline spans, list rows). The detection internals — `brace_body` / `all_bodies` brace-matching, `has_top_level_background` depth-1 detection, `declares_touch_floor` + `len_to_rem` (skips non-absolute units rather than reading them as 0) — live at `tests/style_lint.rs:365-836`.

## Deliberate non-coverage (do not "fix" these)

| Gap | Why it is intentional |
|---|---|
| The admin-**allowed** HTTP path | `is_admin` reads a process env var; setting it would race parallel test workers. The allowed-path behaviour is exercised by driving the fan-out **core fn** directly (`system_messages.rs`, `dev_reload.rs`). |
| Full Web-Push **send** (`notify_new_message`) | needs a live push service. Only the payload's **row read** (`load_notification_info`) is pinned (`push.rs`). |
| Per-blob media ACL | the media store has a documented per-blob non-ACL posture; safety is path-canonicalization + MIME handling, not per-object authorization (`media.rs`). |

## Cross-graph note

What "always-on" means for the test tree:

- `style_lint.rs` is the **only** suite with no `#[cfg(feature)]` gate — it compiles and runs in ssr, hydrate, and nova (pure fs scan).
- `sync_events.rs` and `skeleton_switch.rs` are ssr-gated but **DB-free**: `sync_events` pins `protocol::SyncEvent` wire shapes (serde-only; `protocol.rs` must compile to `wasm32-unknown-unknown`), `skeleton_switch` pins the `ui::shell::act` skeleton stubs. Both exercise the always-on / serde-only surface (see [disjoint feature graphs](01-overview.md)).
- Every other suite is ssr-only and needs the live DB.

## Source map

Harness and canaries:
- `tests/common/mod.rs` — the shared harness (`Arena`, `arena*()`, `raw_db`/`test_db`, `register_account`, `send`/`*_json`, `open_sse`/`next_sse_data`, `SseRead`, `NS_COUNTER`, `random_id`). **Not** a test binary.
- `tests/retry_canary.rs` — live SurrealDB `Display`-text matchers for `server::retry::{is_write_conflict,is_unique_violation}`; the accepted aborted-transaction false-positive + M-33.
- `tests/schema_apply.rs` — prod-shaped migration + idempotent boot gate for `storage::SCHEMA`.
- `tests/style_lint.rs` — the lone all-graphs static scan; motion + WebKit-1:1 + deck-bug + 44px registries.

The 25 server suites: `tests/{auth,guilds,messages,personas,lorebook,friends,dms,cameos,events,sync_events,typing_drafts,roll,read_state,unread,mentions,accent,emoji,media,cache_control,feedback,push,soft_delete,system_messages,dev_reload,skeleton_switch}.rs`.

Code under test (imported from the crate; defined elsewhere):
- `src/server/` — `make_router`, `AppState` (+ `with_draft_ttl` / `with_sse_recheck_period`), `retry::{is_write_conflict,is_unique_violation}`, `dev_reload::broadcast_reload`, `system_messages::{broadcast_system_message,validate_broadcast_body}`, `push::load_notification_info`, `purge_soft_deleted`, `messages::ORACLE_ANSWERS`. → [request lifecycle](02-request-lifecycle.md), [auth & privacy](05-auth-privacy.md), [REST reference](../reference/rest-api.md)
- `src/storage/schema.surql` (`storage::SCHEMA`) — applied verbatim as the boot path does. → [data model](03-data-model.md)
- `src/protocol.rs` (`SyncEvent`) — wire shapes; serde-only, wasm-safe. → [realtime SSE](04-realtime-sse.md)
- `src/markup/` (`parse`/`Node`) — link grammar + scheme whitelist round-trip. → [markup engine](06-markup-engine.md)
- `style/*.scss`, `src/ui/shell/` — scanned by `style_lint`. → [styling & chrome](08-styling-chrome.md), [UI shell](07-ui-shell.md)

Gate: `cargo test --features ssr` (0 failed) + live SurrealDB on `ws://127.0.0.1:8000`. Static subset in pre-commit: `cargo test --features ssr --test style_lint`.
