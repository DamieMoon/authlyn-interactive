# authlyn-interactive — Claude working notes

## What this is

A self-hosted chat application with end-to-end encryption, reached from the public internet via DDNS. Solo project; Damien is the only developer and tester. He tests the running app mostly remotely (not from the LAN), so anything that only works from `localhost`/`192.168.*` will block him.

## Stack

- **Backend / SSR:** axum + Leptos `ssr` feature
- **Frontend:** Leptos `hydrate` feature (WASM, single crate)
- **Database:** SurrealDB, run as an external server (dev script: `./scripts/dev-db.sh`)
- **E2EE:** Signal-style Double Ratchet via [`vodozemac`](https://crates.io/crates/vodozemac) (Matrix's audited implementation)

Single crate, no workspace. Server-only code (e.g. `src/db.rs`) lives behind `#[cfg(feature = "ssr")]` so it never compiles into the WASM bundle.

**Module map.**

- `src/app.rs` — Leptos root component; shared by ssr and hydrate.
- `src/protocol.rs` — shared wire-format DTOs (serde-JSON, no ssr gate).
- `src/crypto/` — vodozemac wrappers: `identity`, `olm`, `megolm`, `prekey`, `pickle` (libolm-compat pickle for at-rest Account encryption), plus `attachment` (AES-256-CTR + SHA-256 + JWK, Matrix `m.encrypted` v2). Built for both ssr and hydrate.
- `src/server/` (ssr-only) — axum routing layer: `keys`, `keyshare`, `rooms`, `messages`, `media`, plus `retry` (SurrealDB write-conflict backoff), `state` (`AppState`), `datetime` (RFC3339 fixed-nanos helper — see gotcha below).
- `src/storage/` (ssr-only) — SurrealDB schema (`schema.surql`) + bootstrap.
- `src/db.rs` (ssr-only) — DB connection + the connect-with-retry wrapper.

## Conventions

- **Rust toolchain:** pinned via `rust-toolchain.toml` to `channel = "stable"` (plus rustfmt + clippy + wasm32 target). Run `cargo fmt --all` before committing; idiomatic Rust naming (`snake_case` fns/vars, `PascalCase` types).
- **Versioning:** CalVer (`YYYY.M.D`). Each release also gets a random codename — generate one with `./scripts/release-name.sh` and set `[package.metadata.release].codename` in `Cargo.toml`.
- **WASM gotcha:** `vodozemac` pulls `getrandom 0.2`, which needs the `js` feature when compiling to `wasm32-unknown-unknown`. The fix lives under `[target.'cfg(target_arch = "wasm32")'.dependencies]` in `Cargo.toml` — leave it there.
- **Lockfile:** `Cargo.lock` is committed (this is an app, not a library).
- **No license file** (private repo, internal use).
- **SurrealDB datetime serialization:** never `<string>` cast in a query that drives an `ORDER BY` or a cursor — the cast produces variable-precision sub-second output that lex-mis-orders rows at format-class boundaries. Project raw `datetime` columns and format on the Rust side via `src/server/datetime.rs::to_rfc3339_fixed`. Background: `surrealdb-string-datetime-cast-quirk` memory entry; commit `d39f892`.
- **SurrealDB SDK pin:** `surrealdb = "=3.1.0-beta.3"` is exact — the WebSocket subprotocol must match the on-machine `surreal` 3.x binary. Don't `cargo update -p surrealdb` blind; bump the binary on the Pi in lockstep.
- **Media storage root:** `$MEDIA_STORAGE_DIR` (defaults to `./media` in dev, `/opt/authlyn/media` on the Pi via the systemd unit's `ReadWritePaths`). `main.rs` creates the dir at startup; `AppState` canonicalizes the path once at construction so the GET path-traversal `starts_with` check is a free comparison. Local `./media/` is gitignored.
- **Matrix `m.encrypted` v2 base64 conventions:** `iv` and `hashes.sha256` are **unpadded** (`STANDARD_NO_PAD`); the JWK `k` field is base64url-no-pad. Padded base64 here silently breaks wire compat with every other Matrix client. The convention lives in `src/crypto/attachment.rs:55-60`; the spec reviewer for step 9 caught this against MSC1420 + matrix-js-sdk.

## Dev loop

```sh
./scripts/dev-db.sh        # terminal 1 — SurrealDB on 127.0.0.1:8000
cp .env.example .env       # once
cargo leptos watch         # terminal 2 — app on 127.0.0.1:3000
```

Integration tests in `tests/` hit a real SurrealDB — keep `./scripts/dev-db.sh` running while you `cargo test`. Each test reserves its own namespace via `tests/common::arena` so parallel runs don't collide.

## Deployment target

Self-hosted on a Raspberry Pi 4B (8GB), publicly reachable over HTTPS via a TP-Link DDNS hostname; the router forwards ports to the Pi via UPnP. The Pi runs aarch64 Linux, so production binaries cross-compile from macOS (also aarch64) to `aarch64-unknown-linux-gnu`. The pipeline is live — see *Branching and auto-deploy* below.

Pi-specific machine state (LAN IP, DDNS hostname, SSH alias, port-collision rules, currently chosen ports) lives in the project memory entry [`pi-deployment`](../../.claude/projects/-Users-damien-Developer-authlyn-interactive/memory/pi-deployment.md) and is loaded automatically at session start via `MEMORY.md`.

### Port-collision rule (hard constraint)

The Pi already runs other services that bind public ports (xray-core, a Discord activity). Before binding a new public port:

1. SSH to the Pi and check `sudo ss -tlnp` for what's already listening.
2. Update the `pi-deployment` memory entry with the port chosen and what it's for.

Local dev defaults (`127.0.0.1:3000` for the app, `127.0.0.1:8000` for SurrealDB) are loopback-only and don't conflict, so they stay as the dev defaults. Production ports are a separate decision at deploy time.

## Branching and auto-deploy

- `main` is the working branch. Commits land here freely; nothing deploys.
- `release` is the deploy target. Promote with `git push origin main:release` when a batch of commits is ready to ship.
- CI (`.github/workflows/build-release.yml`) builds on every push to `release` and updates the rolling `latest` GitHub Release. The Pi-side timer (`deploy/authlyn-updater.timer`) polls every 5 minutes and atomic-swaps + restarts on SHA change.
- Rolling back a bad ship: `git push origin <good-sha>:release --force-with-lease`. CI re-runs against the good commit and the puller picks it up on the next tick.
- Pi-side machine state (chosen ports, install layout) lives in the project memory entry `pi-deployment`.

## Current status

Landed: SurrealDB schema + routing plan steps 1–9 (key upload/claim, room create/join/leave, keyshare deposit/inbox, message send + LIVE-select receive, encrypted attachments via `crypto::attachment` + `server::media`), CI cross-compile pipeline, Pi auto-deploy. Live origin serves these as of 2026-05-23.

Still open:
- **Routing plan step 10** — Leptos UI smoke replacing the stock welcome page (`src/app.rs` is still `cargo leptos new` boilerplate). Step 10 is what wires the routing protocol end-to-end into the browser; without it, the live origin's `/` page still renders "Welcome to Leptos!".
- **Auth / login** — `server::keys` and the rest still run against the v1 device-ID-header stub (see the comment at the top of `keys.rs`). No follow-up plan written yet; design + impl are both open.
- **Streaming media** — `server::media` buffers the full ciphertext (≤ 16 MiB per the per-route cap) in RAM per request. ~500 concurrent uploads = the 8 GB Pi's memory ceiling. Acceptable for the 1 MiB v1 acceptance target and single-user workload; switch to `tokio::fs::File` + `Body::from_stream` when those assumptions stop holding.

Active plan/spec docs live in `docs/superpowers/{plans,specs}/`.
