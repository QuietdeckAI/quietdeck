// Quietdeck's service worker. It keeps the app's shell (index.html and the assets it names) so the installed app
// opens without waiting on the network, and it shows pings. API calls always go to the network.
//
// sw.js itself doesn't change between deploys, so the shell isn't refreshed by reinstalling: every page load fetches
// index.html from the deck first, and the cache is brought in line with the assets that copy names.

const SHELL = "quietdeck-shell";

async function refreshShell(response) {
  const cache = await caches.open(SHELL);
  const html = await response.clone().text();
  const assets = new Set([...html.matchAll(/(?:src|href)="(\/assets\/[^"]+)"/g)].map((m) => m[1]));
  await cache.put("/", response);
  for (const request of await cache.keys()) {
    const path = new URL(request.url).pathname;
    if (path.startsWith("/assets/") && !assets.has(path)) await cache.delete(request);
  }
  const have = new Set((await cache.keys()).map((r) => new URL(r.url).pathname));
  await cache.addAll([...assets].filter((a) => !have.has(a)));
}

self.addEventListener("install", (event) => {
  event.waitUntil(
    fetch("/", { cache: "no-cache" })
      .then((r) => (r.ok ? refreshShell(r) : undefined))
      .catch(() => undefined)
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== "GET" || url.origin !== self.location.origin) return;
  if (url.pathname.startsWith("/api/") || url.pathname.startsWith("/hooks/")) return;

  if (request.mode === "navigate") {
    event.respondWith(
      fetch(request)
        .then((response) => {
          if (response.ok && response.headers.get("content-type")?.startsWith("text/html")) {
            event.waitUntil(refreshShell(response.clone()));
          }
          return response;
        })
        .catch(async () => (await caches.match("/")) ?? Response.error()),
    );
    return;
  }

  if (url.pathname.startsWith("/assets/")) {
    // Hashed file names: a cached copy is always right.
    event.respondWith(
      caches.match(request).then(
        (hit) =>
          hit ??
          fetch(request).then(async (response) => {
            if (response.ok) await (await caches.open(SHELL)).put(request, response.clone());
            return response;
          }),
      ),
    );
  }
});

// A ping. The deck sends Declarative Web Push JSON, which Safari can show without this handler; other browsers
// need it. Nothing is fetched here: everything the notification shows is in the push.
self.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = {};
  }
  const n = data.notification ?? {};
  event.waitUntil(
    self.registration.showNotification(n.title ?? "Quietdeck", {
      body: n.body ?? "",
      tag: n.tag,
      // A newer push about the same thread replaces the old one, and still alerts.
      renotify: Boolean(n.tag),
      icon: "/icon-192.png",
      data: { navigate: n.navigate ?? "/" },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL(event.notification.data?.navigate ?? "/", self.location.origin);
  if (target.origin !== self.location.origin) return;
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((windows) => {
      const open = windows[0];
      if (open) return open.navigate(target.href).then((w) => (w ?? open).focus());
      return self.clients.openWindow(target.href);
    }),
  );
});
