const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { runInNewContext } = require('node:vm');
const path = require('node:path');
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.attributes = {}; this.className = ''; this.text = ''; this.value = ''; this.dataset = {}; }
  set textContent(value) { this.text = value; this.children = []; }
  get textContent() { return this.text + this.children.map(child => child.textContent).join(''); }
  append(...children) { this.children.push(...children); }
  prepend(...children) { this.children.unshift(...children); }
  replaceChildren(...children) { this.text = ''; this.children = children; }
  setAttribute(key, value) { this.attributes[key] = value; }
  get classList() { return { toggle: (name, on) => { const names=new Set(this.className.split(' ').filter(Boolean)); if(on) names.add(name); else names.delete(name); this.className=[...names].join(' '); } }; }
  addEventListener(key, callback) { (this.events ??= {})[key] = callback; }
  focus() { this.focused = true; }
  scrollIntoView() {}
  querySelectorAll(selector) { return all(this).slice(1).filter(e => selector === 'button' ? e.tagName === 'button' : selector.startsWith('.') ? e.className.split(' ').includes(selector.slice(1)) : e.tagName === selector); }
  querySelector(selector) { return this.querySelectorAll(selector)[0]; }
}
const all = root => [root, ...root.children.flatMap(all)];
const button = (root, text) => all(root).find(e => e.tagName === 'button' && e.textContent === text);
const cls = (root, name) => all(root).find(e => e.className.split(' ').includes(name));
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
function panel(remote = false) {
  const root = new Element('div'), bar = new Element('div'), timers = [], requests = [];
  let now = 1800000000000, registration, subscription = false, failure = false, tailnet = true;
  const state = { iphone: { enabled: true, port: 47863, devices: [{ id: 'one', name: 'iPhone Luc', status: 'active', connected: true, last_seen: now }] } };
  const host = { connected: true, state,
    register(value) { registration = value; },
    async refresh() { registration.onState(state, { connected: true, conversation: null }); },
    async api(url, body) {
      requests.push({ url, body: JSON.parse(JSON.stringify(body ?? null)) });
      if (url.endsWith('/status')) return state.iphone;
      if (url.endsWith('/revoke')) state.iphone.devices = state.iphone.devices.filter(d => d.id !== body.id);
      if (url.endsWith('/code')) return { code: 'BB3977625C', expires: now + 120000, ...(tailnet ? { qr: 'data:image/svg+xml;base64,UVI=' } : {}) };
      if (url.endsWith('/serve')) return failure ? { error: 'État Tailscale indisponible.' } : { running: true };
      if (url.endsWith('/presence')) return { ok: true, push: subscription };
      if (url.endsWith('/push')) { subscription = body.subscription !== null; return { ok: true }; }
      if (url.endsWith('/push-key')) return { key: 'AAAA' };
      if (url === '/api/notifications') return { settings: { enabled: true } };
      return state.iphone;
    },
  };
  const nav = { standalone: false, serviceWorker: { ready: Promise.resolve({ pushManager: { getSubscription: async () => ({ toJSON: () => ({ endpoint: 'fake' }) }) } }), getRegistration: async () => ({ pushManager: { getSubscription: async () => ({ unsubscribe: async () => true }) } }) } };
  const notification = { requestPermission: async () => 'granted' };
  const window = { GrokbotPanels: host, EzerbotIPhone: { remote }, PushManager: class {}, Notification: notification };
  runInNewContext(readFileSync(path.join(__dirname, 'panels/iphone.js'), 'utf8'), {
    window, document: { createElement: tag => new Element(tag), querySelector: selector => selector === '.topbar-actions' ? bar : null, addEventListener() {}, visibilityState: 'visible' },
    location: { href: 'https://mac.qa.ts.net/' }, history: {}, URL, navigator: nav, Notification: notification, matchMedia: () => ({ matches: false }), atob, Uint8Array,
    Date: class extends Date { constructor(...args) { super(...(args.length ? args : [now])); } static now() { return now; } },
    setInterval: fn => timers.push(fn), setTimeout: () => 1, clearTimeout() {}, queueMicrotask,
    // WKWebView's missing delegate cannot be used as a fallback by any action.
    confirm() { throw new Error('Native confirm must never be called'); },
  });
  registration.settingsSection(root); registration.onState(state, { connected: true, conversation: null });
  return { root, bar, state, requests, timers, nav, host, registration,
    advance(ms) { now += ms; }, failServe() { failure = true; }, noTailnet() { tailnet = false; }, refresh: () => host.refresh() };
}
test('revoke and both Serve actions use cancellable in-page decisions, never native confirm', async () => {
  for (const [label, approve, route, payload] of [
    ['Révoquer', 'Révoquer', '/api/iphone/revoke', { id: 'one' }],
    ['Publier Ezerbot avec Tailscale', 'Publier', '/api/iphone/serve', { start: true }],
    ['Arrêter le service HTTPS Tailscale', 'Arrêter', '/api/iphone/serve', { start: false }],
  ]) {
    const f = panel();
    button(f.root, label).onclick();
    assert.ok(cls(f.root, 'iphone-confirmation'));
    assert.equal(f.requests.length, 0);
    button(cls(f.root, 'iphone-confirmation'), 'Annuler').onclick();
    assert.equal(f.requests.length, 0); assert.equal(cls(f.root, 'iphone-confirmation'), undefined);
    button(f.root, label).onclick();
    await button(cls(f.root, 'iphone-confirmation'), approve).onclick();
    assert.deepEqual(f.requests.find(r => r.url === route), { url: route, body: payload });
    assert.equal(cls(f.root, 'iphone-confirmation'), undefined);
    if (route.endsWith('revoke')) assert.equal(f.state.iphone.devices.length, 0);
    else {
      const notice = cls(f.root, 'iphone-notice'); assert.equal(notice.role, 'status');
      const actions = cls(f.root, 'iphone-actions');
      assert.equal(f.root.children.indexOf(notice), f.root.children.indexOf(actions) + 1);
    }
  }
});
test('failed stop is an alert near its controls, never a false success', async () => {
  const f = panel(); f.failServe(); button(f.root, 'Arrêter le service HTTPS Tailscale').onclick();
  await button(cls(f.root, 'iphone-confirmation'), 'Arrêter').onclick();
  const alert = all(f.root).find(e => e.role === 'alert'); assert.equal(alert.textContent, 'État Tailscale indisponible.');
  assert.ok(!f.root.textContent.includes('Service HTTPS Tailscale arrêté.'));
});
test('codes disappear on expiry and consumption; an older pending device does not consume a new code', async () => {
  const f = panel(); await button(f.root, 'Créer un QR de jumelage').onclick();
  assert.equal(cls(f.root, 'iphone-code').textContent, 'BB39 7762 5C');
  f.advance(120001); await f.timers[0]();
  assert.equal(cls(f.root, 'iphone-code'), undefined); assert.match(f.root.textContent, /Code expiré/);
  await button(f.root, 'Créer un QR de jumelage').onclick();
  f.state.iphone.devices.push({ id: 'two', name: 'Nouveau téléphone', status: 'pending' }); await f.refresh();
  assert.equal(cls(f.root, 'iphone-code'), undefined); assert.match(f.root.textContent, /Code utilisé/);
  await button(f.root, 'Créer un QR de jumelage').onclick();
  assert.equal(cls(f.root, 'iphone-code').textContent, 'BB39 7762 5C');
  assert.match(f.root.textContent, /Connecté maintenant/);
  assert.equal(f.bar.children[0].attributes['aria-label'], 'iPhone connecté');
});
test('the code card shows the QR to scan, or says how to type the code without Tailscale', async () => {
  const f = panel(); await button(f.root, 'Créer un QR de jumelage').onclick();
  const card = cls(f.root, 'iphone-pairing-card'), image = cls(card, 'iphone-qr');
  assert.equal(image.tagName, 'img'); assert.equal(image.src, 'data:image/svg+xml;base64,UVI=');
  assert.match(image.alt, /appareil photo de l’iPhone/);
  assert.equal(cls(card, 'iphone-code').textContent, 'BB39 7762 5C');
  assert.match(card.textContent, /Scannez avec l’appareil photo/);
  assert.ok(button(f.root, 'Créer un nouveau QR'));
  f.advance(120001); await f.timers[0]();
  assert.equal(cls(f.root, 'iphone-pairing-card'), undefined);
  const plain = panel(); plain.noTailnet(); await button(plain.root, 'Créer un QR de jumelage').onclick();
  assert.equal(cls(plain.root, 'iphone-qr'), undefined);
  assert.match(cls(plain.root, 'iphone-pairing-card').textContent, /QR indisponible.*Vérifiez Tailscale/);
  assert.equal(cls(plain.root, 'iphone-code').textContent, 'BB39 7762 5C');
});
test('phone notification state exposes one appropriate action and actionable errors', async () => {
  const f = panel(true); await flush();
  assert.ok(button(f.root, 'Activer les notifications sur cet iPhone'));
  assert.equal(button(f.root, 'Désactiver les notifications sur cet iPhone'), undefined);
  await button(f.root, 'Activer les notifications sur cet iPhone').onclick();
  assert.match(cls(f.root, 'iphone-warning').textContent, /écran d’accueil/);
  assert.equal(cls(f.root, 'iphone-warning').role, 'alert');
  f.nav.standalone = true; await button(f.root, 'Activer les notifications sur cet iPhone').onclick();
  assert.ok(button(f.root, 'Désactiver les notifications sur cet iPhone'));
  assert.equal(button(f.root, 'Activer les notifications sur cet iPhone'), undefined);
  await button(f.root, 'Désactiver les notifications sur cet iPhone').onclick();
  assert.ok(button(f.root, 'Activer les notifications sur cet iPhone'));
});
test('notification transport errors clear after reconnect without hiding a failed settings action', async () => {
  const root=new Element('div'), top=new Element('div'), panels=new Element('div'); let poll, offline=true, failSave=false;
  const host={ register(value){value.settingsSection(root);}, async api(url){if(offline)throw Error('Mac injoignable.');if(failSave && url.endsWith('/settings'))throw Error('Réglage non enregistré.');return {settings:{enabled:true,dnd:false},items:[],unread:0};} };
  runInNewContext(readFileSync(path.join(__dirname,'panels/notifications.js'),'utf8'),{
    window:{GrokbotPanels:host,EzerbotIPhone:{remote:true},addEventListener(){}},
    document:{createElement:tag=>new Element(tag),createElementNS:(_,tag)=>new Element(tag),querySelector:s=>s==='.topbar-actions'?top:null,querySelectorAll:()=>[],getElementById:()=>panels,hasFocus:()=>false,visibilityState:'visible',addEventListener(){}},
    setInterval:fn=>{poll=fn;},setTimeout(){},requestAnimationFrame(){},
  });
  await flush();assert.match(cls(root,'notification-warning').textContent,/Mac injoignable/);
  offline=false;await poll();assert.equal(cls(root,'notification-warning'),undefined);
  const input=all(root).find(e=>e.dataset.notificationSetting==='enabled');input.checked=false;failSave=true;await input.onchange();
  await poll();assert.match(cls(root,'notification-warning').textContent,/Réglage non enregistré/);
});
test('pairing brands and installs first, explains revocation, remembers the name and resets errors', async () => {
  const body = new Element('body'), main = new Element('main'); body.append(main);
  const store = new Map([['ezerbot.iphone.name', 'iPhone personnel']]); let validCode = false;
  const window = { fetch: async url => {
    if (url === '/api/iphone/claim') return { ok: validCode, json: async () => validCode ? { receipt: 'test-receipt' } : { error: 'Code incorrect ou expiré.' } };
    if (url === '/api/iphone/exchange') return { ok: true, json: async () => ({ pending: true }) };
    return { ok: url === '/api/ping', status: url === '/api/ping' ? 200 : 401 };
  }, addEventListener() {}, innerHeight: 852 };
  runInNewContext(readFileSync(path.join(__dirname, 'iphone.js'), 'utf8'), {
    window, location: { hostname: '127.0.0.1', port: '47863', hash: '', pathname: '/', search: '' }, innerHeight: 852,
    document: { body, createElement: tag => new Element(tag), querySelectorAll: () => [], documentElement: { dataset: {}, style: { setProperty() {} } } },
    navigator: {}, matchMedia: () => ({ matches: false }), localStorage: { getItem: k => store.get(k), setItem: (k,v) => store.set(k,v) },
    setTimeout() {}, clearTimeout() {}, AbortSignal,
  });
  await window.fetch('/api/ping'); await window.fetch('/api/state');
  const form = all(body).find(e => e.tagName === 'form');
  assert.ok(cls(form, 'iphone-pairing-brand')); assert.ok(main.inert);
  const inputs = all(form).filter(e => e.tagName === 'input'); assert.equal(inputs[0].value, 'iPhone personnel');
  assert.ok(form.children.indexOf(cls(form, 'iphone-install-note')) < form.children.indexOf(inputs[0].parentElement ?? all(form).find(e => e.tagName === 'label')));
  assert.match(cls(form, 'iphone-warning').textContent, /révoqué/);
  inputs[0].value = 'Téléphone renommé'; inputs[1].value = 'INVALIDE';
  await form.onsubmit({ preventDefault() {} });
  assert.equal(cls(form, 'iphone-warning').role, 'alert'); assert.equal(store.get('ezerbot.iphone.name'), 'Téléphone renommé');
  validCode = true; await form.onsubmit({ preventDefault() {} }); await flush();
  assert.equal(cls(form, 'iphone-warning'), undefined); assert.match(form.textContent, /En attente de confirmation/);
});
test('opening the QR link prefills the address and code, drops the fragment and logs nothing', async () => {
  const body = new Element('body'), main = new Element('main'); body.append(main);
  const replaced = [], logged = [], claims = [];
  const window = { fetch: async (url, options) => {
    if (url === '/api/iphone/claim') { claims.push(JSON.parse(options.body)); return { ok: true, json: async () => ({ receipt: 'test-receipt' }) }; }
    if (url === '/api/iphone/exchange') return { ok: true, json: async () => ({ pending: true }) };
    return { ok: false, status: 401 };
  }, addEventListener() {}, innerHeight: 852 };
  const location = { hostname: 'mac.qa.ts.net', port: '', hash: '#jumelage=bb3977625c', pathname: '/', search: '' };
  const console = Object.fromEntries(['log', 'info', 'warn', 'error', 'debug'].map(level => [level, (...args) => logged.push(args.join(' '))]));
  runInNewContext(readFileSync(path.join(__dirname, 'iphone.js'), 'utf8'), {
    window, location, innerHeight: 852, console,
    history: { replaceState: (_state, _title, url) => { replaced.push(url); location.hash = ''; } },
    document: { body, createElement: tag => new Element(tag), querySelectorAll: () => [], documentElement: { dataset: {}, style: { setProperty() {} } } },
    navigator: { standalone: true }, matchMedia: () => ({ matches: true }), localStorage: { getItem: () => null, setItem() {} },
    setTimeout() {}, clearTimeout() {}, AbortSignal,
  });
  assert.deepEqual(replaced, ['/']);
  await window.fetch('/api/state');
  const form = all(body).find(e => e.tagName === 'form');
  const inputs = all(form).filter(e => e.tagName === 'input');
  assert.equal(inputs[1].value, 'BB3977625C');
  assert.match(form.textContent, /Adresse et code remplis depuis le QR/);
  await form.onsubmit({ preventDefault() {} }); await flush();
  assert.deepEqual(claims, [{ name: 'Mon iPhone', code: 'BB3977625C' }]);
  assert.match(form.textContent, /En attente de confirmation sur le Mac/);
  assert.ok(logged.every(line => !/BB3977625C/i.test(line)));
});
test('Safari opened from the QR says to type the code once in the installed app', async () => {
  const body = new Element('body'); const window = { fetch: async () => ({ ok: false, status: 401 }), addEventListener() {}, innerHeight: 852 };
  runInNewContext(readFileSync(path.join(__dirname, 'iphone.js'), 'utf8'), {
    window, location: { hostname: 'mac.qa.ts.net', port: '', hash: '#jumelage=BB3977625C', pathname: '/', search: '' }, innerHeight: 852,
    history: { replaceState() {} },
    document: { body, createElement: tag => new Element(tag), querySelectorAll: () => [], documentElement: { dataset: {}, style: { setProperty() {} } } },
    navigator: {}, matchMedia: () => ({ matches: false }), localStorage: { getItem: () => null, setItem() {} },
    setTimeout() {}, clearTimeout() {}, AbortSignal,
  });
  await window.fetch('/api/state');
  assert.match(cls(body, 'iphone-install-note').textContent, /tapez-y une seule fois le code affiché sur le Mac/);
});
