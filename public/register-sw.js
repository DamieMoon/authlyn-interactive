// Registers the service worker and drives a coordinated, user-gated update flow:
// when an updated SW finishes installing and is WAITING (sw.js no longer calls
// skipWaiting() on install), we show a dismissible "new version available"
// banner. Tapping Refresh posts {type:"SKIP_WAITING"} to the waiting worker;
// it activates + clients.claim()s, which fires `controllerchange`, and we then
// reload the page EXACTLY ONCE. No auto-reload before the tap, so an in-progress
// message draft is never lost. The banner is styled by `.sw-update-banner` in
// style/_modal.scss.
(function () {
  if (!("serviceWorker" in navigator)) return;

  // Guard so the controllerchange handler reloads only once (avoids a loop if
  // several updates land in quick succession).
  var refreshing = false;
  navigator.serviceWorker.addEventListener("controllerchange", function () {
    if (refreshing) return;
    refreshing = true;
    location.reload();
  });

  function showBanner(worker) {
    // Don't double-insert if a banner is already up.
    if (document.getElementById("sw-update-banner")) return;

    var banner = document.createElement("div");
    banner.id = "sw-update-banner";
    banner.className = "sw-update-banner";

    var msg = document.createElement("span");
    msg.textContent = "A new version is available.";

    var refresh = document.createElement("button");
    refresh.type = "button";
    refresh.className = "sw-update-refresh";
    refresh.textContent = "Refresh";
    refresh.addEventListener("click", function () {
      // Ask the waiting worker to take over; the reload happens on the
      // resulting controllerchange. Fall back to a plain reload if for some
      // reason there's no waiting worker to message.
      if (worker) {
        worker.postMessage({ type: "SKIP_WAITING" });
      } else {
        location.reload();
      }
    });

    var dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.className = "sw-update-dismiss";
    dismiss.setAttribute("aria-label", "Dismiss");
    dismiss.textContent = "×"; // ×
    dismiss.addEventListener("click", function () {
      banner.remove();
    });

    banner.appendChild(msg);
    banner.appendChild(refresh);
    banner.appendChild(dismiss);
    document.body.appendChild(banner);
  }

  window.addEventListener("load", function () {
    navigator.serviceWorker
      .register("/sw.js")
      .then(function (reg) {
        // Case 1: a worker is already waiting (updated SW installed during a
        // previous visit / before this listener was attached).
        if (reg.waiting && navigator.serviceWorker.controller) {
          showBanner(reg.waiting);
        }

        // Case 2: an update is found now — watch the new worker until it's
        // installed-and-waiting (controller present ⇒ it's an update, not the
        // very first install).
        reg.addEventListener("updatefound", function () {
          var installing = reg.installing;
          if (!installing) return;
          installing.addEventListener("statechange", function () {
            if (
              installing.state === "installed" &&
              navigator.serviceWorker.controller
            ) {
              showBanner(reg.waiting || installing);
            }
          });
        });

        // Proactively check for a new SW now and whenever the app is brought
        // back to the foreground. A resumed PWA (esp. Android, from the app
        // switcher) may never do a cold navigation, so without this the browser
        // might not notice a new release until its periodic 24h check.
        reg.update().catch(function () {});
        document.addEventListener("visibilitychange", function () {
          if (document.visibilityState === "visible") {
            reg.update().catch(function () {});
          }
        });
      })
      .catch(function (e) {
        console.error("SW registration failed:", e);
      });
  });
})();

// Manual update check, invoked by the account modal's "Check for updates"
// button. Mirrors the banner's Refresh flow: force a registration update, and
// if that turns up a WAITING worker, tell it to skip waiting. The
// controllerchange listener above then reloads the page exactly once. Returns
// a human-readable status string for the caller to surface.
window.authlynCheckForUpdate = async function () {
  if (!("serviceWorker" in navigator)) return "Updates not supported here.";
  var reg = await navigator.serviceWorker.getRegistration();
  if (!reg) return "App not installed as a PWA yet.";
  try {
    await reg.update();
  } catch (e) {
    return "Update check failed.";
  }
  if (reg.waiting) {
    reg.waiting.postMessage({ type: "SKIP_WAITING" }); // controllerchange listener reloads
    return "Updating to the latest version…";
  }
  return "You're on the latest version.";
};
