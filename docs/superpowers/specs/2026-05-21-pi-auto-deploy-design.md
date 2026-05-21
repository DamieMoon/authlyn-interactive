# Pi auto-deploy for authlyn-interactive — design

## Context

`authlyn-interactive` is being developed against CLAUDE.md's testing posture: Damien tests the running app **mostly remotely** via the DDNS hostname, not from inside the LAN. Anything that only works on `localhost` / `192.168.*` blocks him. The routing plan currently in flight (`/Users/damien/.claude/plans/i-turned-on-plan-purring-wozniak.md`) will, by its step 8, need a public origin to be meaningfully testable. This spec covers how to land that public origin and keep it fresh automatically.

The Pi already hosts another project, `ku-chronicles-kalmar-os` ("KalmarOS"), which has a working auto-deploy pattern: CI cross-compiles, publishes a rolling `latest` GitHub Release, a systemd timer on the Pi polls and atomically swaps. That pattern is the starting point. This spec adapts it to `authlyn-interactive` (Leptos SSR + axum + SurrealDB instead of split server/client + SQLite), and resolves the collision that KalmarOS currently occupies the `authlyn.tplinkdns.com:8443` slot the project's name implies.

## Decisions locked (brainstorm output)

- **KalmarOS stays.** It keeps `authlyn.tplinkdns.com:8443` and `/opt/kalmaros/` untouched. authlyn-interactive moves to a different slot on the same Pi.
- **Public origin:** same hostname, new port — `authlyn.tplinkdns.com:8444`.
- **Build strategy:** CI cross-compiles to `aarch64-unknown-linux-gnu` via `cargo leptos build` (with `cargo-zigbuild` as the linker for that target). No local `deploy.sh` in v1.
- **SurrealDB:** dedicated systemd unit on the Pi, file-backed at `/var/lib/surrealdb/`. Framed as shared Pi infra so future projects can reuse it.
- **Full adaptation:** no `kalmaros` / `KalmarOS` / `ku-chronicles` strings in this repo. Enforced by a grep job in CI.

## Topology

```
Internet
   ├─ :80    ─┐
   ├─ :8443  ─┼─→ Pi router UPnP → Pi NIC
   └─ :8444  ─┘  (new port-forward)

Pi (192.168.0.153)
   ├─ Caddy
   │   ├─ :80    Let's Encrypt HTTP-01 (shared by both apps)
   │   ├─ :8443  authlyn.tplinkdns.com → 127.0.0.1:8080  (KalmarOS, untouched)
   │   └─ :8444  authlyn.tplinkdns.com → 127.0.0.1:8081  (authlyn-interactive, new)
   │
   ├─ surrealdb user → surrealdb.service      :8000 loopback, file-backed
   ├─ authlyn user   → authlyn.service        :8081 loopback (After=surrealdb.service)
   └─ damien user    → authlyn-updater.{service,timer}  (poll + swap every 5 min)
```

## Port audit (2026-05-21)

`ssh pi sudo ss -tlnp` against the live Pi confirmed all three proposed ports are free:

| Port | Status | Use |
|------|--------|-----|
| `:8000` | free | SurrealDB loopback |
| `:8081` | free | authlyn-interactive loopback (behind Caddy) |
| `:8444` | free | authlyn-interactive public origin |

Already in use on the Pi: `:22` (sshd), `:80` (Caddy), `:443` (xray), `:2019` (Caddy admin API, loopback-only), `:8080` (KalmarOS), `:8443` (Caddy → KalmarOS). The audit will be re-run by the bootstrap script before binding; if any of our picks have been taken in the meantime it fails loudly and refuses to proceed until the port choice is updated here and in the `pi-deployment` memory entry.

## Build & release pipeline

`.github/workflows/build-release.yml`:

- **Trigger:** push to the `release` branch. `main` keeps active development; ship by `git push origin main:release`. Manual `workflow_dispatch` available for re-running a build.
- **Permissions:** `contents: write` for `softprops/action-gh-release` to update the tag + assets.
- **Concurrency:** `group: build-release`, `cancel-in-progress: true` — back-to-back pushes only ship the latest.

**Steps:**

1. `actions/checkout@v4` with `fetch-depth: 1`.
2. `dtolnay/rust-toolchain@stable` with `targets: aarch64-unknown-linux-gnu`.
3. `Swatinem/rust-cache@v2` (cache cargo registry + target).
4. `mlugg/setup-zig@v2` at a pinned version (currently `0.13.0`).
5. `cargo install --locked cargo-zigbuild`.
6. `cargo install --locked cargo-leptos`.
7. Write a `.cargo/config.toml` snippet (or commit one) that sets `cargo-zigbuild` as the linker for `aarch64-unknown-linux-gnu`.
8. `cargo leptos build --release --bin-target-triple aarch64-unknown-linux-gnu` — produces the cross-compiled server binary and the WASM/CSS bundle in `target/site/`.
9. Stage the artifact:
   - Copy `target/aarch64-unknown-linux-gnu/release/authlyn-interactive` → `stage/`.
   - Copy `target/site/` → `stage/site/`.
   - Emit `stage/build.json` with `{sha, sha_short, built_at, built_epoch, ref}`.
10. `tar -czf authlyn-<sha_short>.tar.gz -C stage .`
11. Update rolling `latest` release via `softprops/action-gh-release@v2` (`prerelease: true`, `make_latest: 'false'`).
12. Prune superseded assets (`gh release delete-asset` for every asset that isn't the one just uploaded) so the Pi puller never sees stale tar.gz files.

**Fallback if `--bin-target-triple` is flaky.** Split the build into two cargo invocations: `cargo-zigbuild build --release --target aarch64-unknown-linux-gnu` for the server bin, then `cargo leptos build --release --lib-only` (or `cargo build --release --features hydrate --target wasm32-unknown-unknown` plus the CSS / wasm-bindgen post-processing cargo-leptos would have done). Merge the outputs into `stage/`. Verify the integrated path works first; only fall back if it doesn't.

## Pi-side updater

`deploy/pi-updater.sh` — ported from the KalmarOS script, fully renamed:

- **Auth:** reads PAT from `/opt/authlyn/.github_token` (mode `0600`, owned by `authlyn:authlyn`). Refuses to run if missing or empty.
- **Manifest fetch:** `GET https://api.github.com/repos/DamieMoon/authlyn-interactive/releases/tags/latest` with the PAT.
- **Asset selection:** the rolling `latest` release accumulates assets per CI run, and GitHub returns them in stable alphabetical order, which would pin the puller to whichever asset has the lexicographically smallest SHA prefix. Defense: `jq '[.assets[]] | sort_by(.created_at) | last'` to pick the most recently uploaded one. (CI's prune step is the belt-and-suspenders.)
- **SHA compare:** extract `<sha_short>` from asset name `authlyn-<sha_short>.tar.gz`, compare to `jq -r .sha_short` in `/opt/authlyn/build.json`. If equal and `build.json` exists, log "up to date" and exit 0.
- **Download:** to `mktemp -d` staging, with `Accept: application/octet-stream` and the PAT. Verify size > 0.
- **Extract + validate:** `tar -xzf` into staging. Require `{authlyn-interactive binary, site/, build.json}` all present. Verify `jq -r .sha_short` inside the extracted `build.json` matches the asset name's sha — guards against an artifact whose name lies about its contents.
- **Atomic swap:**
  - `sudo install -m 0755 -o authlyn -g authlyn .../authlyn-interactive /opt/authlyn/authlyn-interactive.new`
  - `sudo mv -f /opt/authlyn/authlyn-interactive.new /opt/authlyn/authlyn-interactive` (single `rename(2)`)
  - `sudo rsync -a --delete --chown=authlyn:authlyn .../site/ /opt/authlyn/site/`
  - `sudo install -m 0644 -o authlyn -g authlyn .../build.json /opt/authlyn/build.json`
- **Restart:** `sudo systemctl restart authlyn`. Wait 3s, check `systemctl is-active --quiet authlyn`; log error and exit 1 if not (the timer will fire again in 5 min).

`deploy/authlyn-updater.service` — `Type=oneshot`, `User=damien`, `Group=damien`, `ExecStart=/opt/authlyn/pi-updater.sh`. Minimal hardening (the script needs `sudo` via NOPASSWD for install ops).

`deploy/authlyn-updater.timer` — `OnBootSec=30s`, `OnUnitActiveSec=5min`, `Persistent=true`, `WantedBy=timers.target`.

## App + DB supervision

Three new systemd units land in this spec; one already exists on the Pi (`caddy.service`) and stays untouched.

### `deploy/surrealdb.service`

```
[Unit]
Description=SurrealDB (shared Pi infra)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=surrealdb
Group=surrealdb
WorkingDirectory=/var/lib/surrealdb
ExecStart=/usr/local/bin/surreal start \
    --user root --pass root \
    --bind 127.0.0.1:8000 \
    file:///var/lib/surrealdb
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

Root creds are placeholder-equivalents for v1; the auth follow-up plan replaces them with a SurrealDB scope plus per-app credentials.

### `deploy/authlyn.service`

```
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

`ReadWritePaths=/opt/authlyn/media` reserves the encrypted-attachment storage location for routing plan step 9. No other writable paths inside `/opt/authlyn` — the binary + `site/` + `build.json` get replaced by the updater running as `damien` (via NOPASSWD sudo), not by the app itself.

### DB-connect retry on cold start

systemd reports `surrealdb.service` as "started" the moment the `surreal` process exists, not when its WebSocket listener has finished binding `127.0.0.1:8000`. After a fresh boot the gap is ~hundreds of milliseconds. `authlyn.service`'s `After=surrealdb.service` won't bridge it; if `db::connect()` runs in that window it panics, and `Restart=on-failure`+`RestartSec=2s` cycles the unit — which the puller's 3-second post-restart `is-active` check can land inside, treating the cycling unit as broken.

Fix lives in the app: wrap `db::connect()` with a bounded retry (e.g., 10 attempts, 500 ms backoff, ~5 s total). On exhaustion, panic with a clear error so it shows in `journalctl -u authlyn`. The retry helper lives in `src/db.rs` (so tests can opt out by calling `connect()` directly) and is the one main.rs calls. The implementation plan owns the exact code shape; this spec mandates that the retry exists.

### `deploy/authlyn-updater.service` and `deploy/authlyn-updater.timer`

See the Pi-side updater section above for behavior. Files mirror the KalmarOS shape with all strings renamed.

## TLS + routing

Append a new site block to `/etc/caddy/Caddyfile`:

```caddy
authlyn.tplinkdns.com:8444 {
    encode zstd gzip

    # authlyn-interactive's Leptos+axum server binds 127.0.0.1:8081.
    # Caddy terminates TLS and reverse-proxies.
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
```

KalmarOS's existing `:8443` block is unchanged. Caddy auto-issues a Let's Encrypt cert for the hostname-on-8444 site; since the SAN is the same as the existing `:8443` cert, Caddy reuses the cert material internally — no second issuance.

A copy of this site block lives at `deploy/Caddyfile.authlyn-interactive.snippet` in the repo so the bootstrap script can append it idempotently. The full `/etc/caddy/Caddyfile` lives on the Pi; this repo only ships its own snippet.

## Paths, ownership, secrets

```
/opt/authlyn/                  authlyn:authlyn  0750
├── authlyn-interactive        authlyn:authlyn  0755   (the binary)
├── site/                      authlyn:authlyn  0755   (Leptos WASM/CSS bundle)
├── .env                       authlyn:authlyn  0600   (runtime config)
├── .github_token              authlyn:authlyn  0600   (PAT, contents:read)
├── build.json                 authlyn:authlyn  0644   ("what's running")
├── pi-updater.sh              authlyn:authlyn  0755
└── media/                     authlyn:authlyn  0750   (encrypted attachments, reserved)

/var/lib/surrealdb/            surrealdb:surrealdb  0750  (file-backend data dir)
```

`/opt/authlyn/.env` for v1 holds exactly:

```
# SurrealDB
SURREAL_URL=ws://127.0.0.1:8000
SURREAL_USER=root
SURREAL_PASS=root
SURREAL_NS=authlyn
SURREAL_DB=prod

# Leptos runtime (read by `get_configuration(None)` in main.rs).
# Without these the binary defaults to :3000 and looks for site assets
# under `./target/site/` (CWD-relative), neither of which matches the Pi
# layout. SITE_PKG_DIR mirrors the cargo-leptos default.
LEPTOS_OUTPUT_NAME=authlyn-interactive
LEPTOS_SITE_ROOT=site
LEPTOS_SITE_PKG_DIR=pkg
LEPTOS_SITE_ADDR=127.0.0.1:8081
```

`SURREAL_DB=prod` differs from dev's `dev` so a dev SurrealDB inadvertently pointed at the Pi can't trample prod data. Real DB auth, scoped tokens, and password-derived pickle keys are out of scope here and handled by the auth follow-up plan.

`/opt/authlyn/.github_token` holds a fine-grained PAT with `contents:read` on this repo only. Owner: `authlyn`. The puller (`damien` via NOPASSWD sudo) reads it via `sudo cat`.

## One-time Pi setup (bootstrap)

`deploy/pi-bootstrap.sh` — idempotent. Run once via `ssh pi sudo bash -s < deploy/pi-bootstrap.sh`. Stops loudly on the first error. Steps:

1. Re-run the port audit. If any of `:8000`, `:8081`, `:8444` is taken, bail with a diff against the recorded ports and a pointer to update this spec + the `pi-deployment` memory entry.
2. Install the SurrealDB binary at a pinned version. Dev is on `v3.0.4` and the Rust SDK in `Cargo.toml` is pinned to `=3.1.0-beta.3`; that exact combo is what's tested working. Bootstrap downloads `https://github.com/surrealdb/surrealdb/releases/download/v3.0.4/surreal-v3.0.4.linux-arm64.tgz`, verifies it against a SHA-256 checked into the bootstrap script (compute on first setup), extracts to `/usr/local/bin/surreal`, and confirms `surreal version` prints `3.0.4`. **Do not** pipe `https://install.surrealdb.com | sh` — that fetches latest and silently introduces version skew. When bumping the SDK or the binary, update both in lockstep: pin in `Cargo.toml`, version + checksum in the bootstrap script, dev binary on the Mac. All three must move together.
3. Create system users + groups: `authlyn` (no login shell, no home), `surrealdb` (same).
4. Create dirs with the ownership and modes documented above: `/opt/authlyn`, `/opt/authlyn/media`, `/var/lib/surrealdb`. `/var/log/caddy` is already present from KalmarOS.
5. Write `/opt/authlyn/.env` and `/opt/authlyn/.github_token` from values passed as environment variables to the bootstrap script. The script never echoes them; only confirms the files exist with correct mode + owner.
6. Install the three unit files into `/etc/systemd/system/`. `systemctl daemon-reload`. Enable + start `surrealdb.service` and `authlyn-updater.timer`. Enable but don't start `authlyn.service` (first start fails until the puller drops the binary in).
7. Install a scoped sudoers fragment for `damien` covering only the install operations the puller needs (`/usr/bin/install`, `/bin/mv`, `/bin/rsync`, `/bin/chown`, `/bin/systemctl restart authlyn`). One file in `/etc/sudoers.d/authlyn-updater`, mode `0440`, validated with `visudo -c`.
8. Append `deploy/Caddyfile.authlyn-interactive.snippet` to `/etc/caddy/Caddyfile` if its marker comment isn't already present. `caddy validate --config /etc/caddy/Caddyfile`. `systemctl reload caddy`.
9. Print a clear manual-step reminder: **add the router port-forward `external :8444 → 192.168.0.153:8444`** (the only step that isn't automatable from the Pi).

After step 9, every `git push origin main:release` is hands-off until step 9's port-forward gets removed.

## Branch model

The repo has only `main` today. Two changes:

1. Create the `release` branch from current `main`: `git checkout -b release && git push -u origin release`.
2. Add a "Branching and auto-deploy" section to `CLAUDE.md` documenting:
   - `main` is the working branch.
   - `git push origin main:release` triggers a ship.
   - The Pi auto-pulls within 5 minutes.
   - Recovery: pushing an older `main` SHA via `git push origin <sha>:release --force-with-lease` retracts to a known-good build (the workflow re-runs and the puller picks up the new artifact).

## No-remnants discipline

A CI step (early in `build-release.yml`) runs:

```sh
if git grep -inE 'kalmaros|KalmarOS|kalmaroS|ku-chronicles' -- ':!docs/superpowers/specs/' ; then
  echo "::error::KalmarOS-derived names leaked into the codebase"
  exit 1
fi
```

The `:!docs/superpowers/specs/` exclude means this design doc can refer to KalmarOS for context without failing the build; everything else must be clean.

Pre-commit equivalent (`scripts/check-no-remnants.sh`) ships in the repo for local runs.

## Verification

End-to-end success criteria for the first deploy:

0. **Pi is bootstrapped.** `deploy/pi-bootstrap.sh` has completed successfully and `ssh pi systemctl is-active surrealdb.service` returns `active` and `ssh pi systemctl list-timers authlyn-updater.timer` shows the timer armed. **This must happen before** the first `release` push. Otherwise CI produces an artifact no Pi will pull and the run silently misses.
1. `git push origin main:release` fires the workflow. CI run passes.
2. The rolling `latest` GH Release shows exactly one asset, `authlyn-<sha_short>.tar.gz`, with the SHA from `release`'s HEAD.
3. Within ≤ 5 minutes (timer tick) the Pi's `authlyn-updater.service` runs:
   - `journalctl -u authlyn-updater.service` shows `new build available` → download → install → `restarting authlyn` → no errors.
4. `systemctl is-active authlyn.service` → `active`.
5. `journalctl -u authlyn.service --since '1 minute ago'` shows `SurrealDB schema applied` (from `main.rs:25`) and `listening on http://127.0.0.1:8081`.
6. From a network *outside* the LAN: `curl -I https://authlyn.tplinkdns.com:8444/` returns `200` and a Caddy-issued TLS cert chain (Let's Encrypt root, SAN matches the hostname).
7. Browser hits the same URL and renders the Leptos welcome page (placeholder until routing plan step 10 lands the chat UI).
8. KalmarOS is unaffected: `curl -I https://authlyn.tplinkdns.com:8443/` continues to work, `systemctl is-active kalmaros` is still `active`.

Recovery test: deploy a deliberately broken commit (e.g., `panic!()` in `main`), confirm the puller refuses to restart into a non-active state. Push a known-good `main` SHA back to `release`. Within 5 minutes, the Pi is back on the good build.

## Out of scope (follow-up plans)

- **DB credentials, auth, scopes.** The auth follow-up plan replaces root/root and the placeholder env vars with a SurrealDB scope + per-user pickle keys.
- **Schema migrations.** `db::apply_schema` runs `IF NOT EXISTS` on every boot, which is safe for *adding* new tables/fields but cannot alter existing definitions (e.g., adding a `tier` variant or the `summary_*` columns from the privacy-slider follow-up). Until the auth plan lands, prod data on the Pi is treated as **disposable** — bumping the schema means `surreal stop` + `rm -rf /var/lib/surrealdb/*` + restart. A proper migration mechanism (numbered `.surql` files applied in order, tracked in a `schema_migrations` table) lands alongside the auth plan when there's actual user data to preserve.
- **Backups.** Same reason — disposable until auth lands. Backup automation comes with real users.
- **Observability beyond journalctl.** Loki/Prom/Grafana not in v1.
- **Blue-green / canary.** v1 is restart-in-place. Zero-downtime deploy is a later concern.
- **Cert renewal monitoring.** Caddy auto-renews; we get alert noise only when the existing Let's Encrypt notification email surfaces a problem.
- **The actual chat UI.** This spec ships the pipeline. The Leptos welcome page is what lands until the routing plan reaches step 10.

## Files this spec produces (for the implementation plan)

New files in this repo:

- `.github/workflows/build-release.yml`
- `.cargo/config.toml` (cross-compile linker config)
- `deploy/pi-updater.sh`
- `deploy/pi-bootstrap.sh`
- `deploy/surrealdb.service`
- `deploy/authlyn.service`
- `deploy/authlyn-updater.service`
- `deploy/authlyn-updater.timer`
- `deploy/Caddyfile.authlyn-interactive.snippet`
- `deploy/sudoers.authlyn-updater`
- `deploy/.env.example`
- `scripts/check-no-remnants.sh`

Modified:

- `CLAUDE.md` — append the "Branching and auto-deploy" section.

Updated post-bootstrap:

- The `pi-deployment` memory entry — replace the "TBD" prod ports with the audited choices, and append the actual deploy date.

## References

- KalmarOS template files (read 2026-05-21): `~/Developer/ku-chronicles-kalmar-os/{.github/workflows/build-release.yml, deploy/*}`. Adapted, not copied verbatim. No KalmarOS strings remain in this project.
- Routing plan: `/Users/damien/.claude/plans/i-turned-on-plan-purring-wozniak.md`.
- Project memory `pi-deployment` for Pi access details, current port choices, and post-bootstrap updates.
