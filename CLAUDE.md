# CLAUDE.md — authlyn-interactive

Self-hosted, server-trusted roleplay chat platform (Discord + SillyTavern: guilds → channels, personas, lorebooks, friends). Single Rust crate: axum + Leptos 0.8 + SurrealDB. This file is the **project operating manual** — only what a session needs in-context every turn.

Behavior, communication language, persistence philosophy, novelty handling, and the session-start calibration ritual live in the global `~/.claude/CLAUDE.md` — **not** restated here.

**Lives elsewhere — read there, don't copy here:**
- **Stack + directory layout:** `README.md`.
- **Every dependency's purpose + the ssr/hydrate/nova graph constraints + the cargo-leptos config:** the `#` comments in `Cargo.toml` (dense and authoritative).
- **Toolchain probe, Bash allowlist, hooks:** `.claude/settings.json`.
- **Project knowledge (architecture, invariant rationale, incidents, decisions):** **ctx** is canonical (global rule). Tombstoned blocks are dead — don't cite them.

## Toolchain prereqs
`cargo-leptos` installed, and `rustup target add wasm32-unknown-unknown` (the hydrate clippy step and `cargo leptos build` both need it). The `SessionStart` hook prints cargo / cargo-leptos / surreal presence + dev-DB status each session.

## Build / run / test / check (exact invocations)
- **Dev DB** (required before run *and* tests): `surreal start --user root --pass root --bind 127.0.0.1:8000 memory` — or `/dev-db` (health-checks first). ns `authlyn` / db `dev`.
- **Run:** `cp .env.example .env`, then `cargo leptos watch`. App → `http://127.0.0.1:3000`, DB → `ws://127.0.0.1:8000`.
- **Tests:** `cargo test --features ssr` — **requires both `--features ssr` AND a live SurrealDB** on `ws://127.0.0.1:8000`. 26 suites in `tests/*.rs` drive the axum router via `tower::ServiceExt::oneshot` (no port bind); `tests/common/mod.rs` gives each worker an isolated namespace + media tempdir. Gate = **0 failed**.
- **Quality gate `/check`:** `cargo fmt --all --check` → `cargo clippy --features ssr --no-deps -- -D warnings` → `cargo clippy --features hydrate --target wasm32-unknown-unknown --no-deps -- -D warnings`. This is the fmt+clippy **subset** of `.githooks/pre-commit`, which *also* runs the #43 motion-doctrine `@keyframes` scan over staged `.scss` (precise check: `cargo test --features ssr --test style_lint`).
- **nova is NOT in `/check`** — build it by hand when touched: `cargo build --release --bin nova-mcp --features nova`. For changes to the always-on spine (`protocol.rs`, `markup/`), "compiles under all three graphs" is part of done.
- **Regenerate `public/emoji.json`:** `cargo run --example gen_emoji_json`.
- `rustfmt` runs automatically on every `Edit`/`Write` of a `.rs` file (settings.json `PostToolUse` hook) — don't hand-run it per file.

## Branch / deploy / pipelines
- Work happens on branch **`mendicant-bias`**; merge to **`main` only after explicit owner approval** (design spec).
- **Prod deploy is FROZEN.** `/deploy` and `.github/workflows/deploy.yml` still target the **retired** host *fenrir* (Pi). Prod is now **novahome** (x86_64); both must be **repointed before any v27 deploy**. Do not run `/deploy` as-is.
- **Test deck** (`/test-deploy`): the shared review surface on **novahome**, `https://192.168.0.239:3434`, `authlyn-test` service, ns `authlyn` / db `test`. Feature branch + committed tree only — **never `main`, never prod**. Pushes to both GitHub (backup) and the novahome bare repo (build source); verify the deployed SHA. This is the iOS/Android review target.
- Never re-create the intentionally-deleted `deploy/` `scripts/` `end2end/` trees (commit `c2aba1c`) or the `visual-gate/` tooling (commit `68b65bd`).

## Disjoint feature graphs (hard rule — never cross-import)
Three graphs, mutually exclusive at the binary level:
- **ssr** (server): axum + tokio + surrealdb + leptos/ssr — never compiles to wasm.
- **hydrate** (browser/WASM): leptos/hydrate + gloo-* + web-sys — never enters the server runtime.
- **nova** (`src/bin/nova-mcp.rs`, `--features nova`): MCP bridge — imports zero ssr/hydrate.

Only `src/protocol.rs` (wire DTOs) and `src/markup/` are always-on; both must compile to `wasm32-unknown-unknown` (serde-only — no axum/surrealdb/tokio). `nova` carries `required-features` so the default build / cargo-leptos never pulls it. Per-crate membership is in `Cargo.toml [features]`.

## Conventions
- **Commits — Conventional Commits** + trailing milestone tag: `type(scope): subject (M5/P2)`, imperative subject, `type ∈ {feat, fix, refactor, docs, chore, a11y}`. Body explains the invariant/finding touched; add a `Tests:` line and the `Co-Authored-By` trailer.
- **Handlers:** `verb_noun` lowercase (`create_guild`, `list_messages`); no `handle_` prefix; static routes rank over dynamic.
- **DTOs** (`protocol.rs`): suffix `Request/Response/Summary/Detail/Envelope/Item/Entry`; PATCH-shaped DTOs derive `Default` + all-`Option<>`; wire is serde JSON.
- **Docs:** every module a `//!` header; public REST fns lead with `/// VERB /path — intent`. (Dependency `#`-comments: see `Cargo.toml`.)
- **Tests:** `tests/*.rs`, `#[tokio::test]`, full-sentence `snake_case` names; the shared harness stays `tests/common/mod.rs`.
- **Versioning:** the current scheme (CalVer `YYYY.M.D` + a manual two-word codename) is owned by `README.md` + `Cargo.toml`. **PENDING at v27:** the design spec retires CalVer — v27 ships as **SemVer `27.0.0`**, codename **`mendicant-bias`**. Cargo.toml is still on `2026.6.1` / `saffron-tide`; flip this line + README **at the release**, not before.

## Namespace: project milestones (M#) vs. calibration warnings (W#)
The behavioral-calibration block owns the **`W#`** namespace (W1–W19, incl. W6a–e) — RLHF warning axes. To avoid collision, the project release-wave scheme uses **`M#`** (Milestone), never `W#`.
- **`M#`** = a project release wave. Sub-tokens: **`/P#`** = phase, **`/T#`** = task; bare **`#N`** = a review-finding id. Commit trailer: `(M5/P2)`. Plan/spec slug: `…-m5-skelettvagen.md`.
- **`W#`** is calibration-only — never write a bare project wave as `W#`.
- Number mapping is **identity**: historical `W#` == `M#` on the same number (W5 = M5 = *Skelettvägen*). Pre-reboot commits and existing code comments keep their `W#/P#` form (history is immutable) — read `W5/P2` there as `M5/P2`. The four surviving plan files keep their `-w1..-w4-` names; the next plan is the first `-m5-` file.
- ctx: tag wave knowledge `milestone` / `m5`, never `w5`.

Reading a bare token: a `/P#` or `/T#` suffix in a feature/release context = project (use `M#`); "drift / axis / warning" wording or the calibration block = `W#`.

## Operating invariants & footguns (the tests are canonical — read the test, not memory)
The full numbered catalogue was in the now-deleted `docs/ARCHITECTURE.md` (commit `68b65bd`). The surviving source of truth is **the integration tests (`tests/*.rs`) and the code**. The few that crash boot / break security / waste hours and aren't caught until too late:
- **Server-trusted + privacy-404:** identity comes from the session cookie only (never client-supplied); authorization is re-derived per mutate; non-membership is a **404, not a 403**. (`tests/{auth,guilds,personas}.rs`.)
- **Schema migration** (`src/storage/schema.surql`, pinned by `tests/schema_apply.rs`): a new field on a populated table must be `option<>` **or** get an idempotent backfill UPDATE *before* any row-revalidating UPDATE, or schema-apply crash-loops on boot. Widening an enum ASSERT needs `DEFINE FIELD OVERWRITE`, not `IF NOT EXISTS` (which silently keeps the old narrower ASSERT and rejects the new value).
- **SSE bus is id-only:** every mutation calls `AppState::emit`/`emit_for`; frames carry ids, never content (typing drafts especially). `/events` re-checks the session every frame and ≥ every 30s. Extend this to any new realtime surface. (`tests/sync_events.rs`.)
- **Write-conflict retry:** racy `CREATE` → `with_write_conflict_retry` → idempotent **409, never 500**. The error-string matchers in `src/server/retry.rs` are load-bearing and pinned against live DB text by `tests/retry_canary.rs`.
- **SurrealDB pin:** SDK `=3.1.0-beta.3`; the on-machine `surreal` CLI and the SDK must share the **3.x** major (divergent write-conflict error texts otherwise).
- **WebKit Secure-cookie trap:** Safari/WebKit drops the `Secure` session cookie over `http://localhost` (Chromium accepts it) → the browser "logs in" 200 but `/auth/me` stays 401. **Test WebKit/iOS over HTTPS with the dev root CA** (`authlyn-dev-rootCA.pem`, gitignored at the repo root) — the deck `https://192.168.0.239:3434`, or a local HTTPS front. The old `http://localhost` + `authlyn_session secure:false` injection is **retired** (owner ruling 2026-06-17): the root CA makes WebKit trust the HTTPS cert, so the `Secure` cookie is accepted.
- **Never** point smoke / health / load / registration flows at the prod URL or `SURREAL_DB=prod` — use `localhost:3000` + the dev DB, a disposable namespace, or the deck.

## ctx persistence triggers (project-specific)
Proactively `ctx save` (and say where) after: a new/changed invariant or its rationale; a schema-apply / migration hazard; a SurrealDB version-skew or error-text finding; a perf measurement for a sync/realtime change (measure end-to-end — rows, bytes, client work — never assume); a deploy/infra fact (novahome repoint, deck coordinates); or a lesson from a review/incident.

## UI fidelity
Compile-green is necessary, never sufficient, for UI. The visual oracle is the surviving skeleton prototypes in `docs/superpowers/specs/assets/2026-06-12-skelettvagen/` (`a-orbit.html`, `b-deck.html`, `c-hud.html`). The deleted `visual-gate/` Playwright tooling must not be re-created; a fidelity-gate method is owner-driven.

**Touch-target floor — Mendicant Bias is touch-first (hard rule, owner ruling 2026-06-17).** Every interactive control in the **Mendicant Bias** UX — the whole product, across all skeletons and the shared modals/panes; **not** scoped to `sk-orbit` — meets a **≥ 44px (`2.75rem`) tap target**: `min-height`, plus `min-width` for square/icon buttons. This is a product-wide baseline and correctness, not polish — compact desktop-density controls are the regression Mendicant Bias exists to retire. Apply the floor at the control's **base/shared definition** so every surface inherits it (e.g. raise the compact ghosts in `_base.scss`, `_trash.scss`, `_wave_b.scss`, `_sidebar.scss`, `_wardrobe.scss` themselves) — **not** as an `.app.sk-orbit` override. It is already anchored in places (`.account-logout`, the modal close `.account-head .row-edit`, `.sk-orbit-pill`, `.composer .send`, `.sk-orbit-chip`, `.accent-swatch` are all ≥ 44px); match it on every control. There is **no blanket `button { min-height }`** — it distorts the bespoke chrome (composer orb, map nodes, swipe-strip); apply the floor per control.
