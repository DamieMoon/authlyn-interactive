# CLAUDE.md — authlyn-interactive

Self-hosted, server-trusted roleplay chat platform (Discord + SillyTavern: guilds → channels, personas, lorebooks, friends). Single Rust crate: axum + Leptos 0.8 + SurrealDB. This file is the **project operating manual** — only what a session needs in-context every turn.

**Lives elsewhere — read there, don't copy here:**
- **Stack + directory layout:** `README.md`.
- **Every dependency's purpose + the ssr/hydrate/nova graph constraints + the cargo-leptos config:** the `#` comments in `Cargo.toml` (dense and authoritative).
- **Toolchain probe, Bash allowlist, hooks:** `.claude/settings.json`.
- **Deep architecture + REST reference (the connective narrative):** `docs/` — start at `docs/README.md`. Drift-anchored (every invariant cites its pinning test); the integration tests stay canonical.

## Toolchain prereqs
`cargo-leptos` installed, and `rustup target add wasm32-unknown-unknown` (the hydrate clippy step and `cargo leptos build` both need it). The `SessionStart` hook prints cargo / cargo-leptos / surreal presence + dev-DB status each session.

## Build / run / test / check (exact invocations)
- **Dev DB** (required before run *and* tests): `surreal start --user root --pass root --bind 127.0.0.1:8000 memory` — or `/dev-db` (health-checks first). ns `authlyn` / db `dev`.
- **Run:** `cp .env.example .env`, then `cargo leptos watch`. App → `http://127.0.0.1:3000`, DB → `ws://127.0.0.1:8000`.
- **Tests:** `cargo test --features ssr` — **requires both `--features ssr` AND a live SurrealDB** on `ws://127.0.0.1:8000`. 28 suites in `tests/*.rs` drive the axum router via `tower::ServiceExt::oneshot` (no port bind); `tests/common/mod.rs` gives each worker an isolated namespace + media tempdir. Gate = **0 failed**.
- **Quality gate `/check`:** `cargo fmt --all --check` → `cargo clippy --features ssr --no-deps -- -D warnings` → `cargo clippy --features hydrate --target wasm32-unknown-unknown --no-deps -- -D warnings`. This is the fmt+clippy **subset** of `.githooks/pre-commit`, which *also* runs the **`tests/style_lint`** static-scan suite (`cargo test --features ssr --test style_lint`) — the motion-doctrine `@keyframes` scan plus the deck-bug-class regression guards (see *UI fidelity*).
- **nova is NOT in `/check`** — build it by hand when touched: `cargo build --release --bin nova-mcp --features nova`. For changes to the always-on spine (`protocol.rs`, `markup/`), "compiles under all three graphs" is part of done.
- **Regenerate `public/emoji.json`:** `cargo run --example gen_emoji_json`.
- `rustfmt` runs automatically on every `Edit`/`Write` of a `.rs` file (settings.json `PostToolUse` hook) — don't hand-run it per file.

## Branch / deploy / pipelines
- Work happens on branch **`mendicant-bias`**; merge to **`main` only after explicit owner approval** (design spec).
- **Prod = novahome** (x86_64): service `authlyn-prod`, `https://authlyn.damienmoon.sh`. `/deploy` + `.github/workflows/deploy.yml` are repointed to novahome and **a push to `main` auto-deploys to prod** (build → prod-DB backup → `/opt/authlyn-prod/deploy.sh`; health-check `:8083` + auto-rollback). **v27.0.0 `mendicant-bias` shipped 2026-06-22** (tag `v27.0.0` on `main`); each future prod promotion stays owner-gated. The retired Pi *fenrir* path is gone — don't target it.
- **Test deck** (`/test-deploy`): the shared review surface on **novahome**, `authlyn-test` service, ns `authlyn` / db `test`. Address: `https://authlyndev.damienmoon.sh` (cloudflared, publicly-trusted cert — **the iOS/WebKit/Android review + probe target**, no root CA needed). The LAN-IP `https://192.168.0.239:3434` (self-signed) and its dev root CA are **retired** — don't probe or review there. Feature branch + committed tree only — **never `main`, never prod**. Pushes to both GitHub (backup) and the novahome bare repo (build source, via the `ssh novahome` alias — the bare-IP remote URL fails publickey; `git remote` is repointed to the alias); verify the deployed SHA.
- Never re-create the intentionally-deleted `deploy/` `scripts/` `end2end/` trees (commit `c2aba1c`) or the `visual-gate/` tooling (commit `68b65bd`).

## Disjoint feature graphs (hard rule — never cross-import)
Three graphs, mutually exclusive at the binary level: **ssr** (server runtime, never wasm), **hydrate** (browser/WASM, never the server runtime), **nova** (`src/bin/nova-mcp.rs`, `--features nova` — MCP bridge, imports zero ssr/hydrate). Per-graph crate membership + each dependency's purpose live in `Cargo.toml [features]` and its `#`-comments.

Only `src/protocol.rs` (wire DTOs) and `src/markup/` are always-on; both must compile to `wasm32-unknown-unknown` (serde-only — no axum/surrealdb/tokio). `nova` carries `required-features` so the default build / cargo-leptos never pulls it.

## Conventions
- **Commits — Conventional Commits** + trailing milestone tag: `type(scope): subject (M5/P2)`, imperative subject, `type ∈ {feat, fix, refactor, docs, chore, a11y}`. Body explains the invariant/finding touched; add a `Tests:` line and the `Co-Authored-By` trailer.
- **Handlers:** `verb_noun` lowercase (`create_guild`, `list_messages`); no `handle_` prefix; static routes rank over dynamic.
- **DTOs** (`protocol.rs`): suffix `Request/Response/Summary/Detail/Envelope/Item/Entry`; PATCH-shaped DTOs derive `Default` + all-`Option<>`; wire is serde JSON.
- **Docs:** every module a `//!` header; public REST fns lead with `/// VERB /path — intent`. (Dependency `#`-comments: see `Cargo.toml`.)
- **Tests:** `tests/*.rs`, `#[tokio::test]`, full-sentence `snake_case` names; the shared harness stays `tests/common/mod.rs`.
- **Versioning:** **SemVer** (from **v27** — CalVer `YYYY.M.D` retired at the v27 release). Current: **`27.0.0`**, codename **`mendicant-bias`** (`Cargo.toml` `version` + `[package.metadata.release].codename`; README mirrors the scheme). Bump per SemVer on each release; codename is a manual two-word name.

## Namespace: release waves (M#)
Project release waves are **`M#`** (Milestone). Sub-tokens: **`/P#`** = phase, **`/T#`** = task; bare **`#N`** = a review-finding id. Commit trailer: `(M5/P2)`

## Operating invariants & footguns (the tests are canonical — read the test, not memory)
The full numbered catalogue was in the now-deleted `docs/ARCHITECTURE.md` (commit `68b65bd`). The surviving source of truth is **the integration tests (`tests/*.rs`) and the code**. The few that crash boot / break security / waste hours and aren't caught until too late:
- **Server-trusted + privacy-404:** identity comes from the session cookie only (never client-supplied); authorization is re-derived per mutate; non-membership is a **404, not a 403**. (`tests/{auth,guilds,personas}.rs`.)
- **Schema migration** (`src/storage/schema.surql`, pinned by `tests/schema_apply.rs`): a new field on a populated table must be `option<>` **or** get an idempotent backfill UPDATE *before* any row-revalidating UPDATE, or schema-apply crash-loops on boot. Widening an enum ASSERT needs `DEFINE FIELD OVERWRITE`, not `IF NOT EXISTS` (which silently keeps the old narrower ASSERT and rejects the new value).
- **SSE bus is id-only:** every mutation calls `AppState::emit`/`emit_for`; frames carry ids, never content (typing drafts especially). `/events` re-checks the session every frame and ≥ every 30s. Extend this to any new realtime surface. (`tests/sync_events.rs`.)
- **Write-conflict retry:** racy `CREATE` → `with_write_conflict_retry` → idempotent **409, never 500**. The error-string matchers in `src/server/retry.rs` are load-bearing and pinned against live DB text by `tests/retry_canary.rs`.
- **SurrealDB pin:** SDK `=3.1.0-beta.3`; the on-machine `surreal` CLI and the SDK must share the **3.x** major (divergent write-conflict error texts otherwise).
- **WebKit Secure-cookie trap:** Safari/WebKit drops the `Secure` session cookie over `http://localhost` (Chromium accepts it) → the browser "logs in" 200 but `/auth/me` stays 401. **Test WebKit/iOS over HTTPS at the deck's public domain `https://authlyndev.damienmoon.sh`** (cloudflared, publicly-trusted cert → already in iOS's trust store, so the `Secure` cookie is accepted with **no** per-device cert step). Both earlier workarounds (the `http://localhost` + `secure:false` injection; the LAN dev root CA) are **retired** — don't reintroduce a self-signed / root-CA path for WebKit testing.
- **Never** point smoke / health / load / registration flows at the prod URL or `SURREAL_DB=prod` — use `localhost:3000` + the dev DB, a disposable namespace, or the deck.

## UI fidelity
Compile/clippy/ssr/Chromium-green is necessary, never sufficient — real WebKit/touch/visual defects (scrim blackout, missing glow, dead-end panes, sub-44px targets, iOS side-scroll) pass all of it and only surface on the owner's iPhone, and a fixed property has not survived the next rewrite. So:
- **Visual oracle:** the skeleton prototypes in `docs/superpowers/specs/assets/2026-06-12-skelettvagen/` (`a-orbit.html`, `b-deck.html`, `c-hud.html`). The deleted `visual-gate/` Playwright tooling stays deleted; a fidelity-gate method is owner-driven.
- **`tests/style_lint.rs` is the external signal per class** (pure static scan, no browser/DB — *not* a substitute for the owner deck-pass). When adding or altering chrome, keep its guards green and **extend the curated registries** rather than disabling one.
- **Touch-target floor (hard rule, owner ruling 2026-06-17):** every interactive control in Mendicant Bias — the whole product, **not** scoped to `sk-orbit` — meets a **≥ 44px (`2.75rem`)** tap target (`min-height`, plus `min-width` for square/icon buttons). Apply the floor at the control's **base/shared definition** so every surface inherits it — **not** as an `.app.sk-orbit` override. There is no blanket `button { min-height }` (it distorts the bespoke chrome). Enforced by the curated `registered_interactive_controls_declare_44px_touch_floor` registry — a new floored control joins it by hand.
