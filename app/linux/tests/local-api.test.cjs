const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const template = fs.readFileSync(path.join(__dirname, '../src/local-api.js'), 'utf8');
const identity = 'synthetic-test-identity';

function install(origin, originalFetch, location = `${origin}/`, frames = []) {
  class BrowserRequest extends Request {
    constructor(input, init) {
      super(typeof input === 'string' ? new URL(input, location) : input, init);
    }
  }
  const posted = [];
  const window = {URL, Headers, Request: BrowserRequest, fetch: originalFetch, location: {href: location}, frames, ipc: {postMessage: raw => posted.push(JSON.parse(raw))}};
  window.posted = posted;
  vm.runInNewContext(template.replace('__EZERBOT_PARAMETERS__', JSON.stringify([
    origin, origin.replace('http:', 'ezerbot-resource:'), identity,
  ])), {window, document: {baseURI: location}});
  return window;
}

test('authorizes only the exact origin and preserves effective Request headers and bodies', async () => {
  const requests = [];
  const origin = 'http://127.0.0.1:32123';
  const window = install(origin, request => { requests.push(request); return Promise.resolve(request); });
  const sent = await window.fetch('/api/message', {
    method: 'POST', headers: {'X-Grokbot-Request': 'local', 'Content-Type': 'application/json'}, body: '{"text":"hello"}',
  });
  assert.equal(sent.headers.get('X-Ezerbot-Launch'), identity);
  assert.equal(sent.headers.get('X-Grokbot-Request'), 'local');
  assert.equal(sent.headers.get('Content-Type'), 'application/json');
  assert.equal(sent.credentials, 'omit');
  assert.equal(sent.redirect, 'error');
  assert.equal(await sent.text(), '{"text":"hello"}');

  const original = new Request(`${origin}/api/state`, {headers: {'X-Original': 'replaced'}});
  const replacement = await window.fetch(original, {headers: {'X-Other': 'kept', 'X-Ezerbot-Launch': 'caller-value'}});
  assert.equal(replacement.headers.get('X-Original'), null);
  assert.equal(replacement.headers.get('X-Other'), 'kept');
  assert.equal(replacement.headers.get('X-Ezerbot-Launch'), identity);

  for (const url of ['http://127.0.0.1:32124/api/state', 'http://localhost:32123/api/state', 'https://127.0.0.1:32123/api/state', 'https://outside.example/api/state']) {
    const request = await window.fetch(url, {headers: {'X-Ezerbot-Launch': identity, 'X-Other': 'kept'}});
    assert.equal(request.headers.get('X-Ezerbot-Launch'), null);
    assert.equal(request.headers.get('X-Other'), 'kept');
    assert.equal(request.redirect, 'follow');
  }
  const count = requests.length;
  await assert.rejects(window.fetch('http://user:pass@127.0.0.1:32123/api/state'));
  assert.equal(requests.length, count);
});

test('installs immutable Linux capabilities and resource helpers only in the backend document', () => {
  const origin = 'http://127.0.0.1:32123';
  const window = install(origin, () => {});
  assert.equal(window.EzerbotLocal.resourceURL('/api/file?download=1#part'), 'ezerbot-resource://127.0.0.1:32123/api/file?download=1#part');
  for (const raw of ['/icon-192.png', 'http://127.0.0.1:32124/api/file', 'https://outside.example/api/file']) {
    assert.equal(window.EzerbotLocal.resourceURL(raw), raw);
  }
  assert.equal(window.EzerbotShell.platform, 'linux');
  assert.deepEqual(Array.from(window.EzerbotShell.unavailablePanels), ['voice', 'voice-conversation', 'voice-call', 'meetings', 'screen', 'demonstrations']);
  assert.ok(Object.isFrozen(window.EzerbotShell));
  assert.ok(Object.isFrozen(window.EzerbotShell.unavailablePanels));
  assert.equal(Object.getOwnPropertyDescriptor(window, 'EzerbotShell').writable, false);
  assert.equal(Object.getOwnPropertyDescriptor(window, 'EzerbotLocal').configurable, false);
  const outside = install(origin, () => {}, 'https://outside.example/');
  assert.equal(outside.EzerbotShell, undefined);
  assert.equal(outside.EzerbotLocal, undefined);
});

function listen(handler) {
  const server = http.createServer(handler);
  return new Promise(resolve => server.listen(0, '127.0.0.1', () => resolve({
    server, origin: `http://127.0.0.1:${server.address().port}`,
  })));
}

test('rejects 302 and 307 redirects before any launch header reaches an attacker', async () => {
  let attackerRequests = 0;
  const attacker = await listen((request, response) => {
    attackerRequests += 1;
    assert.equal(request.headers['x-ezerbot-launch'], undefined);
    response.end('outside');
  });
  let localRequests = 0;
  const local = await listen((request, response) => {
    localRequests += 1;
    assert.equal(request.headers['x-ezerbot-launch'], identity);
    response.writeHead(request.url === '/302' ? 302 : 307, {Location: `${attacker.origin}/stolen`});
    response.end();
  });
  try {
    const window = install(local.origin, fetch);
    for (const status of [302, 307]) await assert.rejects(window.fetch(`/${status}`));
    assert.equal(localRequests, 2);
    assert.equal(attackerRequests, 0);
    await window.fetch(`${attacker.origin}/direct`, {headers: {'X-Ezerbot-Launch': identity}});
    assert.equal(attackerRequests, 1);
  } finally {
    await Promise.all([local.server, attacker.server].map(server => new Promise(resolve => {
      server.close(resolve);
      server.closeAllConnections();
    })));
  }
});

test('floating commands carry the launch token and settle by id', async () => {
  const window = install('http://127.0.0.1:32123', () => {});
  const shell = window.EzerbotShell;
  assert.ok(Object.isFrozen(shell.floating));
  const first = shell.floating.postMessage({command: 'show'});
  const second = shell.floating.postMessage({command: 'configure', shortcut: 'cmd+option+j'});
  assert.deepEqual(window.posted.map(item => [item.launch, item.id]), [[identity, 1], [identity, 2]]);
  assert.deepEqual(window.posted[1].body, {command: 'configure', shortcut: 'cmd+option+j'});
  shell.settle(2, {status: 'registered'}, null);
  shell.settle(1, null, 'Commande refusée.');
  assert.deepEqual(await second, {status: 'registered'});
  await assert.rejects(first, /Commande refusée\./);
  shell.settle(1, true, null);
  shell.settle(99, true, null);
});

test('the main window reports whether the conversation opened', async () => {
  const window = install('http://127.0.0.1:32123', () => {});
  const asked = [];
  window.EzerbotFloatingMain = {openConversation: async id => { asked.push(id); return id === 'ok' ? {opened: true} : {opened: false, reason: 'Fil absent.'}; }};
  await window.EzerbotShell.openConversation(5, 'ok');
  await window.EzerbotShell.openConversation(6, 'gone');
  assert.deepEqual(asked, ['ok', 'gone']);
  assert.deepEqual(window.posted.map(item => item.body), [
    {command: 'conversationOpened', request: 5, opened: true, reason: ''},
    {command: 'conversationOpened', request: 6, opened: false, reason: 'Fil absent.'},
  ]);
  delete window.EzerbotFloatingMain;
  await window.EzerbotShell.openConversation(7, 'x');
  assert.equal(window.posted.at(-1).body.opened, false);
});

test('the bridge refuses while a same-origin frame is open, not for a sandboxed one', async () => {
  const sandboxed = {get document() { throw new Error('SecurityError'); }};
  const frames = [sandboxed];
  const window = install('http://127.0.0.1:32123', () => {}, undefined, frames);
  window.EzerbotShell.floating.postMessage({command: 'show'});
  assert.equal(window.posted.length, 1, 'a sandboxed frame blocked the page');
  frames.push({document: {}});
  await assert.rejects(window.EzerbotShell.floating.postMessage({command: 'show'}), /refusée/);
  assert.equal(window.posted.length, 1, 'a command went out with a same-origin frame open');
  frames.pop();
  window.EzerbotShell.floating.postMessage({command: 'show'});
  assert.equal(window.posted.length, 2);
});
