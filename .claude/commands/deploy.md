---
name: deploy
description: Build a release and deploy it to the LIVE systemd service (production on novahome)
---
Production = **novahome** (mendicant-bias host), service `authlyn-prod`, `/opt/authlyn-prod`, app `127.0.0.1:8083`, public `https://authlyn.damienmoon.sh` (cloudflared tunnel). DB = `authlyn/prod` on the novahome SurrealDB. The retired **fenrir** path (`/opt/authlyn/deploy.sh`, `authlyn.service`) is gone — do not target it.

**This is production and outward-facing — confirm before the privileged step unless the user clearly just said to deploy.** Prod now runs **v27.0.2 `mendicant-bias`** (v27.0.0 shipped 2026-06-22, v27.0.1 promoted 2026-06-23, v27.0.2 promoted 2026-06-29 [AGPL-3.0-or-later relicense]; merge-to-`main` is the autodeploy trigger). Each prod promotion stays a **separate owner-gated** decision — don't push `main` / promote unless the owner explicitly says so.

## Routine path — GitHub Actions (preferred)
`.github/workflows/deploy.yml` auto-deploys **every push to `main`** (docs-only pushes ignored) on the self-hosted runner on novahome: checkout → `cargo leptos build --release` → backup prod DB to `/data/prod_backups` → `sudo -n /opt/authlyn-prod/deploy.sh "$GITHUB_WORKSPACE"` (swap + health-check `:8083` + auto-rollback). Trigger manually with `gh workflow run "Deploy to novahome"`. Because the trigger is merge-to-`main`, gating v27 = gating the merge.

**One-time runner registration (owner-interactive — required before the workflow can run):** on novahome, as user `damien`:
```
mkdir -p ~/actions-runner && cd ~/actions-runner
curl -O -L https://github.com/actions/runner/releases/latest/download/actions-runner-linux-x64.tar.gz   # check the current asset name
tar xzf actions-runner-linux-x64.tar.gz
./config.sh --url https://github.com/<owner>/authlyn-interactive --token <RUNNER_TOKEN> --labels novahome --name novahome
sudo ./svc.sh install damien && sudo ./svc.sh start
```
Get `<RUNNER_TOKEN>` from GitHub → repo Settings → Actions → Runners → New self-hosted runner (Linux x64). The runner **must** run as `damien` (so `~/.cargo` is on PATH and the scoped NOPASSWD sudoers entry for `/opt/authlyn-prod/deploy.sh` applies).

## Manual path — hotfix / ad-hoc (no runner needed)
Build on novahome and invoke the same bridge. From the Mac:
1. Push the commit to the novahome bare repo and check it out in the prod worktree:
   `git push ssh://novahome/home/damien/authlyn-testdeck.git <branch-or-sha>:refs/heads/<ref>` then on novahome `cd ~/authlyn-prod && git fetch && git checkout -f <sha>`.
2. `ssh novahome 'export PATH=$HOME/.cargo/bin:$PATH && cd ~/authlyn-prod && cargo leptos build --release'` — slow, wait for it. If the build fails, STOP.
3. `ssh novahome 'sudo -n /opt/authlyn-prod/deploy.sh /home/damien/authlyn-prod'` — rotates `.bak`, swaps the binary + `site/`, restarts `authlyn-prod`, health-checks `:8083`, **auto-rolls-back** on failure. Relay its `[deploy]` lines verbatim.
4. Confirm: `systemctl is-active authlyn-prod`, the `[deploy] HTTP probe 127.0.0.1:8083 -> 200` line, and `curl -so /dev/null -w '%{http_code}' https://authlyn.damienmoon.sh/`. Report the deployed git rev.

## Notes
- The privileged swap+restart lives entirely in root-owned `/opt/authlyn-prod/deploy.sh` (NOPASSWD-scoped to that one script via `/etc/sudoers.d/authlyn-deploy`); the build (runner or manual) only invokes it. The script builds nothing.
- Roll back on command: `sudo /opt/authlyn-prod/deploy.sh --rollback` (restores the previous `.bak` + restarts).
- Untouched by deploy: `/opt/authlyn-prod/.env`, the DB (`authlyn/prod`), and `/data/authlyn-prod/media`. The pre-deploy DB backup is the GHA path's safety net (`/data/prod_backups`, newest 15 kept).
- The **test deck** (`authlyn-test` on novahome, `:8082`, `authlyndev.damienmoon.sh`) is updated separately (`/opt/authlyn-test/deploy-test.sh`) — NOT by this command.
