#!/usr/bin/env bash
# =============================================================================
# cloud-e2e.sh — run the Playwright end-to-end suite in a cloud session
# =============================================================================
#
# On-demand (NOT a per-session hook — too heavy): invoke explicitly with
#   bash scripts/cloud-e2e.sh
#
# Self-contained full-stack e2e: builds the app, ensures SurrealDB + the app
# server are up, installs the Playwright stack, runs end2end/. The smoke test
# (end2end/tests/roleplay-smoke.spec.ts) defaults to http://127.0.0.1:3000/,
# which is the Leptos site-addr from Cargo.toml [package.metadata.leptos].
#
# Mirrors the manual local e2e flow (start app, then run Playwright) but
# orchestrated end to end so a cloud agent can run it unattended.
# =============================================================================
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# 1. Build the full app: SSR server binary + hydrate wasm + SCSS -> target/site.
echo "[cloud-e2e] cargo leptos build…"
cargo leptos build

# 2. Ensure SurrealDB is up (the SessionStart hook normally already started it).
if ! curl -fsS http://127.0.0.1:8000/health >/dev/null 2>&1; then
  echo "[cloud-e2e] starting SurrealDB…"
  nohup surreal start --user root --pass root --bind 127.0.0.1:8000 memory \
    >/tmp/surreal.log 2>&1 &
  for _ in $(seq 1 30); do
    curl -fsS http://127.0.0.1:8000/health >/dev/null 2>&1 && break
    sleep 1
  done
fi

# 3. Launch the app on 127.0.0.1:3000 in the background; always tear it down.
echo "[cloud-e2e] launching app on 127.0.0.1:3000…"
cargo leptos serve >/tmp/authlyn-e2e.log 2>&1 &
APP_PID=$!
trap 'kill "$APP_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
  curl -fsS http://127.0.0.1:3000/ >/dev/null 2>&1 && break
  sleep 1
done

# 4. Install the Playwright stack (browsers + OS deps) and run the suite.
echo "[cloud-e2e] installing + running Playwright…"
cd end2end
npm ci
npx playwright install --with-deps chromium firefox webkit
npx playwright test
