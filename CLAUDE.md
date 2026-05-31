# CLAUDE.md

Project memory for **authlyn-interactive**, built fresh from current code. A tracked file that still references a deleted `deploy/`, `scripts/`, or `end2end/` path is a DANGLING reference to REPORT — never honor or restore it (see Gotchas).

## Working style
Before writing, reviewing, or refactoring code, follow `andrej-karpathy-skills:karpathy-guidelines` (invoke the Skill for full text):
- Think before coding: state assumptions, surface tradeoffs, ask when unclear.
- Simplicity first: minimum code that solves it; nothing speculative.
- Surgical changes: touch only what the task needs; match surrounding style.
- Goal-driven: define a verifiable success check, loop until it passes.

## Identity
A Discord-style roleplay chat app: accounts/sessions, guilds, channels, messages, personas (editors + per-channel wear), media uploads, and Web Push.

## Stack
- Single Rust crate, **Leptos 0.8 full-stack**: axum 0.8 SSR server + WASM (`hydrate()`) browser bundle from one `src/` tree. `crate-type = ["cdylib","rlib"]`.
- **SurrealDB 3.x** over WebSocket (Rust SDK pinned `=3.1.0-beta.3`, Cargo.toml:61); 17 SCHEMAFULL tables in `src/storage/schema.surql`, embedded via `include_str!`.
- Built with **cargo-leptos** (config in `[package.metadata.leptos]`). Key crates: leptos_axum, tokio, argon2 (argon2id), image (thumbnails), web-push (VAPID), gloo-net/web-sys (client), rmcp/reqwest (nova).
- Toolchain pinned in `rust-toolchain.toml`: stable + rustfmt + clippy + `wasm32-unknown-unknown`.

## Commands
First run on a fresh clone (in order — `cargo`/`cargo-leptos` are not vendored):
1. `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh && cargo install cargo-leptos` — `rust-toolchain.toml` then auto-provisions wasm32 + rustfmt + clippy.
2. `surreal start --user root --pass root --bind 127.0.0.1:8000 memory` — the dev SurrealDB the app and tests connect to.
3. `cp .env.example .env && cargo leptos watch` — SSR + WASM + SCSS, live reload at http://127.0.0.1:3000. cargo-leptos loads `.env`; the compiled binary does NOT (no dotenv dep), so a raw `cargo run`/`nova-mcp` needs env exported.

- **Build:** `cargo leptos build --release`.
- **Test:** `cargo test --features ssr` — needs the `ssr` feature (harness is `#![cfg(feature="ssr")]`) AND a live SurrealDB on 127.0.0.1:8000 (root/root); each worker isolates a unique namespace.
- **Format / Lint:** `cargo fmt --all`; `cargo clippy --features ssr` (server graph) AND `cargo clippy --features hydrate --target wasm32-unknown-unknown` (browser graph — disjoint deps, not covered by the ssr lint).
- **nova-mcp:** build `cargo build --release --bin nova-mcp --features nova`; run `NOVA_PASSWORD=... NOVA_AUTHLYN_URL=http://127.0.0.1:3000 ./target/release/nova-mcp` (nova defaults to :8081, but the dev server is on :3000).
- **emoji regen (rare; on Unicode bumps only):** `cargo run --example gen_emoji_json` → `public/emoji.json`.

## Architecture
- Entry points: server `src/main.rs` (cfg `ssr`, `#[tokio::main]`); WASM `hydrate()` in `src/lib.rs` (cfg `hydrate`). Routing shell `src/app.rs`.
- Three disjoint feature graphs: `ssr` (server: axum/surrealdb/tokio/argon2/image/web-push), `hydrate` (browser: web-sys/gloo-net), `nova` (standalone MCP bridge `src/bin/nova-mcp.rs`, `required-features=["nova"]`, never in the Leptos build).
- Layout: shared always-on `protocol.rs` (serde wire DTOs), `markup/` (roleplay rich-text parser), `app.rs`, `ui/`; ssr-only `server/<domain>/`, `db.rs`, `storage/`; hydrate-only `client/api.rs`. Newcomer path: `app.rs` → `ui/shell/` → `client/api.rs` or `ui/shell/act/` → `protocol.rs` → `server/<domain>/` → `storage/schema.surql`.
- **Disjointness rule:** shared/WASM-bound modules import ZERO ssr crates (surrealdb/axum/tower); the hydrate bundle must stay free of ssr-only crates. All ssr/hydrate deps are `optional = true`, pulled only by their feature. `act/` submodules pair a hydrate-real impl with an ssr no-op stub so views call `act::xxx` ungated.
- Real-time is **client polling + in-memory typing state** (`AppState.typing`), NOT LIVE SELECT (which exists only in tests).

## Invariants you must not break (cited to current code)
- Server identity comes ONLY from the `authlyn_session` cookie, never the request body (`server/auth/session.rs` AuthAccount extractor); DB stores only the SHA-256 of the token.
- Authorization is re-derived on every mutate: guild role from `guild_member` (`permissions.rs` caller_role) + channel membership + persona ownership; never trust stored state.
- Unauthorized collapses to a privacy-404 with an identical body — never reveal existence (`server/access.rs` resolve_membership).
- SQL is parameterized only: user VALUES via `.bind()` / `type::record(...)`; only compile-time consts (`MSG_PROJECTION`), loop indices, or static column fragments are ever spliced into query text (`messages/reading.rs`).
- Media: server-minted random 16-byte id for the on-disk path (`media.rs` random_media_id); GET canonicalizes + `starts_with(media_dir)` (`media.rs` serve_original); image-only MIME allowlist + `X-Content-Type-Options: nosniff`.
- The markup parser must stay panic-free on arbitrary input (`src/markup/tree.rs` — Root frame is never popped/closed).
- Message cursor is composite `(sent_at, id_key)` with strict tie-break; bind datetimes via `type::datetime($p)` — never `<string>`-cast a datetime feeding ORDER BY (`reading.rs` cursor branch; schema header warns of lex-misorder).
- Persona send-path: re-check `can_edit_persona` for BOTH suggested and stored persona on every send (`messages/posting.rs`).
- Soft-delete hides on read AND is immutable to management mutations (`permissions.rs` ensure_guild_live, called by require_manager).
- Persona identity (name/description/color/avatar) is SNAPSHOTTED onto the message row at send; never resolve it live (`posting.rs` persist_message).
- Racy CREATE on a UNIQUE index → `with_write_conflict_retry`, then 409/idempotent, never 500 (`src/server/retry.rs`).
- `is_admin` is fail-closed: empty admin set authorizes no one (`permissions.rs`). Purge cascades to children (`server/mod.rs` purge_soft_deleted).
- SCHEMAFULL NONE-coercion: a field added to a populated table must be `option<>` or get an idempotent UPDATE backfill BEFORE any other UPDATE, or schema apply crash-loops boot.
- `record<...>` links are type annotations only — SurrealDB does NOT enforce referential existence; cleanup/cascade is the app's job.

## Conventions
- **Commits:** Conventional Commits `type(scope): subject` (incl. non-standard `a11y`). Review fixes append `(review F-Dxx-x)` (ids in `docs/CODE-REVIEW-2026-05-29.md`); phased work uses `(Wn/Cn)`. Bodies explain the invariant/finding + fix, end with a `Tests:` line naming new test fns, then trailer `Co-Authored-By: Claude Opus 4.x (1M context)`.
- **Versioning:** CalVer `YYYY.M.D` in `[package].version`; human codename in `[package.metadata.release].codename` (currently `giggly-crescent`). Codename bumping is MANUAL — pick a fresh two-word name.
- **Formatting:** default rustfmt — there is NO `rustfmt.toml`; do not invent project rules.
- **Tests:** integration suites in `tests/*.rs` (14 files), `#[tokio::test]` async fns with long snake_case full-sentence names; shared harness `tests/common/mod.rs` (kept as a subdir `mod.rs` so Cargo does not treat it as its own test binary).
- **Doc-comment density (match this):** every Cargo.toml dependency carries a `#` comment stating purpose + features + ssr-vs-hydrate build-graph constraint; every module has a `//!` header stating scope + feature-gating; public REST fns lead with `/// VERB /path — intent.`.

## Recommended skills & workflow
- Before merging, run **code-review** (or superpowers `requesting-code-review`) + **verification-before-completion** — the invariants above are security-load-bearing, so success claims need a real `cargo test --features ssr` run against a live SurrealDB.
- New work: **test-driven-development** (matches the `tests/*.rs` + `tests/common/mod.rs` harness); **systematic-debugging** on any failure before proposing a fix.
- Keep this file current with **claude-md-management** when commands or structure change.

## Gotchas
- The intentionally-deleted `deploy/`, `scripts/`, `end2end/` tooling must never be restored or re-referenced (the last commit removed it on purpose). Stale references were scrubbed 2026-05-30; reintroducing one is a regression. `.githooks/pre-commit` is now a working opt-in fmt+clippy gate (enable: `git config core.hooksPath .githooks`).
- `src/server/retry.rs` docs were scrubbed of stale Megolm/`server::keys`/`/rooms/{id}/join` language (2026-05-30); they now describe the real authlyn consumers (registration, `guild_member`, `persona_editor`, `friendship`, `custom_emoji`, `channel_active_persona`, `push_subscription`). Do not reintroduce the E2EE-domain wording.
- `.gitignore` whitelists `.claude/`: only `settings.json` is tracked (`/.claude/*` + `!/.claude/settings.json`); everything else, incl. `settings.local.json` and harness worktrees, stays local. To share a new `.claude/` file, add an explicit `!` negation.
- SurrealDB skew: installed binary is 3.0.4 vs SDK `=3.1.0-beta.3` — first suspect on WS handshake errors. The two also emit DIFFERENT write-conflict text ("Transaction write conflict…" vs "Write conflict, retry the transaction…"); `retry::is_write_conflict` matches both (case-insensitive "write conflict" / "can be retried"), pinned by the `tests/retry_canary.rs` canary (review F-D6 live adjudication).
- The "feedback inbox" (admin account-modal) is the `feedback` table; live data is in SurrealDB **ns `authlyn` / db `prod`**, NOT the `.env.example` `dev` default (empty). The surrealdb MCP WS client can't handshake 3.0.4 (the skew above) — read it over HTTP `/sql` (`curl … -H "surreal-ns: authlyn" -H "surreal-db: prod" --data "SELECT … FROM feedback WHERE status!='deleted' ORDER BY created_at DESC;"`) or the `surreal sql` CLI. `GET /feedback` is admin-gated in-app (`AUTHLYN_ADMIN_USERNAMES`); a direct DB read bypasses that gate (read-only triage only).
- Two body-limit route groups (512 KiB JSON, 64 MiB media) are mandatory: `RequestBodyLimitLayer` composes with MIN-limit semantics, and media must also raise axum `DefaultBodyLimit` or uploads silently cap at ~2 MB (`server/mod.rs`).
- Two bin targets exist, so `bin-target = "authlyn-interactive"` in `[package.metadata.leptos]` is load-bearing — removing it makes `cargo leptos build` error with "Several bin targets found".
- Building `ssr` needs system libcurl (web-push) and the `image` crate at build time — native, ssr-only.
