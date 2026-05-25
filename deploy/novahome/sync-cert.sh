#!/usr/bin/env bash
# Pull the authlyn.tplinkdns.com Let's Encrypt cert from the Pi (FENRIR) into
# the dir novahome's Caddy reads, then reload Caddy only if it changed.
#
# Why this exists: novahome cannot ACME this hostname — public :80 (HTTP-01)
# and :443 (TLS-ALPN-01) both route to the Pi. The Pi mints + renews the cert
# (a co-hosted Discord-activity site keeps it alive on the same hostname, :8443/:80). Invoked by
# authlyn-cert-sync.timer (nightly). Runs as root.
#
# Pi side is a forced-command SSH key:
#   command="sudo tar -cf - -C <certdir> ."
# so this key can ONLY stream the cert dir — it cannot get a shell or run
# anything else, even though it authenticates as damien (NOPASSWD sudo).
set -euo pipefail

PI_HOST="${PI_HOST:-damien@192.168.0.153}"
PI_KEY="${PI_KEY:-/etc/caddy/cert-pull/id_ed25519}"
DEST="/var/lib/caddy/authlyn-cert"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

# The forced command ignores any client args and streams the cert dir as tar.
ssh -i "$PI_KEY" -o BatchMode=yes -o StrictHostKeyChecking=accept-new "$PI_HOST" \
  | tar -xf - -C "$TMP"

CRT="$TMP/authlyn.tplinkdns.com.crt"
KEY="$TMP/authlyn.tplinkdns.com.key"
[ -s "$CRT" ] && [ -s "$KEY" ] || { echo "sync-cert: pulled tar missing crt/key; aborting (Caddy keeps old cert)" >&2; exit 1; }

install -d -o caddy -g caddy -m 0710 "$DEST"
changed=1
if cmp -s "$CRT" "$DEST/authlyn.tplinkdns.com.crt" 2>/dev/null \
   && cmp -s "$KEY" "$DEST/authlyn.tplinkdns.com.key" 2>/dev/null; then
  changed=0
fi
install -o caddy -g caddy -m 0644 "$CRT" "$DEST/authlyn.tplinkdns.com.crt"
install -o caddy -g caddy -m 0640 "$KEY" "$DEST/authlyn.tplinkdns.com.key"

if [ "$changed" = 1 ]; then
  echo "sync-cert: cert changed; reloading caddy"
  systemctl reload caddy
else
  echo "sync-cert: cert unchanged"
fi
