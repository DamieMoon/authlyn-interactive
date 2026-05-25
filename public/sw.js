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

const CACHE_VERSION = "authlyn-v4";
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
  self.skipWaiting();
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

// Network-first with cache fallback. Successful GET responses for precache-able
// assets get refreshed into the cache so the offline fallback stays current.
async function networkFirst(request, { offlineFallback } = {}) {
  try {
    const response = await fetch(request);
    if (request.method === "GET" && response && response.ok) {
      const copy = response.clone();
      caches.open(CACHE_VERSION).then((cache) => cache.put(request, copy));
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

self.addEventListener("fetch", (event) => {
  const { request } = event;

  // Only handle same-origin GETs; let everything else (POSTs to the API,
  // cross-origin requests) pass straight through to the network.
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  // Navigations: network-first, fall back to the offline page when offline.
  if (request.mode === "navigate") {
    event.respondWith(networkFirst(request, { offlineFallback: "/offline.html" }));
    return;
  }

  // App bundle (JS/WASM/CSS under /pkg/): network-first so app code never
  // goes stale; cached copy only as an offline fallback.
  if (url.pathname.startsWith("/pkg/")) {
    event.respondWith(networkFirst(request));
    return;
  }

  // Precached static shell assets: cache-first is safe (versioned, refreshed
  // on activate via the precache list).
  if (PRECACHE.includes(url.pathname)) {
    event.respondWith(
      caches.match(request).then((cached) => cached || networkFirst(request))
    );
    return;
  }

  // Everything else: network-first, cache as offline fallback.
  event.respondWith(networkFirst(request));
});

// Notifications shown via registration.showNotification() (the installed-PWA /
// standalone path — the `new Notification()` constructor is unavailable there).
// Clicking one focuses an existing app window if present, else opens a new one.
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then((clients) => {
        for (const client of clients) {
          if ("focus" in client) return client.focus();
        }
        if (self.clients.openWindow) return self.clients.openWindow("/");
      })
  );
});
