# visual-gate — Omloppsbana swipe-void regression net

A standing **headed-Playwright visual gate** that fails if the orbit
(`sk-orbit`) swipe-strip ever renders an **empty neighbor "peek" pane** again
— the W5/P2 "swipe-void" bug, where swiping toward a list edge (or in a
single-channel guild) exposed a black void instead of a designed boundary.

This is **dev-only tooling**. It is **NOT** part of the Rust build, has no CI
job, and must only ever run against a **locally-running dev server + the dev
DB**. It never touches prod or the novahome test deck (see the root
`CLAUDE.md` prod-guardrail). `node_modules/` and `shots/` are git-ignored.

## Run it

```sh
# 1) Start the dev stack in other terminals (see root CLAUDE.md):
#    surreal start --user root --pass root --bind 127.0.0.1:8000 memory
#    cargo leptos watch              # serves http://127.0.0.1:3000

# 2) Then, from this dir:
cd visual-gate
npm install
npx playwright install chromium webkit   # one-time: fetch the engines
npm run gate
```

Exit code is non-zero if any device fails an assertion. Per-device screenshots
land in `shots/` (e.g. `android-pixel7-single-channel.png`).

Override the target (still must be local) with `AUTHLYN_GATE_URL`
(default `http://localhost:3000`). A prod/deck-looking URL is hard-refused.

## What it does

1. **Seed** (`seed.mjs`, REST only): registers a fresh random user in the
   disposable dev DB, then creates two guilds —
   - **single-channel** (a new guild ships with one default `general`
     channel), and
   - **multi-channel** (`general` + two more text channels = 3).
2. **Device matrix** (`run-gate.mjs`), mobile-first per owner order:

   | id               | engine   | emulation                              |
   | ---------------- | -------- | -------------------------------------- |
   | `android-pixel7` | chromium | `devices['Pixel 7']`                   |
   | `ios-iphone14`   | webkit   | `devices['iPhone 14']`                 |
   | `pwa-standalone` | webkit   | iPhone 14 + `display-mode: standalone` |
   | `desktop-safari` | webkit   | 1280×800                               |
   | `desktop-chrome` | chromium | 1280×800                               |

3. **Per device**: inject the `authlyn_session` cookie (captured from
   register's `Set-Cookie`; `secure:false` for the WebKit/localhost trap),
   force orbit via `localStorage['authlyn.skeleton'] = '"orbit"'` (the JSON
   quotes are load-bearing — gloo-storage JSON-encodes) then reload, open a
   channel through the orbit map (pill → far-server node, which auto-opens that
   guild's first channel; or a channel node), and **assert**:
   - `.sk-orbit-strip` exists with exactly three `.sk-orbit-pane`;
   - both neighbor panes contain a `.sk-orbit-peek` with **visible content**
     (never empty);
   - the **single-channel** guild shows `.sk-orbit-peek-edge` ("orbit's edge")
     on **both** sides;
   - the **multi-channel** guild's next peek is a named neighbor
     (`.sk-orbit-peek-name`).

## Honored gotchas

- **WebKit + remote IP** throws "The Internet connection appears to be
  offline" — so the gate only ever targets `localhost`.
- **WebKit drops the Secure cookie over `http://localhost`** — the session
  cookie is injected with `secure:false`. The server legitimately sets
  `Secure` (correct prod behavior); this is the documented client-side test
  workaround.
- **gloo-storage JSON-encodes** — the stored skeleton value is `"orbit"` with
  quotes, not `orbit`.

## Real iPhone testing is a SEPARATE manual gate

Emulated WebKit (`devices['iPhone 14']`) is close but **not identical** to
Safari on a physical iOS device — gesture inertia, the real safe-area insets, a
true installed PWA, and WebKit's actual rendering can differ. Per the
mobile-first / UX-equality principles, real-device iOS verification remains a
**manual** step (open the installed PWA on an iPhone, swipe a single-channel
guild to each edge, confirm the "orbit's edge" boundary instead of a void).
This automated gate is the fast everyday net, not a replacement for that.
