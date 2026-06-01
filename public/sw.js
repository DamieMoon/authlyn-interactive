// authlyn service worker.
//
// authlyn is a Leptos hydrate (WASM) app: the JS/WASM bundle and the SSR'd
// navigations MUST never be served stale, or hydration mismatches and broken
// app code result. Strategy:
//   - network-first for navigations and for the /pkg/ bundle (JS/WASM/CSS),
//     falling back to a cached offline page / cached bundle only when the
//     network is unavailable.
//   - the static PWA shell assets (manifest, icons, offline page) are
//     precached so install/offline works.
// The cache name is versioned; bump CACHE_VERSION to invalidate. Old caches
// are deleted on activate.

// CACHE_VERSION is stamped per build: the `GET /sw.js` handler (server/mod.rs)
// replaces `__BUILD_REV__` with the compile-time git short rev (`BUILD_REV` from
// build.rs), so every release is automatically a new SW. The browser then sees an
// update and `register-sw.js` shows the "new version available" refresh banner.
// (A non-served local build keeps the literal placeholder — harmless, just not unique.)
const CACHE_VERSION = "authlyn-__BUILD_REV__";
const PRECACHE = [
  "/manifest.webmanifest",
  "/icons/icon-192.png",
  "/icons/icon-512.png",
  "/icons/icon-maskable-512.png",
  "/offline.html",
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE_VERSION).then((cache) => cache.addAll(PRECACHE))
  );
  // NB: deliberately NO self.skipWaiting() here. A new SW installs and then
  // WAITS, so it never swaps the bundle out from under a live session. The
  // client (register-sw.js) shows a "new version" banner and only posts
  // {type:"SKIP_WAITING"} (handled below) when the user taps Refresh — so the
  // activation + the single reload are user-gated and coordinated.
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys
            .filter((key) => key !== CACHE_VERSION)
            .map((key) => caches.delete(key))
        )
      )
      .then(() => self.clients.claim())
  );
});

// Network-first. Caches successful GETs only when `cache` is set (static,
// versioned assets) so the offline fallback stays current. Dynamic API data is
// NEVER cached here — use networkOnly for that.
async function networkFirst(request, { cache = false, offlineFallback } = {}) {
  try {
    const response = await fetch(request);
    if (cache && request.method === "GET" && response && response.ok) {
      const copy = response.clone();
      caches.open(CACHE_VERSION).then((c) => c.put(request, copy));
    }
    return response;
  } catch (err) {
    const cached = await caches.match(request);
    if (cached) return cached;
    if (offlineFallback) {
      const fallback = await caches.match(offlineFallback);
      if (fallback) return fallback;
    }
    throw err;
  }
}

// Network-only, bypassing the HTTP cache. For dynamic API responses (messages,
// guilds, personas, …) that must never be served stale — caching these flashed
// ancient messages on cold open before the live fetch landed.
async function networkOnly(request) {
  return fetch(request, { cache: "no-store" });
}

self.addEventListener("fetch", (event) => {
  const { request } = event;

  // Only handle same-origin GETs; let everything else (POSTs to the API,
  // cross-origin requests) pass straight through to the network.
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  // App bundle (JS/WASM/CSS under /pkg/): network-first so app code never
  // goes stale; cached copy only as an offline fallback.
  if (url.pathname.startsWith("/pkg/")) {
    event.respondWith(networkFirst(request, { cache: true }));
    return;
  }

  // Media blobs are immutable (keyed by id + width) — safe to cache for
  // performance and offline use.
  if (url.pathname.startsWith("/media/")) {
    event.respondWith(networkFirst(request, { cache: true }));
    return;
  }

  // Precached static shell assets: cache-first is safe (versioned, refreshed
  // on activate via the precache list).
  if (PRECACHE.includes(url.pathname)) {
    event.respondWith(
      caches.match(request).then((cached) => cached || networkFirst(request, { cache: true }))
    );
    return;
  }

  // Navigations (SSR shell): network-first for fresh app code, offline page
  // when offline. NOT cached — the shell is session-specific and a cached copy
  // can paint a stale view on cold open.
  if (request.mode === "navigate") {
    event.respondWith(networkFirst(request, { offlineFallback: "/offline.html" }));
    return;
  }

  // Everything else — dynamic JSON API (/channels, /guilds, /personas,
  // /friends, /auth, /push, …): network-only, never cached, HTTP cache
  // bypassed. This is the stale-message-flash fix.
  event.respondWith(networkOnly(request));
});

// ---------------------------------------------------------------------------
// Web Push
// ---------------------------------------------------------------------------

// Parse the push payload defensively; fall back to a generic notification so
// the handler never silently drops a push event.
function parsePushPayload(event) {
  if (!event.data) {
    return { title: "authlyn", body: "New notification" };
  }
  try {
    return event.data.json();
  } catch {
    return { title: "authlyn", body: event.data.text() || "New notification" };
  }
}

self.addEventListener("push", (event) => {
  event.waitUntil(
    (async () => {
      const {
        title = "authlyn",
        body = "",
        icon = "/icons/icon-192.png",
        badge = "/icons/icon-192.png",
        tag,
        channel,
        guild,
        message,
        // The message author's persona avatar media id (server omits it when the
        // persona has no avatar). Mapped to the large notification `image`.
        image,
        // accept a pre-built data blob or synthesise one from top-level fields
        data,
      } = parsePushPayload(event);

      const notifData = data ?? (channel ? { channel, guild, message } : {});

      // ALWAYS show a notification on a push. iOS revokes the subscription if a
      // push event resolves without a showNotification() call (the
      // userVisibleOnly contract), so we deliberately do NOT suppress for a
      // focused window — that would silently kill push on iOS PWAs. (A
      // focused-client suppression could be gated on non-iOS later.)
      await self.registration.showNotification(title, {
        // App icon is the small monochrome-ish badge AND the per-notification
        // icon; the persona avatar (when present) fills the large `image` slot
        // that was previously rendered as an empty white square. Omit `image`
        // entirely when there's no avatar so no placeholder shows.
        body,
        icon,
        badge,
        ...(image != null && { image: "/media/" + image }),
        // tag deduplicates: a second push with the same tag replaces the first.
        // Only set it when the payload provides one so unrelated pushes stack.
        ...(tag != null && { tag, renotify: true }),
        data: notifData,
      });
    })()
  );
});

// ---------------------------------------------------------------------------
// Page → SW messages: the client posts {type: "CLEAR_NOTIFS_TAG", tag} when a
// channel becomes the open / focused channel, so we close any notifications
// the user has now visibly seen (feedback row kx24k2cwftdppidhmh0e). Without
// this, mobile notifications stack indefinitely in the OS tray even after the
// user reads the channel that produced them.
// ---------------------------------------------------------------------------
self.addEventListener("message", (event) => {
  const msg = event.data;
  if (!msg || typeof msg !== "object") return;
  // The page asks a WAITING worker to activate (user tapped Refresh). This is
  // the only path that calls skipWaiting() — see the install handler's note.
  // Once it activates, clients.claim() fires controllerchange, and the page
  // reloads exactly once (register-sw.js).
  if (msg.type === "SKIP_WAITING") {
    self.skipWaiting();
    return;
  }
  if (msg.type === "CLEAR_NOTIFS_TAG" && typeof msg.tag === "string") {
    event.waitUntil(
      self.registration
        .getNotifications({ tag: msg.tag })
        .then((notifs) => notifs.forEach((n) => n.close()))
        .catch(() => {})
    );
  }
});

// ---------------------------------------------------------------------------
// Notification click — focus existing window or open a new one; honour the
// deep-link channel carried in notification.data.
// ---------------------------------------------------------------------------
self.addEventListener("notificationclick", (event) => {
  event.notification.close();

  const { channel, guild, message } = event.notification.data ?? {};
  let target = "/";
  if (channel) {
    const params = new URLSearchParams({ channel });
    if (guild) params.set("server", guild);
    if (message) params.set("m", message);
    target = `/?${params.toString()}`;
  }

  event.waitUntil(
    (async () => {
      const windowClients = await self.clients.matchAll({
        type: "window",
        includeUncontrolled: true,
      });

      // Prefer an already-focused window, then any visible window, then any
      // window at all.
      const focused = windowClients.find((c) => c.focused);
      const visible = windowClients.find((c) => c.visibilityState === "visible");
      const existing = focused ?? visible ?? windowClients[0] ?? null;

      if (existing) {
        // Try to navigate to the deep-link URL; fall back to postMessage so the
        // app can handle routing itself (navigate() throws if the client is
        // cross-origin or the SW scope doesn't cover the URL).
        try {
          await existing.navigate(target);
        } catch {
          existing.postMessage({
            type: "NOTIFICATION_CLICK",
            channel: channel ?? null,
            server: guild ?? null,
            message: message ?? null,
            url: target,
          });
        }
        await existing.focus();
        return;
      }

      // No existing window — open a fresh one at the deep-link URL.
      await self.clients.openWindow(target);
    })()
  );
});
