// Run with: node --test app/ui/panels/routines.test.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

class Element {
  constructor(tag) { this.tagName = tag.toUpperCase(); this.children = []; this.dataset = {}; this.scrollTop = 0; this.open = true; this.isConnected = true; this.classList = { toggle() {}, add() {}, remove() {} }; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute() {}
  addEventListener() {}
  querySelectorAll(selector) {
    const tags = selector.split(',').map(s => s.trim().toUpperCase());
    return this.children.filter(c => c instanceof Element).flatMap(c => [...(tags.includes(c.tagName) ? [c] : []), ...c.querySelectorAll(selector)]);
  }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map(c => c?.textContent ?? String(c)).join(''); }
}
const routine = { routine_id: 'routine-test', bot_id: 'grokbot', conversation_id: 'thread-test', name: 'Bilan', enabled: false, trigger: { kind: 'every', interval_ms: 60000 }, time_zone: 'UTC' };
const occurrence = (id, status = 'done', nonce = 'other-client') => ({ occurrence_id: id, manual_nonce: nonce, scheduled_at_utc: 100, schedule_version: 1, status: { status }, conversation_id: routine.conversation_id, message_id: `reply-${id}` });
const page = (items, extra = {}) => ({ occurrences: items, missed_ranges: [], running: false, cursor: '10.0', ranges_cursor: '0.0', next_cursor: null, next_ranges_cursor: null, ...extra });
function harness(api) {
  let clock = 10000, sequence = 0;
  const storage = new Map();
  const panels = { state: { bots: [{ id: 'grokbot', name: 'Ezerbot' }], routines: { items: [{ routine }] } }, api, refresh: async () => {}, notify() {}, register(panel) { this.panel = panel; } };
  const window = { GrokbotPanels: panels };
  const context = vm.createContext({ window, document: { createElement: tag => new Element(tag), createTextNode: text => ({ textContent: text }), getElementById: () => null }, Intl, Date: class extends Date { static now() { return clock; } }, crypto: { randomUUID: () => `intent-${++sequence}` }, sessionStorage: { getItem: k => storage.get(k), setItem: (k,v) => storage.set(k,v) }, console, TextEncoder });
  // Expose lexical state only in this test VM; production has no test API.
  const source = fs.readFileSync(`${__dirname}/routines.js`, 'utf8').replace('  panels.register({', `
  window.test = { runNow, showHistory, pollHistory, edit, mutate, back, settleLaunch, launches, histories,
    setup(r) { dialog = document.createElement('dialog'); botSelect = document.createElement('select'); botSelect.value = r.bot_id;
      content = document.createElement('div'); status = document.createElement('p'); toolbar = document.createElement('div');
      view = 'list'; current = null; revision = 1; historyView = null; },
    prepareResume(r) { view = 'edit'; current = r; },
    get status() { return status; }, get content() { return content; }, get history() { return historyView; }
  };
  panels.register({`);
  vm.runInContext(source, context);
  window.test.setup(routine);
  return { t: window.test, panels, advance: () => { clock += 4000; }, storage };
}

test('concurrent clicks coalesce; an aliased run does not swallow the next intentional launch', async () => {
  let launches = 0, nonce, resolveRead;
  let hold = true;
  const h = harness(async (url, payload) => {
    if (url === '/api/routines/run-now') { launches++; nonce = payload.nonce; return { occurrence: occurrence(`run-${launches}`) }; }
    if (hold) { hold = false; await new Promise(resolve => { resolveRead = resolve; }); }
    return page(launches ? [occurrence(`run-${launches}`)] : []);
  });
  const first = h.t.runNow(routine);
  await h.t.runNow(routine); // busy is set before the history read
  resolveRead(); await first;
  assert.equal(launches, 1);
  assert.equal(h.t.launches.get(routine.routine_id).occurrenceId, 'run-1');
  assert.equal(h.t.histories.get(routine.routine_id).own.nonce, nonce);
  h.advance(); h.t.settleLaunch(routine);
  assert.equal(h.t.launches.size, 0);
  await h.t.runNow(routine);
  assert.equal(launches, 2);
  assert.equal(nonce, 'intent-2');
});

test('history recovers from an error without refresh, preserves page order and deduplicates', async () => {
  let fail = true, newer = false;
  const h = harness(async url => {
    if (fail) throw new Error('Le service est temporairement indisponible.');
    if (url.includes('cursor=10.1')) return page([occurrence('older')], { cursor: '10.1' });
    if (url.includes('cursor=10.0')) return page([occurrence('first')], { next_cursor: '10.1' });
    return page([occurrence(newer ? 'new-arrival' : 'first')], { next_cursor: '10.1' });
  });
  await h.t.showHistory(routine);
  assert.match(h.t.status.textContent, /Historique indisponible/);
  assert.match(h.t.status.textContent, /Réessayer/);
  fail = false; h.advance(); await h.t.pollHistory();
  assert.equal(h.t.status.textContent, '');
  assert.equal(h.t.history.occurrences.size, 1);
  await h.t.showHistory(routine, 'occurrences');
  newer = true; h.advance(); await h.t.pollHistory();
  assert.equal(h.t.history.newer, true);
  assert.deepEqual([...h.t.history.occurrences.keys()], ['first', 'older']);
  assert.equal(h.t.content.querySelectorAll('article').length, 2);
  assert.match(h.t.content.textContent, /Voir le résultat de cette exécution/);
});

test('a stale history response cannot replace the newly opened routine', async () => {
  let resolveOld;
  const h = harness(url => url.includes('routine_id=routine-test') ? new Promise(resolve => { resolveOld = resolve; }) : Promise.resolve(page([occurrence('current')])));
  const old = h.t.showHistory(routine);
  h.t.back();
  await h.t.showHistory({ ...routine, routine_id: 'routine-new', name: 'Nouvelle' });
  resolveOld(page([occurrence('stale')])); await old;
  assert.deepEqual([...h.t.history.occurrences.keys()], ['current']);
  assert.match(h.t.content.textContent, /Nouvelle/);
});

test('a lost launch reply persists the intent and a retry confirms without running twice', async () => {
  let calls = 0, nonce;
  const h = harness(async (url, payload) => {
    if (url === '/api/routines/run-now') { calls++; nonce = payload.nonce; throw new Error('La connexion est interrompue.'); }
    return page(calls ? [occurrence('accepted-before-disconnect', 'done', nonce)] : []);
  });
  await h.t.runNow(routine);
  assert.equal(h.t.launches.get(routine.routine_id).unknown, true);
  assert.match(h.t.status.textContent, /Réessayer/);
  assert.match(h.storage.get('ezerbot.routine-launches'), /intent-1/);
  h.advance(); await h.t.runNow(routine);
  assert.equal(calls, 1);
  assert.equal(h.t.launches.get(routine.routine_id).unknown, false);
  assert.match(h.t.status.textContent, /confirmé dans l’historique/);
});

test('activation errors keep a French retry action and release the panel lock', async () => {
  let calls = 0;
  const h = harness(async () => { calls++; throw new Error('storage failure'); });
  await h.t.mutate('/api/routines/resume', {}, 'Routine activée.');
  assert.match(h.t.status.textContent, /Action refusée par le moteur/);
  assert.match(h.t.status.textContent, /Réessayer/);
  await h.t.mutate('/api/routines/resume', {}, 'Routine activée.');
  assert.equal(calls, 2);
});

test('a lost activation reply is reconciled from server state without manual refresh', async () => {
  const reviewed = { ...routine, modified_at: 10, schedule_version: 1 };
  const h = harness(async url => {
    if (url.startsWith('/api/routines?')) return { routines: [reviewed] };
    throw new Error('La connexion au moteur local est interrompue.');
  });
  // Opening an editor is not needed here: set its identity in the test VM.
  h.t.prepareResume(reviewed);
  await h.t.mutate('/api/routines/resume', {}, 'Routine activée.', reviewed);
  assert.match(h.t.status.textContent, /Action non confirmée/);
  h.panels.state.routines.items = [{ routine: { ...reviewed, enabled: true } }];
  h.panels.panel.onState();
  assert.match(h.t.status.textContent, /État récupéré automatiquement/);
  assert.match(h.t.content.textContent, /Mettre en pause/);
});

test('confirmation read errors are French and actionable', async () => {
  const h = harness(async () => { throw new Error('database busy'); });
  await h.t.edit(routine, true);
  assert.match(h.t.status.textContent, /Confirmation indisponible/);
  assert.match(h.t.status.textContent, /Réessayer/);
  assert.doesNotMatch(h.t.status.textContent, /database busy/);
});
