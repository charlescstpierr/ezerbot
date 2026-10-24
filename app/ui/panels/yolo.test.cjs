// Run with: node --test app/ui/panels/yolo.test.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.listeners = {}; this.dataset = {}; this.attributes = {}; this.className = ''; this.hidden = false; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  addEventListener(name, action) { this.listeners[name] = action; }
  get textContent() { return this._text ?? this.children.map(c => c.textContent ?? String(c)).join(''); }
  set textContent(text) { this._text = String(text); this.children = []; }
}
const plain = (value) => JSON.parse(JSON.stringify(value));
const all = (element) => [element, ...element.children.flatMap(all)];

function mount(state, { fail } = {}) {
  let registration;
  const requests = [];
  const panels = {
    state,
    register(panel) { registration = panel; return true; },
    async api(path, body) { requests.push({ path, body }); if (fail) throw new Error(fail); return {}; },
    async refresh() { refreshed += 1; },
    notify() {},
  };
  let refreshed = 0;
  const document = { createElement: (tag) => new Element(tag), createTextNode: (text) => Object.assign(new Element('#text'), { _text: text }) };
  vm.runInNewContext(fs.readFileSync(require.resolve('./00-yolo.js'), 'utf8'), { window: { GrokbotPanels: panels }, document, JSON, Date, Intl, String });
  const root = new Element('div');
  registration.settingsSection(root);
  return { root, requests, panels, registration, refreshed: () => refreshed };
}
const bots = [{ id: 'grokbot', name: 'Ezerbot' }, { id: 'pair', name: 'Pair' }];
const yolo = (overrides = {}) => ({
  mode: 'yolo', home: '/Users/vous',
  bots: [{ bot_id: 'grokbot', mode: 'inherit', effective: 'yolo' }, { bot_id: 'pair', mode: 'ask', effective: 'ask' }],
  log: [
    { id: 2, at: Date.UTC(2026, 9, 23, 15, 4), bot_id: 'grokbot', tool: 'write_file', summary: 'Écriture de /Users/vous/notes.txt (12 octets)', source: 'global' },
    { id: 1, at: Date.UTC(2026, 9, 23, 15, 3), bot_id: 'pair', tool: 'run_shell', summary: 'Commande « ls » dans /Users/vous', source: 'bot' },
  ],
  ...overrides,
});

test('le panneau annonce le Mode YOLO activé, ses garde-fous et son journal', () => {
  const { root } = mount({ bots, yolo: yolo() });
  const text = root.textContent;
  assert.match(text, /Mode YOLO \(tous les droits\)/);
  assert.match(text, /Activé : tous les outils sont ouverts/);
  for (const guard of [/Les données d’Ezerbot restent illisibles/, /macOS demande ses propres permissions/, /Échap et Arrêter arrêtent toujours le bot/]) assert.match(text, guard);
  assert.match(text, /Écriture de \/Users\/vous\/notes\.txt \(12 octets\)/);
  assert.match(text, /Commande « ls » dans \/Users\/vous/);
  assert.match(text, /réglage du bot/);
  const [on, ask] = all(root).filter(e => e.dataset.yoloMode);
  assert.equal(on.attributes['aria-pressed'], 'true');
  assert.equal(ask.attributes['aria-pressed'], 'false');
});

test('un refus d’action interdite est nommé tel quel dans le journal', () => {
  const refused = { id: 3, at: Date.UTC(2026, 9, 23, 15, 5), bot_id: 'grokbot', tool: 'mcp_stripe_ab12_0123456789abcdef', summary: 'Refusé (action interdite : paiement) · Connecteur Stripe · create_payment', source: 'forbidden' };
  const { root } = mount({ bots, yolo: yolo({ log: [refused] }) });
  const text = root.textContent;
  assert.match(text, /Ezerbot · action interdite/);
  assert.match(text, /Refusé \(action interdite : paiement\)/);
});

test('un clic sur « Demander avant d’agir » revient au mode d’avant, puis un clic rallume le YOLO', async () => {
  const { root, requests, refreshed } = mount({ bots, yolo: yolo() });
  const ask = all(root).find(e => e.dataset.yoloMode === 'ask');
  await ask.listeners.click();
  assert.deepEqual(plain(requests), [{ path: '/api/yolo/mode', body: { mode: 'ask' } }]);
  assert.equal(refreshed(), 1);
  const asking = mount({ bots, yolo: yolo({ mode: 'ask' }) });
  assert.match(asking.root.textContent, /Désactivé : chaque action protégée affiche une carte/);
  assert.equal(all(asking.root).find(e => e.dataset.yoloMode === 'ask').attributes['aria-pressed'], 'true');
  await all(asking.root).find(e => e.dataset.yoloMode === 'yolo').listeners.click();
  assert.deepEqual(plain(asking.requests), [{ path: '/api/yolo/mode', body: { mode: 'yolo' } }]);
  // Le bouton déjà actif n'envoie rien.
  await all(asking.root).find(e => e.dataset.yoloMode === 'ask').listeners.click();
  assert.equal(asking.requests.length, 1);
});

test('chaque bot peut suivre le réglage global, toujours agir ou toujours demander', async () => {
  const { root, requests } = mount({ bots, yolo: yolo() });
  const selects = all(root).filter(e => e.tagName === 'select');
  assert.deepEqual(selects.map(s => s.name), ['yolo-grokbot', 'yolo-pair']);
  assert.deepEqual(selects.map(s => s.value), ['inherit', 'ask']);
  assert.deepEqual(all(selects[0]).filter(e => e.tagName === 'option').map(o => o.textContent), ['Suivre le réglage global', 'Mode YOLO', 'Demander avant d’agir']);
  selects[1].value = 'yolo';
  await selects[1].listeners.change();
  assert.deepEqual(plain(requests), [{ path: '/api/yolo/bot', body: { bot_id: 'pair', mode: 'yolo' } }]);
  const badges = all(root).filter(e => e.className === 'yolo-effective');
  assert.deepEqual(badges.map(b => b.textContent), ['Sans demander', 'Avec cartes']);
});

test('un refus du serveur reste visible dans le panneau, sans changer l’état affiché', async () => {
  const { root } = mount({ bots, yolo: yolo() }, { fail: 'Bot introuvable.' });
  await all(root).find(e => e.dataset.yoloMode === 'ask').listeners.click();
  const alert = all(root).find(e => e.className === 'yolo-error');
  assert.equal(alert.hidden, false);
  assert.equal(alert.attributes.role, 'alert');
  assert.match(alert.textContent, /Bot introuvable/);
});

test('sans action automatique le journal le dit, et un état indisponible ne plante pas', () => {
  const { root } = mount({ bots, yolo: yolo({ log: [] }) });
  assert.match(root.textContent, /Aucune action faite automatiquement pour l’instant/);
  const broken = mount({ bots, yolo: { error: 'Mode YOLO indisponible.', mode: 'ask' } });
  assert.match(broken.root.textContent, /Mode YOLO indisponible/);
  const absent = mount({ bots });
  assert.match(absent.root.textContent, /Mode YOLO indisponible/);
});
