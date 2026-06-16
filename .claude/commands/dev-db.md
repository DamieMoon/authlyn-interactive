---
name: dev-db
description: Start the local dev SurrealDB (in-memory, root/root, ws://127.0.0.1:8000)
---
Check whether a SurrealDB is already listening on `127.0.0.1:8000` (e.g. `curl -s http://127.0.0.1:8000/health`). If one is already up, say so and stop.

If not, start it in the background:

`surreal start --user root --pass root --bind 127.0.0.1:8000 memory`

This is the dev database the app and the `cargo test --features ssr` suite connect to (root/root, namespace defaults `authlyn`/`dev`; the test harness isolates its own per-worker namespaces).
