---
name: check
description: Run the project quality gate — rustfmt + clippy on the ssr + hydrate graphs (the freya/native graph is a separate manual check)
---
Run the project's quality gate and report results concisely. Source `~/.cargo/env` first if `cargo` isn't on PATH.

1. `cargo fmt --all --check`
2. `cargo clippy --features ssr --no-deps -- -D warnings` (server graph)
3. `cargo clippy --features hydrate --target wasm32-unknown-unknown --no-deps -- -D warnings` (browser/WASM graph — disjoint deps)

This mirrors the opt-in `.githooks/pre-commit` gate. Summarize any failures with the offending `file:line`.
