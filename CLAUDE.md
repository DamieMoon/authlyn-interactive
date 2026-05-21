# authlyn-interactive — Claude working notes

## What this is

A self-hosted chat application with end-to-end encryption, reached from the public internet via DDNS. Solo project; Damien is the only developer and tester. He testing the running app mostly remotely (not from the LAN), so anything that only works from `localhost`/`192.168.*` will block him.

## Stack

- **Backend / SSR:** axum + Leptos `ssr` feature
- **Frontend:** Leptos `hydrate` feature (WASM, single crate)
- **Database:** SurrealDB, run as an external server (dev script: `./scripts/dev-db.sh`)
- **E2EE:** Signal-style Double Ratchet via [`vodozemac`](https://crates.io/crates/vodozemac) (Matrix's audited implementation)

Single crate, no workspace. Server-only code (e.g. `src/db.rs`) lives behind `#[cfg(feature = "ssr")]` so it never compiles into the WASM bundle.

## Conventions

- **Rust toolchain:** pinned via `rust-toolchain.toml` to `channel = "stable"` (plus rustfmt + clippy + wasm32 target). Run `cargo fmt --all` before committing; idiomatic Rust naming (`snake_case` fns/vars, `PascalCase` types).
- **Versioning:** CalVer (`YYYY.M.D`). Each release also gets a random codename — generate one with `./scripts/release-name.sh` and set `[package.metadata.release].codename` in `Cargo.toml`.
- **WASM gotcha:** `vodozemac` pulls `getrandom 0.2`, which needs the `js` feature when compiling to `wasm32-unknown-unknown`. The fix lives under `[target.'cfg(target_arch = "wasm32")'.dependencies]` in `Cargo.toml` — leave it there.
- **Lockfile:** `Cargo.lock` is committed (this is an app, not a library).
- **No license file** (private repo, internal use).

## Dev loop

```sh
./scripts/dev-db.sh        # terminal 1 — SurrealDB on 127.0.0.1:8000
cp .env.example .env       # once
cargo leptos watch         # terminal 2 — app on 127.0.0.1:3000
```

## Deployment target

Self-hosted on a Raspberry Pi 4B (8GB), publicly reachable over HTTPS via a TP-Link DDNS hostname; the router forwards ports to the Pi via UPnP. See `CLAUDE.local.md` for the LAN IP, DDNS hostname, username, and SSH key (gitignored).

The Pi runs aarch64 Linux, so production binaries cross-compile from macOS (also aarch64) to `aarch64-unknown-linux-gnu`. Deploy story is not built yet.

### Port-collision rule (hard constraint)

The Pi already runs other services that bind public ports: **xray-core** (proxy, typically 443/80/high) and a **Discord activity** (HTTPS). authlyn-interactive must pick ports that don't collide with anything already in use on the Pi. Before binding a new public port:

1. SSH to the Pi and check `sudo ss -tlnp` for what's already listening.
2. Update `CLAUDE.local.md` with the port chosen and what it's for.

Local dev defaults (`127.0.0.1:3000` for the app, `127.0.0.1:8000` for SurrealDB) are loopback-only and don't conflict, so they stay as the dev defaults. Production ports for the Pi are a separate decision to be made at deploy time.

## Out of scope (Damien to design)

- Chat schema in SurrealDB
- Pre-key bundle exchange + message routing on top of `vodozemac`
- Auth / login
- CI
- Cross-compile + deploy pipeline to the Pi
