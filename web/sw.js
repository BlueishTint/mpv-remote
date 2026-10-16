"use strict";
// Exists so browsers treat the page as an installable app. The UI is useless without mpv, so there is
// no offline cache: requests go straight to the network, with a short message if the PC is unreachable.

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

self.addEventListener("fetch", (e) => {
  if (e.request.method !== "GET") return;
  e.respondWith(
    fetch(e.request).catch(() =>
      new Response("Can't reach your PC. Is mpv running and are you on the same Wi-Fi?", {
        status: 503,
        headers: { "Content-Type": "text/plain; charset=utf-8" },
      })),
  );
});
