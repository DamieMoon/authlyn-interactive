#!/usr/bin/env bash
# Pi-side auto-deploy. Polls the GitHub Releases API for the rolling
# `latest` tag, compares its sha_short against /opt/authlyn/build.json,
# and on mismatch downloads + atomically swaps + restarts authlyn.
#
# Wired in via deploy/authlyn-updater.{service,timer}. Idempotent and
# safe-by-default: any failure leaves the previous version running.

set -euo pipefail

REPO="DamieMoon/authlyn-interactive"
TAG="latest"
INSTALL_DIR="/opt/authlyn"
TOKEN_FILE="$INSTALL_DIR/.github_token"
INSTALLED_BUILD_JSON="$INSTALL_DIR/build.json"
SERVICE_NAME="authlyn"

log() { echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] pi-updater: $*"; }

# 1. Require the PAT (private repo). The file is owned by authlyn,
# mode 0600, so we read via sudo.
if ! sudo test -f "$TOKEN_FILE"; then
    log "ERROR: $TOKEN_FILE missing. Provision a fine-grained PAT (contents:read)."
    exit 1
fi
TOKEN="$(sudo cat "$TOKEN_FILE")"
if [ -z "$TOKEN" ]; then
    log "ERROR: $TOKEN_FILE empty."
    exit 1
fi

# 2. Fetch release manifest. Use the explicit /releases/tags/<TAG>
# endpoint (not /releases/latest, which targets the non-prerelease
# latest and we mark releases prerelease=true).
API_URL="https://api.github.com/repos/${REPO}/releases/tags/${TAG}"
RELEASE_JSON="$(curl -sSfL \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "$API_URL")"

# 3. Pick the most recently uploaded asset. GitHub returns assets in
# alphabetical-by-name order; sorting by created_at picks the artifact
# CI just published rather than pinning to the lexicographically
# smallest SHA prefix (the asset-ordering bug the spec warns about).
LATEST_ASSET="$(echo "$RELEASE_JSON" | jq '[.assets[]] | sort_by(.created_at) | last')"
ASSET_URL="$(echo "$LATEST_ASSET" | jq -r '.url // ""')"
ASSET_NAME="$(echo "$LATEST_ASSET" | jq -r '.name // ""')"

REMOTE_SHA_SHORT="$(echo "$ASSET_NAME" | sed -n 's/^authlyn-\([a-f0-9]\{1,\}\)\.tar\.gz$/\1/p')"
INSTALLED_SHA_SHORT="$(jq -r '.sha_short // ""' "$INSTALLED_BUILD_JSON" 2>/dev/null || echo '')"

if [ -z "$REMOTE_SHA_SHORT" ] || [ -z "$ASSET_URL" ]; then
    log "ERROR: could not parse release (asset=$ASSET_NAME); bailing without changes."
    exit 1
fi

if [ "$REMOTE_SHA_SHORT" = "$INSTALLED_SHA_SHORT" ] && [ -n "$INSTALLED_SHA_SHORT" ]; then
    log "up to date (sha_short=$INSTALLED_SHA_SHORT); nothing to do"
    exit 0
fi

log "new build available: remote sha_short=$REMOTE_SHA_SHORT, installed=$INSTALLED_SHA_SHORT, asset=$ASSET_NAME"

# 4. Download to staging.
STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT

ASSET_PATH="$STAGING/$ASSET_NAME"
curl -sSfL \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Accept: application/octet-stream" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    -o "$ASSET_PATH" \
    "$ASSET_URL"

if [ ! -s "$ASSET_PATH" ]; then
    log "ERROR: downloaded asset is empty or missing; bailing."
    exit 1
fi

log "downloaded $ASSET_NAME ($(stat -c %s "$ASSET_PATH") bytes); extracting"

EXTRACT_DIR="$STAGING/extracted"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$ASSET_PATH" -C "$EXTRACT_DIR"

# 5. Validate artifact shape + SHA match.
if [ ! -f "$EXTRACT_DIR/authlyn-interactive" ] || [ ! -d "$EXTRACT_DIR/site" ] || [ ! -f "$EXTRACT_DIR/build.json" ]; then
    log "ERROR: artifact missing one of {authlyn-interactive, site/, build.json}; bailing."
    exit 1
fi

ARTIFACT_SHA="$(jq -r '.sha // ""' "$EXTRACT_DIR/build.json")"
ARTIFACT_SHA_SHORT="$(jq -r '.sha_short // ""' "$EXTRACT_DIR/build.json")"
if [ "$ARTIFACT_SHA_SHORT" != "$REMOTE_SHA_SHORT" ]; then
    log "ERROR: artifact build.json sha_short ($ARTIFACT_SHA_SHORT) != asset name sha ($REMOTE_SHA_SHORT); bailing."
    exit 1
fi

# 6. Atomic swap.
log "installing"
sudo install -m 0755 -o authlyn -g authlyn "$EXTRACT_DIR/authlyn-interactive" "$INSTALL_DIR/authlyn-interactive.new"
sudo mv -f "$INSTALL_DIR/authlyn-interactive.new" "$INSTALL_DIR/authlyn-interactive"
sudo rsync -a --delete --chown=authlyn:authlyn "$EXTRACT_DIR/site/" "$INSTALL_DIR/site/"
sudo install -m 0644 -o authlyn -g authlyn "$EXTRACT_DIR/build.json" "$INSTALL_DIR/build.json"

# 7. Restart and confirm.
log "restarting $SERVICE_NAME"
sudo systemctl restart "$SERVICE_NAME"
sleep 3
if ! systemctl is-active --quiet "$SERVICE_NAME"; then
    log "ERROR: service is not active after restart; check journalctl -u $SERVICE_NAME"
    exit 1
fi

log "deployed sha=$ARTIFACT_SHA (sha_short=$ARTIFACT_SHA_SHORT) successfully"
