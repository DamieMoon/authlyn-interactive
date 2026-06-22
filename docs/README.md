# authlyn-interactive — documentation

`authlyn-interactive` is a self-hosted, server-trusted roleplay chat platform on the Discord + SillyTavern model: guilds → channels, personas, lorebooks, friends. It ships as a **single Rust crate** — `axum` + `Leptos 0.8` (server-side render *and* WASM hydrate) over an external `SurrealDB`, with a standalone MCP bridge bin behind an optional feature. Identity is server-trusted (session cookie only; no browser-side crypto). For the stack, the directory layout, and the dev/run/test invocations see [`../README.md`](../README.md); for every dependency's purpose and the feature-graph constraints see the `#`-comments in [`../Cargo.toml`](../Cargo.toml). **This `docs/` tree is the reference manual; the day-to-day operating manual — what a working session needs in-context every turn — is [`../CLAUDE.md`](../CLAUDE.md).** Read it first if you are about to change code.

## Conventions used throughout these docs

- **Drift-anchoring (hard rule).** This tree replaces a prior `docs/ARCHITECTURE.md` that was deleted for rotting into stale, unanchored claims. Therefore **every behavioral invariant cites its pinning test** as `tests/<file>.rs::<test_name>` (or `src/<file>.rs:<line>` when only code-pins it). An invariant with no real pin is marked `(unpinned)` — citations are never invented. The tests are canonical: when a doc and a test disagree, **the test wins** — read the test, then fix the doc.
- **Reference, don't duplicate.** Stack, dependencies, toolchain, build/run/test commands, commit/handler/DTO conventions, and deploy posture live in [`../README.md`](../README.md), [`../Cargo.toml`](../Cargo.toml), [`../.claude/settings.json`](../.claude/settings.json), and [`../CLAUDE.md`](../CLAUDE.md). Docs link to those; they do not copy them.
- **Three disjoint feature graphs**, mutually exclusive at the binary level and **never cross-imported**: `ssr` (server runtime, never WASM), `hydrate` (browser/WASM, never the server runtime), `nova` (`src/bin/nova-mcp.rs`, `--features nova` — the MCP bridge, imports zero ssr/hydrate). Only `src/protocol.rs` (wire DTOs) and `src/markup/` are always-on and must compile to `wasm32-unknown-unknown` (serde-only). Each doc states which graph it concerns.

## Reading order

New to the crate? Read in this order: **01 → 02 → 03 → 05**, then branch to whatever subsystem you are touching. The numbering is the recommended path; each doc also stands alone.

### Architecture (`architecture/`)

| Doc | Read this when… |
| --- | --- |
| [`architecture/01-overview.md`](architecture/01-overview.md) | You are new to the crate and need the system shape: the three feature graphs, the always-on spine (`protocol.rs` + `markup/`), and how `ssr`/`hydrate`/`nova` fit together. Start here. |
| [`architecture/02-request-lifecycle.md`](architecture/02-request-lifecycle.md) | You need to trace an HTTP request end to end — axum router, middleware, `AppState`, handler dispatch, the SurrealDB call, and how the same router is driven test-side via `tower::ServiceExt::oneshot` (no port bind). |
| [`architecture/03-data-model.md`](architecture/03-data-model.md) | You are touching storage: the SurrealDB schema (`src/storage/schema.surql`), tables/fields/indexes, and the schema-migration footgun (new field must be `option<>` or get an idempotent backfill before any revalidating UPDATE, or boot crash-loops). |
| [`architecture/04-realtime-sse.md`](architecture/04-realtime-sse.md) | You are adding or changing a realtime surface. The SSE bus is **id-only** — frames carry ids, never content (typing drafts especially); every mutation calls `AppState::emit`/`emit_for`; `/events` re-checks the session each frame and ≥ every 30s. |
| [`architecture/05-auth-privacy.md`](architecture/05-auth-privacy.md) | You are writing anything that reads identity or authorizes a mutation. Identity is the **session cookie only** (never client-supplied), authorization is **re-derived per mutate**, and non-membership is a **privacy-404, not a 403**. Includes the WebKit `Secure`-cookie trap. |
| [`architecture/06-markup-engine.md`](architecture/06-markup-engine.md) | You are working on chat markup — the always-on `src/markup/` parser (tokenize → tree → blocks), which must stay serde-only and compile to WASM for both `ssr` and `hydrate`. |
| [`architecture/07-ui-shell.md`](architecture/07-ui-shell.md) | You are building Leptos UI: the `hydrate`-graph app shell (`src/ui/`, `src/app.rs`), the Discord-style shell layout, panes, and client-side REST (`src/client/`). |
| [`architecture/08-styling-chrome.md`](architecture/08-styling-chrome.md) | You are adding or altering chrome/CSS. Covers the motion doctrine, the Liquid-Glass material, the **≥44px touch-target floor** (product-wide, applied at the control's base definition), and the curated `tests/style_lint.rs` static-scan registries you must keep green and extend by hand. |
| [`architecture/09-testing.md`](architecture/09-testing.md) | You are writing or running tests. The harness (`tests/common/mod.rs`), the live-SurrealDB + `--features ssr` requirement, per-worker namespace isolation, the `/check` quality gate, and the `style_lint` static scan. |
| [`architecture/10-nova-mcp.md`](architecture/10-nova-mcp.md) | You are touching the MCP bridge — `src/bin/nova-mcp.rs` behind `--features nova`. It talks to the running HTTP API as the seeded `nova.` account and exposes it over MCP; imports zero ssr/hydrate. **Not in `/check`** — build it by hand when touched. |
| [`architecture/11-build-deploy-pwa.md`](architecture/11-build-deploy-pwa.md) | You are building, deploying, or working on the PWA surface. `cargo-leptos`, the `/check` and pre-commit gates, the **frozen prod deploy** (still points at retired host *fenrir*), and the test deck (`https://authlyndev.damienmoon.sh` on novahome). |

### Reference (`reference/`)

| Doc | Read this when… |
| --- | --- |
| [`reference/rest-api.md`](reference/rest-api.md) | You need the route/DTO matrix — every REST endpoint, its method/path, request/response DTO (`src/protocol.rs`), and auth requirement. The lookup table, not the narrative. |
| [`reference/conventions.md`](reference/conventions.md) | You need the in-repo conventions distilled: Conventional-Commits + milestone tag, `verb_noun` handler naming, DTO suffixes, the `//!`/`///` doc rules, and the CalVer→SemVer versioning state. (Sources: [`../CLAUDE.md`](../CLAUDE.md), [`../Cargo.toml`](../Cargo.toml).) |

## Pinned anchors (the load-bearing invariants and their tests)

These are the few that crash boot, break security, or waste hours — verified to exist in `tests/`:

| Invariant | Pinning test |
| --- | --- |
| Non-membership returns a privacy-**404**, not a 403; authorization re-derived per mutate | `tests/guilds.rs::nonmember_get_guild_is_404`, `tests/personas.rs` (per-test privacy-404 asserts) |
| Full schema applies over a prod-shaped populated DB crash-free + idempotently; new field is `option<>` or backfilled before revalidation | `tests/schema_apply.rs::applying_full_schema_over_prod_shaped_populated_db_is_crash_free_and_idempotent` |
| SSE frames are id-only; targeted sync events pin their wire shape | `tests/sync_events.rs::targeted_sync_events_pin_their_wire_shape`, `tests/sync_events.rs::sync_event_serializes_with_snake_case_type_tags` |
| Write-conflict matcher tracks live SurrealDB error text → idempotent 409, never 500 | `tests/retry_canary.rs::is_write_conflict_matches_real_surrealdb_conflict` |
| `nova.` system account is seeded and cannot log in | `tests/schema_apply.rs::nova_dot_system_account_is_seeded_and_cannot_log_in` |
| Every registered interactive control declares a ≥44px (2.75rem) touch floor | `tests/style_lint.rs::registered_interactive_controls_declare_44px_touch_floor` |
| Motion doctrine: `@keyframes` may not animate paint/layout properties | `tests/style_lint.rs::no_keyframes_animate_paint_or_layout_properties` |

The WebKit `Secure`-cookie trap (Safari drops the `Secure` session cookie over `http://localhost`; test WebKit/iOS over HTTPS at the deck) is documented in [`architecture/05-auth-privacy.md`](architecture/05-auth-privacy.md) and [`../CLAUDE.md`](../CLAUDE.md); it is environmental, not test-pinned `(unpinned)`.

## Source map

- [`../README.md`](../README.md) — stack, directory layout, dev/run commands (canonical; not duplicated here).
- [`../CLAUDE.md`](../CLAUDE.md) — day-to-day operating manual: build/test/check invocations, deploy posture, the load-bearing invariants + footguns.
- [`../Cargo.toml`](../Cargo.toml) — feature graphs and every dependency's purpose in `#`-comments; CalVer version + codename.
- [`../.claude/settings.json`](../.claude/settings.json) — toolchain probe, Bash allowlist, the `rustfmt`-on-write and `SessionStart` hooks.
- `tests/*.rs` — 28 integration suites; canonical for every behavioral invariant cited across this tree. Each doc's own "Source map" lists the tests pinning its claims.
- `tests/common/mod.rs` — shared harness: per-worker isolated namespace + media tempdir; drives the axum router via `tower::ServiceExt::oneshot`.
