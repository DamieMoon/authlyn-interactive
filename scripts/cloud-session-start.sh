#!/usr/bin/env bash
# =============================================================================
# cloud-session-start.sh — SessionStart bootstrap for Claude Code web sessions
# =============================================================================
#
# Wired via .claude/settings.json (SessionStart hook, matcher startup|resume),
# so it runs at the start of EVERY Claude Code session — local AND cloud.
#
# It no-ops locally (guarded on CLAUDE_CODE_REMOTE) so it never disturbs the
# dev machine. In a Claude Code *web* cloud session it does the two things a
# fresh clone needs that aren't carried over:
#
#   1. core.hooksPath — repo-local git config (set once by hand locally via
#      `git config core.hooksPath .githooks`) is NOT part of the clone, so the
#      pre-commit gate would be inactive in the cloud. Re-point it here.
#
#   2. SurrealDB — not pre-installed in the base image and not running. Start it
#      exactly as scripts/dev-db.sh does (in-memory, root/root, 127.0.0.1:8000).
#      The code reads SURREAL_* via std::env with fallbacks that match these
#      values (src/db.rs, tests/common/mod.rs), so NO .env / env-vars are needed.
#
# Heavy work (toolchain, cargo-leptos, the SurrealDB binary, the Playwright
# stack) is installed by the environment's cached *setup script*, not here —
# see ~/Downloads/authlyn-cloud-setup.md. e2e is run on demand by
# scripts/cloud-e2e.sh, not from this per-session hook.
# =============================================================================

# No `set -e`: a hook must never abort a session start. Best-effort throughout.
set -uo pipefail

# Cloud sessions set CLAUDE_CODE_REMOTE=true. Do nothing locally.
[ "${CLAUDE_CODE_REMOTE:-}" != "true" ] && exit 0

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}" || exit 0

# (1) Activate the project's pre-commit gate (mirrors the local .githooks wiring).
git config core.hooksPath .githooks || true

# (2) Start an in-memory SurrealDB on 127.0.0.1:8000 if nothing answers there.
if ! curl -fsS http://127.0.0.1:8000/health >/dev/null 2>&1; then
  echo "[cloud-session-start] starting SurrealDB on 127.0.0.1:8000 (in-memory)…"
  nohup surreal start --user root --pass root --bind 127.0.0.1:8000 memory \
    >/tmp/surreal.log 2>&1 &
  for _ in $(seq 1 30); do
    if curl -fsS http://127.0.0.1:8000/health >/dev/null 2>&1; then
      echo "[cloud-session-start] SurrealDB ready."
      break
    fi
    sleep 1
  done
fi

exit 0
