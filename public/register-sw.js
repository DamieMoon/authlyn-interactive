// Registers the service worker and surfaces a gentle "new version available"
// banner when an updated SW takes control (the `controllerchange` event, which
// fires because sw.js does skipWaiting() + clients.claim()). Deliberately NO
// auto-reload — the user taps Refresh, so an in-progress message draft is never
// lost. The banner is styled by `.sw-update-banner` in style/_modal.scss.
(function () {
  if (!("serviceWorker" in navigator)) return;

  // A controller already present means this page is being taken over by an
  // UPDATED worker (not the very first install) — only then is it an update.
  var hadController = !!navigator.serviceWorker.controller;

  navigator.serviceWorker.addEventListener("controllerchange", function () {
    if (!hadController || document.getElementById("sw-update-banner")) return;

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
      location.reload();
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
  });

  window.addEventListener("load", function () {
    navigator.serviceWorker.register("/sw.js").catch(function (e) {
      console.error("SW registration failed:", e);
    });
  });
})();
