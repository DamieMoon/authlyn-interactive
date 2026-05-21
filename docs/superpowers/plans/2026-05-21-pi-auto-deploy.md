# Pi auto-deploy implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land authlyn-interactive auto-deploy to the Raspberry Pi. Every `git push origin main:release` is built in CI, picked up by a Pi-side systemd timer within 5 minutes, and served at `https://authlyn.tplinkdns.com:8444` via Caddy.

**Architecture:** CI cross-compiles to `aarch64-unknown-linux-gnu` via `cargo-leptos` + `cargo-zigbuild`, packages a tar.gz, uploads to a rolling `latest` GitHub Release. A Pi-side puller (systemd timer + shell script) polls every 5 minutes, atomically swaps binary + site bundle on SHA change, restarts the app. SurrealDB runs as a Pi-wide systemd unit (file-backed). Caddy reverse-proxies a new site block on `:8444` to the app's loopback port. KalmarOS at `:8443` is untouched.

**Tech Stack:** GitHub Actions, cargo-zigbuild, cargo-leptos, systemd, Caddy, SurrealDB (binary `v3.0.4` + SDK `=3.1.0-beta.3`), bash.

**Source of truth:** `docs/superpowers/specs/2026-05-21-pi-auto-deploy-design.md`.

**Task-shape note.** Tasks 1, 10, 11 contain testable code (Rust + bash). Static config tasks (units, snippets, env templates) skip the TDD shape — their "test" is a parse/validate command. Pi-side tasks (14, 15, 17, 18) include verification commands rather than commits.

---

## Task 1: DB-connect retry wrapper

Spec section: *App + DB supervision → DB-connect retry on cold start.*

**Files:**
- Modify: `src/db.rs`
- Modify: `src/main.rs:19-25`

- [ ] **Step 1: Write the failing tests**

Append to `src/db.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    pub(crate) async fn retry<F, Fut, T, E>(
        mut op: F,
        max_attempts: u32,
        backoff: Duration,
    ) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let mut last_err: Option<E> = None;
        for attempt in 1..=max_attempts {
            match op().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    eprintln!("attempt {attempt}/{max_attempts}: {e}");
                    last_err = Some(e);
                    if attempt < max_attempts {
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }
        Err(last_err.expect("retry called with max_attempts >= 1"))
    }

    #[tokio::test]
    async fn retry_succeeds_after_transient_failures() {
        let counter = Arc::new(AtomicU32::new(0));
        let result: Result<i32, &'static str> = retry(
            || {
                let counter = counter.clone();
                async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    if n < 3 { Err("not yet") } else { Ok(42) }
                }
            },
            5,
            Duration::from_millis(1),
        )
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn retry_returns_last_error_on_exhaustion() {
        let counter = Arc::new(AtomicU32::new(0));
        let result: Result<i32, &'static str> = retry(
            || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Err("never")
                }
            },
            3,
            Duration::from_millis(1),
        )
        .await;

        assert_eq!(result, Err("never"));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
```

- [ ] **Step 2: Move `retry` out of `#[cfg(test)]` so prod code can call it**

Replace the tests module with this pair (tests stay test-gated; the helper goes into the module proper):

```rust
use std::future::Future;
use std::time::Duration;

async fn retry<F, Fut, T, E>(
    mut op: F,
    max_attempts: u32,
    backoff: Duration,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_err: Option<E> = None;
    for attempt in 1..=max_attempts {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                eprintln!("attempt {attempt}/{max_attempts}: {e}");
                last_err = Some(e);
                if attempt < max_attempts {
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(last_err.expect("retry called with max_attempts >= 1"))
}

pub async fn connect_with_retries() -> surrealdb::Result<Surreal<Client>> {
    retry(connect, 10, Duration::from_millis(500)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn retry_succeeds_after_transient_failures() {
        let counter = Arc::new(AtomicU32::new(0));
        let result: Result<i32, &'static str> = retry(
            || {
                let counter = counter.clone();
                async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    if n < 3 { Err("not yet") } else { Ok(42) }
                }
            },
            5,
            Duration::from_millis(1),
        )
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn retry_returns_last_error_on_exhaustion() {
        let counter = Arc::new(AtomicU32::new(0));
        let result: Result<i32, &'static str> = retry(
            || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Err("never")
                }
            },
            3,
            Duration::from_millis(1),
        )
        .await;

        assert_eq!(result, Err("never"));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
```

- [ ] **Step 3: Update main.rs to use `connect_with_retries`**

`src/main.rs` lines 19-25 currently:

```rust
let _surreal = db::connect()
    .await
    .expect("SurrealDB connect failed (is `./scripts/dev-db.sh` running?)");
db::apply_schema(&_surreal)
    .await
    .expect("SurrealDB schema apply failed");
```

Change `db::connect()` to `db::connect_with_retries()`. The expect message becomes:

```rust
let _surreal = db::connect_with_retries()
    .await
    .expect("SurrealDB connect failed after 10 retries (is `./scripts/dev-db.sh` running locally, or `surrealdb.service` on the Pi?)");
```

- [ ] **Step 4: Run the tests**

```
cargo test --features ssr --no-default-features --lib db::tests
```

Expected: both `retry_succeeds_after_transient_failures` and `retry_returns_last_error_on_exhaustion` pass.

- [ ] **Step 5: Run `cargo fmt --all` and `cargo check`**

```
cargo fmt --all && cargo check --features ssr --no-default-features
```

Expected: no diff after fmt, `Finished ... 0.XXs`.

- [ ] **Step 6: Commit**

```
git add src/db.rs src/main.rs
git commit -m "$(cat <<'EOF'
Retry SurrealDB connect with backoff for Pi cold-start

systemd reports surrealdb.service as 'started' before its WebSocket
listener has finished binding. Without retries, authlyn.service panics
on the first connect attempt after a fresh boot, and the puller's
post-restart is-active check can land inside the restart-loop window.

Adds a generic retry helper plus connect_with_retries (10 attempts,
500ms backoff, ~5s ceiling). Tests cover the success-after-N-failures
and exhaustion paths via an injected operation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: (skipped — no `.cargo/config.toml` needed)

**Background:** an earlier draft of this plan created `.cargo/config.toml` with `linker = "cargo-zigbuild"`. That's wrong — `cargo-zigbuild` ships as a cargo subcommand only (no linker wrapper binary exists), confirmed by the upstream README. The file was created at commit `3a52e91` and reverted at the commit recording this plan revision. **Skip this task** — no equivalent file needs to land; Task 12 invokes `cargo zigbuild` directly for the server-bin step (see its updated body below).

No commit needed for this task (the file was already deleted in the plan-revision commit).

---

## Task 3: Production env template

Spec section: *Paths, ownership, secrets → `.env`.*

**Files:**
- Create: `deploy/.env.example`

- [ ] **Step 1: Create `deploy/.env.example`**

```env
# Production .env for /opt/authlyn/.env on the Pi.
# Copy on first bootstrap; never commit a populated copy.

# --- SurrealDB ---
SURREAL_URL=ws://127.0.0.1:8000
SURREAL_USER=root
SURREAL_PASS=root
SURREAL_NS=authlyn
SURREAL_DB=prod

# --- Leptos runtime ---
# get_configuration(None) in main.rs reads these from the environment.
# Without them, the binary defaults to :3000 and looks for site assets
# under ./target/site/ (CWD-relative). On the Pi, Caddy reverse-proxies
# :8444 → 127.0.0.1:8081, and the bundle lives at /opt/authlyn/site/.
LEPTOS_OUTPUT_NAME=authlyn-interactive
LEPTOS_SITE_ROOT=site
LEPTOS_SITE_PKG_DIR=pkg
LEPTOS_SITE_ADDR=127.0.0.1:8081
```

- [ ] **Step 2: Verify shape**

```
grep -E '^(SURREAL_|LEPTOS_)' deploy/.env.example | wc -l
```

Expected: `9` (5 SURREAL_ + 4 LEPTOS_ lines).

- [ ] **Step 3: Commit**

```
git add deploy/.env.example
git commit -m "$(cat <<'EOF'
Add production .env template for the Pi

Holds SurrealDB connection + Leptos runtime env vars (output name,
site root, site addr) that get_configuration(None) reads at startup.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Caddy site block snippet

Spec section: *TLS + routing.*

**Files:**
- Create: `deploy/Caddyfile.authlyn-interactive.snippet`

- [ ] **Step 1: Create the snippet**

```caddy
# === authlyn-interactive ===
# Appended to /etc/caddy/Caddyfile by deploy/pi-bootstrap.sh.
# Detected by the leading marker comment so re-runs are idempotent.

authlyn.tplinkdns.com:8444 {
    encode zstd gzip

    # axum binds 127.0.0.1:8081 (see LEPTOS_SITE_ADDR in /opt/authlyn/.env).
    # Caddy terminates TLS on :8444 and reverse-proxies in.
    reverse_proxy 127.0.0.1:8081

    header {
        -Server
        Strict-Transport-Security "max-age=31536000"
        X-Content-Type-Options "nosniff"
        Referrer-Policy "no-referrer"
    }

    log {
        output file /var/log/caddy/authlyn-interactive.log
        format console
    }
}
# === /authlyn-interactive ===
```

- [ ] **Step 2: Verify markers**

```
grep -c '^# === authlyn-interactive ===' deploy/Caddyfile.authlyn-interactive.snippet
grep -c '^# === /authlyn-interactive ===' deploy/Caddyfile.authlyn-interactive.snippet
```

Expected: `1` and `1`. The bootstrap script uses the opening marker as the idempotency check.

- [ ] **Step 3: Commit**

```
git add deploy/Caddyfile.authlyn-interactive.snippet
git commit -m "$(cat <<'EOF'
Add Caddy site block for authlyn.tplinkdns.com:8444

Appended to /etc/caddy/Caddyfile by pi-bootstrap.sh. Marker comments
make appending idempotent. KalmarOS's :8443 block is untouched.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: SurrealDB systemd unit

Spec section: *App + DB supervision → `deploy/surrealdb.service`.*

**Files:**
- Create: `deploy/surrealdb.service`

- [ ] **Step 1: Create the unit file**

```ini
[Unit]
Description=SurrealDB (shared Pi infra)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=surrealdb
Group=surrealdb
WorkingDirectory=/var/lib/surrealdb
ExecStart=/usr/local/bin/surreal start --user root --pass root --bind 127.0.0.1:8000 file:///var/lib/surrealdb
Restart=on-failure
RestartSec=2s
TimeoutStartSec=15s

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictRealtime=true
LockPersonality=true
ReadWritePaths=/var/lib/surrealdb

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Sanity-check that the file parses (informational only on Mac dev — systemd-analyze isn't available)**

```
grep -E '^(ExecStart|User|Group|WorkingDirectory)=' deploy/surrealdb.service
```

Expected: 4 lines printed. Full `systemd-analyze verify` runs on the Pi during bootstrap.

- [ ] **Step 3: Commit**

```
git add deploy/surrealdb.service
git commit -m "$(cat <<'EOF'
Add surrealdb.service unit for the Pi

Runs surreal as a dedicated surrealdb user, file-backed at
/var/lib/surrealdb, bound to 127.0.0.1:8000. Standard systemd
hardening + ReadWritePaths scoped to the data dir.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: App systemd unit

Spec section: *App + DB supervision → `deploy/authlyn.service`.*

**Files:**
- Create: `deploy/authlyn.service`

- [ ] **Step 1: Create the unit file**

```ini
[Unit]
Description=authlyn-interactive (E2EE roleplay chat)
Documentation=https://github.com/DamieMoon/authlyn-interactive
Requires=surrealdb.service
After=surrealdb.service network-online.target
Wants=network-online.target

[Service]
Type=simple
User=authlyn
Group=authlyn
WorkingDirectory=/opt/authlyn
ExecStart=/opt/authlyn/authlyn-interactive
EnvironmentFile=/opt/authlyn/.env
Restart=on-failure
RestartSec=2s
TimeoutStartSec=15s
TimeoutStopSec=10s

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictRealtime=true
LockPersonality=true
ReadWritePaths=/opt/authlyn/media

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Sanity check**

```
grep -E '^(Requires|After|ExecStart|EnvironmentFile)=' deploy/authlyn.service
```

Expected: 4 lines printed (`Requires=surrealdb.service`, `After=surrealdb.service network-online.target`, `ExecStart=/opt/authlyn/authlyn-interactive`, `EnvironmentFile=/opt/authlyn/.env`).

- [ ] **Step 3: Commit**

```
git add deploy/authlyn.service
git commit -m "$(cat <<'EOF'
Add authlyn.service unit for the Pi

Runs the cross-compiled binary as a dedicated authlyn user with
EnvironmentFile loading /opt/authlyn/.env. Hard depends on
surrealdb.service. ReadWritePaths scoped to /opt/authlyn/media so
the rest of /opt/authlyn is immutable from the app's point of view.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Updater systemd service + timer

Spec section: *Pi-side updater.*

**Files:**
- Create: `deploy/authlyn-updater.service`
- Create: `deploy/authlyn-updater.timer`

- [ ] **Step 1: Create `deploy/authlyn-updater.service`**

```ini
[Unit]
Description=authlyn-interactive auto-deploy updater (poll GitHub Releases, swap, restart)
Documentation=https://github.com/DamieMoon/authlyn-interactive
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
User=damien
Group=damien
ExecStart=/opt/authlyn/pi-updater.sh
StandardOutput=journal
StandardError=journal
TimeoutStartSec=180s

# Minimal sandboxing — the script needs sudo and write access to
# /opt/authlyn via NOPASSWD. The timer surfaces persistent failures
# through journalctl rather than thrashing on retry.
NoNewPrivileges=false
ProtectSystem=false
```

- [ ] **Step 2: Create `deploy/authlyn-updater.timer`**

```ini
[Unit]
Description=authlyn-interactive auto-deploy updater (every 5 minutes)
Documentation=https://github.com/DamieMoon/authlyn-interactive

[Timer]
OnBootSec=30s
OnUnitActiveSec=5min
Persistent=true

[Install]
WantedBy=timers.target
```

- [ ] **Step 3: Sanity check**

```
grep -E '^(Type|ExecStart|User|OnBootSec|OnUnitActiveSec)=' deploy/authlyn-updater.service deploy/authlyn-updater.timer
```

Expected: 5 lines spanning the two files.

- [ ] **Step 4: Commit**

```
git add deploy/authlyn-updater.service deploy/authlyn-updater.timer
git commit -m "$(cat <<'EOF'
Add authlyn-updater systemd service + 5-minute timer

The oneshot service runs /opt/authlyn/pi-updater.sh as damien. The
timer fires 30s after boot then every 5 minutes; Persistent=true
catches up on a missed firing.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Sudoers fragment for the puller

Spec section: *One-time Pi setup → step 7.*

**Files:**
- Create: `deploy/sudoers.authlyn-updater`

- [ ] **Step 1: Create the fragment**

```
# /etc/sudoers.d/authlyn-updater
# Installed mode 0440 by pi-bootstrap.sh; validated with `visudo -c`.
# Scopes damien's NOPASSWD privileges to exactly the operations
# /opt/authlyn/pi-updater.sh performs.

Cmnd_Alias AUTHLYN_INSTALL = /usr/bin/install, /usr/bin/mv, /usr/bin/rsync, /usr/bin/chown
Cmnd_Alias AUTHLYN_SVC     = /usr/bin/systemctl restart authlyn, /usr/bin/systemctl is-active authlyn
Cmnd_Alias AUTHLYN_TOKEN   = /usr/bin/cat /opt/authlyn/.github_token, /usr/bin/test -f /opt/authlyn/.github_token

damien ALL=(root) NOPASSWD: AUTHLYN_INSTALL, AUTHLYN_SVC, AUTHLYN_TOKEN
```

- [ ] **Step 2: Sanity check the file's structure**

```
grep -c '^Cmnd_Alias' deploy/sudoers.authlyn-updater
grep -c '^damien ALL=(root) NOPASSWD:' deploy/sudoers.authlyn-updater
```

Expected: `3` and `1`. Full `visudo -c` validation happens on the Pi.

- [ ] **Step 3: Commit**

```
git add deploy/sudoers.authlyn-updater
git commit -m "$(cat <<'EOF'
Scope damien NOPASSWD sudoers fragment for the auto-deploy puller

Allows /opt/authlyn/pi-updater.sh to install + atomic-mv + rsync +
systemctl restart authlyn without a password, and to read the PAT
file at /opt/authlyn/.github_token. Everything else still prompts.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Local "no remnants" guard

Spec section: *No-remnants discipline.*

**Files:**
- Create: `scripts/check-no-remnants.sh`

- [ ] **Step 1: Create the script**

```bash
#!/usr/bin/env bash
# Fail if any KalmarOS-derived names slipped into the repo.
# Excludes the design spec, where naming the template is legitimate.

set -euo pipefail

PATTERN='kalmaros|KalmarOS|kalmaroS|ku-chronicles'
EXCLUDE_PATH='docs/superpowers/specs/'

if git grep -in -E "$PATTERN" -- ":!$EXCLUDE_PATH" > /tmp/authlyn-remnants.$$ 2>/dev/null; then
    echo "KalmarOS-derived names leaked into the codebase:" >&2
    cat /tmp/authlyn-remnants.$$ >&2
    rm -f /tmp/authlyn-remnants.$$
    exit 1
fi

rm -f /tmp/authlyn-remnants.$$
echo "no-remnants check OK"
```

- [ ] **Step 2: Make executable + run**

```
chmod +x scripts/check-no-remnants.sh
./scripts/check-no-remnants.sh
```

Expected output: `no-remnants check OK`. Exit code `0`.

- [ ] **Step 3: Commit**

```
git add scripts/check-no-remnants.sh
git commit -m "$(cat <<'EOF'
Add scripts/check-no-remnants.sh

Local equivalent of the CI grep job that ensures no KalmarOS-derived
names slip into the repo. Runs against tracked files only.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Pi-side puller script

Spec section: *Pi-side updater.*

**Files:**
- Create: `deploy/pi-updater.sh`

- [ ] **Step 1: Create the script**

```bash
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
# smallest SHA prefix (see the spec for the KalmarOS bug we're avoiding).
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
if ! sudo systemctl is-active --quiet "$SERVICE_NAME"; then
    log "ERROR: service is not active after restart; check journalctl -u $SERVICE_NAME"
    exit 1
fi

log "deployed sha=$ARTIFACT_SHA (sha_short=$ARTIFACT_SHA_SHORT) successfully"
```

- [ ] **Step 2: Make executable + syntax check**

```
chmod +x deploy/pi-updater.sh
bash -n deploy/pi-updater.sh
```

Expected: no output (script parses).

- [ ] **Step 3: Run the no-remnants check (the script we just wrote shouldn't fail it)**

```
./scripts/check-no-remnants.sh
```

Expected: `no-remnants check OK`.

- [ ] **Step 4: Commit**

```
git add deploy/pi-updater.sh
git commit -m "$(cat <<'EOF'
Add Pi-side updater script

Polls the rolling 'latest' GitHub Release every 5 minutes (via the
timer in Task 7), compares sha_short against /opt/authlyn/build.json,
and on mismatch downloads + validates + atomically swaps + restarts.
Sorts assets by created_at so an alphabetically-small SHA can't pin
the Pi to an old build.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Pi-side bootstrap script

Spec section: *One-time Pi setup.*

**Files:**
- Create: `deploy/pi-bootstrap.sh`

- [ ] **Step 1: Create the script**

```bash
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

# 1. Port audit. Bail if any of our targets is taken.
log "auditing ports :8000 :8081 :8444"
for port in 8000 8081 8444; do
    if ss -tlnp "( sport = :${port} )" | grep -q LISTEN; then
        echo "ERROR: port :${port} is already listening on the Pi:" >&2
        ss -tlnp "( sport = :${port} )" >&2
        echo "Update the spec + the pi-deployment memory entry with a new port before re-running." >&2
        exit 1
    fi
done

# 2. Install surreal binary at pinned version.
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

# 3. Users + groups.
if ! id -u authlyn >/dev/null 2>&1; then
    log "creating authlyn user"
    useradd --system --no-create-home --shell /usr/sbin/nologin authlyn
fi
if ! id -u surrealdb >/dev/null 2>&1; then
    log "creating surrealdb user"
    useradd --system --no-create-home --shell /usr/sbin/nologin surrealdb
fi

# 4. Directories.
log "creating /opt/authlyn{,/media} and /var/lib/surrealdb"
install -d -o authlyn -g authlyn -m 0750 /opt/authlyn /opt/authlyn/media
install -d -o surrealdb -g surrealdb -m 0750 /var/lib/surrealdb
install -d -m 0755 /var/log/caddy

# 5. Secrets.
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

# 6. Systemd units. Install pi-updater.sh first so the timer has
# something to fire.
log "installing systemd units + pi-updater.sh"
install -m 0755 -o authlyn -g authlyn "$(dirname "$0")/pi-updater.sh" /opt/authlyn/pi-updater.sh
install -m 0644 "$(dirname "$0")/surrealdb.service"        /etc/systemd/system/surrealdb.service
install -m 0644 "$(dirname "$0")/authlyn.service"          /etc/systemd/system/authlyn.service
install -m 0644 "$(dirname "$0")/authlyn-updater.service"  /etc/systemd/system/authlyn-updater.service
install -m 0644 "$(dirname "$0")/authlyn-updater.timer"    /etc/systemd/system/authlyn-updater.timer
systemctl daemon-reload

# 7. Sudoers. Validate before installing.
log "installing sudoers fragment"
visudo -cf "$(dirname "$0")/sudoers.authlyn-updater" >/dev/null
install -m 0440 -o root -g root "$(dirname "$0")/sudoers.authlyn-updater" /etc/sudoers.d/authlyn-updater

# 8. Caddy snippet. Idempotent: only append if marker absent.
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

# 9. Enable + start.
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
```

- [ ] **Step 2: Make executable + syntax check**

```
chmod +x deploy/pi-bootstrap.sh
bash -n deploy/pi-bootstrap.sh
```

Expected: no output.

- [ ] **Step 3: No-remnants check**

```
./scripts/check-no-remnants.sh
```

Expected: `no-remnants check OK`.

- [ ] **Step 4: Commit**

```
git add deploy/pi-bootstrap.sh
git commit -m "$(cat <<'EOF'
Add Pi bootstrap script

Idempotent one-time setup: audits ports, installs pinned surreal
v3.0.4 aarch64 binary, creates authlyn + surrealdb users, lays out
/opt/authlyn + /var/lib/surrealdb, installs secrets (from env vars),
installs + reloads systemd units, validates + installs the scoped
sudoers fragment, and appends the Caddy site block behind a marker
so re-runs are safe.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: GitHub Actions workflow

Spec section: *Build & release pipeline.*

**Files:**
- Create: `.github/workflows/build-release.yml`

- [ ] **Step 1: Create the workflow**

```yaml
# Cross-compiles authlyn-interactive for aarch64-unknown-linux-gnu on
# every push to `release`, packages the server binary + site/ +
# build.json into a tar.gz, and updates a rolling `latest` GitHub
# Release. A Pi-side systemd timer (deploy/authlyn-updater.timer)
# polls and atomically swaps on SHA change. See
# docs/superpowers/specs/2026-05-21-pi-auto-deploy-design.md.
#
# Build strategy: split into two steps because cargo-zigbuild only
# works as a cargo subcommand (no linker wrapper for cargo-leptos to
# call into). `cargo zigbuild` cross-compiles the server binary with
# zig handling glibc version compat; `cargo leptos build --lib-only`
# produces the WASM/CSS bundle (target-agnostic). Outputs are merged
# in the stage/ dir.

name: Build and publish rolling release

on:
  push:
    branches: [release]
  workflow_dispatch: {}

permissions:
  contents: write

concurrency:
  group: build-release
  cancel-in-progress: true

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4
        with:
          fetch-depth: 1

      - name: No-remnants guard
        run: ./scripts/check-no-remnants.sh

      - name: Set up Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-unknown-linux-gnu, wasm32-unknown-unknown

      - name: Cache cargo registry + target
        uses: Swatinem/rust-cache@v2

      - name: Set up zig
        uses: mlugg/setup-zig@v2
        with:
          version: 0.13.0

      - name: Install cargo-zigbuild
        run: cargo install --locked cargo-zigbuild

      - name: Install cargo-leptos
        run: cargo install --locked cargo-leptos

      - name: Cross-compile server binary (cargo-zigbuild)
        run: |
          cargo zigbuild --release \
            --features ssr --no-default-features \
            --target aarch64-unknown-linux-gnu \
            --bin authlyn-interactive

      - name: Build WASM/CSS bundle (cargo-leptos --lib-only)
        run: cargo leptos build --release --lib-only

      - name: Compute build metadata
        id: meta
        run: |
          echo "sha_short=${GITHUB_SHA:0:7}" >> "$GITHUB_OUTPUT"
          echo "build_time=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$GITHUB_OUTPUT"
          echo "build_epoch=$(date -u +%s)" >> "$GITHUB_OUTPUT"

      - name: Stage release artifact
        run: |
          mkdir -p stage
          cp target/aarch64-unknown-linux-gnu/release/authlyn-interactive stage/
          cp -r target/site stage/site
          cat > stage/build.json <<EOF
          {
            "sha": "${GITHUB_SHA}",
            "sha_short": "${{ steps.meta.outputs.sha_short }}",
            "built_at": "${{ steps.meta.outputs.build_time }}",
            "built_epoch": ${{ steps.meta.outputs.build_epoch }},
            "ref": "${GITHUB_REF}"
          }
          EOF
          tar -czf "authlyn-${{ steps.meta.outputs.sha_short }}.tar.gz" -C stage .

      - name: Update rolling 'latest' release
        uses: softprops/action-gh-release@v2
        with:
          tag_name: latest
          name: 'Latest release build'
          body: |
            Auto-built from `release` on ${{ steps.meta.outputs.build_time }}.

            - Commit: `${{ github.sha }}`
            - Short: `${{ steps.meta.outputs.sha_short }}`
            - Built: `${{ steps.meta.outputs.build_time }}`

            The Pi-side updater (deploy/pi-updater.sh) polls this release
            tag every five minutes and downloads on SHA change.
          files: |
            authlyn-*.tar.gz
          prerelease: true
          make_latest: 'false'

      - name: Prune superseded assets from rolling 'latest'
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          KEEP="authlyn-${{ steps.meta.outputs.sha_short }}.tar.gz"
          gh release view latest \
            --repo "$GITHUB_REPOSITORY" \
            --json assets \
            | jq -r --arg keep "$KEEP" '.assets[] | select(.name != $keep) | .name' \
            | while read -r name; do
                [ -z "$name" ] && continue
                echo "Pruning superseded asset: $name"
                gh release delete-asset latest "$name" \
                  --repo "$GITHUB_REPOSITORY" --yes \
                  || echo "  (delete failed for $name; continuing)"
              done
```

- [ ] **Step 2: Validate workflow YAML locally**

```
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/build-release.yml'))" && echo "yaml OK"
```

Expected: `yaml OK`.

- [ ] **Step 3: No-remnants check (the workflow itself must pass)**

```
./scripts/check-no-remnants.sh
```

Expected: `no-remnants check OK`.

- [ ] **Step 4: Commit**

```
mkdir -p .github/workflows
git add .github/workflows/build-release.yml
git commit -m "$(cat <<'EOF'
Add CI workflow for the rolling 'latest' release

Triggered on push to release. Cross-compiles via cargo-leptos +
cargo-zigbuild to aarch64-unknown-linux-gnu, stages binary + site
bundle + build.json, uploads to the rolling 'latest' release as
authlyn-<sha_short>.tar.gz, then prunes superseded assets so the
puller only ever sees the freshest build. Runs the no-remnants
guard first.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: CLAUDE.md branching docs

Spec section: *Branch model.*

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Append a "Branching and auto-deploy" section**

After the existing `## Deployment target` section and before `## Out of scope (Damien to design)`, insert:

```markdown
## Branching and auto-deploy

- `main` is the working branch. Commits land here freely; nothing deploys.
- `release` is the deploy target. Promote with `git push origin main:release` when a batch of commits is ready to ship.
- CI (`.github/workflows/build-release.yml`) builds on every push to `release` and updates the rolling `latest` GitHub Release. The Pi-side timer (`deploy/authlyn-updater.timer`) polls every 5 minutes and atomic-swaps + restarts on SHA change.
- Rolling back a bad ship: `git push origin <good-sha>:release --force-with-lease`. CI re-runs against the good commit and the puller picks it up on the next tick.
- Pi-side machine state (chosen ports, install layout) lives in the project memory entry `pi-deployment`.
```

- [ ] **Step 2: No-remnants check**

```
./scripts/check-no-remnants.sh
```

Expected: `no-remnants check OK`.

- [ ] **Step 3: Commit**

```
git add CLAUDE.md
git commit -m "$(cat <<'EOF'
Document the release-branch model in CLAUDE.md

main is the working branch; release is the deploy target. Promote
with git push origin main:release. Adds rollback instructions and
the pointer to the pi-deployment memory entry.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Run pi-bootstrap.sh on the Pi

Spec section: *One-time Pi setup.* Spec verification: *step 0.*

**Files:** none modified locally. State changes happen on the Pi only.

**Prerequisites:** Tasks 1-13 committed. A populated `.env` file (copy from `deploy/.env.example`) and a fine-grained GitHub PAT with `contents:read` on `DamieMoon/authlyn-interactive` ready locally.

- [ ] **Step 1: Stage local .env (do not commit)**

```
cp deploy/.env.example /tmp/authlyn.env
# Edit /tmp/authlyn.env if any prod-specific values differ from the
# template defaults. For v1 the template is correct as-is.
```

- [ ] **Step 2: Confirm the PAT is ready**

Generate a fine-grained PAT at `https://github.com/settings/personal-access-tokens/new` with:
- Resource owner: `DamieMoon`
- Repositories: `Only select repositories → authlyn-interactive`
- Repository permissions: `Contents: Read-only`

Export it for the bootstrap call:

```
read -r -s AUTHLYN_GITHUB_TOKEN
export AUTHLYN_GITHUB_TOKEN
```

(Paste the token; the shell won't echo it.)

- [ ] **Step 3: Re-run the port audit (sanity check immediately before bootstrap)**

```
ssh pi sudo ss -tlnp '( sport = :8000 or sport = :8081 or sport = :8444 )' | grep -E '(LISTEN|State)' || echo "all three ports free"
```

Expected: `all three ports free`. If any port is listed, **stop**: update the spec + `pi-deployment` memory entry with a new port and revisit the relevant deploy files before continuing.

- [ ] **Step 4: Copy deploy/ over to the Pi**

```
rsync -avz deploy/ pi:/tmp/authlyn-deploy/
```

Expected: shells through, lists the 9 deploy files.

- [ ] **Step 5: Copy the env file to the Pi, then run the bootstrap**

`AUTHLYN_ENV_FILE` is a Pi-side path, so the .env has to land there first:

```
scp /tmp/authlyn.env pi:/tmp/authlyn.env
ssh pi sudo AUTHLYN_ENV_FILE=/tmp/authlyn.env \
            AUTHLYN_GITHUB_TOKEN="$AUTHLYN_GITHUB_TOKEN" \
            bash /tmp/authlyn-deploy/pi-bootstrap.sh
```

Expected output ends with:

```
[bootstrap] reloading caddy

Bootstrap complete.

Next manual step (off-Pi):
  Add a router port-forward on the TP-Link admin UI:
    External port 8444 → 192.168.0.153:8444 (TCP)
...
```

- [ ] **Step 6: Verify state on the Pi**

```
ssh pi 'systemctl is-active surrealdb.service'
ssh pi 'systemctl list-timers authlyn-updater.timer --no-pager'
ssh pi 'ls -la /opt/authlyn/ /var/lib/surrealdb/'
ssh pi 'sudo ss -tlnp "( sport = :8000 or sport = :8444 )"'
```

Expected:
- `surrealdb.service` is `active`.
- `authlyn-updater.timer` is listed with `Next:` a near-future time.
- `/opt/authlyn/` contains `.env`, `.github_token`, `pi-updater.sh`, `media/`.
- `/var/lib/surrealdb/` exists.
- `:8000` and `:8444` listening.

- [ ] **Step 7: Clean up local + Pi temp files**

```
rm /tmp/authlyn.env
ssh pi 'rm -f /tmp/authlyn.env'
ssh pi 'rm -rf /tmp/authlyn-deploy'
unset AUTHLYN_GITHUB_TOKEN
```

(No commit — Task 14 only changes the Pi.)

---

## Task 15: Add router port-forward for :8444

**Files:** none. Action is on the TP-Link router admin UI.

- [ ] **Step 1: Open the router admin UI**

In a browser, go to `http://192.168.0.1/` (or whatever the TP-Link LAN admin URL is) and log in.

- [ ] **Step 2: Add a port-forward**

Navigate to `Forwarding → Virtual Servers` (or the TP-Link equivalent). Add:
- Service Port: `8444`
- Internal IP: `192.168.0.153` (the Pi)
- Internal Port: `8444`
- Protocol: `TCP`
- Status: `Enabled`

Save / Apply.

- [ ] **Step 3: Verify externally**

From outside the LAN (e.g., from a phone on cellular), run:

```
curl -sI https://authlyn.tplinkdns.com:8444/ -o /dev/null -w '%{http_code} %{ssl_verify_result}\n'
```

Expected before first deploy: `502 0` (Caddy is up, TLS is good, but no upstream listening yet on :8081 because authlyn.service hasn't received an artifact). If you get a connection timeout, the port-forward isn't routing yet — re-check the router config.

(No commit.)

---

## Task 16: Create `release` branch and trigger first deploy

**Files:** none modified locally beyond branch state.

- [ ] **Step 1: Run the no-remnants guard locally**

```
./scripts/check-no-remnants.sh
```

Expected: `no-remnants check OK`.

- [ ] **Step 2: Create the release branch from main**

```
git checkout -b release
git push -u origin release
git checkout main
```

Expected: GitHub Actions starts a `Build and publish rolling release` run within ~30 seconds. Confirm with:

```
gh run list --workflow=build-release.yml --limit 1
```

Expected: a run with status `in_progress` (or `queued`).

- [ ] **Step 3: Watch the run to completion**

```
gh run watch
```

Expected: all steps green; finishes with a release-update step. The run takes ~3-5 minutes for a cold cache.

- [ ] **Step 4: Confirm the release has exactly one asset**

```
gh release view latest --json assets --jq '.assets[].name'
```

Expected: a single line, `authlyn-<sha_short>.tar.gz`. If more than one is listed, the prune step has misfired — investigate before continuing.

(No commit; only branch state changed remotely.)

---

## Task 17: End-to-end verification and memory update

Spec section: *Verification.*

**Files:**
- Update: project memory entry `pi-deployment`.

- [ ] **Step 1: Wait for the Pi puller (≤ 5 minutes from release publish)**

```
ssh pi journalctl -u authlyn-updater.service --since '6 minutes ago' --no-pager | tail -30
```

Expected: a recent run with lines:

```
[...] pi-updater: new build available: remote sha_short=<sha>, installed=, asset=authlyn-<sha>.tar.gz
[...] pi-updater: downloaded ... bytes); extracting
[...] pi-updater: installing
[...] pi-updater: restarting authlyn
[...] pi-updater: deployed sha=<full> (sha_short=<short>) successfully
```

If the puller hasn't fired yet, kick it manually:

```
ssh pi sudo systemctl start authlyn-updater.service
```

Then re-tail.

- [ ] **Step 2: Confirm authlyn.service is active**

```
ssh pi 'systemctl is-active authlyn.service'
ssh pi 'journalctl -u authlyn.service --since "1 minute ago" --no-pager | tail -20'
```

Expected: `active`, and the log shows `SurrealDB schema applied` and `listening on http://127.0.0.1:8081`.

- [ ] **Step 3: Hit the public origin from outside the LAN**

From cellular (not the LAN):

```
curl -sI https://authlyn.tplinkdns.com:8444/
curl -s https://authlyn.tplinkdns.com:8444/ | head -20
```

Expected: `HTTP/2 200`, `content-type: text/html`, and the body contains `Welcome to Leptos` (the placeholder home page is what's in main today).

- [ ] **Step 4: Confirm KalmarOS unaffected**

```
curl -sI https://authlyn.tplinkdns.com:8443/
ssh pi 'systemctl is-active kalmaros.service'
```

Expected: `:8443` still answers, KalmarOS service active.

- [ ] **Step 5: Update the `pi-deployment` memory file with the deploy date and confirmation**

Edit `/Users/damien/.claude/projects/-Users-damien-Developer-authlyn-interactive/memory/pi-deployment.md`:

1. In the ports table caption, change `audited 2026-05-21, not yet bound` to `bound 2026-05-21`.
2. Append a new section at the bottom:

```markdown
## Deploy history

- 2026-05-21 — first successful auto-deploy. Bound ports: app `:8081` loopback, `:8444` public via Caddy. SurrealDB `:8000` loopback. Tested from outside the LAN; KalmarOS at `:8443` unaffected.
```

(No git commit — memory lives outside the repo.)

---

## Task 18: Recovery test

Spec section: *Verification — recovery test.*

**Files:**
- Temporary: a throwaway commit on `main` that intentionally panics, then a revert.

- [ ] **Step 1: Create a deliberately-broken commit on main**

In `src/main.rs`, after the existing `db::connect_with_retries` call, add a panic:

```rust
panic!("RECOVERY TEST — should not deploy");
```

Then:

```
git add src/main.rs
git commit -m "test: recovery-test panic (will be reverted)"
git push origin main:release
```

- [ ] **Step 2: Watch CI build and confirm it succeeds**

```
gh run watch
```

(The panic is a runtime issue, not a compile error — CI will build it fine.)

- [ ] **Step 3: Watch the Pi reject the bad build**

```
ssh pi journalctl -u authlyn-updater.service --since '10 minutes ago' --no-pager
```

Expected: the puller downloads, swaps, restarts, but the post-restart `is-active` check fails because the binary panics on startup. The log shows `ERROR: service is not active after restart`.

```
ssh pi 'systemctl is-active authlyn.service'
```

Expected: `activating` or `failed` (the unit's `Restart=on-failure` keeps cycling but never becomes `active`).

The public origin is now 502:

```
curl -sI https://authlyn.tplinkdns.com:8444/ | head -1
```

Expected: `HTTP/2 502`.

- [ ] **Step 4: Revert and re-ship**

```
git revert HEAD
git push origin main:release
```

- [ ] **Step 5: Watch the recovery**

```
gh run watch
ssh pi journalctl -u authlyn-updater.service -f
```

Within 5 minutes the puller picks up the reverted build and `authlyn.service` becomes `active` again. The public origin returns to `200`.

- [ ] **Step 6: Final verification**

```
curl -sI https://authlyn.tplinkdns.com:8444/ | head -1
```

Expected: `HTTP/2 200`.

(No commit — main is already clean post-revert.)

---

## Self-review checklist (already run against the spec; recorded for transparency)

- **Spec coverage.** Every major spec section maps to a task:
  - Topology, port audit → Tasks 4 + 14 + 17
  - Build & release pipeline → Tasks 2 + 12
  - Pi-side updater → Tasks 7 + 10
  - App + DB supervision → Tasks 5 + 6, plus retry in Task 1
  - TLS + routing → Task 4 + Task 11 step 8
  - Paths & secrets → Tasks 3 + 11
  - One-time Pi setup → Tasks 11 + 14
  - Branch model → Tasks 13 + 16
  - No-remnants discipline → Task 9 + Task 12 step 2
  - Verification → Tasks 16-18
- **Placeholder scan.** No "TBD" / "implement later" / vague directives. Every code block is complete.
- **Type / name consistency.** `authlyn-interactive` binary name, `authlyn` user, `surrealdb` user, `/opt/authlyn`, `/var/lib/surrealdb`, ports `:8000`/`:8081`/`:8444`, `release` branch, `latest` release tag, `authlyn-<sha_short>.tar.gz` asset naming — consistent across all 18 tasks.
