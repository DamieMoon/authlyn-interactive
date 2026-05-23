#!/usr/bin/env bash
# Pre-commit gate: format check + clippy on both compile targets, then the no-remnants guard.
# Lints native (ssr) and wasm (hydrate) because the deploy ships both and CI runs no lints.
# Runnable by hand; the .githooks/pre-commit shim execs this. Bypass a commit with --no-verify.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

echo "[precommit] cargo fmt --check"
cargo fmt --all -- --check

echo "[precommit] clippy (ssr / native)"
cargo clippy --no-default-features --features ssr --all-targets -- -D warnings

echo "[precommit] clippy (hydrate / wasm32)"
cargo clippy --no-default-features --features hydrate --target wasm32-unknown-unknown -- -D warnings

echo "[precommit] no-remnants guard"
./scripts/check-no-remnants.sh

echo "[precommit] all checks passed"
