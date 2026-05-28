# authlyn-interactive — orientation map

> Thin, stable map only. Canonical knowledge lives in **ctx** (`ctx query …`) — do not duplicate it here. Behavior-steering lives in built-in memory. The permission classifier reads this file, so policy stated here has teeth.

## What this is
Self-hosted, **server-trusted** roleplay chat platform (Discord + SillyTavern style). Solo project (Damien). Tested **remotely on novahome** via DDNS — novahome is the only runtime test path; anything that only works on localhost/LAN won't help.

## Start here
- Orient each session: `ctx query "authlyn current status"`.
- Knowledge → ctx (`ctx query` / `ctx save`). The SessionStart hook injects a project brief, but ctx is the source of truth — query it directly when you need anything beyond the brief.

## Stack (thin — canonical static map: `docs/ARCHITECTURE.md`)
Single Rust crate: axum + Leptos 0.8 (ssr + hydrate), SurrealDB (external). Server code behind `#[cfg(feature = "ssr")]`; browser client = gloo-net REST + cookie auth. See `docs/ARCHITECTURE.md` for crate layout, request lifecycle, data model, and the 15-point invariant gate.

## Hard constraints
- **Deploy:** no auto-deploy *pipeline* (user decision 2026-05-25). Canonical deploy = **`authlyn-deploy`** (= `./scripts/deploy.sh`; manual, agent-runnable; `--help` for usage; (re)install the command via `scripts/install-deploy-command.sh`). What it automates / by-hand fallback: `ctx query "novahome deploy commands"`.
- **Port-collision:** before binding a new public port on novahome, SSH and check `ss -tlnp`; record the port in ctx.
- **Lint gate:** `./scripts/precommit.sh` is the only lint gate (CI runs none) — keep it green. It does NOT compile SCSS; use `cargo leptos build` for that.
- **SurrealDB SDK** pinned `=3.1.0-beta.3` — don't bump blind. Never `<string>`-cast a datetime feeding `ORDER BY`/cursor (`ctx query "surrealdb datetime ordering"`).

## Map → repo (canonical static map)
`docs/ARCHITECTURE.md` — crate layout · three feature sets · request lifecycle · data model · 15-point invariant gate · conventions. This is the durable structural reference; ctx holds the living/episodic layer below.

## Map → ctx (query by topic)
status/backlog · novahome machine state & deploy · pivot/decision history · per-topic gotchas not covered in `docs/ARCHITECTURE.md`.
