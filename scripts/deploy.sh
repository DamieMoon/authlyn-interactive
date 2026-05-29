#!/usr/bin/env bash
# =============================================================================
# deploy.sh — deploy authlyn to novahome from the Mac (one command)
# =============================================================================
#
# THE WORKFLOW THIS FITS INTO
#   Edit here on the Mac -> commit (the .githooks/pre-commit gate runs fmt +
#   clippy on both targets + the no-remnants check) -> push to origin -> run this.
#   This script is the last step: it makes novahome build origin/main and swap it
#   into the live service.
#
#   This is the MANUAL deploy path (Mac -> novahome over ssh). Autodeploy also
#   exists now: a `v*` version-tag push runs scripts/novahome-deploy.sh on a
#   self-hosted runner ON novahome (.github/workflows/deploy-novahome.yml, added
#   2026-05-29). Running this script IS a manual deploy of origin/main; both
#   paths drive the same engine (novahome-deploy.sh).
#
# HOW IT WORKS
#   The real work is in scripts/novahome-deploy.sh, run ON novahome (the build is
#   linux/x86_64 + cargo-leptos; the Mac can't produce the binary). This wrapper
#   does the Mac-side preflight, then runs that script remotely — see the ssh line
#   at the bottom for how the script is delivered and why it's piped over stdin.
#
# USAGE
#   ./scripts/deploy.sh                 # deploy origin/main (must already be pushed)
#   ./scripts/deploy.sh --push          # push main -> origin first, then deploy
#   ./scripts/deploy.sh --ref <gitref>  # deploy a specific ref on novahome
#   ./scripts/deploy.sh --skip-build    # re-swap novahome's existing build (no rebuild)
#   ./scripts/deploy.sh --help          # print this header
# =============================================================================

set -euo pipefail

# Resolve this script's REAL directory (following symlinks) so the command works
# from any cwd AND when invoked via a PATH symlink like `authlyn-deploy`. The repo
# root is the parent of scripts/; cd there so the git commands and the relative
# `< scripts/novahome-deploy.sh` path below are stable.
SOURCE="${BASH_SOURCE[0]}"
while [ -L "$SOURCE" ]; do
  dir="$(cd -P "$(dirname "$SOURCE")" >/dev/null 2>&1 && pwd)"
  SOURCE="$(readlink "$SOURCE")"
  case "$SOURCE" in /*) ;; *) SOURCE="$dir/$SOURCE" ;; esac   # relative symlink -> absolute
done
SCRIPT_DIR="$(cd -P "$(dirname "$SOURCE")" >/dev/null 2>&1 && pwd)"
cd "$SCRIPT_DIR/.." || { echo "[deploy] cannot find repo root from $SCRIPT_DIR" >&2; exit 1; }

HOST=damien@novahome              # ssh target (resolves via ssh config / DNS)
REMOTE_SCRIPT=scripts/novahome-deploy.sh

# ---- args (flags here are forwarded to the remote script) -------------------
PUSH=0
REMOTE_ARGS=""   # plain string, not an array: safe under macOS bash 3.2 + `set -u`
while [ $# -gt 0 ]; do
  case "$1" in
    --push)       PUSH=1; shift ;;
    --ref)        REMOTE_ARGS="$REMOTE_ARGS --ref ${2:?--ref needs a value}"; shift 2 ;;
    --skip-build) REMOTE_ARGS="$REMOTE_ARGS --skip-build"; shift ;;
    -h|--help)    awk 'NR==1{next} /^#/{print; next} {exit}' "$0"; exit 0 ;;   # print the leading header comment block
    *) echo "[deploy] unknown arg: $1" >&2; exit 2 ;;
  esac
done

# The deploy ships origin/main, NOT your working tree — so uncommitted edits won't
# go out. Just a heads-up; not fatal.
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "[deploy] note: working tree has uncommitted changes — the deploy ships origin/main, not these."
fi

if [ "$PUSH" -eq 1 ]; then
  echo "[deploy] pushing main -> origin"
  git push origin main
fi

# Safety: if local main has commits not yet on origin, the deploy (which builds
# origin/main) would silently ship STALE code. Refuse, and say how to fix.
git fetch --quiet origin
ahead="$(git rev-list --count origin/main..main 2>/dev/null || echo 0)"
if [ "$ahead" -gt 0 ]; then
  echo "[deploy] ERROR: local main is $ahead commit(s) ahead of origin/main." >&2
  echo "[deploy]        push first (git push origin main) or re-run with --push." >&2
  exit 1
fi

echo "[deploy] triggering build+deploy on $HOST …"
# Deliver the remote script over stdin to `bash -s` so the CURRENT local copy
# always runs (even uncommitted), rather than whatever stale copy is checked out
# on novahome. Flags after `--` become the remote script's positional args ($@).
#
# Deliberately NOT `ssh -n`: -n points stdin at /dev/null, which would starve the
# pipe and the remote `bash -s` would read an empty script. (The `ssh -n` rule is
# for the OPPOSITE case — `ssh host 'cmd'` forms where a heredoc/loop on the
# caller's stdin would otherwise be swallowed.)
ssh "$HOST" "bash -s --$REMOTE_ARGS" < "$REMOTE_SCRIPT"
