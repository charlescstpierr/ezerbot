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
  let shellCached = true;
  const cache = { addAll: async assets => writes.push(...assets), put: async request => writes.push(typeof request === 'string' ? request : request.url) };
  const caches = { open: async () => cache, keys: async () => [], delete: async () => true, match: async request => shellCached ? { offline: request } : undefined };
  let offline = false, denied = false;
  const context = { self, caches, URL, fetch: async () => {
    if (offline) throw new Error('offline');
    if (denied) return { ok: false, status: 403, clone() { throw new Error('un 403 ne doit pas être mis en cache'); } };
    return { ok: true, clone: () => ({}) };
  } };
  vm.runInNewContext(fs.readFileSync(path.join(root, 'sw.js'), 'utf8'), context);
  async function fire(name, fields = {}) { const waits = []; let response;
    events.get(name)({ ...fields, waitUntil: promise => waits.push(promise), respondWith: promise => { response = promise; } });
    if (response) response = await response; await Promise.all(waits); return response;
  }
  return { fire, notices, opened, writes, offline: () => { offline = true; }, deny: () => { denied = true; }, emptyCache: () => { shellCached = false; } };
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
test('GET / returning 403 renders the cached shell and never stores the denial', async () => {
  const w = worker();
  w.deny();
  const cached = await w.fire('fetch', { request: { method: 'GET', url: 'https://mac.qa.ts.net/' } });
  assert.equal(cached.offline, '/');
  assert.equal(w.writes.includes('/'), false);
  w.emptyCache();
  const denied = await w.fire('fetch', { request: { method: 'GET', url: 'https://mac.qa.ts.net/' } });
  assert.equal(denied.ok, false);
  assert.equal(denied.status, 403);
  const script = await w.fire('fetch', { request: { method: 'GET', url: 'https://mac.qa.ts.net/iphone.js' } });
  assert.equal(script.status, 403);
  assert.deepEqual(w.writes, []);
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

const DISABLED = 'L’accès iPhone est désactivé sur le Mac.';
const DISABLED_BANNER = 'L’accès iPhone est désactivé sur le Mac. Réactivez-le dans Paramètres › iPhone. Reconnexion automatique…';
function phoneShell() {
  const byId = new Map(), timers = [];
  let clock = 0, serial = 1, mode = 'ok', thread = [];
  const store = new Map([['grokbot.selected', 'c1']]);
  const server = {
    state_revision: 3,
    account: { name: 'Espace personnel' },
    conversations: [{ id: 'c1', title: 'Discussion iPhone', bot_id: 'grokbot', kind: 'chat', favorite: false, updated_at: '2026-10-21T00:00:00.000Z', messages: [{ id: 'm0', role: 'assistant', body: 'Bonjour', state: 'complete', created_at: '2026-10-21T00:00:00.000Z' }] }],
    bots: [{ id: 'grokbot', name: 'Ezerbot', color: 'citron', shape: 'galet', deleted: false, is_default: true }],
    provider: { connected: false, signin: 'idle', models: [], welcomed: true },
    preferences: { theme: 'dark', accent: 'citron' },
  };
  class Element {
    constructor(tag) { this.tagName = tag; this.children = []; this.attributes = {}; this._class = ''; this.listeners = {}; this.value = ''; this.style = { setProperty() {} }; this.scrollHeight = 0; this.scrollTop = 0; this.clientHeight = 640; this.clientWidth = 390; }
    get className() { return this._class; }
    set className(value) { this._class = String(value); this.attributes.class = this._class; }
    get id() { return this.attributes.id || ''; }
    set id(value) { this.attributes.id = String(value); byId.set(this.attributes.id, this); }
    get dataset() { const el = this; return new Proxy({}, { get: (_, key) => el.attributes[`data-${String(key).replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`)}`], set: (_, key, value) => { el.attributes[`data-${String(key).replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`)}`] = String(value); return true; }, deleteProperty: (_, key) => { delete el.attributes[`data-${String(key).replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`)}`]; return true; } }); }
    get classList() { const el = this; return { add: (...names) => names.forEach(name => el.classList.toggle(name, true)), remove: (...names) => names.forEach(name => el.classList.toggle(name, false)), toggle: (name, on) => { const names = new Set(el.className.split(/\s+/).filter(Boolean)); const enabled = on === undefined ? !names.has(name) : on; if (enabled) names.add(name); else names.delete(name); el.className = [...names].join(' '); return enabled; }, contains: name => el.className.split(/\s+/).includes(name) }; }
    set textContent(value) { this.text = value == null ? '' : String(value); this.children = []; }
    get textContent() { return (this.text || '') + this.children.map(child => child.textContent).join(''); }
    append(...children) { for (const child of children) { if (child == null || typeof child !== 'object') continue; child.parent = this; this.children.push(child); } }
    prepend(...children) { this.children.unshift(...children); children.forEach(child => { child.parent = this; }); }
    replaceChildren(...children) { this.text = ''; this.children = []; this.append(...children); }
    setAttribute(key, value) { this.attributes[key] = String(value); if (key === 'class') this._class = String(value); if (key === 'id') byId.set(String(value), this); }
    getAttribute(key) { return this.attributes[key]; }
    removeAttribute(key) { delete this.attributes[key]; }
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
    querySelectorAll(selector) { return queryAll(this, selector); }
    querySelector(selector) { return queryAll(this, selector)[0] || null; }
    get parentElement() { return this.parent || null; }
    contains(node) { return node === this || this.children.some(child => child.contains?.(node)); }
    closest() { return null; }
    focus() {}
    select() {}
    showModal() { this.open = true; }
    close() { this.open = false; }
    remove() { if (this.parent) this.parent.children = this.parent.children.filter(child => child !== this); }
    get isConnected() { return Boolean(this.parent); }
    get localName() { return this.tagName; }
    cloneNode() { const copy = new Element(this.tagName); copy.className = this.className; Object.assign(copy.attributes, this.attributes); return copy; }
    getBoundingClientRect() { return { left: 0, top: 500, width: 390, height: 40, bottom: 540, right: 390 }; }
    hidePopover() {}
    showPopover() {}
  }
  function matches(el, selector) {
    const re = /([#.])?([A-Za-z0-9_-]+)|\[([^\]=~|^$*\s]+)(?:([~|^$*]?=)"?([^"\]]*)"?)?\]/g;
    let saw = false, found;
    while ((found = re.exec(selector))) {
      saw = true;
      if (found[1] === '.') { if (!el.className.split(/\s+/).includes(found[2])) return false; }
      else if (found[1] === '#') { if (el.id !== found[2]) return false; }
      else if (found[2]) { if (el.tagName !== found[2]) return false; }
      else if (!found[4]) { if (el.attributes[found[3]] == null) return false; }
      else if (String(el.attributes[found[3]]) !== found[5]) return false;
    }
    return saw;
  }
  const walk = node => (node.children || []).flatMap(child => [child, ...walk(child)]);
  function queryAll(root, selector) {
    return selector.split(',').flatMap(group => {
      let scope = [root];
      for (const part of group.trim().split(/\s+/).filter(Boolean)) scope = scope.flatMap(node => walk(node).filter(el => matches(el, part)));
      return scope;
    });
  }
  function create(tag, props = {}) {
    const el = new Element(tag);
    if (props.id) el.id = props.id;
    if (props.className) el.className = props.className;
    if (props.hidden) el.hidden = true;
    Object.entries(props.attrs || {}).forEach(([key, value]) => el.setAttribute(key, value));
    return el;
  }
  const documentElement = create('html');
  const body = create('body');
  documentElement.append(body);
  const banner = create('div', { id: 'connection-banner', className: 'connection-banner', hidden: true, attrs: { role: 'alert' } });
  const bannerText = create('span', { id: 'connection-message' });
  banner.append(bannerText);
  const send = create('button', { id: 'send-button' });
  const use = create('use');
  const svg = create('svg');
  svg.append(use); send.append(svg); send.disabled = true;
  const favorite = create('button', { id: 'favorite-button' });
  favorite.append(create('span'));
  const plan = create('div', { id: 'plan-banner', hidden: true });
  plan.append(create('span'));
  const dialog = create('dialog', { id: 'plan-dialog' });
  const usage = create('a');
  const description = create('p', { className: 'delete-description' });
  dialog.append(usage, description);
  const hero = create('button', { attrs: { 'data-mascot': 'hero' } });
  const stage = create('div');
  stage.append(hero);
  const main = create('main', { className: 'main-panel' });
  const dock = create('div', { className: 'composer-dock' });
  const form = create('form', { id: 'composer-form' });
  const composer = create('div', { id: 'composer-input' });
  form.append(composer);
  dock.append(form);
  body.append(banner, send, favorite, plan, dialog, stage, main, create('div', { className: 'bot-picker' }), dock);
  const document = {
    documentElement, body, title: '', activeElement: body,
    createElement: tag => create(tag),
    createElementNS: (_ns, tag) => create(tag),
    createTextNode: text => { const node = create('text'); node.textContent = text; return node; },
    getElementById: id => byId.get(id) || body.append(create('div', { id })) || byId.get(id),
    querySelector: selector => queryAll(documentElement, selector)[0] || null,
    querySelectorAll: selector => queryAll(documentElement, selector),
    addEventListener() {},
  };
  let composerInput;
  const window = {
    EzerbotIPhone: { remote: true, pairing() {} },
    matchMedia: () => ({ matches: false, addEventListener() {} }),
    GrokbotChat: {
      createDeepLinkQueue: () => ({ receive() {}, setReady() {} }),
      createApprovalFollow: () => ({ arm() {}, bind() {}, cancel() {}, observe: () => null }),
      isPendingReply: () => false,
      searchableMessageText: message => message.body || '',
      connectionMessages: (messages) => messages,
      composerConnectionState: ({ offline = false, live = false, busy = false }) => ({ offline, live: live && !offline, busy: busy && !offline, mode: live && !offline ? "stop" : "send", icon: live && !offline ? "#i-stop" : "#i-send", title: offline ? "Moteur local indisponible" : "Envoyer le message", label: "Envoyer le message" }),
      renderThread: (_node, options) => { thread = options.messages; },
      revealPendingApproval() {},
      teachPrompt: '',
      exportMessage: message => message,
      mountInput: (el, options) => {
        const editor = create('div', { className: 'gb-editor' });
        composerInput = { options, mentions: [], _text: '', _doc: { type: 'doc', content: [] }, setReadOnly() {}, setPlaceholder() {}, setInvalid() {}, focus() {} };
        Object.defineProperty(composerInput, 'value', { get: () => composerInput._text, set: value => { composerInput._text = String(value ?? ''); } });
        Object.defineProperty(composerInput, 'document', { get: () => composerInput._doc, set: value => { composerInput._doc = value; } });
        editor.editor = { commands: { clearContent() { composerInput._text = ''; composerInput._doc = { type: 'doc', content: [] }; } } };
        el.append(editor);
        return composerInput;
      },
    },
    addEventListener() {},
    innerHeight: 852,
  };
  async function fetch(url) {
    if (mode === 'offline') throw new TypeError('Failed to fetch');
    if (mode === 'disabled') return { ok: false, status: 403, json: async () => ({ error: DISABLED }) };
    if (mode === 'mac-only') return { ok: false, status: 403, json: async () => ({ error: 'Cette action reste disponible uniquement sur le Mac.' }) };
    if (url === '/api/state') return { ok: true, status: 200, json: async () => structuredClone(server) };
    if (url === '/api/ping') return { ok: true, status: 200, json: async () => ({ revision: server.state_revision }) };
    return { ok: true, status: 200, json: async () => ({}) };
  }
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'app.js'), 'utf8'), {
    window, document, localStorage: { getItem: key => store.has(key) ? store.get(key) : null, setItem: (key, value) => store.set(key, String(value)), removeItem: key => store.delete(key) },
    fetch, console, structuredClone, TextEncoder, AbortController, URL, crypto, innerHeight: 852, navigator: {},
    setTimeout: (fn, ms = 0) => { const id = serial++; timers.push({ id, fn, at: clock + Number(ms) }); return id; },
    clearTimeout: id => { const index = timers.findIndex(timer => timer.id === id); if (index >= 0) timers.splice(index, 1); },
    ResizeObserver: class { observe() {} disconnect() {} },
  });
  async function settle() { for (let i = 0; i < 40; i += 1) await Promise.resolve(); }
  async function advance(ms) {
    clock += ms;
    for (let guard = 0; guard < 20; guard += 1) {
      const due = timers.filter(timer => timer.at <= clock).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      timers.splice(timers.indexOf(due), 1);
      const result = due.fn();
      if (result?.then) await result;
      await settle();
    }
  }
  return { banner, bannerText, send, form, store, server, composer: () => composerInput, thread: () => thread, advance, settle, setMode: value => { mode = value; } };
}
test('a disabled iPhone listener shows its banner and blocks send within one ping', async () => {
  const phone = phoneShell();
  await phone.settle();
  const text = 'Brouillon pendant la désactivation';
  phone.composer().value = text;
  phone.composer().options.onInput();
  assert.equal(phone.send.disabled, false);
  assert.equal(phone.banner.hidden, true);
  phone.setMode('disabled');
  await phone.advance(5000);
  assert.equal(phone.banner.hidden, false);
  assert.equal(phone.bannerText.textContent, DISABLED_BANNER);
  assert.equal(phone.banner.getAttribute('role'), 'alert');
  assert.equal(phone.send.disabled, true);
  assert.equal(phone.composer().value, text, 'an idle refusal must not clear the draft');
  phone.setMode('ok');
  phone.composer().value = text;
  phone.composer().options.onInput();
  await phone.advance(1500);
  assert.equal(phone.banner.hidden, true);
  assert.equal(phone.send.disabled, false);
});
test('a Mac-only 403 does not look like a disabled iPhone listener', async () => {
  const phone = phoneShell();
  await phone.settle();
  phone.composer().value = 'Action locale';
  phone.composer().options.onInput();
  phone.setMode('mac-only');
  await phone.advance(5000);
  assert.equal(phone.banner.hidden, true);
  assert.equal(phone.send.disabled, false);
  assert.equal(phone.composer().value, 'Action locale');
});
test('a send refused because iPhone access is disabled keeps the text out of the thread', async () => {
  const phone = phoneShell();
  await phone.settle();
  const text = 'Message important pendant la désactivation';
  phone.composer().value = text;
  phone.composer().options.onInput();
  phone.setMode('disabled');
  await phone.form.listeners.submit[0]({ preventDefault() {} });
  await phone.settle();
  assert.equal(phone.composer().value, text);
  assert.equal(phone.store.get('grokbot.draft.c1'), text);
  assert.equal(phone.bannerText.textContent, DISABLED_BANNER);
  assert.equal(phone.send.disabled, true);
  assert.equal(phone.thread().some(message => message.body === text), false);
  assert.equal(phone.server.conversations[0].messages.some(message => message.body === text), false);
  phone.setMode('ok');
  await phone.advance(1500);
  assert.equal(phone.banner.hidden, true);
  assert.equal(phone.composer().value, text);
  assert.equal(phone.thread().some(message => message.body === text), false);
  phone.composer().options.onInput();
  assert.equal(phone.send.disabled, false);
});
test('a remote network failure keeps the draft without claiming the listener is disabled', async () => {
  const phone = phoneShell();
  await phone.settle();
  const text = 'Brouillon pendant la coupure';
  phone.composer().value = text;
  phone.composer().options.onInput();
  phone.setMode('offline');
  await phone.form.listeners.submit[0]({ preventDefault() {} });
  await phone.settle();
  assert.equal(phone.composer().value, text);
  assert.equal(phone.store.get('grokbot.draft.c1'), text);
  assert.match(phone.bannerText.textContent, /Mac injoignable/);
  assert.equal(phone.send.disabled, true);
  assert.equal(phone.thread().some(message => message.body === text && message.state === 'unknown'), true);
});
test('iPhone does not mount Mac dictation, voice mode, or meeting capture', () => {
  for (const file of ['panels/voice.js', 'panels/voice-conversation.js', 'panels/meetings.js']) {
    let registered = false;
    const send = { id: 'send-button', parent: { children: [], append(child) { this.children.push(child); } }, before(child) { send.parent.children.push(child); } };
    vm.runInNewContext(fs.readFileSync(path.join(__dirname, file), 'utf8'), {
      window: { EzerbotIPhone: { remote: true }, GrokbotPanels: { register() { registered = true; } }, addEventListener() {} },
      document: { createElement: () => ({ className: '', children: [], dataset: {}, setAttribute() {}, append() {}, addEventListener() {} }), createTextNode: () => ({}), getElementById: () => send, querySelector: () => null, addEventListener() {} },
      localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
      crypto, setTimeout() {}, clearTimeout() {},
    });
    assert.equal(registered, false, file);
    assert.equal(send.parent.children.length, 0, file);
  }
});
