// Run with: node --test app/ui/panels/routines.test.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

class Element {
  constructor(tag, document) { this.ownerDocument = document; this.tagName = tag.toUpperCase(); this.children = []; this.dataset = {}; this.attributes = {}; this.listeners = {}; this.scrollTop = 0; this.open = true; this.isConnected = true; this.classList = { toggle() {}, add() {}, remove() {} }; }
  append(...children) { for (const child of children) if (child instanceof Element) child.parentElement = this; this.children.push(...children); }
  get firstChild() { return this.children[0]; }
  contains(value) { return !!value && (this === value || this.children.some(c => c instanceof Element && c.contains(value))); }
  replaceChildren(...children) { if (this.contains(this.ownerDocument.activeElement)) this.ownerDocument.activeElement = this.ownerDocument.body; this.children = children; }
  setAttribute(key, value) { this.attributes[key] = value; }
  getAttribute(key) { return this.attributes[key]; }
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { if (!this.disabled) return this.listeners.click?.(); }
  focus() { if (!this.disabled) this.ownerDocument.activeElement = this; }
  scrollIntoView() { this.scrollCalls = (this.scrollCalls || 0) + 1; }
  querySelectorAll(selector) {
    const selectors = selector.split(',').map(s => s.trim());
    const matches = el => selectors.some(s => {
      if (s.startsWith('.')) return el.className?.split(' ').includes(s.slice(1));
      const data = /^\[data-([a-z-]+)\]$/.exec(s);
      if (data) return data[1].replace(/-([a-z])/g, (_, c) => c.toUpperCase()) in el.dataset;
      return el.tagName === s.toUpperCase();
    });
    return this.children.filter(c => c instanceof Element).flatMap(c => [...(matches(c) ? [c] : []), ...c.querySelectorAll(selector)]);
  }
  set textContent(text) { this.replaceChildren(String(text)); }
  get textContent() { return this.children.map(c => c?.textContent ?? String(c)).join(''); }
}
const routine = { routine_id: 'routine-test', bot_id: 'grokbot', conversation_id: 'thread-test', name: 'Bilan', enabled: false, trigger: { kind: 'every', interval_ms: 60000 }, time_zone: 'UTC' };
const occurrence = (id, status = 'done', nonce = 'other-client') => ({ occurrence_id: id, manual_nonce: nonce, scheduled_at_utc: 100, schedule_version: 1, status: { status }, conversation_id: routine.conversation_id, message_id: `reply-${id}` });
const page = (items, extra = {}) => ({ occurrences: items, missed_ranges: [], running: false, cursor: '10.0', ranges_cursor: '0.0', next_cursor: null, next_ranges_cursor: null, ...extra });
function harness(api) {
  let clock = 10000, sequence = 0;
  const storage = new Map();
  const panels = { connected: true, state: { bots: [{ id: 'grokbot', name: 'Ezerbot' }], routines: { items: [{ routine }] } }, api, refresh: async () => {}, notify() {}, register(panel) { this.panel = panel; } };
  const window = { GrokbotPanels: panels };
  const document = { createElement: tag => new Element(tag, document), createTextNode: text => ({ textContent: text }), getElementById: () => null };
  document.body = document.createElement('body'); document.activeElement = document.body;
  const context = vm.createContext({ window, document, Intl, Date: class extends Date { static now() { return clock; } }, crypto: { randomUUID: () => `intent-${++sequence}` }, sessionStorage: { getItem: k => storage.get(k), setItem: (k,v) => storage.set(k,v) }, console, TextEncoder });
  // Expose lexical state only in this test VM; production has no test API.
  const source = fs.readFileSync(`${__dirname}/routines.js`, 'utf8').replace('  panels.register({', `
  window.test = { renderList, loadFunnel, revealed, runNow, showHistory, pollHistory, retryLoadedPages, edit, mutate, back, settleLaunch, launches, histories, frenchError,
    setup(r) { dialog = document.createElement('dialog'); botSelect = document.createElement('select'); botSelect.value = r.bot_id;
      content = document.createElement('div'); status = document.createElement('p'); toolbar = document.createElement('div'); notices = document.createElement('div');
      dialog.append(notices, status, content); document.body.append(dialog);
      view = 'list'; current = null; revision = 1; historyView = null; },
    prepareResume(r) { view = 'edit'; current = r; },
    get status() { return status; }, get content() { return content; }, get history() { return historyView; }, get dialog() { return dialog; }, get notices() { return notices; }
  };
  panels.register({`);
  vm.runInContext(source, context);
  window.test.setup(routine);
  return { t: window.test, panels, document, advance: () => { clock += 4000; }, storage };
}

test('page and polling failures retain membership, cursor, position and an inline retry', async () => {
  let failed = false;
  const reads = [];
  const h = harness(async url => {
    reads.push(url);
    if (failed) throw new Error('Le service est indisponible.');
    if (url.includes('&cursor=10.2')) return page([occurrence('fifth'), occurrence('sixth')], { cursor: '10.2' });
    if (url.includes('&cursor=10.1')) return page([occurrence('third'), occurrence('fourth')], { cursor: '10.1', next_cursor: '10.2' });
    return page([occurrence('first'), occurrence('second')], { next_cursor: '10.1' });
  });
  await h.t.showHistory(routine); await h.t.showHistory(routine, 'occurrences');
  h.t.dialog.scrollTop = 7410;
  failed = true; await h.t.showHistory(routine, 'occurrences');
  assert.equal(h.t.history.occurrences.size, 4);
  assert.equal(h.t.dialog.scrollTop, 7410);
  assert.match(h.t.content.textContent, /Impossible de charger la suite/);
  h.advance(); await h.t.pollHistory();
  assert.equal(h.t.dialog.scrollTop, 7410);
  assert.match(h.t.content.textContent, /Impossible de charger la suite/);
  assert.equal(h.t.status.scrollCalls || 0, 0);
  assert.equal(h.t.notices.textContent, ''); // polling must not replace the explicit page error
  failed = false; reads.length = 0; await h.t.retryLoadedPages(routine);
  assert.equal(h.t.history.occurrences.size, 4);
  assert.equal(h.t.history.pages.size, 2);
  assert.equal(h.t.history.next, '10.2');
  assert.equal(h.t.dialog.scrollTop, 7410);
  assert.match(h.t.content.textContent, /Impossible de charger la suite/);
  assert.ok(reads.some(url => url.includes('&cursor=10.1')));
  const retry = h.t.content.querySelectorAll('button').find(b => b.dataset.historyFocus === 'retry-occurrences');
  retry.focus(); await retry.click();
  assert.equal(h.t.history.occurrences.size, 6);
  assert.equal(h.t.dialog.scrollTop, 7410);
  assert.equal(h.document.activeElement.dataset.historyFocus, 'occurrence-fifth');
  assert.doesNotMatch(h.t.content.textContent, /Impossible de charger la suite/);
});

test('loading a page preempts polling, shows progress and restores keyboard focus', async () => {
  let resolvePoll, resolveMore, holdPoll = false;
  const h = harness(async url => {
    if (url.includes('&cursor=10.1')) return new Promise(resolve => { resolveMore = resolve; });
    if (holdPoll) return new Promise(resolve => { resolvePoll = resolve; });
    return page([occurrence('first')], { next_cursor: '10.1' });
  });
  await h.t.showHistory(routine); h.t.dialog.scrollTop = 300;
  holdPoll = true; h.advance(); const polling = h.t.pollHistory();
  const load = h.t.content.querySelectorAll('button').find(b => b.dataset.loadMore === 'occurrences');
  load.focus(); const loading = h.t.showHistory(routine, 'occurrences');
  assert.equal(load.disabled, true); assert.equal(load.textContent, 'Chargement…');
  await h.t.showHistory(routine, 'occurrences'); // dedicated double-click guard
  resolveMore(page([occurrence('second')], { cursor: '10.1' })); await loading;
  resolvePoll(page([occurrence('stale')])); await polling;
  assert.deepEqual([...h.t.history.occurrences.keys()], ['first', 'second']);
  assert.equal(h.document.activeElement.dataset.historyFocus, 'occurrence-second');
  assert.equal(h.t.dialog.scrollTop, 300);
});

test('an offline notice is visible inside the modal, actionable and cleared on recovery', async () => {
  const h = harness(async () => page([occurrence('first')]));
  await h.t.showHistory(routine); h.t.dialog.scrollTop = 600;
  h.panels.panel.onConnection(false);
  assert.match(h.t.notices.textContent, /Moteur local injoignable/);
  assert.equal(h.t.notices.children[0].getAttribute('role'), 'alert');
  assert.match(h.t.notices.textContent, /Réessayer/);
  assert.equal(h.t.dialog.scrollTop, 600);
  h.panels.panel.onConnection(true);
  assert.equal(h.t.notices.textContent, '');
  assert.equal(h.t.history.occurrences.size, 1);
  assert.equal(h.t.dialog.scrollTop, 600);
});

test('fresh arrivals update a single page at the top and stay announced during a longer reading', async () => {
  let latest = 'first';
  const h = harness(async url => url.includes('&cursor=') ? page([occurrence('first')]) : page([occurrence(latest)]));
  await h.t.showHistory(routine); latest = 'second'; h.advance(); await h.t.pollHistory();
  assert.deepEqual([...h.t.history.occurrences.keys()], ['second']);
  assert.match(h.t.content.textContent, /1 occurrence chargée/);
  assert.doesNotMatch(h.t.content.textContent, /1 occurrences/);
  h.t.dialog.scrollTop = 500; latest = 'third'; h.advance(); await h.t.pollHistory();
  assert.equal(h.t.history.newer, true);
  assert.match(h.t.notices.textContent, /Voir les plus récentes/);
});

test('a missed-range retry keeps its cursor, expanded section and focus', async () => {
  let fail = false;
  const range = n => ({ schedule_version: 1, from_inclusive: n, until_exclusive: n + 1, trigger: routine.trigger, time_zone: 'UTC' });
  const h = harness(async url => {
    if (url.includes('&ranges_cursor=20.1')) {
      if (fail) throw new Error('Le service est indisponible.');
      return page([], { missed_ranges: [range(2)], cursor: '10.0', ranges_cursor: '20.1' });
    }
    return page([], { missed_ranges: [range(1)], ranges_cursor: '20.0', next_ranges_cursor: '20.1' });
  });
  await h.t.showHistory(routine); h.t.history.rangesOpen = true; h.t.dialog.scrollTop = 500;
  fail = true; await h.t.showHistory(routine, 'ranges');
  assert.equal(h.t.history.ranges.size, 1);
  assert.equal(h.t.history.nextRanges, '20.1');
  assert.match(h.t.content.textContent, /Impossible de charger la suite/);
  fail = false;
  const retry = h.t.content.querySelectorAll('button').find(b => b.dataset.historyFocus === 'retry-ranges');
  retry.focus(); await retry.click();
  assert.equal(h.t.history.ranges.size, 2);
  assert.equal(h.t.history.rangesOpen, true);
  assert.equal(h.t.dialog.scrollTop, 500);
  assert.equal(h.document.activeElement.dataset.historyFocus, 'range-1.2');
});

test('diagnostics identify invalid schedules and zones without generic action wording', () => {
  const h = harness(async () => page([]));
  assert.match(h.t.frenchError({ message: 'invalid schedule: failed to find timezone X' }, true), /fuseau horaire.*invalide/);
  assert.match(h.t.frenchError({ message: 'schedule never fires after its reference' }, true), /créneau futur/);
  assert.match(h.t.frenchError({ message: 'unknown routine' }, true), /introuvable/);
  assert.match(h.t.frenchError({ message: 'new diagnostic' }, true), /Diagnostic technique non traduit/);
});

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
  assert.match(h.t.status.textContent, /Vérifier \/ réessayer le lancement/);
  assert.equal(h.t.status.querySelectorAll('button').length, 0);
  assert.match(h.storage.get('ezerbot.routine-launches'), /intent-1/);
  h.advance(); await h.t.runNow(routine);
  assert.equal(calls, 1);
  assert.equal(h.t.launches.get(routine.routine_id).unknown, false);
  assert.match(h.t.status.textContent, /confirmé dans l’historique/);
});

test('a launch preflight error points to the action that actually exists', async () => {
  const h = harness(async () => { throw new Error('Le service est indisponible.'); });
  await h.t.runNow(routine);
  assert.match(h.t.status.textContent, /Lancement non envoyé.*Lancer maintenant/);
  assert.doesNotMatch(h.t.status.textContent, /Vérifier \/ réessayer/);
  assert.match(h.t.content.textContent, /Lancer maintenant/);
});

test('a confirmed launch with an unreadable history does not suggest a second launch', async () => {
  let accepted = false;
  const h = harness(async url => {
    if (url === '/api/routines/run-now') { accepted = true; return { occurrence: occurrence('accepted') }; }
    if (accepted) throw new Error('Le service est indisponible.');
    return page([]);
  });
  await h.t.runNow(routine);
  assert.match(h.t.status.textContent, /Lancement enregistré.*Historique/);
  assert.doesNotMatch(h.t.status.textContent, /non envoyé|Vérifier \/ réessayer/);
  h.advance(); await h.t.pollHistory();
  assert.match(h.t.status.textContent, /Lancement enregistré.*Historique/);
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

test('a preflight failure is not reported as an uncertain activation', async () => {
  const reviewed = { ...routine, modified_at: 10, schedule_version: 1 };
  const calls = [];
  const h = harness(async url => { calls.push(url); throw new Error('La connexion au moteur local est interrompue.'); });
  h.t.prepareResume(reviewed);
  await h.t.mutate('/api/routines/resume', {}, 'Routine activée.', reviewed);
  assert.deepEqual(calls, ['/api/routines?bot_id=grokbot']);
  assert.match(h.t.status.textContent, /Activation non envoyée/);
  assert.doesNotMatch(h.t.status.textContent, /sera vérifié|saisies/);
  assert.match(h.t.status.textContent, /Réessayer/);
});

test('a submitted activation that did not persist is reconciled explicitly as still paused', async () => {
  const reviewed = { ...routine, modified_at: 10, schedule_version: 1 };
  const h = harness(async url => {
    if (url.startsWith('/api/routines?')) return { routines: [reviewed] };
    throw new Error('La connexion au moteur local est interrompue.');
  });
  h.t.prepareResume(reviewed);
  await h.t.mutate('/api/routines/resume', {}, 'Routine activée.', reviewed);
  assert.match(h.t.status.textContent, /Action non confirmée/);
  h.panels.panel.onState();
  assert.match(h.t.status.textContent, /n’a pas été enregistrée.*toujours en pause/);
  assert.match(h.t.status.textContent, /Relire la routine/);
  assert.doesNotMatch(h.t.status.textContent, /sera vérifié|saisies/);
});

test('activation conflicts offer one direct re-reading action without editor advice', async () => {
  const reviewed = { ...routine, modified_at: 10, schedule_version: 1 };
  const h = harness(async () => ({ routines: [{ ...reviewed, modified_at: 11 }] }));
  h.t.prepareResume(reviewed);
  await h.t.mutate('/api/routines/resume', {}, 'Routine activée.', reviewed);
  assert.match(h.t.status.textContent, /Conflit d’activation/);
  assert.match(h.t.status.textContent, /Relire la routine/);
  assert.doesNotMatch(h.t.status.textContent, /saisies|Revenez|éditeur|Réessayer/);
});

test('confirmation read errors are French and actionable', async () => {
  const h = harness(async () => { throw new Error('database busy'); });
  await h.t.edit(routine, true);
  assert.match(h.t.status.textContent, /Confirmation indisponible/);
  assert.match(h.t.status.textContent, /Réessayer/);
  assert.doesNotMatch(h.t.status.textContent, /database busy/);
});

test('the list shows how many catch-up occurrences wait and when the next one is planned', () => {
  const h = harness(async () => ({}));
  const queued = (queue) => { h.panels.state.routines.items = [{ routine, queue }]; h.t.renderList(); return h.t.content.querySelectorAll('[data-queue]'); };
  assert.equal(queued({ waiting: 0, next_at: null }).length, 0);
  const [one] = queued({ waiting: 1, next_at: Date.UTC(2026, 9, 22, 14, 0, 0) });
  assert.match(one.textContent, /^1 occurrence en attente · prochaine prévue /);
  assert.match(one.textContent, /14:00:00/);
  const [many] = queued({ waiting: 3, next_at: Date.UTC(2026, 9, 22, 15, 0, 0) });
  assert.match(many.textContent, /^3 occurrences en attente/);
  assert.equal(many.attributes.role, 'status');
});

test('the panel states the catch-up rule: the queue after an outage, the sixty-minute window otherwise, and that a pause is not caught up', () => {
  const source = fs.readFileSync(`${__dirname}/routines.js`, 'utf8');
  const note = /"(Les routines horaires partent quand[^"]*)"/.exec(source)[1];
  assert.equal(note, 'Les routines horaires partent quand le moteur local est ouvert, ou quand l’agent en arrière-plan est activé. Si le Mac était éteint ou en veille, ou si l’agent activé n’a pas pu tourner, chaque créneau dû est rattrapé au prochain passage, dans l’ordre. Si le Mac était allumé et que ni l’app ni l’agent ne tournait, un seul créneau des 60 dernières minutes est rattrapé ; les autres sont marqués manqués. Une pause ne se rattrape pas.');
});

const hook = { ...routine, name: 'Déploiement', trigger: { kind: 'webhook' }, enabled: true, body: { kind: 'inline_prompt', prompt: 'Traite l’appel reçu.' } };
function webhookHarness(funnel, extra = {}) {
  const h = harness(async url => {
    if (url === '/api/routines/funnel') return funnel;
    if (extra.api) return extra.api(url);
    throw new Error(`inattendu : ${url}`);
  });
  h.panels.state.routines.items = [{ routine: hook }];
  h.panels.state.routine_webhooks = { listener: { port: 47864, listening: true, error: null }, funnel: {}, items: { [hook.routine_id]: { trigger_id: 'trigger-abc', url: null } } };
  return h;
}

test('a published webhook shows its HTTPS Funnel URL and the Bearer reminder, with the token masked', async () => {
  const h = webhookHarness({ state: 'published', reason: null, public_base: 'https://mac.qa.ts.net:8443/hooks' });
  await h.t.loadFunnel();
  const text = h.t.content.textContent;
  assert.match(text, /https:\/\/mac\.qa\.ts\.net:8443\/hooks\/routine-test\/trigger-abc/);
  assert.match(text, /Authorization: Bearer <jeton>/);
  assert.match(text, /Jeton Bearer : ••••/);
  assert.match(text, /Webhook : démarre à chaque appel|Webhook/);
  assert.doesNotMatch(text, /ezwh_/);
  assert.match(text, /Retirer la publication/);
});

test('without a confirmed Funnel the panel gives the reason and no public URL', async () => {
  for (const [state, reason] of [['tailscale_stopped', 'Tailscale est arrêté ou déconnecté du tailnet.'], ['refused', 'Le tailnet a refusé Funnel.'], ['unconfirmed', 'La publication Funnel n’est pas encore confirmée par Tailscale.']]) {
    const h = webhookHarness({ state, reason, public_base: null });
    await h.t.loadFunnel();
    const text = h.t.content.textContent;
    assert.match(text, new RegExp(reason.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
    assert.match(text, /Pas d’URL publique/);
    assert.doesNotMatch(text, /https:\/\//);
  }
});

test('a stale published poll never shows an URL until the live check confirms it', async () => {
  const h = webhookHarness({ state: 'tailscale_stopped', reason: 'Tailscale est arrêté ou déconnecté du tailnet.', public_base: null });
  h.panels.state.routine_webhooks.items[hook.routine_id].url = 'https://old.ts.net:8443/hooks/x/y';
  h.panels.state.routine_webhooks.funnel = { state: 'published', public_base: 'https://old.ts.net:8443/hooks' };
  h.t.renderList();
  assert.doesNotMatch(h.t.content.textContent, /old\.ts\.net/);
  await h.t.loadFunnel();
  assert.doesNotMatch(h.t.content.textContent, /old\.ts\.net/);
});

test('the token is shown in clear after creation, then masked and still copyable', async () => {
  const copied = [];
  const h = webhookHarness({ state: 'off', reason: 'Le webhook n’est pas publié sur Internet.', public_base: null }, {
    api: async url => {
      if (url === '/api/routines') return { routine: hook, webhook: { trigger_id: 'trigger-abc', token: 'ezwh_secret_token' } };
      if (url === '/api/routines/webhook/token') return { token: 'ezwh_secret_token' };
      throw new Error(`inattendu : ${url}`);
    },
  });
  h.panels.copyText = async text => { copied.push(text); };
  await h.t.mutate('/api/routines', { bot_id: 'grokbot', name: 'Déploiement', trigger: { kind: 'webhook' } }, 'Routine créée.');
  assert.match(h.t.content.textContent, /ezwh_secret_token/);
  assert.match(h.t.content.textContent, /Copiez ce jeton maintenant/);
  h.t.revealed.clear(); h.t.renderList();
  assert.doesNotMatch(h.t.content.textContent, /ezwh_/);
  assert.match(h.t.content.textContent, /Jeton Bearer : ••••/);
  const copy = h.t.content.querySelectorAll('button').find(b => b.textContent === 'Copier le jeton');
  await copy.click();
  assert.deepEqual(copied, ['ezwh_secret_token']);
});

test('history names the Webhook trigger and its reception time', async () => {
  const received = { ...occurrence('hook-run', 'done', 'webhook:trigger-abc:call:1'), trigger: { kind: 'webhook', label: 'Webhook', received_at: 1_760_000_000_000 } };
  const h = webhookHarness({ state: 'off', reason: '', public_base: null }, { api: async () => page([received]) });
  await h.t.showHistory(hook);
  assert.match(h.t.content.textContent, /Webhook · reçu le/);
  assert.doesNotMatch(h.t.content.textContent, /Manuel|Planifié/);
});

test('a routine editor offers a Webhook trigger without schedule fields', async () => {
  const h = webhookHarness({ state: 'off', reason: '', public_base: null }, { api: async () => ({ routines: [hook] }) });
  await h.t.edit(hook);
  const select = h.t.content.querySelectorAll('select').find(el => el.children.some(o => o.value === 'webhook'));
  assert.ok(select, 'the trigger selector lists webhook');
  assert.equal(select.value, 'webhook');
});

test('a withdrawal Tailscale refused keeps offering the retry and shows no public URL', async () => {
  const h = webhookHarness({ state: 'withdraw_failed', reason: 'Tailscale n’a pas pu retirer la publication Funnel : l’adresse publique peut rester joignable. Réessayez de la retirer.', public_base: null });
  await h.t.loadFunnel();
  assert.match(h.t.content.textContent, /peut rester joignable/);
  assert.match(h.t.content.textContent, /Retirer la publication/);
  assert.doesNotMatch(h.t.content.textContent, /Publier avec Tailscale Funnel/);
  assert.doesNotMatch(h.t.content.textContent, /https:\/\//);
});

test('a publication stays withdrawable after the last webhook routine is gone', async () => {
  const h = webhookHarness({ state: 'published', reason: null, public_base: 'https://mac.qa.ts.net:8443/hooks' });
  h.panels.state.routines.items = [{ routine }];
  h.panels.state.routine_webhooks = { listener: { port: 47864, listening: true, error: null }, funnel: { enabled: true, state: 'published' }, items: {} };
  await h.t.loadFunnel();
  assert.match(h.t.content.textContent, /Publication sur Internet/);
  assert.match(h.t.content.textContent, /Retirer la publication/);
  h.panels.state.routine_webhooks.funnel.enabled = false;
  h.t.renderList();
  assert.doesNotMatch(h.t.content.textContent, /Publication sur Internet/);
});

test('an enabled publication Tailscale cannot confirm stays removable, even without any webhook routine', async () => {
  const h = webhookHarness({ state: 'unconfirmed', enabled: true, reason: 'La publication Funnel n’est pas encore confirmée par Tailscale.', public_base: null });
  h.panels.state.routines.items = [{ routine }];
  h.panels.state.routine_webhooks = { listener: { port: 47864, listening: true, error: null }, funnel: { enabled: true, state: 'unconfirmed' }, items: {} };
  await h.t.loadFunnel();
  assert.match(h.t.content.textContent, /Retirer la publication/);
  assert.doesNotMatch(h.t.content.textContent, /Publier avec Tailscale Funnel/);
  assert.doesNotMatch(h.t.content.textContent, /https:\/\//);
});

const everyMinute = { kind: 'every', interval_ms: 60000, phase_ms: null };
const groupRoutine = { ...routine, name: 'Groupe', enabled: true, body: { kind: 'inline_prompt', prompt: 'Rapport' }, trigger: { kind: 'group', members: [{ id: 'trigger-a', trigger: everyMinute }, { id: 'trigger-b', trigger: { kind: 'webhook' } }, { id: 'trigger-c', trigger: { kind: 'webhook' } }] } };
const labelled = (h, text) => h.t.content.querySelectorAll('.form-label').filter(l => String(l.children[0]).startsWith(text));
const inputOf = label => label.children.find(c => c.tagName === 'INPUT' || c.tagName === 'SELECT');
const type = (input, value) => { input.value = value; input.listeners.input?.(); };
function groupEditor(sent) {
  return harness(async (url, body) => {
    if (url.startsWith('/api/routines?')) return { routines: [groupRoutine] };
    sent.push({ url, body });
    return { routine: groupRoutine, webhooks: [] };
  });
}

test('a group editor lists its simple triggers and refuses a group of fewer than two', async () => {
  const sent = [], h = groupEditor(sent);
  await h.t.edit(groupRoutine);
  const remove = () => h.t.content.querySelectorAll('button').filter(b => b.textContent === 'Retirer ce déclencheur');
  assert.equal(remove().length, 3);
  assert.doesNotMatch(h.t.content.textContent, /Groupe : .*Groupe/);
  remove()[2].click(); remove()[1].click();
  assert.equal(remove().length, 1);
  const preview = h.t.content.querySelectorAll('.routines-preview')[0];
  assert.match(preview.textContent, /au moins deux déclencheurs/);
  h.t.content.querySelectorAll('form')[0].listeners.submit({ preventDefault() {} });
  assert.match(h.t.status.textContent, /au moins deux déclencheurs/);
  assert.equal(sent.length, 0);
});

test('saving a group keeps the IDs of the triggers it already has and sends only what changed', async () => {
  const sent = [], h = groupEditor(sent);
  await h.t.edit(groupRoutine);
  // The second "Cron" label is the first trigger of the group, after the routine's own field.
  type(inputOf(labelled(h, 'Cron (5 champs)')[1]), '@every 2m');
  h.t.content.querySelectorAll('form')[0].listeners.submit({ preventDefault() {} });
  await new Promise(resolve => setImmediate(resolve));
  const update = sent.find(s => s.url === '/api/routines/update');
  assert.deepEqual(JSON.parse(JSON.stringify(update.body.trigger)), { kind: 'group', members: [
    { id: 'trigger-a', trigger: { kind: 'every', interval_ms: 120000, phase_ms: null } },
    { id: 'trigger-b', trigger: { kind: 'webhook' } },
    { id: 'trigger-c', trigger: { kind: 'webhook' } },
  ] });
});

test('each webhook of a group has its own URL and its own token, shown once after saving', async () => {
  const h = webhookHarness({ state: 'published', reason: null, public_base: 'https://mac.qa.ts.net:8443/hooks' }, {
    api: async url => { if (url === '/api/routines/update') return { routine: groupRoutine, webhooks: [{ trigger_id: 'trigger-b', token: 'ezwh_first' }, { trigger_id: 'trigger-c', token: 'ezwh_second' }] }; throw new Error(`inattendu : ${url}`); },
  });
  h.panels.state.routines.items = [{ routine: groupRoutine }];
  h.panels.state.routine_webhooks.items = { [groupRoutine.routine_id]: { trigger_id: 'trigger-b', webhooks: [{ trigger_id: 'trigger-b' }, { trigger_id: 'trigger-c' }] } };
  await h.t.mutate('/api/routines/update', { bot_id: 'grokbot', routine_id: groupRoutine.routine_id }, 'Routine enregistrée.');
  await h.t.loadFunnel();
  const text = h.t.content.textContent;
  assert.match(text, /Webhook 1/);
  assert.match(text, /Webhook 2/);
  assert.match(text, /hooks\/routine-test\/trigger-b/);
  assert.match(text, /hooks\/routine-test\/trigger-c/);
  assert.match(text, /ezwh_first/);
  assert.match(text, /ezwh_second/);
  h.t.revealed.clear(); h.t.renderList();
  assert.doesNotMatch(h.t.content.textContent, /ezwh_/);
});

test('history names the simple trigger of a group that started each run', async () => {
  const schedule = { ...occurrence('group-schedule', 'done', null), trigger: { kind: 'schedule', label: 'Horaire', description: 'Chaque minute, à partir de l’activation', trigger_id: 'trigger-a' } };
  const call = { ...occurrence('group-hook', 'done', 'webhook:trigger-c:call:1'), trigger: { kind: 'webhook', label: 'Webhook 2', received_at: 1_760_000_000_000, trigger_id: 'trigger-c' } };
  const h = webhookHarness({ state: 'off', reason: '', public_base: null }, { api: async () => page([schedule, call]) });
  await h.t.showHistory(groupRoutine);
  assert.match(h.t.content.textContent, /Horaire \(Chaque minute, à partir de l’activation\) · /);
  assert.match(h.t.content.textContent, /Webhook 2 · reçu le/);
});

// The harness clock reads 10 000 ms: the one-shot at 5 000 ms is in the past.
const pinned = { ...groupRoutine, time_zone: 'UTC', trigger: { kind: 'group', members: [
  { id: 'trigger-a', trigger: { kind: 'cron', expression: '0 9 * * *', time_zone: 'UTC' } },
  { id: 'trigger-b', trigger: everyMinute },
  { id: 'trigger-c', trigger: { kind: 'once', at: 5000 } },
] } };
const savedTrigger = async (routineToEdit, change) => {
  const sent = [], h = harness(async (url, body) => {
    if (url.startsWith('/api/routines?')) return { routines: [routineToEdit] };
    sent.push({ url, body });
    return { routine: routineToEdit, webhooks: [] };
  });
  await h.t.edit(routineToEdit);
  change(h);
  h.t.content.querySelectorAll('form')[0].listeners.submit({ preventDefault() {} });
  await new Promise(resolve => setImmediate(resolve));
  return { sent, h };
};

test('saving a group keeps the explicit timezone of a trigger even when it equals the routine timezone', async () => {
  const { sent } = await savedTrigger(pinned, h => type(inputOf(labelled(h, 'Cron (5 champs)')[2]), '@every 2m'));
  const members = JSON.parse(JSON.stringify(sent.find(s => s.url === '/api/routines/update').body.trigger.members));
  assert.deepEqual(members[0].trigger, { kind: 'cron', expression: '0 9 * * *', time_zone: 'UTC' });
  assert.equal(members[1].trigger.interval_ms, 120000);
});

test('a one-shot of the group that already fired and was not touched does not block saving the rest', async () => {
  const { sent, h } = await savedTrigger(pinned, h => type(inputOf(labelled(h, 'Cron (5 champs)')[2]), '@every 2m'));
  assert.ok(sent.some(s => s.url === '/api/routines/update'), h.t.status.textContent);
});

test('a one-shot of the group moved into the past is still refused', async () => {
  const { sent, h } = await savedTrigger(pinned, h => type(labelled(h, 'Date et heure UTC')[3].children.find(c => c.tagName === 'INPUT'), '1970-01-01T00:00:01.000'));
  assert.equal(sent.length, 0);
  assert.match(h.t.status.textContent, /instant futur/);
});

test('reviewing a group before its activation offers no way to change its triggers', async () => {
  const h = harness(async url => url.startsWith('/api/routines?') ? { routines: [groupRoutine] } : {});
  await h.t.edit(groupRoutine, true);
  const labels = h.t.content.querySelectorAll('button').map(b => b.textContent);
  assert.ok(labels.includes('Activer cette routine'));
  assert.ok(!labels.includes('Retirer ce déclencheur'));
  assert.ok(!labels.includes('Ajouter un déclencheur'));
});

test('missed ranges of two group members with the same version and start are both listed', async () => {
  const range = (member) => ({ routine_id: routine.routine_id, member_id: member, schedule_version: 1, from_inclusive: 1000, until_exclusive: 5000, time_zone: 'UTC', trigger: everyMinute });
  const h = harness(async () => page([], { missed_ranges: [range('trigger-a'), range('trigger-b')] }));
  await h.t.showHistory(routine);
  assert.match(h.t.content.textContent, /Plages d’occurrences manquées \(2\)/);
  assert.equal(h.t.content.querySelectorAll('.routines-warning').length, 2);
});

test('replacing a webhook of a group by an identical one is saved, not mistaken for no change', async () => {
  const two = { ...groupRoutine, trigger: { kind: 'group', members: [{ id: 'trigger-a', trigger: { kind: 'webhook' } }, { id: 'trigger-b', trigger: { kind: 'webhook' } }] } };
  const { sent, h } = await savedTrigger(two, h => {
    h.t.content.querySelectorAll('button').find(b => b.textContent === 'Retirer ce déclencheur').click();
    h.t.content.querySelectorAll('button').find(b => b.textContent === 'Ajouter un déclencheur').click();
    const kinds = h.t.content.querySelectorAll('select').filter(el => el.children.some(o => o.value === 'webhook') && !el.children.some(o => o.value === 'group'));
    type(kinds[kinds.length - 1], 'webhook');
  });
  const update = sent.find(s => s.url === '/api/routines/update');
  assert.ok(update, h.t.status.textContent);
  const members = JSON.parse(JSON.stringify(update.body.trigger.members));
  assert.deepEqual(members, [{ id: 'trigger-b', trigger: { kind: 'webhook' } }, { trigger: { kind: 'webhook' } }]);
});

// ---------------------------------------------------------------- Courriel ----

const mailRoutine = { ...routine, name: 'Courrier', enabled: true, body: { kind: 'inline_prompt', prompt: 'Résume le courriel.' }, trigger: { kind: 'mail' } };
const triggerSelect = h => h.t.content.querySelectorAll('select').find(el => el.children.some(o => o.value === 'group'));
function mailEditor(connected, sent = []) {
  const h = harness(async (url, body) => {
    if (url.startsWith('/api/routines?')) return { routines: [mailRoutine] };
    sent.push({ url, body });
    return { routine: mailRoutine };
  });
  h.panels.state.routine_mail = { connected };
  return h;
}
const choose = (h, select, value) => { select.value = value; h.t.content.querySelectorAll('form')[0].listeners.input(); };
const submit = async h => { h.t.content.querySelectorAll('form')[0].listeners.submit({ preventDefault() {} }); await new Promise(resolve => setImmediate(resolve)); };

test('the editor offers the new-mail trigger, and with Gmail connected it saves { kind: "mail" }', async () => {
  const sent = [], h = mailEditor(true, sent);
  await h.t.edit(null);
  const select = triggerSelect(h);
  assert.ok(select.children.some(o => o.value === 'mail' && /Courriel/.test(o.textContent)));
  type(labelled(h, 'Nom').map(inputOf)[0], 'Courrier');
  type(h.t.content.querySelectorAll('textarea')[0], 'Résume le courriel.');
  choose(h, select, 'mail');
  assert.match(h.t.content.querySelectorAll('.routines-preview')[0].textContent, /nouveau message de la boîte Gmail connectée/);
  await submit(h);
  const created = sent.find(s => s.url === '/api/routines');
  assert.deepEqual(JSON.parse(JSON.stringify(created.body.trigger)), { kind: 'mail' });
});

test('without a connected Gmail the editor explains it and sends nothing', async () => {
  const sent = [], h = mailEditor(false, sent);
  await h.t.edit(null);
  type(labelled(h, 'Nom').map(inputOf)[0], 'Courrier');
  type(h.t.content.querySelectorAll('textarea')[0], 'Résume le courriel.');
  choose(h, triggerSelect(h), 'mail');
  const preview = h.t.content.querySelectorAll('.routines-preview')[0];
  assert.match(preview.textContent, /Gmail n’est pas connecté/);
  assert.match(preview.textContent, /Paramètres › Connecteurs/);
  assert.match(preview.textContent, /n’est pas enregistrée avec ce déclencheur/);
  await submit(h);
  assert.match(h.t.status.textContent, /Gmail n’est pas connecté/);
  assert.equal(sent.length, 0);
});

test('a routine already armed on mail can still be renamed after Gmail is disconnected', async () => {
  const sent = [], h = mailEditor(false, sent);
  await h.t.edit(mailRoutine);
  assert.equal(triggerSelect(h).value, 'mail');
  type(labelled(h, 'Nom').map(inputOf)[0], 'Courrier du matin');
  await submit(h);
  const update = sent.find(s => s.url === '/api/routines/update');
  assert.ok(update, h.t.status.textContent);
  assert.equal(update.body.name, 'Courrier du matin');
  assert.equal(update.body.trigger, undefined);
});

test('a group can hold the new-mail trigger, and refuses a new one while Gmail is absent', async () => {
  const withMail = { ...groupRoutine, trigger: { kind: 'group', members: [{ id: 'trigger-a', trigger: everyMinute }, { id: 'trigger-b', trigger: { kind: 'mail' } }] } };
  const sent = [];
  const h = harness(async (url, body) => { if (url.startsWith('/api/routines?')) return { routines: [withMail] }; sent.push({ url, body }); return { routine: withMail }; });
  h.panels.state.routine_mail = { connected: false };
  await h.t.edit(withMail);
  const kinds = h.t.content.querySelectorAll('select').filter(el => el.children.some(o => o.value === 'mail') && !el.children.some(o => o.value === 'group'));
  assert.deepEqual(kinds.map(k => k.value), ['schedule', 'mail']);
  // The mail trigger already in the group stays valid: editing the other one is saved.
  type(inputOf(labelled(h, 'Cron (5 champs)')[1]), '@every 2m');
  await submit(h);
  const update = sent.find(s => s.url === '/api/routines/update');
  assert.ok(update, h.t.status.textContent);
  assert.deepEqual(JSON.parse(JSON.stringify(update.body.trigger.members[1])), { id: 'trigger-b', trigger: { kind: 'mail' } });
  // A mail trigger added to the group while Gmail is absent is refused.
  sent.length = 0;
  await h.t.edit(withMail);
  h.t.content.querySelectorAll('button').find(b => b.textContent === 'Ajouter un déclencheur').click();
  const added = h.t.content.querySelectorAll('select').filter(el => el.children.some(o => o.value === 'mail') && !el.children.some(o => o.value === 'group'));
  type(added[added.length - 1], 'mail');
  await submit(h);
  assert.match(h.t.status.textContent, /Gmail n’est pas connecté/);
  assert.equal(sent.length, 0);
});

test('the history names a run started by a mail with its sender and subject', async () => {
  const started = { ...occurrence('mail-run', 'done', 'mail::m1'), trigger: { kind: 'mail', label: 'Courriel', received_at: 1_760_000_000_000, expediteur: 'Alice <alice@example.com>', objet: 'Facture d’octobre' } };
  const h = harness(async () => page([started]));
  await h.t.showHistory(mailRoutine);
  const text = h.t.content.textContent;
  assert.match(text, /Courriel · Alice <alice@example.com> · Facture d’octobre · reçu le/);
  assert.doesNotMatch(text, /Manuel|Planifié/);
});

test('the list describes a mail routine', async () => {
  const h = harness(async () => ({ routines: [mailRoutine] }));
  h.panels.state.routines.items = [{ routine: mailRoutine }];
  h.t.renderList();
  assert.match(h.t.content.textContent, /Courriel : démarre à chaque nouveau message de la boîte connectée/);
});
