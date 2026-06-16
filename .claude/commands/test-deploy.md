---
name: test-deploy
description: Build the current committed HEAD and deploy it to the authlyn TEST DECK (novahome, NOT prod). Push → novahome fetch+build → deploy-test.sh → health check.
---
Deploy the current committed HEAD of the working branch to the **test deck** (the shared review surface the owner co-tests on his phone). This is the test deck on **novahome** — NEVER production. Source `~/.cargo/env` locally if needed; novahome already has the toolchain.

GUARDRAILS (stop and report if violated):
- Deck only: `https://192.168.0.239:3434`, served by the `authlyn-test` systemd service, ns `authlyn` / db `test`. NEVER touch prod, `main`, or `SURREAL_DB=prod`.
- Must be on a FEATURE branch (not `main`). The working tree must be COMMITTED (the deck builds from its own clone, so anything uncommitted/unpushed won't ship).
- The deck is x86_64; the release is built ON novahome (`/home/damien/authlyn-testdeck`), not cross-compiled from the Mac.

STEPS:
1. Verify branch + clean tree: `git -C /Users/damien/Developer/authlyn-interactive status --short --branch`. Capture the short SHA: `git rev-parse --short HEAD`. Abort if on `main` or if there are uncommitted code changes that should ship.
2. Push the branch to BOTH remotes (they serve different purposes — push both):
   - **GitHub backup** (off-machine backup of the work vs data loss — owner ruling 2026-06-16; NOT how the deck builds): `git push origin <branch>`.
   - **Deck source** — novahome `~/authlyn-testdeck`'s `origin` is the LOCAL bare `/home/damien/authlyn-testdeck.git`, NOT GitHub (and novahome CANNOT auth to the private GitHub repo). Push there or step 3 builds STALE code: `GIT_SSH_COMMAND="ssh -i ~/.ssh/id_ed25519_novahome" git push ssh://damien@192.168.0.239/home/damien/authlyn-testdeck.git <branch>`. (The step-3 SHA-verify is what catches a stale bare repo — do not skip it.)
3. Build on novahome (SSH `-i ~/.ssh/id_ed25519_novahome damien@192.168.0.239`):
   `cd ~/authlyn-testdeck && git fetch origin <branch> && git reset --hard FETCH_HEAD && git rev-parse --short HEAD && export PATH=$HOME/.cargo/bin:$PATH && cargo leptos build --release`
   Use `git reset --hard FETCH_HEAD` — NOT `git checkout <sha>`, which can fail with "pathspec did not match" right after a fetch and SILENTLY leave the OLD commit checked out (the build then ships nothing). **Verify the printed SHA equals `<sha>` BEFORE deploying**, and capture cargo's real exit via `${PIPESTATUS[0]}` (a `| tail` pipe hides the cargo exit code). This produces `target/release/authlyn-interactive` + `target/site`. (Slow — minutes; run in the background and wait.)
4. Swap + restart (SSH, sudo nopass works): `sudo /opt/authlyn-test/deploy-test.sh`. The script keeps one `.bak` generation, restarts `authlyn-test`, and self-health-checks (`127.0.0.1:8082`). It builds NOTHING — step 3 must have produced the binary first.
5. Verify from the Mac: `curl -sk -o /dev/null -w '%{http_code}\n' -m 8 https://192.168.0.239:3434/` (expect a normal response, not a connection failure), and confirm the deployed SHA matches `<sha>`.
6. Report: deployed SHA, service state, and HTTP probe result. If the deck is the iOS-sim gate target, note that the sim now shows the new build.

ROLLBACK if unhealthy: the previous binary is at `/opt/authlyn-test/authlyn-interactive.bak` and site at `site.bak` — restore + `sudo systemctl restart authlyn-test`.
