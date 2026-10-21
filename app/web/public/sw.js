"use strict";
// Only immutable public shell assets. No API response, transcript, device cookie,
// push subscription or media enters CacheStorage. Revocation is server-enforced.
const CACHE = "ezerbot-shell-iphone-v1";
const ASSETS = ["/", "/styles.css", "/mascot.css", "/chat.css", "/panels.css", "/iphone.css", "/mascot.js", "/chat.js", "/iphone.js", "/app.js", "/panels.js", "/manifest.webmanifest", "/icon-192.png", "/icon-512.png"];
self.addEventListener("install", event => { event.waitUntil(caches.open(CACHE).then(cache => cache.addAll(ASSETS)).then(() => self.skipWaiting())); });
self.addEventListener("activate", event => { event.waitUntil(caches.keys().then(keys => Promise.all(keys.filter(key => key.startsWith("ezerbot-shell-") && key !== CACHE).map(key => caches.delete(key)))).then(() => self.clients.claim())); });
self.addEventListener("fetch", event => {
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin || event.request.method !== "GET" || !ASSETS.includes(url.pathname) || (url.search && url.pathname !== "/")) return;
  event.respondWith(fetch(event.request).then(response => {
    if (response.ok) {
      const copy = response.clone();
      event.waitUntil(caches.open(CACHE).then(cache => cache.put(url.pathname, copy)));
      return response;
    }
    // A disabled listener answers 403 no-store. Reopen the installed shell from
    // cache and never store the denial. An empty cache keeps that 403, including
    // /iphone.js — the server refusal itself stays unchanged.
    return caches.match(url.pathname).then(cached => cached || response);
  }).catch(() => caches.match(url.pathname)));
});
self.addEventListener("push", event => {
  let data; try { data = event.data.json(); } catch { return; }
  if (typeof data.title !== "string" || typeof data.id !== "string") return;
  // The Mac already applies per-kind settings, DND and current-phone presence.
  event.waitUntil(self.registration.showNotification(data.title, { body: "Ouvrez Ezerbot pour consulter cet événement.", icon: "/icon-192.png", badge: "/icon-192.png", tag: data.id, data: { notification: data.id } }));
});
self.addEventListener("notificationclick", event => {
  event.notification.close(); const id = event.notification.data?.notification;
  if (typeof id !== "string") return;
  event.waitUntil(self.clients.matchAll({ type: "window", includeUncontrolled: true }).then(async clients => {
    const client = clients.find(client => new URL(client.url).origin === self.location.origin);
    if (client) { client.postMessage({ notification: id }); await client.focus(); }
    else await self.clients.openWindow(`/?notification=${encodeURIComponent(id)}`);
  }));
});
