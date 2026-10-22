// Run with: node --test app/ui/panels/usage.test.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
class Element {
  constructor(tag) {
    this.tagName = tag; this.children = []; this.dataset = {}; this.attrs = {}; this.events = {}; this.hidden = false;
    const classes = new Set();
    this.classList = { contains: c => classes.has(c), toggle(c, on) { if (on) classes.add(c); else classes.delete(c); } };
  }
  append(...children) { this.children.push(...children); }
  after(child) { this.afterNode = child; }
  replaceChildren(...children) { this.children = children; }
  setAttribute(k, v) { this.attrs[k] = v; }
  addEventListener(k, v) { this.events[k] = v; }
  focus() { this.focused = true; }
  scrollIntoView() {}
  showModal() { this.open = true; }
  querySelectorAll(selector) {
    const matches = e => selector.startsWith('.') ? e.className?.split(' ').includes(selector.slice(1)) : e.tagName === selector;
    return this.children.filter(c => c instanceof Element).flatMap(c => [...(matches(c) ? [c] : []), ...c.querySelectorAll(selector)]);
  }
  querySelector(selector) {
    if (selector.startsWith('[data-view=')) return this.querySelectorAll('button').find(e => e.dataset.view === selector.match(/"([^"]+)"/)[1]);
    return this.querySelectorAll(selector)[0];
  }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map(c => c instanceof Element ? c.textContent : c).join(' '); }
}
const local = () => ({ total: { input_tokens: 1200, output_tokens: 300, cached_input_tokens: 100, measured_runs: 4, unknown_runs: 1 }, by_bot: [{ name: '<img onerror=evil>', deleted: false, tokens: { input_tokens: 1200, output_tokens: 300, measured_runs: 4, unknown_runs: 1 } }], by_source: ['chat', 'routine', 'group', 'delegation', 'bot_to_bot'].map(source => ({ source, tokens: { input_tokens: 240, output_tokens: 60, measured_runs: 1, unknown_runs: 0 } })) });
const quota = used => ({ status: 'ready', quota: { plan: 'plus', checked_at: 1800000000, primary: { used_percent: used, remaining_percent: 100 - used, limit_window_seconds: 604800, reset_at: 1800500000 } } });
function harness(api = async () => ({ codex: quota(80), local: local() })) {
  const section = new Element('section'), dialog = new Element('dialog'), topbar = new Element('header');
  const header = new Element('div'); header.className = 'dialog-header'; dialog.append(header, section);
  let nextPoll;
  const panels = { api, refresh: async () => {}, register(p) { this.panel = p; p.settingsSection(section); } };
  const document = { visibilityState: 'visible', createElement: t => new Element(t), getElementById: () => dialog, querySelector: () => topbar };
  vm.runInNewContext(fs.readFileSync(`${__dirname}/usage.js`, 'utf8'), { window: { GrokbotPanels: panels }, document, Intl, Date, setInterval() {}, setTimeout(f) { nextPoll = f; return 1; }, clearTimeout() { nextPoll = null; }, MutationObserver: class { observe() {} } });
  return { panels, section, dialog, nav: header.afterNode, warning: topbar.afterNode, poll: () => nextPoll?.() };
}
const flush = () => new Promise(resolve => setImmediate(resolve));
const state = codex => ({ provider: { backend: 'codex', connected: true }, usage: { codex } });

test('80% is inclusive and both remaining quota and reset are rendered without HTML injection', () => {
  const h = harness();
  h.panels.panel.onState(state(quota(79.9)));
  assert.equal(h.warning.hidden, true);
  h.panels.panel.onState(state(quota(80)));
  assert.equal(h.warning.hidden, false);
  assert.match(h.section.textContent, /20 % restants/);
  assert.match(h.section.textContent, /Réinitialisation le/);
  assert.match(h.section.textContent, /Seuil de 80 % atteint/);
  assert.equal(h.section.querySelector('progress').value, 80);
  h.panels.panel.onState(state({ status: 'signin' }));
  assert.equal(h.warning.hidden, true);
  assert.equal(h.section.querySelectorAll('progress').length, 0);
  assert.match(h.section.textContent, /Connectez votre compte/);
});

test('Settings Usage opens the local breakdown; source tab includes all five sources', async () => {
  const h = harness();
  h.nav.querySelectorAll('button')[1].events.click(); await flush();
  assert.equal(h.dialog.classList.contains('usage-focus'), true);
  assert.match(h.section.textContent, /1.500 tokens/);
  assert.match(h.section.textContent, /<img onerror=evil>/); // literal text, never innerHTML
  assert.match(h.section.textContent, /total partiel, jamais estimé/);
  h.section.querySelector('[data-view="source"]').events.click();
  for (const label of ['Chat', 'Routines', 'Groupes', 'Délégation', 'Entre bots']) assert.ok(h.section.textContent.includes(label));
  h.nav.querySelectorAll('button')[0].events.click();
  assert.equal(h.dialog.classList.contains('usage-focus'), false);
});

test('unknown and error states never fabricate a quota; refresh errors stay in the modal', async () => {
  const h = harness(async () => { throw new Error('Connexion interrompue'); });
  for (const status of ['unsupported', 'unavailable', 'loading']) {
    h.panels.panel.onState(state({ status }));
    assert.equal(h.section.querySelectorAll('progress').length, 0);
    assert.equal(h.warning.hidden, true);
  }
  h.nav.querySelectorAll('button')[1].events.click(); await flush();
  assert.match(h.section.textContent, /Connexion interrompue/);
  assert.equal(h.section.attrs['aria-busy'], 'false');
  assert.equal(h.section.querySelector('.usage-error').attrs.role, 'alert');
});

test('an asynchronous quota updates even while the workspace database stays idle', async () => {
  const h = harness();
  h.panels.panel.onState(state({ status: 'loading' }));
  h.poll(); await flush();
  assert.equal(h.warning.hidden, false);
  assert.match(h.section.textContent, /20 % restants/);
});

test('a late quota read cannot restore signed-out identity or gauge', async () => {
  let finish;
  const h = harness(() => new Promise(resolve => { finish = resolve; }));
  h.panels.panel.onState(state(quota(85)));
  h.nav.querySelectorAll('button')[1].events.click();
  h.panels.panel.onState({ provider: { backend: 'codex', connected: false }, usage: { codex: { status: 'signin' } } });
  finish({ codex: quota(85), local: local() }); await flush();
  assert.equal(h.warning.hidden, true);
  assert.equal(h.section.querySelectorAll('progress').length, 0);
  assert.match(h.section.textContent, /Connectez votre compte/);
});

const windowQuota = (...seconds) => ({ status: 'ready', quota: { plan: 'plus', checked_at: 1800000000, primary: { used_percent: 10, remaining_percent: 90, limit_window_seconds: seconds[0], reset_at: 1800500000 }, secondary: seconds[1] && { used_percent: 10, remaining_percent: 90, limit_window_seconds: seconds[1], reset_at: 1800500000 } } });
const cardTitles = h => h.section.querySelectorAll('h4').map(e => e.textContent);

test('quota window labels agree in number: « 1 heure », « 1 jour », then plurals', () => {
  const h = harness();
  h.panels.panel.onState(state(windowQuota(3600, 86400)));
  assert.deepEqual(cardTitles(h), ['Quota sur 1 heure', 'Quota sur 1 jour']);
  h.panels.panel.onState(state(windowQuota(18000, 172800)));
  assert.deepEqual(cardTitles(h), ['Quota sur 5 heures', 'Quota sur 2 jours']);
  h.panels.panel.onState(state(windowQuota(604800)));
  assert.deepEqual(cardTitles(h), ['Quota hebdomadaire']);
});

test('sources are listed as chat, routine, group, delegation, bot-to-bot whatever order the server sends', async () => {
  const alphabetical = ['bot_to_bot', 'chat', 'delegation', 'group', 'routine'];
  const data = { ...local(), by_source: alphabetical.map(source => ({ source, tokens: { input_tokens: 240, output_tokens: 60, measured_runs: 1, unknown_runs: 0 } })) };
  const h = harness(async () => ({ codex: quota(10), local: data }));
  h.nav.querySelectorAll('button')[1].events.click(); await flush();
  h.section.querySelector('[data-view="source"]').events.click();
  const rows = h.section.querySelectorAll('tbody')[0].children.map(tr => tr.querySelector('span').textContent);
  assert.deepEqual(rows, ['Chat', 'Routines', 'Groupes', 'Délégation', 'Entre bots']);
});

test('a missing bot reads « Bot supprimé » once; a named deleted bot keeps its suffix', async () => {
  const tokens = { input_tokens: 10, output_tokens: 5, measured_runs: 1, unknown_runs: 0 };
  const data = { ...local(), by_bot: [{ name: 'Bot supprimé', deleted: true, tokens }, { name: 'Atlas', deleted: true, tokens }, { name: 'Zoé', deleted: false, tokens }] };
  const h = harness(async () => ({ codex: quota(10), local: data }));
  h.nav.querySelectorAll('button')[1].events.click(); await flush();
  const rows = h.section.querySelectorAll('tbody')[0].children.map(tr => tr.querySelector('span').textContent);
  assert.deepEqual(rows, ['Bot supprimé', 'Atlas (supprimé)', 'Zoé']);
});

test('a settings link to Usage shows the section; any other settings link leaves Usage', async () => {
  const h = harness();
  h.dialog.showModal();
  h.panels.panel.onSettingsLink('usage'); await flush();
  assert.equal(h.dialog.classList.contains('usage-focus'), true);
  assert.match(h.section.textContent, /1.500 tokens/);
  h.panels.panel.onSettingsLink('appearance');
  assert.equal(h.dialog.classList.contains('usage-focus'), false);
});
