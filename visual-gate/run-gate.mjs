// Omloppsbana visual gate — standing headed-Playwright check that the orbit
// swipe-strip's neighbor "peek" panes are NEVER visually empty.
//
// This is the regression net for the W5/P2 "swipe-void" bug: when you swiped
// to a channel that had no neighbor on one side (a MULTI-channel list edge),
// the prev/next pane rendered an empty black void instead of a designed
// boundary. The fix (`neighbor_preview` in src/ui/shell/sk_orbit/mod.rs)
// renders an "orbit's edge" affordance at list edges and a "# name" preview
// for real neighbors.
//
// W5/P2 #d FLIPPED the single-channel contract: a 1-channel guild has nowhere
// to swipe, so it now mounts NO prev/next peek panes at all (only the current
// pane). So this gate asserts, on the full mobile-first device matrix:
//   - single-channel guild → exactly ONE pane, NO peeks, NO "orbit's edge";
//   - multi-channel guild   → prev/cur/next with a named neighbor + the
//     "orbit's edge" boundary at the true list edge (unchanged).
//
// DEV-ONLY tooling. NOT part of the Rust build. Runs against a LOCALLY-running
// dev server (http://localhost:3000) + the dev DB. NEVER point it at prod or
// the novahome test deck (root CLAUDE.md prod-guardrail). The seed registers a
// throwaway user in the disposable dev DB each run.
//
// Usage:
//   cd visual-gate && npm install && npx playwright install chromium webkit
//   npm run gate            # against an already-running `cargo leptos watch`
//
// Honored gotchas:
//   - Playwright's WebKit build errors "The Internet connection appears to be
//     offline" against a remote IP, so we ONLY ever target localhost.
//   - WebKit drops the Secure session cookie over http://localhost, so for
//     webkit contexts we inject `authlyn_session` with secure:false. The
//     server legitimately sets Secure (correct prod behavior); this is the
//     documented client-side test workaround (root CLAUDE.md). Chromium
//     accepts the localhost cookie either way; we inject secure:false there
//     too so a single code path serves both engines.
//   - Real iPhone / iOS-Safari testing stays a SEPARATE manual gate — emulated
//     WebKit is close but not identical (see README).

import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { mkdirSync } from "node:fs";
import { chromium, webkit, devices } from "playwright";

import { seed, SESSION_COOKIE } from "./seed.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const SHOTS_DIR = join(__dirname, "shots");

const BASE = process.env.AUTHLYN_GATE_URL || "http://localhost:3000";

// Hard refusal: the gate must never run against prod / the deck.
if (/\bprod\b/i.test(BASE) || /\bdeck\b/i.test(BASE)) {
  console.error(
    `Refusing to run the visual gate against ${BASE}. ` +
      `It is dev-only — target localhost:3000 + the dev DB.`,
  );
  process.exit(2);
}

// ---------------------------------------------------------------------------
// Device matrix — mobile-first, per owner ordering (Android, iOS, PWA, then
// the desktop browsers). Each entry names its Playwright browser-type launcher
// and the context options to emulate the device.
// ---------------------------------------------------------------------------
const MATRIX = [
  {
    id: "android-pixel7",
    label: "Android — Chromium / Pixel 7",
    launch: chromium,
    context: { ...devices["Pixel 7"] },
  },
  {
    id: "ios-iphone14",
    label: "iOS — WebKit / iPhone 14",
    launch: webkit,
    context: { ...devices["iPhone 14"] },
  },
  {
    id: "pwa-standalone",
    label: "PWA — WebKit / iPhone 14 (display-mode: standalone)",
    launch: webkit,
    // The installed-app surface: same device metrics, but the page sees
    // matchMedia('(display-mode: standalone)') === true. Playwright/WebKit has
    // no direct standalone toggle, so we shim matchMedia + navigator.standalone
    // via an init script (added per-context below) — enough for the orbit
    // layout, which keys safe-area + chrome off standalone.
    context: { ...devices["iPhone 14"] },
    standalone: true,
  },
  {
    id: "desktop-safari",
    label: "Desktop — WebKit / Safari",
    launch: webkit,
    context: { viewport: { width: 1280, height: 800 } },
  },
  {
    id: "desktop-chrome",
    label: "Desktop — Chromium / Chrome",
    launch: chromium,
    context: { viewport: { width: 1280, height: 800 } },
  },
];

// Init script shimming PWA-standalone display-mode for the dedicated entry.
const STANDALONE_INIT = `
  (() => {
    const orig = window.matchMedia.bind(window);
    window.matchMedia = (q) =>
      /display-mode:\\s*standalone/i.test(q)
        ? { matches: true, media: q, onchange: null,
            addListener() {}, removeListener() {},
            addEventListener() {}, removeEventListener() {}, dispatchEvent() { return false; } }
        : orig(q);
    try { Object.defineProperty(window.navigator, 'standalone', { get: () => true, configurable: true }); } catch {}
  })();
`;

// ---------------------------------------------------------------------------
// Assertion plumbing. We collect failures per device and fail the whole gate
// (non-zero exit) if ANY device hit ANY failure — but run every device first
// so one engine's breakage doesn't mask another's.
// ---------------------------------------------------------------------------
class Failures {
  constructor(device) {
    this.device = device;
    this.items = [];
  }
  check(cond, msg) {
    if (!cond) this.items.push(msg);
  }
  get ok() {
    return this.items.length === 0;
  }
}

/** Force the orbit skeleton via localStorage, then reload so the shell boots
 *  into it. gloo-storage JSON-encodes values, so the STORED string must be the
 *  JSON-quoted form — i.e. `"orbit"` WITH the quotes — or `skeleton_pref()`
 *  reads back garbage and the `.app.sk-orbit` class never applies. */
async function forceOrbit(page) {
  await page.addInitScript(() => {
    // JSON.stringify('orbit') === '"orbit"' — the quotes are load-bearing.
    window.localStorage.setItem("authlyn.skeleton", JSON.stringify("orbit"));
  });
  await page.reload({ waitUntil: "domcontentloaded" });
}

/**
 * Drive the orbit map to open a channel in `guild` (matched by name), then
 * assert the swipe-strip's pane shape for that guild.
 *
 * @param page Playwright page (already logged-in, orbit forced).
 * @param guild { name } the seeded guild to open.
 * @param singleChannel true for the 1-channel guild. W5/P2 #d FLIPPED the
 *   single-channel contract: a 1-channel guild has nowhere to swipe, so it now
 *   renders NO prev/next peek panes at all (was: "orbit's edge" on BOTH sides).
 *   For multi (false) the strip keeps prev/cur/next with ≥1 named neighbor and
 *   the "orbit's edge" boundary at the true list edge.
 * @param f Failures collector.
 * @param shotPath where to save this guild's screenshot.
 */
async function assertGuildPeeks(page, guild, singleChannel, f, shotPath) {
  const tag = `[${f.device}] guild "${guild.name}"`;

  // Open the orbit map via the holographic pill (the ONLY entry — pinch was
  // judge-killed). Wait for it to exist first; it renders inside .sk-orbit-content.
  const pill = page.locator(".sk-orbit-pill");
  await pill.waitFor({ state: "visible", timeout: 15000 });
  await pill.click();

  const map = page.locator(".sk-orbit-map");
  await map.waitFor({ state: "visible", timeout: 10000 });

  // The map shows the CURRENT server's channels as `.sk-orbit-node`, and every
  // OTHER server as a `.sk-orbit-far` node. To open `guild`'s first channel:
  //   - if it's a far server → click its `.sk-orbit-far` (auto-opens channel 0);
  //   - if it's already current → click any `.sk-orbit-node` (its channel 0).
  const far = page.locator(".sk-orbit-far", { hasText: guild.name });
  if ((await far.count()) > 0) {
    await far.first().click();
  } else {
    // Already on this guild — open its first channel node. (Matching by name is
    // unnecessary here; any node belongs to the current guild.)
    const node = page.locator(".sk-orbit-node");
    await node.first().waitFor({ state: "visible", timeout: 10000 });
    await node.first().click();
  }

  // The map closes on selection and the channel pane (Pane::Channel) mounts the
  // swipe strip. Wait for it.
  const strip = page.locator(".sk-orbit-strip");
  await strip.waitFor({ state: "visible", timeout: 15000 });

  // Settle past the far-dive's async channel-load (act::open_server →
  // load_server CLEARS s.sel.channels then fetches). After the strip-collapse fix
  // the neighbor panes mount even mid-load (chan_count 0 ⇒ transient "orbit's
  // edge" placeholders), so a bare "pane attached" no longer means "channels
  // loaded". Wait for the GUILD-SPECIFIC settled state: multi → the named
  // neighbor renders; single → the strip collapses to --single (its lone channel
  // resolved). On a genuine regression this times out and the assertions below
  // still fire with the true state, so it cannot mask a real bug.
  const settled = singleChannel
    ? page.locator(".sk-orbit-strip.sk-orbit-strip--single")
    : page.locator(".sk-orbit-strip .sk-orbit-pane-next .sk-orbit-peek-name");
  await settled.waitFor({ state: "attached", timeout: 12000 }).catch(() => {});

  const prevPane = page.locator(".sk-orbit-strip .sk-orbit-pane-prev");
  const nextPane = page.locator(".sk-orbit-strip .sk-orbit-pane-next");
  const paneCount = await page.locator(".sk-orbit-strip .sk-orbit-pane").count();

  // --- W5/P2 #d: single-channel guild renders ONLY the current pane. ---
  // No swipe targets, no peek panes, no "orbit's edge" — there is nowhere to
  // swipe. This is the FLIPPED contract (was: edges on both sides).
  if (singleChannel) {
    f.check(
      paneCount === 1,
      `${tag}: single-channel guild must render exactly 1 .sk-orbit-pane ` +
        `(the current channel, no swipe), found ${paneCount}`,
    );
    f.check(
      (await prevPane.count()) === 0 && (await nextPane.count()) === 0,
      `${tag}: single-channel guild must have NO prev/next peek panes ` +
        `(prev=${await prevPane.count()}, next=${await nextPane.count()})`,
    );
    f.check(
      (await page.locator(".sk-orbit-strip .sk-orbit-peek").count()) === 0,
      `${tag}: single-channel guild must show NO .sk-orbit-peek (nothing to swipe to)`,
    );
    f.check(
      (await page.locator(".sk-orbit-strip .sk-orbit-peek-edge").count()) === 0,
      `${tag}: single-channel guild must NOT show the "orbit's edge" affordance ` +
        `(the #d flip — a 1-channel guild has no edge, it just IS the channel)`,
    );
    // The lone current pane must still carry the real ChannelPane (the channel
    // is reachable, just un-swipeable).
    f.check(
      (await page.locator(".sk-orbit-strip .sk-orbit-pane-cur").count()) === 1,
      `${tag}: single-channel guild is missing its current pane`,
    );
    await page.screenshot({ path: shotPath, fullPage: false });
    return;
  }

  // --- Multi-channel: the 3-pane strip (prev / cur / next). UNCHANGED. ---
  f.check(
    paneCount === 3,
    `${tag}: expected 3 .sk-orbit-pane, found ${paneCount}`,
  );

  // The neighbor panes each hold exactly one `.sk-orbit-peek` (the void-fix
  // wrapper). A missing peek IS the original bug (an empty void pane).
  const prevPeek = prevPane.locator(".sk-orbit-peek");
  const nextPeek = nextPane.locator(".sk-orbit-peek");
  f.check(
    (await prevPeek.count()) === 1,
    `${tag}: prev pane has no .sk-orbit-peek (empty void pane — the bug)`,
  );
  f.check(
    (await nextPeek.count()) === 1,
    `${tag}: next pane has no .sk-orbit-peek (empty void pane — the bug)`,
  );

  // --- The core anti-void assertion: each peek has VISIBLE content. ---
  // A peek is valid iff it shows EITHER the "orbit's edge" affordance
  // (.sk-orbit-peek-edge) OR a named neighbor (.sk-orbit-peek-name). An empty
  // peek (neither) is the void bug.
  for (const [side, peek] of [
    ["prev", prevPeek],
    ["next", nextPeek],
  ]) {
    const edge = peek.locator(".sk-orbit-peek-edge");
    const name = peek.locator(".sk-orbit-peek-name");
    const hasEdge = (await edge.count()) > 0;
    const hasName = (await name.count()) > 0;
    f.check(
      hasEdge || hasName,
      `${tag}: ${side} peek is EMPTY — neither .sk-orbit-peek-edge nor ` +
        `.sk-orbit-peek-name (this is the swipe-void bug)`,
    );

    // The peek's rendered text must be non-blank — guards against a present-but-
    // empty edge ring or a blank name node. ('aria-hidden' on the parent pane
    // is fine; we read textContent, which ignores aria.)
    const text = ((await peek.textContent()) || "").trim();
    f.check(
      text.length > 0,
      `${tag}: ${side} peek renders no text (visually empty)`,
    );

    // The "orbit's edge" affordance, when present, must actually say so.
    if (hasEdge) {
      const edgeText = page.locator(
        `.sk-orbit-pane-${side} .sk-orbit-peek-edge .sk-orbit-peek-edge-text`,
      );
      const et = ((await edgeText.textContent()) || "").trim().toLowerCase();
      f.check(
        et.includes("edge"),
        `${tag}: ${side} edge affordance text is "${et}" (expected "orbit's edge")`,
      );
    }
  }

  // --- Multi-channel per-guild shape. (Single-channel returned early above.) ---
  // The auto-opened first channel has a real NEXT neighbor (channel index 1),
  // so the next peek must be a named neighbor; the prev peek (left edge of the
  // 3-channel list) is the "orbit's edge" boundary — which STILL exists for a
  // multi-channel list edge (the #d flip removed it only for single-channel).
  const nextIsEdge = (await nextPeek.locator(".sk-orbit-peek-edge").count()) > 0;
  const nextHasName =
    (await nextPeek.locator(".sk-orbit-peek-name").count()) > 0;
  f.check(
    nextHasName,
    `${tag}: multi-channel guild's NEXT peek must be a named neighbor ` +
      `(.sk-orbit-peek-name), got edge=${nextIsEdge}`,
  );

  await page.screenshot({ path: shotPath, fullPage: false });
}

async function runDevice(spec, creds) {
  const f = new Failures(spec.id);
  const browser = await spec.launch.launch();
  // Inject the session cookie at the CONTEXT level (httpOnly, so it can't be
  // set from page JS). secure:false because WebKit drops a Secure cookie over
  // http://localhost; Chromium tolerates it, so one path serves both.
  const context = await browser.newContext({
    ...spec.context,
    baseURL: BASE,
  });
  if (spec.standalone) {
    await context.addInitScript(STANDALONE_INIT);
  }
  const url = new URL(BASE);
  await context.addCookies([
    {
      name: SESSION_COOKIE,
      value: creds.token,
      domain: url.hostname, // "localhost"
      path: "/",
      httpOnly: true,
      secure: false, // WebKit/localhost workaround (server is legitimately Secure)
      sameSite: "Lax",
    },
  ]);

  const page = await context.newPage();
  try {
    // Land on the app; the injected cookie authenticates us (no /login).
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await forceOrbit(page);

    // Confirm the orbit skeleton actually engaged before asserting its DOM.
    await page
      .locator(".app.sk-orbit")
      .waitFor({ state: "attached", timeout: 30000 });

    await assertGuildPeeks(
      page,
      creds.single,
      /* singleChannel */ true,
      f,
      join(SHOTS_DIR, `${spec.id}-single-channel.png`),
    );
    await assertGuildPeeks(
      page,
      creds.multi,
      /* singleChannel */ false,
      f,
      join(SHOTS_DIR, `${spec.id}-multi-channel.png`),
    );
  } catch (err) {
    f.items.push(`[${spec.id}] threw: ${err.message}`);
    // Best-effort failure screenshot for triage.
    try {
      await page.screenshot({ path: join(SHOTS_DIR, `${spec.id}-FAILURE.png`) });
    } catch {
      /* ignore */
    }
  } finally {
    await context.close();
    await browser.close();
  }
  return f;
}

async function main() {
  mkdirSync(SHOTS_DIR, { recursive: true });

  console.log(`Visual gate → ${BASE}`);
  console.log("Seeding a throwaway user + two guilds in the dev DB…");
  const creds = await seed(BASE);
  console.log(
    `Seeded user "${creds.username}"; ` +
      `single="${creds.single.name}" multi="${creds.multi.name}".`,
  );

  const results = [];
  // Sequential by design: every context shares the ONE local dev server, and a
  // single seeded user; running them serially keeps the per-device flows from
  // racing each other's orbit-map navigation. The matrix is small (5).
  for (const spec of MATRIX) {
    process.stdout.write(`\n▶ ${spec.label} … `);
    let f = await runDevice(spec, creds);
    // Playwright's emulated WebKit cold-launches slowly + intermittently (I17:
    // emulated WebKit ≠ a real device) — the FIRST webkit device occasionally
    // exceeds the .app.sk-orbit hydrate-attach timeout. That thrown timeout is
    // the gate's OWN infra flake, not a code regression, so retry the device
    // ONCE; a real break fails both attempts (the retry runs a warmed WebKit).
    if (!f.ok && f.items.some((i) => /threw|Timeout/i.test(i))) {
      process.stdout.write("↻ transient (likely WebKit cold-launch), retry … ");
      f = await runDevice(spec, creds);
    }
    results.push(f);
    console.log(f.ok ? "PASS" : `FAIL (${f.items.length})`);
    for (const item of f.items) console.log(`    ✗ ${item}`);
  }

  const failed = results.filter((r) => !r.ok);
  console.log("\n──────────────────────────────────────────────");
  console.log(
    `Visual gate: ${results.length - failed.length}/${results.length} devices passed. ` +
      `Screenshots in ${SHOTS_DIR}/`,
  );
  if (failed.length > 0) {
    console.error(
      `GATE FAILED on: ${failed.map((r) => r.device).join(", ")}`,
    );
    process.exit(1);
  }
  console.log("GATE PASSED — no empty orbit peek panes on any device.");
}

main().catch((e) => {
  console.error("Visual gate crashed:", e);
  process.exit(1);
});
