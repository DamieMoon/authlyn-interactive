# authlyn-interactive — orientation map

> Thin map: this file points, it doesn't hold knowledge. Project knowledge lives in **ctx** (`ctx query …`); durable structure in `docs/ARCHITECTURE.md`. The permission classifier reads this file, so policy stated here has teeth.

## What this is
Self-hosted, **server-trusted** roleplay chat platform (Discord + SillyTavern style). Solo project (Damien). Runtime test target = novahome's public DDNS HTTPS endpoint; a change that only works on localhost/LAN won't validate there. (Claude Code runs on novahome, so `localhost` == novahome — ctx 019e730c.)

## Start here
- Project status: `ctx query "authlyn current status"`.
- Structure & invariants: `docs/ARCHITECTURE.md`.

## Stack (canonical static map: `docs/ARCHITECTURE.md`)
Single Rust crate: axum + Leptos 0.8, three feature sets ssr + hydrate + nova, SurrealDB (external). Server code behind `#[cfg(feature = "ssr")]`; browser client = gloo-net REST + cookie auth. See `docs/ARCHITECTURE.md` for crate layout, feature sets, request lifecycle, data model, and the 15-point invariant gate.

## Hard constraints
- **Deploy** — manual `authlyn-deploy` (= `./scripts/deploy.sh`; agent-runnable; `--help`; (re)install via `scripts/install-deploy-command.sh`) is canonical. Autodeploy (GitHub Action) decided 2026-05-29 but NOT yet live (ctx 019e70a4). By-hand fallback: `ctx query "novahome deploy commands"`.
- **Port-collision** — before binding a new public port on novahome, SSH and check `ss -tlnp`; record the port in ctx. (novahome is the only host — Pi decommissioned, ctx 019e5e7d.)
- **Lint gate** — keep `./scripts/precommit.sh` green; it's the only lint gate (no CI). It does NOT compile SCSS — use `cargo leptos build`.
- **SurrealDB SDK** — pinned `=3.1.0-beta.3`; don't bump blind. Never `<string>`-cast a datetime feeding `ORDER BY`/cursor (`ctx query "surrealdb datetime ordering"`).

## Map → ctx (query by topic)
status/backlog · novahome machine state & deploy · pivot/decision history · per-topic gotchas not in `docs/ARCHITECTURE.md`.
