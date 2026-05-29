#!/usr/bin/env bash
# Playwright E2E for authlyn. novahome is headless (no browser/GUI libs on the
# host), so tests run inside the official Playwright container, which carries
# Node + browsers + all system deps. This repo's end2end/ is mounted in;
# --network host lets the tests reach the app on localhost. Set SMOKE_BASE to
# point at the target (defaults to the dev server on :3000).
#
# Usage:  scripts/e2e.sh [playwright test args]
#   e.g.  scripts/e2e.sh --project=chromium
#         SMOKE_BASE=https://<ddns-host>/ scripts/e2e.sh
#
# IMAGE tag MUST match end2end/package.json @playwright/test version.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="mcr.microsoft.com/playwright:v1.44.1-jammy"
exec docker run --rm --network host \
  -v "$REPO/end2end:/work" -w /work \
  -e "SMOKE_BASE=${SMOKE_BASE:-http://127.0.0.1:3000/}" \
  "$IMAGE" sh -c 'if [ ! -d node_modules ]; then npm ci; fi; npx playwright test "$@"' sh "$@"
