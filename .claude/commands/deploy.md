---
name: deploy
description: Build a release and deploy it to the LIVE systemd service (production)
---
> ⚠️ **FROZEN — this targets the legacy fenrir prod path** (`/home/damien/authlyn-interactive` + `sudo /opt/authlyn/deploy.sh`). fenrir is RETIRED; mendicant-bias prod = **novahome**, and both `.github/workflows/deploy.yml` and this skill must be **repointed to novahome before any v27 deploy** (CLAUDE.md Deploy · ctx · learnings · I2). The novahome **test deck** is updated separately over SSH (the `authlyn-test` service), NOT by this skill. Do NOT run this against the retired host.

Deploy the **current working tree** to the live `authlyn.service` (production at `/opt/authlyn`, the instance real users hit). Source `~/.cargo/env` first if `cargo` isn't on PATH.

**This is production and outward-facing — confirm before the privileged step unless the user clearly just said to deploy.**

Pre-flight (report, then proceed):
1. Show `git -C /home/damien/authlyn-interactive` current branch, `rev-parse --short HEAD`, and whether the tree is clean. Note that the deploy ships THIS commit (not necessarily `main`). If the user intended the merged release, they should merge to `main` + check it out first.
2. Confirm the one-time bridge is installed: `test -x /opt/authlyn/deploy.sh` and `sudo -n /opt/authlyn/deploy.sh --help >/dev/null 2>&1 || true`. If `/opt/authlyn/deploy.sh` is missing, STOP and tell the user to run the one-time install (see `~/authlyn-deploy.sh` header).

Steps:
1. `cargo leptos build --release` in `/home/damien/authlyn-interactive`. Slow (several minutes) — wait for it. If the build fails, STOP (do not deploy).
2. `sudo /opt/authlyn/deploy.sh` — it rotates `.bak`, swaps the binary + `site/`, restarts the service, health-checks, and **auto-rolls-back** on failure. Relay its `[deploy]` lines verbatim.
3. Confirm result: `systemctl is-active authlyn` and the `[deploy] HTTP probe … -> 200` line. Report the deployed git rev.

Notes:
- The privileged swap+restart lives entirely in the root-owned `/opt/authlyn/deploy.sh` (NOPASSWD-scoped to that one script); I only build + invoke it. I cannot edit what runs as root.
- Roll back on command: `sudo /opt/authlyn/deploy.sh --rollback` (restores the previous `.bak` + restarts).
- Untouched by deploy: `/opt/authlyn/.env`, `certs/`. The DB is `authlyn/prod` (unchanged by a code deploy).
