#!/usr/bin/env bash
# One-time bootstrap of the Pi for authlyn-interactive. Idempotent —
# safe to re-run. Run as root via:
#   ssh pi sudo -E bash -s < deploy/pi-bootstrap.sh
# with these env vars exported on the caller side:
#   AUTHLYN_ENV_FILE       path to a populated .env (we cat it in)
#   AUTHLYN_GITHUB_TOKEN   the fine-grained PAT (contents:read)
#
# All other steps are repeatable without input.

set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: run as root (sudo)." >&2
    exit 1
fi

SURREAL_VERSION="v3.0.4"
SURREAL_ARCH="linux-arm64"
SURREAL_URL="https://github.com/surrealdb/surrealdb/releases/download/${SURREAL_VERSION}/surreal-${SURREAL_VERSION}.${SURREAL_ARCH}.tgz"

log() { echo "[bootstrap] $*"; }

# 1. Port audit. Allow listeners that are our own services (re-run case);
# bail on any other process holding our ports.
log "auditing ports :8000 :8081 :8444"
audit_port() {
    local port="$1" expect_proc="$2"
    local out
    out=$(ss -Htlnp "( sport = :${port} )" 2>/dev/null || true)
    if [ -z "$out" ]; then
        return 0
    fi
    if echo "$out" | grep -q "\"${expect_proc}\""; then
        log "  :${port} held by ${expect_proc} (re-run, ok)"
        return 0
    fi
    echo "ERROR: port :${port} is already listening on the Pi:" >&2
    echo "$out" >&2
    echo "Update the spec + the pi-deployment memory entry with a new port before re-running." >&2
    return 1
}
audit_port 8000 surreal             || exit 1
audit_port 8444 caddy               || exit 1
audit_port 8081 authlyn-interactive || exit 1

# 2. apt-get prerequisites. Pi OS Lite is minimal — jq is missing by
# default. curl, tar, rsync, ss are in base. apt-get install -y is
# idempotent (no-op if already installed).
log "ensuring apt prerequisites: jq curl tar rsync"
DEBIAN_FRONTEND=noninteractive apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends jq curl tar rsync

# 3. Install surreal binary at pinned version.
if [ -x /usr/local/bin/surreal ] && /usr/local/bin/surreal version 2>/dev/null | grep -q "${SURREAL_VERSION#v}"; then
    log "surreal ${SURREAL_VERSION} already installed"
else
    log "downloading surreal ${SURREAL_VERSION} (${SURREAL_ARCH})"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    curl -sSfL "$SURREAL_URL" -o "$tmp/surreal.tgz"
    tar -xzf "$tmp/surreal.tgz" -C "$tmp"
    install -m 0755 "$tmp/surreal" /usr/local/bin/surreal
    log "surreal installed: $(/usr/local/bin/surreal version)"
fi

# 4. Users + groups.
if ! id -u authlyn >/dev/null 2>&1; then
    log "creating authlyn user"
    useradd --system --no-create-home --shell /usr/sbin/nologin authlyn
fi
if ! id -u surrealdb >/dev/null 2>&1; then
    log "creating surrealdb user"
    useradd --system --no-create-home --shell /usr/sbin/nologin surrealdb
fi

# 5. Directories.
log "creating /opt/authlyn{,/media} and /var/lib/surrealdb"
install -d -o authlyn -g authlyn -m 0750 /opt/authlyn /opt/authlyn/media
install -d -o surrealdb -g surrealdb -m 0750 /var/lib/surrealdb
install -d -o caddy -g caddy -m 0755 /var/log/caddy

# Pre-create Caddy's authlyn access log owned by caddy:caddy so the
# reload that loads our new site block can open it. (On some setups
# the file ends up root-owned, which then blocks Caddy from opening
# it; pre-creating with the right owner avoids that race.)
install -o caddy -g caddy -m 0640 /dev/null /var/log/caddy/authlyn-interactive.log

# 6. Secrets.
if [ -n "${AUTHLYN_ENV_FILE:-}" ] && [ -f "$AUTHLYN_ENV_FILE" ]; then
    install -m 0600 -o authlyn -g authlyn "$AUTHLYN_ENV_FILE" /opt/authlyn/.env
    log "installed /opt/authlyn/.env"
elif [ -f /opt/authlyn/.env ]; then
    log "/opt/authlyn/.env already present; not overwriting"
else
    echo "ERROR: /opt/authlyn/.env missing and AUTHLYN_ENV_FILE not provided." >&2
    exit 1
fi

if [ -n "${AUTHLYN_GITHUB_TOKEN:-}" ]; then
    umask 077
    printf "%s" "$AUTHLYN_GITHUB_TOKEN" > /opt/authlyn/.github_token
    chown authlyn:authlyn /opt/authlyn/.github_token
    chmod 0600 /opt/authlyn/.github_token
    log "installed /opt/authlyn/.github_token"
elif [ -f /opt/authlyn/.github_token ]; then
    log "/opt/authlyn/.github_token already present; not overwriting"
else
    echo "ERROR: /opt/authlyn/.github_token missing and AUTHLYN_GITHUB_TOKEN not provided." >&2
    exit 1
fi

# 7. Systemd units. Install pi-updater.sh first so the timer has
# something to fire.
log "installing systemd units + pi-updater.sh"
install -m 0755 -o authlyn -g authlyn "$(dirname "$0")/pi-updater.sh" /opt/authlyn/pi-updater.sh
install -m 0644 "$(dirname "$0")/surrealdb.service"        /etc/systemd/system/surrealdb.service
install -m 0644 "$(dirname "$0")/authlyn.service"          /etc/systemd/system/authlyn.service
install -m 0644 "$(dirname "$0")/authlyn-updater.service"  /etc/systemd/system/authlyn-updater.service
install -m 0644 "$(dirname "$0")/authlyn-updater.timer"    /etc/systemd/system/authlyn-updater.timer
systemctl daemon-reload

# 8. Sudoers. Validate before installing.
log "installing sudoers fragment"
visudo -cf "$(dirname "$0")/sudoers.authlyn-updater" >/dev/null
install -m 0440 -o root -g root "$(dirname "$0")/sudoers.authlyn-updater" /etc/sudoers.d/authlyn-updater

# 9. Caddy snippet. Idempotent: only append if marker absent.
CADDYFILE=/etc/caddy/Caddyfile
SNIPPET="$(dirname "$0")/Caddyfile.authlyn-interactive.snippet"
MARKER='# === authlyn-interactive ==='
if grep -qF "$MARKER" "$CADDYFILE"; then
    log "Caddy snippet already present in $CADDYFILE; skipping"
else
    log "appending Caddy snippet to $CADDYFILE"
    printf '\n' >> "$CADDYFILE"
    cat "$SNIPPET" >> "$CADDYFILE"
fi
caddy validate --config "$CADDYFILE"

# 10. Enable + start.
log "enabling + starting surrealdb.service"
systemctl enable --now surrealdb.service

log "enabling authlyn.service (will start on first successful pull)"
systemctl enable authlyn.service || true

log "enabling + starting authlyn-updater.timer"
systemctl enable --now authlyn-updater.timer

log "reloading caddy"
systemctl reload caddy

cat <<EOF

Bootstrap complete.

Next manual step (off-Pi):
  Add a router port-forward on the TP-Link admin UI:
    External port 8444 → 192.168.0.153:8444 (TCP)
  Otherwise the Pi is unreachable from the public DDNS hostname.

After that, push to release:
  git push origin main:release

The Pi will pull the new build within 5 minutes.
EOF
