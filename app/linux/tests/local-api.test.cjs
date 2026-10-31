const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const template = fs.readFileSync(path.join(__dirname, '../src/local-api.js'), 'utf8');
const identity = 'synthetic-test-identity';

function install(origin, originalFetch, location = `${origin}/`) {
  class BrowserRequest extends Request {
    constructor(input, init) {
      super(typeof input === 'string' ? new URL(input, location) : input, init);
    }
  }
  const window = {URL, Headers, Request: BrowserRequest, fetch: originalFetch, location: {href: location}};
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
  assert.deepEqual(Array.from(window.EzerbotShell.unavailablePanels), ['voice', 'voice-conversation', 'voice-call', 'meetings', 'screen', 'demonstrations', 'floating']);
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
