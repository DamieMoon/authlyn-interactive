#!/usr/bin/env bash
# =============================================================================
# install-deploy-command.sh — install the `authlyn-deploy` shell command
# =============================================================================
#
# Symlinks scripts/deploy.sh into a directory on your PATH, so you can run
# `authlyn-deploy` from anywhere instead of `./scripts/deploy.sh` from the repo.
# deploy.sh resolves the repo from its own (real) location, so the symlink Just
# Works regardless of where you invoke it. Idempotent — safe to re-run (e.g.
# after moving the repo, which refreshes the link target).
#
# Target dir: $AUTHLYN_BIN_DIR if set, else ~/.local/bin (created if missing).
#
# Usage:
#   ./scripts/install-deploy-command.sh
#   AUTHLYN_BIN_DIR=/opt/homebrew/bin ./scripts/install-deploy-command.sh
# =============================================================================
set -euo pipefail

# This script's real dir is repo/scripts; deploy.sh sits right beside it.
SCRIPT_DIR="$(cd -P "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
TARGET_SCRIPT="$SCRIPT_DIR/deploy.sh"
[ -f "$TARGET_SCRIPT" ] || { echo "[install] deploy.sh not found beside this script" >&2; exit 1; }

BIN_DIR="${AUTHLYN_BIN_DIR:-$HOME/.local/bin}"
LINK="$BIN_DIR/authlyn-deploy"

mkdir -p "$BIN_DIR"
ln -sf "$TARGET_SCRIPT" "$LINK"          # -f: refresh an existing link
echo "[install] linked $LINK -> $TARGET_SCRIPT"

# The command is only usable once its dir is on PATH; warn (don't fail) if not.
case ":$PATH:" in
  *":$BIN_DIR:"*) echo "[install] $BIN_DIR is on your PATH — try: authlyn-deploy --help" ;;
  *) echo "[install] NOTE: $BIN_DIR is NOT on your PATH. Add it, e.g.:" >&2
     echo "          echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zshrc && exec zsh" >&2 ;;
esac
