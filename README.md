# authlyn-interactive

Self-hosted, server-trusted roleplay chat platform — Discord + SillyTavern style: guilds → channels, personas, lorebooks, friends. Single Rust crate.

Work in progress. Private / internal use.

## Stack

- **Backend / SSR:** axum + Leptos 0.8 (`ssr`)
- **Frontend:** Leptos 0.8 (`hydrate`, WASM)
- **Database:** SurrealDB (external server)
- **Auth:** session cookies (argon2 password hashing); no browser-side cryptography

## Versioning

CalVer: `YYYY.M.D`. Each release also gets a random two-word codename — pick one
manually and set it in `Cargo.toml` under `[package.metadata.release].codename`.

## Dev

In one terminal, start the database:

```sh
surreal start --user root --pass root --bind 127.0.0.1:8000 memory
```

In another, run the app with live reload:

```sh
cp .env.example .env
cargo leptos watch
```

The app serves at <http://127.0.0.1:3000>; SurrealDB at `ws://127.0.0.1:8000`.

Optional pre-commit gate (fmt + clippy), off by default — enable per-clone with `git config core.hooksPath .githooks`.

## Layout

```
src/
  app.rs           Leptos root (shared ssr & hydrate)
  lib.rs           module wiring + hydrate entrypoint
  main.rs          axum server entrypoint (ssr only)
  db.rs            SurrealDB connection helper (ssr only)
  protocol.rs      shared REST JSON DTOs
  markup.rs        chat markup rendering
  client/          hydrate-only REST client (gloo-net)
  server/          axum routes (ssr only): auth, guilds, personas,
                   messages, lorebook, emoji, friends, media, push, feedback
  storage/         SurrealDB schema (schema.surql)
  ui/              Leptos UI: auth + shell/ (Discord-style app shell)
  bin/nova-mcp.rs  standalone MCP bridge (optional `nova` feature)
```
