# 11 — Build, Quality Gate, Deploy, PWA

How the crate is built and checked, how it ships to the two novahome hosts, and how the
service worker rolls a new build onto an installed PWA without losing an in-progress draft.

This doc **references** the canonical sources rather than restating them:

- Exact build/run/test/check invocations, the toolchain prereqs, and the deploy
  specifics: [`CLAUDE.md`](../../CLAUDE.md).
- Every dependency's purpose, the three feature graphs, and the full
  `[package.metadata.leptos]` config: the `#`-comments in [`Cargo.toml`](../../Cargo.toml).
- The Bash allowlist and hooks: [`.claude/settings.json`](../../.claude/settings.json).
- Stack, directory layout, dev quickstart, the versioning scheme: [`README.md`](../../README.md).

Siblings: [01-overview](01-overview.md) · [04-realtime-sse](04-realtime-sse.md) (the SSE bus
the dev-reload nudge rides) · [07-ui-shell](07-ui-shell.md) (the account modal's version line
and update button) · [08-styling-chrome](08-styling-chrome.md) (the `style_lint` static scan,
the motion doctrine) · [10-nova-mcp](10-nova-mcp.md) (the `nova` binary, excluded from every
gate here).

---

## 1. The three feature graphs, at build time

The crate compiles into three mutually-exclusive graphs (the hard rule lives in
[`CLAUDE.md`](../../CLAUDE.md) and is enforced by membership in
[`Cargo.toml`](../../Cargo.toml) `[features]`):

| Graph | Target | Built by | Profile |
|---|---|---|---|
| **ssr** | native host | `cargo leptos build` bin target (`bin-features = ["ssr"]`) | `release` |
| **hydrate** | `wasm32-unknown-unknown` | `cargo leptos build` lib target (`lib-features = ["hydrate"]`) | `wasm-release` |
| **nova** | native host | **by hand** — `cargo build --release --bin nova-mcp --features nova` | `release` |

`cargo leptos build` produces both ssr and hydrate artifacts in one pass: the server binary
plus `target/site/pkg/` (JS glue + `*_bg.wasm` + the Lightning-CSS-optimized stylesheet). The
crate has **two** `[[bin]]` targets, so `bin-target = "authlyn-interactive"` is set explicitly
or cargo-leptos errors with *"Several bin targets found"*; `nova-mcp` carries
`required-features = ["nova"]` so the default / cargo-leptos build never pulls it
(`Cargo.toml` `[[bin]]`). **nova is not in any gate below — build it manually when touched.**

`wasm-release` (`Cargo.toml [profile.wasm-release]`, `inherits = "release"`) sets
`opt-level = 'z'`, `lto = true`, `codegen-units = 1`, `panic = "abort"` for the smallest WASM
bundle. The `browserquery` is pinned `"defaults, fully supports es6-module, iOS >= 15"` so
Lightning CSS keeps the `-webkit-backdrop-filter` prefix that pre-Safari-18 iOS needs for the
glass chrome (`Cargo.toml`, `browserquery` comment).

---

## 2. The quality gate (`/check`) and the pre-commit superset

### 2.1 `/check` — fmt + clippy on the two app graphs

[`.claude/commands/check.md`](../../.claude/commands/check.md) runs three steps, in order:

```
cargo fmt --all --check
cargo clippy --features ssr    --no-deps -- -D warnings          # server graph
cargo clippy --features hydrate --target wasm32-unknown-unknown --no-deps -- -D warnings   # WASM graph
```

The two clippy passes are the *only* compile-time enforcement that both the ssr and hydrate
graphs build with their disjoint dependency sets — there is no `.rs` unit test for the
cross-graph cfg split (see [01-overview](01-overview.md)). `nova` is **not** here.

### 2.2 `.githooks/pre-commit` — the same fmt+clippy plus an inline `@keyframes` scan

[`.githooks/pre-commit`](../../.githooks/pre-commit) is **opt-in**, enabled per-clone with
`git config core.hooksPath .githooks`, bypassable with `git commit --no-verify`. It sources
`~/.cargo/env` (so it works from GUI git clients), then runs:

1. `cargo fmt --all --check`
2. `cargo clippy --features ssr --no-deps -- -D warnings`
3. `cargo clippy --features hydrate --target wasm32-unknown-unknown --no-deps -- -D warnings`
4. An **inline shell** scan: for every staged `.scss` that contains `@keyframes`, it greps the
   100 lines after a keyframes block for forbidden paint/layout properties
   (`box-shadow`, `background-position`, `filter:`, `width:`, `height:`, `top:`, `left:`) and
   fails the commit if it finds one (motion doctrine `#43`).

**Correction to a common claim:** the pre-commit hook does **not** execute the
`tests/style_lint.rs` suite. Step 4 is a coarse, brace-unaware *shell* approximation that
explicitly defers to the Rust test for precision (its error message and comments tell you to
confirm with `cargo test --features ssr --test style_lint`). The authoritative static scan —
the brace-aware motion-doctrine check plus the deck-bug-class regression registries — runs only
when you invoke that `cargo test` yourself; it is part of the full test gate, not the hook's
executed commands. See [08-styling-chrome](08-styling-chrome.md) for the registries.

So the practical gate ladder is:

| Gate | fmt | clippy ssr | clippy hydrate-wasm | `@keyframes` (shell) | `style_lint` suite | full `tests/*` | nova build |
|---|---|---|---|---|---|---|---|
| `/check` | ✓ | ✓ | ✓ | — | — | — | — |
| `.githooks/pre-commit` | ✓ | ✓ | ✓ | ✓ | — (referenced only) | — | — |
| "done" for chrome / spine changes | ✓ | ✓ | ✓ | ✓ | run by hand | run by hand | run by hand if touched |

The full test suite needs `--features ssr` **and** a live SurrealDB on `ws://127.0.0.1:8000`
(see [09-testing](09-testing.md)); `/check` and the hook do not.

### 2.3 The two automatic hooks

[`.claude/settings.json`](../../.claude/settings.json) defines two harness hooks (distinct from
the git hook above):

- **`PostToolUse` (`Edit|Write`)** — runs `rustfmt --edition 2021` on any `.rs` file the agent
  writes. Do not hand-run `rustfmt` per file.
- **`SessionStart`** — prints presence of `cargo` / `cargo-leptos` / `surreal` and whether the
  dev DB is up on `:8000`.

The same file holds the Bash allowlist, including the exact nova build command and the dev-DB
start command.

---

## 3. Build-time version stamping (`build.rs`)

[`build.rs`](../../build.rs) injects two compile-time env vars via `cargo:rustc-env`:

| Env var | Value | Fallback | Consumed by |
|---|---|---|---|
| `BUILD_REV` | `git rev-parse --short HEAD` | `"dev"` | `GET /sw.js` (`CACHE_VERSION`), account modal |
| `APP_CODENAME` | line-scan of `Cargo.toml` `[package.metadata.release].codename` (no toml dep) | `"dev"` | account modal version line |

Re-stamping is triggered by `cargo:rerun-if-changed` on `.git/HEAD`, `.git/logs/HEAD`
(branch switches + every commit/checkout/reset/merge), and `Cargo.toml`. Both stamps are
best-effort: a non-git build or a reformatted `[package.metadata.release]` block silently falls
back to `"dev"` — harmless except that `BUILD_REV = "dev"` makes `CACHE_VERSION` non-unique (a
local non-served build, which never serves `/sw.js` anyway).

`BUILD_REV` is the engine of the per-release cache bust in §6. The codename scan is line-based
and brittle to a multiline/array reformat of that TOML table (`build.rs`, codename block).

> **Versioning: SemVer from v27.** `Cargo.toml` is on `version = "27.0.1"`,
> `codename = "mendicant-bias"` — the CalVer→SemVer flip shipped at the v27 release
> (2026-06-22), retiring the old `2026.6.1` / `saffron-tide` scheme. The account modal
> prints `CARGO_PKG_VERSION` + `APP_CODENAME` + `BUILD_REV` ([07-ui-shell](07-ui-shell.md)).

---

## 4. Deploy topology (both hosts on **novahome**)

Production was the retired Pi *fenrir*; it is now **novahome** (x86_64). Two systemd services
run on the same host:

| Surface | Service | App port | Public URL (cloudflared) | DB (ns/db) | Privileged bridge |
|---|---|---|---|---|---|
| **Production** | `authlyn-prod` | `127.0.0.1:8083` | `https://authlyn.damienmoon.sh` | `authlyn` / `prod` | `/opt/authlyn-prod/deploy.sh` |
| **Test deck** | `authlyn-test` | `127.0.0.1:8082` | `https://authlyndev.damienmoon.sh` | `authlyn` / `test` | `/opt/authlyn-test/deploy-test.sh` |

Both public certs are publicly-trusted (cloudflared), so iOS/WebKit accepts the `Secure`
session cookie with **no** per-device root-CA step — the WebKit cookie trap fix
(see [05-auth-privacy](05-auth-privacy.md) and [`CLAUDE.md`](../../CLAUDE.md)). The old LAN-IP
`192.168.0.239` + self-signed dev root CA is **retired** — do not probe or review there.

> **v27 shipped (2026-06-22).** `.github/workflows/deploy.yml` + [`/deploy`](../../.claude/commands/deploy.md)
> target **novahome** (`deploy.yml:1` *"Deploy to novahome"*; the retired-fenrir path is gone), and
> `mendicant-bias` was merged to `main` and promoted to prod as **v27.0.0** (tag `v27.0.0`, merge commit
> `96bad5a`). The earlier freeze — prod pinned to the CalVer `2026.6.1` build (commit `5cedd5d`) — is
> **lifted**: a push to `main` now auto-deploys to prod, so every future prod-affecting merge to `main`
> remains an explicitly owner-gated decision. **v27.0.1** (M7 deck-pass bug fixes — orbit far-server rail,
> channel open-at-newest, Station one-step-back nav, CDN `/pkg` no-cache + versioned CSS href) was promoted
> to prod **2026-06-23** (tag `v27.0.1`).

### 4.1 Production CD — push to `main`

[`.github/workflows/deploy.yml`](../../.github/workflows/deploy.yml) is *"Deploy to novahome"*.
It triggers on **push to `main`** (with `paths-ignore: ['**/*.md', 'docs/**']` so a docs-only
push never rebuilds prod) and on `workflow_dispatch`. `concurrency: deploy-novahome` with
`cancel-in-progress: false` serializes deploys so one is never interrupted mid-swap. It runs on
a **self-hosted runner that lives on novahome** (label `novahome`, user `damien`) — no SSH keys
or stored secrets. Steps:

1. `actions/checkout@v7` (target/ stays warm across runs).
2. `cargo leptos build --release` — build failure stops here; no backup, no deploy.
3. Back up the prod DB: `surreal export … --ns authlyn --db prod` to `/data/prod_backups`,
   abort if the export is empty (`test -s`), gzip it, then prune to the newest 15
   (`ls -1t … | tail -n +16 | xargs -r rm -f`). This destructive prune runs before every swap.
4. `sudo -n /opt/authlyn-prod/deploy.sh "$GITHUB_WORKSPACE"`.

Because the trigger *is* merge-to-`main`, **gating v27 = gating the merge** (work happens on
`mendicant-bias`; merge to `main` only after explicit owner approval).

### 4.2 The privileged bridge does no building

`/opt/authlyn-prod/deploy.sh` is root-owned and NOPASSWD-scoped to exactly that one script
(`/etc/sudoers.d/authlyn-deploy`). It **only** rotates one `.bak` generation, swaps the binary
+ `site/`, restarts the service, health-checks `127.0.0.1:8083`, and **auto-rolls-back** on
failure. The build (runner or manual) is what invokes it. Untouched by deploy:
`/opt/authlyn-prod/.env`, the `authlyn/prod` DB, `/data/authlyn-prod/media`. Manual rollback:
`sudo /opt/authlyn-prod/deploy.sh --rollback`.

### 4.3 Manual hotfix path

No runner needed: push the commit to the novahome bare repo, `git checkout -f <sha>` in
`~/authlyn-prod`, `cargo leptos build --release` on novahome, then invoke the same bridge.
Full runbook + the one-time runner registration: [`/deploy`](../../.claude/commands/deploy.md).

### 4.4 Test deck

[`/test-deploy`](../../.claude/commands/test-deploy.md) (and the `test-deploy` skill) ship the
**current committed HEAD of a feature branch** — never `main`, never prod. The deck builds from
its **own** clone (`~/authlyn-testdeck`), so the tree must be committed and pushed:

- Push to **both** remotes — `origin` (GitHub, off-machine backup) **and** `novahome` (the
  *local bare repo* `/home/damien/authlyn-testdeck.git` that the deck actually builds from;
  novahome cannot auth to the private GitHub repo). The `novahome` git remote is repointed to
  the `ssh novahome` SSH-config alias — the raw-IP `ssh://…/…` URL fails publickey because the
  key binding is scoped to `Host novahome`.
- On novahome: `git fetch origin <branch> && git reset --hard FETCH_HEAD` (**not**
  `git checkout <sha>`, which can "pathspec did not match" right after a fetch and silently
  leave the OLD commit), then `cargo leptos build --release`. **Verify the printed short SHA
  equals the expected one before deploying** — that check is what catches a stale bare repo.
- `sudo /opt/authlyn-test/deploy-test.sh` swaps + restarts + self-health-checks `:8082`. It
  builds nothing.

The deck is the iOS/WebKit/touch review + probe target ([UI fidelity](08-styling-chrome.md));
Chromium-green is necessary, never sufficient.

> **Port caveat:** the `nova-mcp` bridge's default `NOVA_BIND` is `127.0.0.1:8082` — the **same
> port** as the test-deck app on the same novahome host. They never run together by default, but
> override `NOVA_BIND` if you ever co-host nova alongside the deck. See [10-nova-mcp](10-nova-mcp.md).

---

## 5. The deck's dev hot-reload nudge (`POST /admin/dev/reload`)

The test deck runs the **compiled** binary, so it has no cargo-leptos live-reload. To refresh
every connected client onto a freshly deployed build, an admin POSTs `/admin/dev/reload`
([`src/server/dev_reload.rs`](../../src/server/dev_reload.rs), route registered at
`src/server/mod.rs:256`). This broadcasts a **payload-free** `SyncEvent::Reload` over the
existing SSE bus ([04-realtime-sse](04-realtime-sse.md)); the client's
`src/ui/shell/act/sync.rs` listens for it and calls `location.reload()`.

Load-bearing properties (all pinned):

- `Reload` **bypasses** the per-connection channel-visibility filter — it reaches a connection
  whose visible-channel set is empty. Pinned by
  `tests/dev_reload.rs::reload_reaches_a_connection_with_no_visible_channels_as_a_named_frame`.
- It is delivered as a **distinct named** `event: reload` frame (not a generic `data:`-only
  `message` frame), so the client binds it to its own listener and never confuses it with a
  notify. Same test (it asserts `event == "reload"`); the wire name is
  `src/server/events.rs:124` (`RELOAD_EVENT_NAME`).
- It stays **id-only** — the frame carries an empty-object sentinel, never content. Pinned by
  `tests/dev_reload.rs::reload_frame_is_payload_free`.
- The endpoint is **fail-closed admin-only**: non-admin → 403, unauth → 401. Pinned by
  `tests/dev_reload.rs::dev_reload_is_403_for_non_admin` and
  `tests/dev_reload.rs::dev_reload_requires_auth`. (The admin gate can't be exercised positively
  through HTTP — the `is_admin` env read races parallel test workers — so the broadcast *logic*
  is driven directly via the `broadcast_reload` core fn; the gate is checked through the router.)

This is the deck-side counterpart to §6's PWA update flow: §5 is server-pushed force-reload for
the shared review surface; §6 is the user-gated update for an *installed* PWA.

---

## 6. PWA service-worker update lifecycle

The PWA is a Leptos **hydrate** app: the `/pkg/` JS+WASM bundle and the SSR'd navigations must
**never** be served stale, or hydration mismatches and broken app code result. The whole
service worker exists to guarantee that while still working offline.

Files: [`public/sw.js`](../../public/sw.js) (the worker),
[`public/register-sw.js`](../../public/register-sw.js) (registration + update UI),
[`public/manifest.webmanifest`](../../public/manifest.webmanifest),
[`public/offline.html`](../../public/offline.html). `<script src="/register-sw.js">` and the
PWA `<link>`/meta tags are emitted by `shell()` ([01-overview](01-overview.md)).

### 6.1 How `/sw.js` is served (the cache-bust seam)

`/sw.js` is served **dynamically** by `serve_service_worker` (`src/server/mod.rs:285`), which
`include_str!`s `public/sw.js` and replaces the literal `__BUILD_REV__` with `env!(BUILD_REV)`
(the §3 stamp). Response headers: `Content-Type: text/javascript; charset=utf-8`,
`Cache-Control: no-cache` (so the browser's periodic SW update check always reads fresh bytes),
`Service-Worker-Allowed: /`.

`/sw.js` is a **sibling** route in `api_routes()` (`src/server/mod.rs:345`), merged *alongside*
`small_body_routes()` and `media_routes()` — it is **not** in the small-body (JSON) group, so
the `no-store` layer never touches it; it carries its own `no-cache` per-response.

### 6.2 `CACHE_VERSION`

```js
const ASSET_REV = "v2";
const CACHE_VERSION = "authlyn-__BUILD_REV__-" + ASSET_REV;   // → "authlyn-<gitrev>-v2" at serve time
```

Two independent bust dimensions (`public/sw.js:21-33`):

- **`__BUILD_REV__`** — automatic, per release, substituted server-side. A new commit → new
  rev → new `CACHE_VERSION` → the browser sees a new SW.
- **`ASSET_REV`** — a **manual**, monotonic suffix. Bump it whenever the precache list or a
  cache-first asset set (e.g. the self-hosted `/fonts/` faces) changes, so the cache name
  rotates even where the build-rev placeholder stays the literal `"dev"`.

`activate` deletes every cache key that is neither the current `CACHE_VERSION` nor its paired
`MEDIA_CACHE`, then `clients.claim()`s.

### 6.3 The no-`skipWaiting` → user-gated → single-reload flow

This is the central correctness rule, a small state machine split across two files:

1. **`sw.js` install** deliberately does **not** call `self.skipWaiting()`. A new worker
   installs and then **waits**, so it can never swap the bundle out from under a live session
   (an in-progress message draft survives). The precache fill uses `cache: "reload"` to bypass
   the HTTP cache so a stale old-release copy can't get baked into the new `CACHE_VERSION`.
2. **`register-sw.js`** registers `/sw.js` on `load`, then watches for an installed-and-waiting
   worker (both the already-waiting case and the live `updatefound → statechange === "installed"`
   case, gated on `navigator.serviceWorker.controller` so the very first install shows no
   banner). When one is waiting it shows the `#sw-update-banner` ("A new version is available."
   + Refresh + dismiss).
3. **Refresh** posts `{ type: "SKIP_WAITING" }` to the waiting worker. `sw.js`'s `message`
   handler is the **only** path that calls `self.skipWaiting()`.
4. The newly-activated worker `clients.claim()`s, which fires `controllerchange`;
   `register-sw.js` reloads the page **exactly once**, guarded by a `refreshing` flag so a burst
   of updates can't loop.
5. `register-sw.js` also calls `reg.update()` on `load` and on every `visibilitychange → visible`
   (a resumed PWA, especially Android from the app switcher, may never do a cold navigation and
   would otherwise miss a release until the browser's ~24h check).
6. `window.authlynCheckForUpdate` (the account modal's "Check for updates" button,
   [07-ui-shell](07-ui-shell.md)) mirrors the Refresh flow: force `reg.update()`, and if that
   turns up a waiting worker, post `SKIP_WAITING`; the same `controllerchange` listener reloads
   once. Returns a human-readable status string.

> **Test coverage gap (be explicit):** none of the SW lifecycle, `CACHE_VERSION` /
> `BUILD_REV` substitution, the fetch strategy matrix, the manifest, or `offline.html` is pinned
> by any `.rs` test. `tests/cache_control.rs` pins the *JSON-group* `no-store` header and the
> *media* immutable-private header, and **explicitly does not cover `/sw.js`** (see its module
> doc). Fidelity here is owner-deck-driven. A future regression guard could assert `/sw.js`
> returns `no-cache` + `Service-Worker-Allowed: /` and that `__BUILD_REV__` was substituted.

### 6.4 Fetch strategy matrix (`sw.js` `fetch` handler)

Same-origin **GET** only — every other method/origin passes straight to the network. Arms are
**order-dependent**; misclassifying one silently breaks caching or persists a session-gated
blob (`public/sw.js:164-247`):

| Match (in order) | Strategy | Why |
|---|---|---|
| `Accept: text/event-stream` (the `/events` SSE) | **passthrough**, never intercepted | a `respondWith()` on an infinite stream ties its survival to SW lifetime (iOS kills idle SWs); SSE is uncacheable and already `no-store` |
| `/pkg/*` (JS/WASM/CSS) | network-first, **revalidate** (`cache: "no-cache"`), cached copy only as offline fallback | the bundle is **stable-named**; without forced revalidation a heuristically-fresh HTTP-cache copy could pair an OLD bundle with a NEW SSR shell → hydration mismatch |
| `/media/*?w=N` (thumbnails) | network-first into a **bounded, logout-clearable** side cache (`MEDIA_CACHE`, 200 entries, oldest-first evict) | thumbnails are session-gated and **not** immutable per URL (bytes change across pipeline versions → revalidating max-age + pipeline-version ETag server-side) |
| `/media/*` without `w` (full originals) | **passthrough** — never persisted in Cache Storage | multi-MB, session-gated; the browser HTTP cache handles them under the server's `immutable` Cache-Control |
| `/fonts/*.woff2` | cache-first (fill revalidates) | avoid a full four-face FOUT on every cold open of the installed PWA |
| precache shell (`manifest`, icons, `offline.html`) | cache-first (refreshed on activate; fill revalidates) | versioned static shell |
| navigations (`mode === "navigate"`, the SSR shell) | network-first → `offline.html` fallback; **not cached** | the shell is session-specific; a cached copy paints a stale view on cold open |
| everything else (dynamic JSON: `/channels`, `/guilds`, `/personas`, `/friends`, `/auth`, `/push`, …) | **network-only**, `cache: "no-store"`, HTTP cache bypassed | the stale-message-flash fix |

The "revalidate" branch keys on `request.mode !== "navigate"`: a `navigate` request can't be
re-constructed with an init dict on older WebKit, and SSR responses carry no validators anyway,
so navigations are always fetched fresh.

### 6.5 SW ↔ page message protocol

`sw.js`'s `message` handler also accepts (`public/sw.js:317-344`):

| Message | Sent when | Effect |
|---|---|---|
| `SKIP_WAITING` | user taps Refresh / "Check for updates" | the waiting worker activates (§6.3) |
| `CLEAR_MEDIA_CACHE` | logout (`src/ui/shell/act/account.rs`) | drops the whole `MEDIA_CACHE` so session-gated thumbnails don't outlive the session |
| `CLEAR_NOTIFS_TAG` (`{ tag }`) | a channel becomes the focused channel | closes already-seen OS notifications for that tag |

`push` always calls `showNotification` (no focused-window suppression) — iOS revokes the
subscription if a push resolves without one (`userVisibleOnly`). `notificationclick` deep-links
into the channel via query params, preferring an existing window. (Push/notify server side:
see the realtime + push subsystems.)

---

## Source map

**Build / gate / stamp**
- `Cargo.toml` — `[package.metadata.leptos]` (bin-target, profiles, browserquery, hash-files
  back-out rationale), `[[bin]] nova-mcp` (`required-features`), `[profile.wasm-release]`.
- `build.rs` — stamps `BUILD_REV` (git short rev) + `APP_CODENAME`; `rerun-if-changed` on
  `.git/HEAD`, `.git/logs/HEAD`, `Cargo.toml`.
- `.claude/commands/check.md` — the `/check` fmt+clippy(ssr)+clippy(hydrate-wasm) trio.
- `.githooks/pre-commit` — opt-in: the same trio + an inline `@keyframes` motion-doctrine shell
  scan; **references but does not run** `tests/style_lint.rs`.
- `.claude/settings.json` — Bash allowlist + `PostToolUse` rustfmt hook + `SessionStart` probe.

**Deploy**
- `.github/workflows/deploy.yml` — prod CD: push-to-`main` → self-hosted novahome runner →
  `cargo leptos build --release` → prod-DB backup → `sudo deploy.sh`.
- `.claude/commands/deploy.md` — prod runbook (`authlyn-prod` :8083, GHA + manual hotfix,
  runner registration, root-owned `deploy.sh` boundary).
- `.claude/commands/test-deploy.md` — test-deck runbook (`authlyn-test` :8082, dual-remote push,
  `reset --hard FETCH_HEAD` + SHA verify).
- `CLAUDE.md` — the novahome deploy + owner-gate rules (the workflow file stays
  canonical for host/cert specifics).

**PWA + dev-reload**
- `public/sw.js` — `CACHE_VERSION`/`ASSET_REV`, no-`skipWaiting` install, fetch strategy matrix,
  message + push + notificationclick handlers.
- `public/register-sw.js` — registration, the waiting-worker banner, `SKIP_WAITING`,
  single-reload `controllerchange` guard, `authlynCheckForUpdate`.
- `public/manifest.webmanifest`, `public/offline.html` — PWA shell + offline fallback.
- `src/server/mod.rs:285` — `serve_service_worker` (`__BUILD_REV__` substitution, `no-cache` +
  `Service-Worker-Allowed: /`); `:345` — `/sw.js` is a sibling of the JSON group (not no-store).
- `src/server/dev_reload.rs`, `src/server/events.rs:124` (`RELOAD_EVENT_NAME`),
  `src/ui/shell/act/sync.rs` — the deck dev-reload nudge.

**Tests that pin claims here**
- `tests/cache_control.rs::json_api_responses_are_no_store`,
  `::no_store_applies_even_to_error_responses`, `::media_route_group_is_not_no_store` — the
  JSON-group `no-store` and media immutable-private headers. **Does not cover `/sw.js`.**
- `tests/dev_reload.rs::reload_reaches_a_connection_with_no_visible_channels_as_a_named_frame`,
  `::reload_frame_is_payload_free`, `::dev_reload_is_403_for_non_admin`,
  `::dev_reload_requires_auth` — the dev hot-reload nudge.
- **Unpinned (no `.rs` test):** the entire SW update lifecycle, `CACHE_VERSION`/`BUILD_REV`
  substitution, the `/sw.js` headers, the fetch strategy matrix, the manifest, `offline.html`,
  and `APP_CODENAME`. Fidelity is owner-deck-driven.
