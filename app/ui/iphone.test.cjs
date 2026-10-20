const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const root = path.resolve(__dirname, '../web/public');
function worker() {
  const events = new Map(), notices = [], opened = [], writes = [];
  const self = { location: { origin: 'https://mac.qa.ts.net' }, addEventListener: (name, handler) => events.set(name, handler),
    registration: { showNotification: async (title, options) => notices.push({ title, options }) },
    clients: { matchAll: async () => [], openWindow: async url => opened.push(url), claim: async () => {} }, skipWaiting: async () => {} };
  const cache = { addAll: async assets => writes.push(...assets), put: async request => writes.push(request.url) };
  const caches = { open: async () => cache, keys: async () => [], delete: async () => true, match: async request => ({ offline: request }) };
  let offline = false;
  const context = { self, caches, URL, fetch: async () => { if (offline) throw new Error('offline'); return { ok: true, clone: () => ({}) }; } };
  vm.runInNewContext(fs.readFileSync(path.join(root, 'sw.js'), 'utf8'), context);
  async function fire(name, fields = {}) { const waits = []; let response;
    events.get(name)({ ...fields, waitUntil: promise => waits.push(promise), respondWith: promise => { response = promise; } });
    if (response) response = await response; await Promise.all(waits); return response;
  }
  return { fire, notices, opened, writes, offline: () => { offline = true; } };
}
test('PWA installs only public shell and never intercepts private API or media', async () => {
  const w = worker(); await w.fire('install'); assert.ok(w.writes.includes('/'));
  assert.ok(w.writes.every(asset => !asset.startsWith('/api/')));
  for (const url of ['/api/state', '/api/attachments/secret', '/api/iphone/exchange']) {
    assert.equal(await w.fire('fetch', { request: { method: 'GET', url: `https://mac.qa.ts.net${url}` } }), undefined);
  }
  w.offline(); const cached = await w.fire('fetch', { request: { method: 'GET', url: 'https://mac.qa.ts.net/' } });
  assert.equal(cached.offline, '/');
  const fromPush = await w.fire('fetch', { request: { method: 'GET', url: 'https://mac.qa.ts.net/?notification=reply%3Aa' } });
  assert.equal(fromPush.offline, '/');
});
test('push displays generic text and click opens an encoded same-origin destination', async () => {
  const w = worker(); await w.fire('push', { data: { json: () => ({ title: 'Autorisation demandée', id: 'input:a/b' }) } });
  assert.equal(w.notices.length, 1); assert.equal(w.notices[0].title, 'Autorisation demandée');
  assert.equal(w.notices[0].options.body, 'Ouvrez Ezerbot pour consulter cet événement.');
  let closed = false; await w.fire('notificationclick', { notification: { data: { notification: 'input:a/b' }, close: () => { closed = true; } } });
  assert.ok(closed); assert.deepEqual(w.opened, ['/?notification=input%3Aa%2Fb']);
  await w.fire('push', { data: { json: () => ({ title: {} }) } }); assert.equal(w.notices.length, 1);
});
test('install manifest describes standalone French app and real maskable PNG dimensions', () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.webmanifest'), 'utf8'));
  assert.equal(manifest.display, 'standalone'); assert.equal(manifest.lang, 'fr-CA'); assert.equal(manifest.scope, '/');
  for (const icon of manifest.icons) { const png = fs.readFileSync(path.join(root, icon.src));
    const size = Number(icon.sizes.split('x')[0]); assert.equal(png.readUInt32BE(16), size); assert.equal(png.readUInt32BE(20), size); assert.ok(icon.purpose.split(' ').includes('maskable'));
  }
});
