#!/usr/bin/env bash
# =============================================================================
# novahome-deploy.sh — build + deploy authlyn ON novahome
# =============================================================================
#
# WHERE THIS RUNS
#   On novahome itself. Normally invoked by scripts/deploy.sh (from the Mac),
#   which pipes this file to `bash -s` over ssh — so the latest copy always runs,
#   even before it's committed. You can also run it directly while ssh'd in:
#       bash ~/authlyn-interactive/scripts/novahome-deploy.sh
#
# WHY IT EXISTS / THE TRIGGER MODEL
#   This is the deploy engine, invoked three ways: the manual `authlyn-deploy`
#   (scripts/deploy.sh, Mac -> novahome over ssh), an agent on request, AND the
#   tag-push autodeploy (.github/workflows/deploy-novahome.yml) on a self-hosted
#   runner ON novahome. The 2026-05-25 "no auto-deploy pipeline" decision was
#   tactical (autodeploy was just not carried over from the old Pi at the novahome
#   migration), and was revised 2026-05-29 to add the `v*`-tag-push path.
#
# WHY THE BUILD HAPPENS HERE (not on the Mac)
#   The server binary is linux/x86_64 and the front bundle is built by
#   cargo-leptos; the Mac is darwin/arm64 and can't produce the deployed binary.
#   So novahome builds natively from its own deploy checkout of origin/main.
#
# DOWNTIME MODEL
#   `cargo leptos build --release` runs BEFORE the service is stopped, so a
#   broken build aborts with ZERO downtime. The actual outage is only the
#   stop -> copy -> start window (a few seconds).
#
# ROLLBACK MODEL
#   Before the swap we copy the live binary + site/ to *.bak. If anything fails
#   AFTER the service is stopped (bad copy, crash-on-boot, failed healthcheck),
#   the ERR trap restores the *.bak artifacts and restarts — a bad deploy can
#   never leave prod down. Failures BEFORE the stop leave the service untouched.
#
# SCHEMA
#   The SurrealDB schema is embedded in the binary (storage/mod.rs include_str!)
#   and applied idempotently on every boot (all DEFINEs use IF NOT EXISTS), so a
#   normal deploy needs no schema step. Only *destructive* migrations (dropping/
#   retyping existing SCHEMAFULL fields) need a manual `surreal sql` step first —
#   intentionally NOT automated here, so the script can't silently lose data.
#
# USAGE
#   novahome-deploy.sh                 # build + deploy origin/main
#   novahome-deploy.sh --ref <gitref>  # deploy a specific ref/branch/tag/sha
#   novahome-deploy.sh --skip-build    # re-swap the existing target/ artifacts
# =============================================================================

set -euo pipefail

# ---- paths / names ----------------------------------------------------------
REPO="$HOME/authlyn-interactive"   # novahome's deploy checkout (tracks origin/main)
OPT=/opt/authlyn                   # where the service runs (systemd WorkingDirectory)
SVC=authlyn                        # systemd unit name
APP_USER=authlyn                   # service user/group; artifacts are chown'd to it
BIN="$OPT/authlyn-interactive"
BIN_BAK="$OPT/authlyn-interactive.bak"
SITE="$OPT/site"
SITE_BAK="$OPT/site.bak"

# ---- args -------------------------------------------------------------------
REF=origin/main      # what to deploy; override with --ref
SKIP_BUILD=0         # --skip-build re-uses whatever is already in target/
while [ $# -gt 0 ]; do
  case "$1" in
    --ref)        REF="${2:?--ref needs a value}"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    *) echo "[deploy] unknown arg: $1" >&2; exit 2 ;;
  esac
done

OLD=""; NEW=""   # short SHAs, filled in after fetch/reset (used in messages + rollback)
STAGE=init       # how far we got; the rollback trap keys off this

# ---- helpers ----------------------------------------------------------------

# Resolve the live HTTP listen address from the service env file. The unit loads
# /opt/authlyn/.env (EnvironmentFile=), where LEPTOS_SITE_ADDR sets the bind addr;
# fall back to the known prod port if it isn't readable/set. .env is 0600 authlyn,
# hence sudo.
get_addr() {
  local a
  a="$(sudo -n grep -hE '^[[:space:]]*LEPTOS_SITE_ADDR=' "$OPT/.env" 2>/dev/null \
        | tail -n1 | sed -E 's/^[^=]*=[[:space:]]*//; s/^["'\'']//; s/["'\'']$//')"
  printf '%s' "${a:-127.0.0.1:8081}"
}

# Poll the local endpoint until it serves 200. A freshly-started process needs
# ~1s to bind, so the first attempt usually fails — expected; we retry for ~12s
# (the unit's TimeoutStartSec is 15s). Returns 0 on first 200, 1 if all fail.
healthcheck() {
  local addr i; addr="$(get_addr)"
  for i in $(seq 1 12); do
    # -fs (no -S): stay quiet on the transient pre-bind failures; only the
    # caller reports if ALL attempts fail.
    if curl -fs -o /dev/null "http://$addr/"; then return 0; fi
    sleep 1
  done
  return 1
}

# ERR-trap handler. Only rolls back if we got past stopping the service (STAGE in
# stopped/swapped/started); earlier failures left prod running, so there's nothing
# to restore. Detaches the trap + `set +e` so the restore path can't recurse or
# abort half-way through.
rollback() {
  local rc="${1:-$?}"
  trap - ERR
  set +e
  case "$STAGE" in
    stopped|swapped|started)
      echo "[deploy] FAILED at stage=$STAGE (rc=$rc) — rolling back to ${OLD:-previous}" >&2
      sudo -n systemctl stop "$SVC"
      sudo -n cp -a "$BIN_BAK" "$BIN"
      sudo -n rm -rf "$SITE"; sudo -n cp -a "$SITE_BAK" "$SITE"
      sudo -n chown -R "$APP_USER:$APP_USER" "$BIN" "$SITE"
      sudo -n systemctl start "$SVC"
      if healthcheck; then
        echo "[deploy] rolled back OK — $SVC healthy on ${OLD:-previous}" >&2
      else
        echo "[deploy] ROLLBACK HEALTHCHECK FAILED — manual intervention needed" >&2
      fi
      ;;
    *)
      echo "[deploy] failed at stage=$STAGE (rc=$rc) — service untouched (still on ${OLD:-current})" >&2
      ;;
  esac
  exit "$rc"
}
trap rollback ERR

# ---- single-flight lock -----------------------------------------------------
# Two concurrent deploys racing the stop/swap would corrupt the deployment. flock
# on a dedicated fd guarantees one-at-a-time; the lock releases automatically when
# this shell exits (fd 9 closes).
exec 9>/tmp/authlyn-deploy.lock
flock -n 9 || { echo "[deploy] another deploy holds the lock; aborting" >&2; exit 1; }

# ---- build (service still up) ----------------------------------------------
# cargo / cargo-leptos live under ~/.cargo but are NOT on the non-login-shell PATH
# that ssh gives us — without this you get "cargo: command not found".
. "$HOME/.cargo/env"

cd "$REPO"
echo "[deploy] fetching origin…"
# --tags so a version-tag $REF (used by the autodeploy workflow) resolves below.
git fetch --quiet --tags origin
OLD="$(git rev-parse --short HEAD)"
# Hard reset is safe here: this checkout is deploy-only (no local development), so
# it just snaps to the exact ref we're shipping.
git reset --hard --quiet "$REF"
NEW="$(git rev-parse --short HEAD)"
echo "[deploy] $OLD -> $NEW  $(git log -1 --format='%s')"

if [ "$SKIP_BUILD" -eq 0 ]; then
  STAGE=building
  echo "[deploy] cargo leptos build --release  (the slow part; service still up)…"
  # Builds all three artifacts: the ssr server bin, the hydrate WASM, and the
  # SCSS->CSS. If it fails, set -e fires the ERR trap while STAGE=building -> the
  # rollback handler takes the "untouched" branch -> clean abort, zero downtime.
  cargo leptos build --release
else
  echo "[deploy] --skip-build: reusing existing target/ artifacts"
fi

# Guard: never proceed to the swap unless the artifacts we're about to copy exist.
SRC_BIN="$REPO/target/release/authlyn-interactive"
SRC_SITE="$REPO/target/site"
[ -x "$SRC_BIN" ]                               || { echo "[deploy] missing $SRC_BIN — build failed?" >&2; exit 1; }
[ -d "$SRC_SITE" ]                              || { echo "[deploy] missing $SRC_SITE — build failed?" >&2; exit 1; }
[ -f "$SRC_SITE/pkg/authlyn-interactive.wasm" ] || { echo "[deploy] missing hydrate wasm — build failed?" >&2; exit 1; }

# ---- swap (downtime window) -------------------------------------------------
STAGE=backup
echo "[deploy] backing up current artifacts (-> *.bak)"
sudo -n cp -a "$BIN" "$BIN_BAK"
sudo -n rm -rf "$SITE_BAK"; sudo -n cp -a "$SITE" "$SITE_BAK"

STAGE=stopped
# Must stop before copying the binary: overwriting the file of a running process
# gives "Text file busy". This begins the (few-second) outage.
echo "[deploy] stopping $SVC  (downtime begins)"
sudo -n systemctl stop "$SVC"

STAGE=swapped
echo "[deploy] swapping binary + site/"
sudo -n cp "$SRC_BIN" "$BIN"
sudo -n rm -rf "$SITE"; sudo -n cp -r "$SRC_SITE" "$SITE"
# The hydration alias — easy to forget, invisible until you click something:
# cargo-leptos emits <name>.wasm, but leptos_axum's HydrationScripts fetch
# <name>_bg.wasm. Without this copy the page renders via SSR but never hydrates,
# so the UI is dead to events.
sudo -n cp "$SITE/pkg/authlyn-interactive.wasm" "$SITE/pkg/authlyn-interactive_bg.wasm"
# Artifacts were copied as root; hand them back to the service user.
sudo -n chown -R "$APP_USER:$APP_USER" "$BIN" "$SITE"

STAGE=started
echo "[deploy] starting $SVC"
sudo -n systemctl start "$SVC"

# ---- verify -----------------------------------------------------------------
echo "[deploy] health-checking http://$(get_addr)/ …"
if ! healthcheck; then
  echo "[deploy] healthcheck FAILED after start" >&2
  rollback 1   # explicit call: an ERR trap does NOT fire on a plain `exit`
fi
STAGE=healthy

echo "[deploy] OK — $SVC active on $NEW"
sudo -n systemctl is-active "$SVC" || true
sudo -n journalctl -u "$SVC" -n 15 --no-pager 2>/dev/null | tail -n 15 || true
