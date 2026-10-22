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
  const clock = { now: 0, intervals: [] };
  const calls = [];
  const panels = { busy: false, refreshes: 0, api: (...args) => { calls.push(args); return api(...args); }, refresh: async () => { panels.refreshes++; }, register(p) { this.panel = p; p.settingsSection(section); } };
  const document = { visibilityState: 'visible', createElement: t => new Element(t), getElementById: () => dialog, querySelector: () => topbar };
  const setInterval = (fn, every) => clock.intervals.push({ fn, every, due: clock.now + every });
  vm.runInNewContext(fs.readFileSync(`${__dirname}/usage.js`, 'utf8'), { window: { GrokbotPanels: panels }, document, Intl, Date, setInterval, setTimeout(f) { nextPoll = f; return 1; }, clearTimeout() { nextPoll = null; }, MutationObserver: class { observe() {} } });
  const advance = ms => {
    const end = clock.now + ms;
    for (let next; (next = clock.intervals.filter(i => i.due <= end).sort((a, b) => a.due - b.due)[0]);) { clock.now = next.due; next.due += next.every; next.fn(); }
    clock.now = end;
  };
  const refresh = () => section.querySelectorAll('button').find(b => b.textContent === 'Actualiser');
  return { panels, document, section, dialog, nav: header.afterNode, warning: topbar.afterNode, calls, advance, refresh, intervals: () => clock.intervals.map(i => i.every), poll: () => nextPoll?.() };
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

const deferred = () => { const d = {}; d.promise = new Promise(resolve => { d.resolve = resolve; }); return d; };

test('the quota is re-read every 60 s through the usage route only, never the whole workspace state', async () => {
  const h = harness();
  assert.deepEqual(h.intervals(), [60000]);
  h.advance(59999); await flush();
  assert.equal(h.calls.length, 0);
  h.advance(1); await flush();
  assert.deepEqual(h.calls, [['/api/usage']]);
  assert.equal(h.panels.refreshes, 0);
  assert.equal(h.warning.hidden, false);
  assert.match(h.section.textContent, /20 % restants/);
  h.advance(60000); await flush();
  assert.equal(h.calls.length, 2);
});

test('the 60 s read is skipped while a message is being sent or the window is hidden', async () => {
  const h = harness();
  h.panels.busy = true;
  h.advance(60000); await flush();
  assert.equal(h.calls.length, 0);
  h.panels.busy = false; h.document.visibilityState = 'hidden';
  h.advance(60000); await flush();
  assert.equal(h.calls.length, 0);
  h.document.visibilityState = 'visible';
  h.advance(60000); await flush();
  assert.equal(h.calls.length, 1);
});

test('the 60 s read does not repaint an unchanged panel nor flash its refresh button', async () => {
  const gate = deferred();
  let first = true;
  const h = harness(() => { if (first) { first = false; return Promise.resolve({ codex: quota(40), local: local() }); } return gate.promise; });
  h.nav.querySelectorAll('button')[1].events.click(); await flush();
  const grid = h.section.querySelector('.usage-quota-grid'), table = h.section.querySelector('table');
  h.advance(60000);
  assert.equal(h.calls.length, 2);
  assert.ok(!h.refresh().disabled);
  assert.notEqual(h.section.attrs['aria-busy'], 'true');
  gate.resolve({ codex: quota(40), local: local() }); await flush();
  assert.equal(h.section.querySelector('.usage-quota-grid'), grid);
  assert.equal(h.section.querySelector('table'), table);
  h.advance(60000); await flush();
});

test('a 60 s read that returns after a provider change cannot restore the previous account', async () => {
  const gate = deferred();
  const h = harness(() => gate.promise);
  h.advance(60000);
  h.panels.panel.onState({ provider: { backend: 'codex', connected: false }, usage: { codex: { status: 'signin' } } });
  h.panels.panel.onState({ provider: { backend: 'codex', connected: true, email: 'autre@example.com' }, usage: { codex: { status: 'signin' } } });
  gate.resolve({ codex: quota(95), local: local() }); await flush();
  assert.equal(h.warning.hidden, true);
  assert.equal(h.section.querySelectorAll('progress').length, 0);
});

test('« Actualiser » pressed during a load is queued, once, instead of ignored', async () => {
  const gates = [];
  const h = harness(() => { const d = deferred(); gates.push(d); return d.promise; });
  h.nav.querySelectorAll('button')[1].events.click();
  assert.equal(h.calls.length, 1);
  assert.ok(!h.refresh().disabled);
  h.refresh().events.click(); h.refresh().events.click();
  assert.equal(h.calls.length, 1);
  gates[0].resolve({ codex: quota(10), local: local() }); await flush();
  assert.deepEqual(h.calls.map(c => c[0]), ['/api/usage', '/api/usage/refresh']);
  assert.equal(JSON.stringify(h.calls[1][1]), '{}');
  gates[1].resolve({ codex: quota(30), local: local() }); await flush();
  assert.equal(h.calls.length, 2);
  assert.match(h.section.textContent, /70 % restants/);
});

test('an account change with Usage open reloads the history; a change while closed does not', async () => {
  const h = harness();
  const account = (backend, connected, email) => ({ provider: { backend, connected, email }, usage: { codex: quota(20) } });
  h.panels.panel.onState(account('codex', true, 'a@example.com'));
  h.dialog.showModal();
  h.nav.querySelectorAll('button')[1].events.click(); await flush();
  assert.equal(h.calls.length, 1);
  h.panels.panel.onState(account('codex', true, 'a@example.com'));
  assert.equal(h.calls.length, 1);
  for (const next of [account('codex', true, 'b@example.com'), account('codex', false, ''), account('grok', true, 'b@example.com')]) {
    const before = h.calls.length;
    h.panels.panel.onState(next); await flush();
    assert.equal(h.calls.length, before + 1);
    assert.equal(h.calls.at(-1)[0], '/api/usage');
    assert.match(h.section.textContent, /1.500 tokens/);
  }
  h.nav.querySelectorAll('button')[0].events.click();
  const closed = h.calls.length;
  h.panels.panel.onState(account('codex', true, 'c@example.com')); await flush();
  assert.equal(h.calls.length, closed);
});

test('an account change during a load discards that read and reloads once after it', async () => {
  const gates = [];
  const h = harness(() => { const d = deferred(); gates.push(d); return d.promise; });
  h.panels.panel.onState({ provider: { backend: 'codex', connected: true, email: 'a@example.com' }, usage: { codex: quota(20) } });
  h.dialog.showModal();
  h.nav.querySelectorAll('button')[1].events.click();
  h.panels.panel.onState({ provider: { backend: 'codex', connected: true, email: 'b@example.com' }, usage: { codex: quota(20) } });
  assert.equal(h.calls.length, 1);
  gates[0].resolve({ codex: quota(95), local: local() }); await flush();
  assert.equal(h.section.querySelectorAll('table').length, 0);
  assert.equal(h.calls.length, 2);
  gates[1].resolve({ codex: quota(20), local: local() }); await flush();
  assert.equal(h.section.querySelectorAll('table').length, 1);
  assert.equal(h.warning.hidden, true);
});

test('a background « refreshing » flag the panel does not display leaves an unavailable quota untouched', () => {
  const h = harness();
  h.panels.panel.onState(state({ status: 'unavailable', message: 'Indisponible.', refreshing: true }));
  const notice = h.section.querySelector('.usage-empty');
  h.panels.panel.onState(state({ status: 'unavailable', message: 'Indisponible.', refreshing: false }));
  assert.equal(h.section.querySelector('.usage-empty'), notice);
});
