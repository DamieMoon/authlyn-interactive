# authlyn-interactive — Claude working notes

## What this is

A self-hosted roleplay chat application with end-to-end encryption, reached from the public internet via DDNS. Solo project; Damien is the only developer and tester. He tests the running app mostly remotely (not from the LAN), so anything that only works from `localhost`/`192.168.*` will block him.

## Persistence

Durable project knowledge (architecture decisions, gotchas, bugs, machine state, status) lives in **ctx** — the PostgreSQL/pgvector knowledge store reached over the `ctx` MCP tools (or the `ctx` CLI: `ctx query "…"`, `ctx save …`). Built-in memory (`MEMORY.md`, `memory/`) is reserved for behavior-steering only. Knowledge lives in exactly one place — don't duplicate between ctx and memory. This file (`CLAUDE.md`) stays a stable, repo-checked-in orientation map; volatile specifics belong in ctx.

## Stack

- **Backend / SSR:** axum + Leptos `ssr` feature
- **Frontend:** Leptos `hydrate` feature (WASM, single crate)
- **Database:** SurrealDB, run as an external server (dev script: `./scripts/dev-db.sh`)
- **E2EE:** Signal-style Double Ratchet via [`vodozemac`](https://crates.io/crates/vodozemac) (Matrix's audited implementation)

Single crate, no workspace. Server-only code (e.g. `src/db.rs`) lives behind `#[cfg(feature = "ssr")]` so it never compiles into the WASM bundle.

**Module map.**

- `src/app.rs` — Leptos root component; shared by ssr and hydrate.
- `src/client/` (hydrate-only) — browser-side E2EE client (`api`, `session`, `store`); added in step 10.
- `src/protocol.rs` — shared wire-format DTOs (serde-JSON, no ssr gate).
- `src/crypto/` — vodozemac wrappers: `identity`, `olm`, `megolm`, `prekey`, `pickle` (libolm-compat pickle for at-rest Account encryption), plus `attachment` (AES-256-CTR + SHA-256 + JWK, Matrix `m.encrypted` v2). Built for both ssr and hydrate.
- `src/server/` (ssr-only) — axum routing layer: `keys`, `keyshare`, `rooms`, `messages`, `media`, plus `retry` (SurrealDB write-conflict backoff), `state` (`AppState`), `datetime` (RFC3339 fixed-nanos helper — see gotcha below).
- `src/storage/` (ssr-only) — SurrealDB schema (`schema.surql`) + bootstrap.
- `src/db.rs` (ssr-only) — DB connection + the connect-with-retry wrapper.

## Conventions

- **Rust toolchain:** pinned via `rust-toolchain.toml` to `channel = "stable"` (plus rustfmt + clippy + wasm32 target). Run `cargo fmt --all` before committing; idiomatic Rust naming (`snake_case` fns/vars, `PascalCase` types).
- **Pre-commit lint gate:** `./scripts/precommit.sh` runs `cargo fmt --check` + clippy on **both** targets (ssr native + hydrate wasm, `-D warnings`) + the no-remnants guard. Wire it once per clone with `git config core.hooksPath .githooks`; bypass a WIP commit with `git commit --no-verify`. CI builds but runs **no** lints, so this hook is the only lint gate — keep it green or `release` ships unlinted.
- **Versioning:** CalVer (`YYYY.M.D`). Each release also gets a random codename — generate one with `./scripts/release-name.sh` and set `[package.metadata.release].codename` in `Cargo.toml`.
- **WASM gotcha:** `vodozemac` pulls `getrandom 0.2`, which needs the `js` feature when compiling to `wasm32-unknown-unknown`. The fix lives under `[target.'cfg(target_arch = "wasm32")'.dependencies]` in `Cargo.toml` — leave it there.
- **Lockfile:** `Cargo.lock` is committed (this is an app, not a library).
- **No license file** (private repo, internal use).
- **SurrealDB datetime serialization:** never `<string>` cast in a query that drives an `ORDER BY` or a cursor — the cast produces variable-precision sub-second output that lex-mis-orders rows at format-class boundaries. Project raw `datetime` columns and format on the Rust side via `src/server/datetime.rs::to_rfc3339_fixed`. Background: commit `d39f892`; full write-up in ctx (`ctx query "surrealdb datetime cast ordering"`).
- **SurrealDB SDK pin:** `surrealdb = "=3.1.0-beta.3"` is exact — the WebSocket subprotocol must match the on-machine `surreal` 3.x binary. Don't `cargo update -p surrealdb` blind; bump the on-host `surreal` binary in lockstep (novahome runs `v3.0.4`).
- **Media storage root:** `$MEDIA_STORAGE_DIR` (defaults to `./media` in dev, `/data/authlyn/media` on novahome via the systemd unit's `ReadWritePaths`). `main.rs` creates the dir at startup; `AppState` canonicalizes the path once at construction so the GET path-traversal `starts_with` check is a free comparison. Local `./media/` is gitignored.
- **Matrix `m.encrypted` v2 base64 conventions:** `iv` and `hashes.sha256` are **unpadded** (`STANDARD_NO_PAD`); the JWK `k` field is base64url-no-pad. Padded base64 here silently breaks wire compat with every other Matrix client. The convention lives in `src/crypto/attachment.rs:55-60`; the spec reviewer for step 9 caught this against MSC1420 + matrix-js-sdk.

## Dev loop

```sh
./scripts/dev-db.sh        # terminal 1 — SurrealDB on 127.0.0.1:8000
cp .env.example .env       # once
cargo leptos watch         # terminal 2 — app on 127.0.0.1:3000
```

Integration tests in `tests/` hit a real SurrealDB — keep `./scripts/dev-db.sh` running while you `cargo test`. Each test reserves its own namespace via `tests/common::arena` so parallel runs don't collide.

## Deployment target

authlyn runs on **novahome** (LAN `192.168.0.239`, x86-64, Ubuntu), public at `https://authlyn.tplinkdns.com:8444` (the router forwards external `:8444` → novahome). Built **natively on novahome** — no cross-compile. Runtime: systemd `surrealdb` + `authlyn` (binds `127.0.0.1:8081`) behind Caddy (`:8444`, explicit cert); DB + media on the `/data` volume (`/data/surrealdb`, `/data/authlyn/media`). Deploy units live in `deploy/novahome/`. Machine state: `ctx query "novahome homelab server"` (`019e5e13`); full layout + migration record: `ctx query "authlyn novahome migration"` (`019e5e7d`).

**TLS cert.** novahome can't run ACME for `authlyn.tplinkdns.com` (public `:80`/`:443` route to the Pi), so it **reuses the Pi's Let's Encrypt cert** — a nightly timer (`authlyn-cert-sync.timer`) pulls it from the Pi over SSH (forced-command, read-only key) and reloads Caddy. The Pi keeps renewing it because a co-hosted Discord-activity site still serves the same hostname on `:8443`.

**The Pi (FENRIR, `192.168.0.153`)** now runs **xray-core only** (VLESS+Reality on `:443`, uses no cert) plus a Discord activity app (`:8443`) — leave it untouched. State: `ctx query "authlyn pi deployment machine state"`. Its old authlyn deploy (aarch64 cross-compile CI + the GitHub-Release puller in `deploy/`) is **superseded**.

### Port-collision rule (hard constraint)

Both boxes run services on public ports (Pi: xray `:443`, a Discord activity `:8443`, Caddy `:80`; novahome: authlyn Caddy `:8444`). Before binding a new public port on either:

1. SSH to the box and check `sudo ss -tlnp` for what's already listening.
2. Record the port chosen and what it's for in ctx (that box's machine-state block).

Local dev defaults (`127.0.0.1:3000` for the app, `127.0.0.1:8000` for SurrealDB) are loopback-only and don't conflict, so they stay as the dev defaults. Production ports are a separate decision at deploy time.

## Branching and deploy

- `main` is the working branch; commits land here freely.
- **Deploy is manual on novahome:** `git -C ~/authlyn-interactive pull` → `cargo leptos build --release` → swap the binary + `site/` into `/opt/authlyn` → `systemctl restart authlyn`. Ensure the `site/pkg/authlyn-interactive_bg.wasm` alias exists (leptos hydration gotcha — copy it from `authlyn-interactive.wasm` if absent).
- The old `release` → `build-release.yml` (aarch64) → Pi-puller pipeline still exists but is **superseded** for authlyn: nothing on novahome consumes it, so pushing to `release` no longer deploys authlyn. Open decision: retire it, or repoint CI to x86-64 and add a novahome pull/rebuild timer.

## Current status

Status (landed steps, open work) is volatile, so it lives in ctx as the single source of truth — `ctx query "authlyn current status"` (block `019e5426`). Don't track it here; this file is the stable orientation map. Active plan/spec docs live in `docs/superpowers/{plans,specs}/`.
