"use strict";
// Only immutable public shell assets. No API response, transcript, device cookie,
// push subscription or media enters CacheStorage. Revocation is server-enforced.
const CACHE = "ezerbot-shell-iphone-v1";
const ASSETS = ["/", "/styles.css", "/mascot.css", "/chat.css", "/panels.css", "/iphone.css", "/mascot.js", "/chat.js", "/iphone.js", "/app.js", "/panels.js", "/manifest.webmanifest", "/icon-192.png", "/icon-512.png"];
// The pieces chat.js loads when a message needs them, cached at install so they work offline.
const ON_DEMAND = ["/chat-code.js", "/chat-mermaid.js", "/chat-math.js", "/katex/KaTeX_AMS-Regular.woff2", "/katex/KaTeX_Caligraphic-Bold.woff2", "/katex/KaTeX_Caligraphic-Regular.woff2", "/katex/KaTeX_Fraktur-Bold.woff2", "/katex/KaTeX_Fraktur-Regular.woff2", "/katex/KaTeX_Main-Bold.woff2", "/katex/KaTeX_Main-BoldItalic.woff2", "/katex/KaTeX_Main-Italic.woff2", "/katex/KaTeX_Main-Regular.woff2", "/katex/KaTeX_Math-BoldItalic.woff2", "/katex/KaTeX_Math-Italic.woff2", "/katex/KaTeX_SansSerif-Bold.woff2", "/katex/KaTeX_SansSerif-Italic.woff2", "/katex/KaTeX_SansSerif-Regular.woff2", "/katex/KaTeX_Script-Regular.woff2", "/katex/KaTeX_Size1-Regular.woff2", "/katex/KaTeX_Size2-Regular.woff2", "/katex/KaTeX_Size3-Regular.woff2", "/katex/KaTeX_Size4-Regular.woff2", "/katex/KaTeX_Typewriter-Regular.woff2"];
const SHELL = [...ASSETS, ...ON_DEMAND];
self.addEventListener("install", event => { event.waitUntil(caches.open(CACHE).then(cache => cache.addAll(SHELL)).then(() => self.skipWaiting())); });
self.addEventListener("activate", event => { event.waitUntil(caches.keys().then(keys => Promise.all(keys.filter(key => key.startsWith("ezerbot-shell-") && key !== CACHE).map(key => caches.delete(key)))).then(() => self.clients.claim())); });
self.addEventListener("fetch", event => {
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin || event.request.method !== "GET" || !SHELL.includes(url.pathname) || (url.search && url.pathname !== "/")) return;
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
  // The text of the reply is never in the payload: only the bot's name, as title.
  const options = { body: "Ouvrez Ezerbot pour consulter cet événement.", icon: "/icon-192.png", badge: "/icon-192.png", tag: data.id, data: { notification: data.id } };
  // iOS shows no action buttons on a web notification: a plain tap opens the conversation.
  if (data.kind === "reply") options.actions = [{ action: "reply", title: "Répondre" }];
  event.waitUntil(self.registration.showNotification(data.title, options));
});
self.addEventListener("notificationclick", event => {
  event.notification.close(); const id = event.notification.data?.notification;
  if (typeof id !== "string") return;
  const reply = event.action === "reply";
  event.waitUntil(self.clients.matchAll({ type: "window", includeUncontrolled: true }).then(async clients => {
    const client = clients.find(client => new URL(client.url).origin === self.location.origin);
    if (client) { client.postMessage(reply ? { notification: id, reply: true } : { notification: id }); await client.focus(); }
    else await self.clients.openWindow(`/?notification=${encodeURIComponent(id)}${reply ? "&reply=1" : ""}`);
  }));
});
