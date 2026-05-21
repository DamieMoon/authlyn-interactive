# authlyn-interactive

Rust + Leptos + SurrealDB chat application with end-to-end encryption.

Work in progress.

## Stack

- **Backend / SSR:** axum + Leptos `ssr`
- **Frontend:** Leptos `hydrate` (WASM)
- **Database:** SurrealDB (external server)
- **E2EE:** Signal-style Double Ratchet via [`vodozemac`](https://crates.io/crates/vodozemac)

## Versioning

CalVer: `YYYY.M.D`. Each release also gets a random codename — generate one with
`./scripts/release-name.sh` and set it in `Cargo.toml` under
`[package.metadata.release].codename`.

## Dev

In one terminal, start the database:

```sh
./scripts/dev-db.sh
```

In another, run the app with live reload:

```sh
cp .env.example .env
cargo leptos watch
```

The app serves at <http://127.0.0.1:3000>; SurrealDB at `ws://127.0.0.1:8000`.

## Layout

```
src/
  app.rs       Leptos UI (shared between ssr & hydrate)
  lib.rs       Module wiring + hydrate entrypoint
  main.rs      axum server entrypoint (ssr only)
  db.rs        SurrealDB connection helper (ssr only)
  crypto/      Double Ratchet primitives (vodozemac)
scripts/
  dev-db.sh
  release-name.sh
```
