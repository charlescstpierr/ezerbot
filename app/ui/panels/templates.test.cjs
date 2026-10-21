// Run with: node --test app/ui/panels/templates.test.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

class Element {
  constructor(tag, document) { this.ownerDocument = document; this.tagName = tag.toUpperCase(); this.children = []; this.dataset = {}; this.attributes = {}; this.listeners = {}; this.style = { setProperty() {} }; this.className = ''; this.open = false; this.isConnected = true; }
  append(...children) { for (const child of children) { if (child instanceof Element) child.parent = this; } this.children.push(...children); this.changed?.(); }
  prepend(...children) { this.children.unshift(...children); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  remove() { if (this.parent) this.parent.children = this.parent.children.filter(c => c !== this); }
  setAttribute(key, value) { this.attributes[key] = value; }
  getAttribute(key) { return this.attributes[key]; }
  addEventListener(name, action) { this.listeners[name] = action; }
  dispatch(name, event = {}) { return this.listeners[name]?.({ preventDefault() {}, ...event }); }
  click() { if (!this.disabled) return this.listeners.click?.(); }
  focus() { this.ownerDocument.activeElement = this; }
  scrollIntoView() {}
  showModal() { this.open = true; }
  close() { this.open = false; this.dispatch('close'); }
  matches(selector) {
    if (selector.startsWith('.')) return this.className.split(' ').includes(selector.slice(1));
    const data = /^\[data-([a-z-]+)(?:=([^\]]+))?\]$/.exec(selector);
    if (data) {
      const key = data[1].replace(/-([a-z])/g, (_, c) => c.toUpperCase());
      return key in this.dataset && (data[2] === undefined || this.dataset[key] === data[2]);
    }
    return this.tagName === selector.toUpperCase();
  }
  querySelectorAll(selector) {
    return this.children.filter(c => c instanceof Element).flatMap(c => [...(c.matches(selector) ? [c] : []), ...c.querySelectorAll(selector)]);
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map(c => c?.textContent ?? String(c)).join(''); }
}

const catalogue = {
  categories: [{ id: 'productivite', name: 'Productivité' }, { id: 'code', name: 'Code et données' }],
  connectors: { gmail: { name: 'Gmail', color: '#EA4335' }, github: { name: 'GitHub', color: '#24292F' } },
  templates: [
    { id: 'tri-boite', category: 'productivite', bot: { name: 'Tri de boîte', color: 'orange', shape: 'carre', description: 'Trie vos courriels.', instructions: 'Trie.' },
      skills: [{ name: 'Trier', description: 'Classe', body: '#' }], routines: [{ name: 'Tri du matin', schedule: '0 8 * * 1-5', prompt: 'Trie', schedule_description: 'Du lundi au vendredi à 08:00' }],
      connectors: ['gmail'], starters: ['Trie mes courriels.'] },
    { id: 'dev', category: 'code', bot: { name: 'Dev', color: 'citron', shape: 'carre', description: 'Développeur senior.', instructions: 'Code.' },
      skills: [{ name: 'Revue de code', description: 'Relit', body: '#' }], routines: [], connectors: ['github'], starters: ['Relis ce diff.'] },
  ],
};

function harness({ draft = '', servers = [], prepare } = {}) {
  const calls = [];
  const storage = new Map();
  const panels = {
    state: { bots: [{ id: 'grokbot', name: 'Ezerbot', template: 'grokbot' }], mcp: { servers }, templates: { bots: {} } },
    draftText: draft, selected: [],
    async api(path, body) {
      calls.push([path, body]);
      if (path === '/api/templates') return structuredClone(catalogue);
      if (path === '/api/templates/create') return { bot: { id: body.bot_id, name: body.document?.bot.name || 'Tri de boîte' }, routines_enabled: body.activate_routines };
      if (path === '/api/templates/decode') return { document: structuredClone(catalogue.templates[0]), template_id: null };
      throw new Error(`unexpected ${path}`);
    },
    refresh: async () => {}, notify(message) { this.message = message; },
    createBotMascot: () => () => new Element('span', document),
    draft() { return this.draftText; },
    appendDraft(text) { this.draftText = this.draftText ? `${this.draftText}\n${text}` : text; },
    selectBot(id, text) { this.selected.push(id); if (typeof text === 'string') this.draftText = text; return true; },
    register(panel) { this.panel = panel; },
  };
  prepare?.(panels, calls);
  const window = { GrokbotPanels: panels };
  const byId = {};
  const document = {
    createElement: tag => new Element(tag, document),
    createElementNS: (_, tag) => new Element(tag, document),
    getElementById: id => byId[id] || null,
    execCommand: () => true,
  };
  document.body = new Element('body', document);
  byId['bot-chips'] = new Element('div', document);
  const observers = [];
  class MutationObserver { constructor(callback) { this.callback = callback; observers.push(this); } observe(target) { target.changed = () => this.callback(); } }
  const context = vm.createContext({
    window, document, MutationObserver, structuredClone, console, URL, Blob,
    crypto: { randomUUID: () => `bot-${calls.length}` },
    matchMedia: () => ({ matches: true }),
    localStorage: { getItem: k => storage.get(k) ?? null, setItem: (k, v) => storage.set(k, v) },
    navigator: { clipboard: { writeText: async () => {} } },
    setTimeout,
  });
  const source = fs.readFileSync(`${__dirname}/templates.js`, 'utf8').replace('  window.EzerbotTemplates = { open };', `
  window.EzerbotTemplates = { open };
  window.test = { get body() { return body; }, get dialog() { return dialog; }, useStarter };`);
  vm.runInContext(source, context);
  return { t: window.test, panels, calls, document, byId, storage, open: window.EzerbotTemplates.open };
}

const tick = () => new Promise(resolve => setTimeout(resolve, 0));

test('the welcome row keeps one « Créer depuis un modèle » entry across re-renders', () => {
  const { byId, document } = harness();
  const chips = byId['bot-chips'];
  chips.replaceChildren(new Element('button', document));
  chips.replaceChildren(new Element('button', document), new Element('button', document));
  const entries = chips.querySelectorAll('[data-template-entry]');
  assert.equal(entries.length, 1);
  assert.match(entries[0].textContent, /Créer depuis un modèle/);
});

test('the gallery lists every template, searches without accents and filters by category', async () => {
  const { t, open } = harness();
  await open();
  assert.equal(t.body.querySelectorAll('.tpl-card').length, 2);
  const search = t.body.querySelector('.tpl-search').querySelector('input');
  search.value = 'boite';
  search.dispatch('input');
  assert.deepEqual(t.body.querySelectorAll('.tpl-card').map(card => card.dataset.template), ['tri-boite']);
  search.value = 'github';
  search.dispatch('input');
  assert.deepEqual(t.body.querySelectorAll('.tpl-card').map(card => card.dataset.template), ['dev']);
  search.value = '';
  search.dispatch('input');
  t.body.querySelector('[data-category=code]').click();
  assert.deepEqual(t.body.querySelectorAll('.tpl-card').map(card => card.dataset.template), ['dev']);
});

test('a template matching a configured connector is recommended first', async () => {
  const { t, open } = harness({ servers: [{ name: 'GitHub local' }] });
  await open();
  const first = t.body.querySelector('.tpl-section');
  assert.match(first.textContent, /Recommandés pour vous/);
  assert.deepEqual(first.querySelectorAll('.tpl-card').map(card => card.dataset.template), ['dev']);
});

test('one click creates the catalogue bot and opens its new thread', async () => {
  const { t, panels, calls, open } = harness();
  await open({ templateId: 'tri-boite' });
  const create = t.body.querySelector('[data-template-create]');
  await create.click();
  const [path, body] = calls.at(-1);
  assert.equal(path, '/api/templates/create');
  assert.equal(body.template_id, 'tri-boite');
  assert.equal(body.document, undefined);
  assert.equal(body.activate_routines, false);
  assert.deepEqual(panels.selected, [body.bot_id]);
  assert.equal(t.dialog.open, false);
  assert.match(panels.message, /est prêt/);
});

test('an imported bot is previewed, then created without display-only fields', async () => {
  const { t, calls, open } = harness();
  await open({ import: true });
  t.body.querySelector('textarea').value = 'ezerbot://app/v1/bot-template?data=abc';
  await t.body.querySelector('form').dispatch('submit');
  await tick();
  assert.match(t.body.textContent, /BOT IMPORTÉ/);
  assert.match(t.body.textContent, /créées en pause/);
  await t.body.querySelector('[data-template-create]').click();
  const [, body] = calls.at(-1);
  assert.equal(body.template_id, undefined);
  assert.equal(body.document.routines[0].schedule_description, undefined);
  assert.equal(body.document.routines[0].prompt, 'Trie');
});

test('a starter is not written when the bot cannot be selected', () => {
  const typed = harness({
    draft: 'Mon brouillon',
    prepare(panels) { panels.selectBot = () => false; },
  });
  assert.equal(typed.t.useStarter('bot-a', 'Trie mes courriels.'), false);
  assert.equal(typed.panels.draftText, 'Mon brouillon');

  const fresh = harness({ prepare(panels) { panels.selectBot = () => false; } });
  assert.equal(fresh.t.useStarter('bot-a', 'Trie mes courriels.'), false);
  assert.equal(fresh.panels.draftText, '');
});

test('creation waits for a refresh that actually contains the new bot', async () => {
  let refreshes = 0;
  const { panels, calls, open, t } = harness({
    prepare(panels, calls) {
      panels.refresh = async () => {
        refreshes += 1;
        const created = calls.find((call) => call[0] === '/api/templates/create');
        if (refreshes >= 2 && created) panels.state.bots.push({ id: created[1].bot_id, name: 'Tri de boîte' });
      };
      panels.selectBot = function (id, text) {
        if (!this.state.bots.some((bot) => bot.id === id && !bot.deleted)) return false;
        this.selected.push(id);
        if (typeof text === 'string') this.draftText = text;
        return true;
      };
    },
  });
  await open({ templateId: 'tri-boite' });
  await t.body.querySelector('[data-template-create]').click();
  const created = calls.find((call) => call[0] === '/api/templates/create');
  assert.equal(refreshes, 2);
  assert.deepEqual(panels.selected, [created[1].bot_id]);
});

test('a starter replaces an untouched starter but never a typed draft', () => {
  const typed = harness({ draft: 'Mon brouillon' });
  typed.t.useStarter('bot-a', 'Trie mes courriels.');
  assert.equal(typed.panels.draftText, 'Mon brouillon\nTrie mes courriels.');

  const fresh = harness();
  fresh.t.useStarter('bot-a', 'Trie mes courriels.');
  assert.equal(fresh.panels.draftText, 'Trie mes courriels.');
  fresh.t.useStarter('bot-b', 'Relis ce diff.');
  assert.equal(fresh.panels.draftText, 'Relis ce diff.');
});
