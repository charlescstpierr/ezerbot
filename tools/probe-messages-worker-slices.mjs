// Controlled replay of exact, manifest-bound function slices. Never bootstrap the worker.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import vm from 'node:vm';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const parserPackage = process.argv[2];
assert(parserPackage, 'Pass the existing REA package.json path; no dependency is installed.');
const require = createRequire(path.resolve(parserPackage));
const parser = require('@babel/parser');
const sha = (data) => createHash('sha256').update(data).digest('hex');
const sourcePath = 'artifacts/releases/0.71.2/client/dist/local-exec-daemon/messages-worker.cjs';
const inventoryPath = 'artifacts/releases/0.71.2/asar-inventory.json';
const bytes = readFileSync(path.join(root, sourcePath));
const source = bytes.toString('utf8');
assert(Buffer.from(source).equals(bytes), 'Source must be losslessly UTF-8.');
const inventoryBytes = readFileSync(path.join(root, inventoryPath));
const inventory = JSON.parse(inventoryBytes);
const member = inventory.entries.find((e) => e.path === 'dist/local-exec-daemon/messages-worker.cjs');
assert.equal(sha(bytes), member.sha256);
assert.equal(inventory.asar_sha256, '05a48b6088fcb89c00b29cce6bbb6b46b377506cd5a394e51285ca12c55c12d2');

const wanted = new Set(['Hr', 'Vr', 'F', 'Pr', 'Wr', 'ze', 'ln', 'Ur', 'Xe',
  'Lr', 'Gr', '$r', 'jr', 'Fr', '_r', 'Ar']);
const tree = parser.parse(source, { sourceType: 'script' });
const slices = [];
for (const node of tree.program.body) {
  const nodes = node.type === 'VariableDeclaration' ? node.declarations : [node];
  for (const selected of nodes) {
    const name = selected.id?.name;
    if (!wanted.has(name)) continue;
    assert(['VariableDeclarator', 'FunctionDeclaration'].includes(selected.type));
    const text = source.slice(selected.start, selected.end);
    slices.push({ name, type: selected.type, start_utf16: selected.start, end_utf16: selected.end,
      start_byte: Buffer.byteLength(source.slice(0, selected.start)),
      end_byte: Buffer.byteLength(source.slice(0, selected.end)),
      raw_line: selected.loc.start.line, sha256: sha(text), text });
  }
}
assert.equal(slices.length, wanted.size);
assert.equal(new Set(slices.map((s) => s.name)).size, wanted.size);
const selectedProgram = slices.map((s) => `${s.type === 'VariableDeclarator' ? 'var ' : ''}${s.text};`).join('\n')
  + '\nthis.selected = { ze, Vr, Hr };';

// These are analyst fixtures, not implementation of a native helper or Messages.
const fixtureApp = '/fixture-only/Grok Bot Computer Use.app';
const fixtureInstall = '/fixture-only/install';
const fixtureService = '/fixture-home/Library/Application Support/FixtureComputerUse/service.json';
const fixtureSocket = '/fixture-only/rpc.sock';
const input = { kind: 'participant', handle: 'fixture-recipient', service: 'iMessage',
  textFile: '/fixture-only/text.txt', sendKey: 'stable-fixture-key', requestId: 'stable-fixture-request' };
const validPayload = { version: 'fixture', verbs: [
  'snapshot-messages-db', 'copy-attachment', 'send-message', 'check-messages-permissions'], via: 'participant' };

function fixture(mode) {
  const state = { socket_attempts: [], service_reads: [], install_calls: [], launch_calls: [], delays_ms: [], fixture_effects: 0 };
  const context = vm.createContext({
    Error, // Same Error realm at injected error boundaries, so original instanceof checks apply.
    Ze: () => fixtureApp,
    xr: (app) => { assert.equal(app, fixtureApp); return { appSupportDir: 'FixtureComputerUse' }; },
    rn: { homedir: () => '/fixture-home' },
    Ye: { randomUUID },
    g: { readFile: async (p, encoding) => {
      assert.equal(p, fixtureService); assert.equal(encoding, 'utf8');
      state.service_reads.push(p); return JSON.stringify({ rpcSocketPath: fixtureSocket });
    } },
    Cr: async (app, options) => {
      assert.equal(app, fixtureApp); assert.equal(options.installDir, fixtureInstall);
      state.install_calls.push({ app, installDir: options.installDir }); return fixtureApp;
    },
    Br: async (app) => { assert.equal(app, fixtureApp); state.launch_calls.push(app); },
    sn: { setTimeout: async (ms) => { assert.equal(ms, 200); state.delays_ms.push(ms); } },
    kr: async (socketPath, serialized, timeoutMs) => {
      assert.equal(socketPath, fixtureSocket);
      assert(serialized.endsWith('\n'));
      const envelope = JSON.parse(serialized);
      state.socket_attempts.push({ socketPath, serialized, envelope, timeout_ms: timeoutMs ?? null });
      if (mode === 'refused-once' && state.socket_attempts.length === 1) {
        throw Object.assign(new Error('ECONNREFUSED fixture before effect'), { code: 'ECONNREFUSED' });
      }
      state.fixture_effects++;
      if (mode === 'lost-reply-once' && state.socket_attempts.length === 1) {
        throw new Error('sidecar_closed');
      }
      return { payload: validPayload };
    },
  });
  // The VM is a dependency-isolation convenience, not a security sandbox claim.
  new vm.Script(selectedProgram, { filename: 'manifest-bound-worker-slices.cjs' }).runInContext(context, { timeout: 1000 });
  const adapter = context.selected.ze({ installDir: fixtureInstall });
  return { state, adapter, selected: context.selected };
}

const cases = [];
{
  const { state, adapter } = fixture('success');
  await adapter.sendMessage(input); await adapter.sendMessage(input);
  assert.equal(state.fixture_effects, 2);
  assert.equal(new Set(state.socket_attempts.map((a) => a.envelope.requestID)).size, 2);
  for (const a of state.socket_attempts) {
    assert.equal(a.envelope.id, 1); assert.equal(a.envelope.method, 'messages/send');
    assert.deepEqual(a.envelope.arguments, { to: input.handle, service: input.service, textFile: input.textFile });
    assert.equal(a.timeout_ms, null);
    assert(!('sendKey' in a.envelope)); assert(!('requestId' in a.envelope));
  }
  cases.push({ id: 'independent-calls', input, ...state });
}
{
  const { state, adapter } = fixture('refused-once');
  await adapter.sendMessage(input);
  assert.equal(state.socket_attempts.length, 2); assert.equal(state.fixture_effects, 1);
  assert.equal(state.socket_attempts[0].serialized, state.socket_attempts[1].serialized);
  assert.equal(state.launch_calls.length, 1); assert.deepEqual(state.delays_ms, [200]);
  cases.push({ id: 'reconnect-before-effect', ...state });
}
{
  const { state, adapter } = fixture('lost-reply-once');
  let error;
  try { await adapter.sendMessage(input); } catch (e) { error = e.message; }
  assert.equal(error, 'sidecar_closed');
  assert.equal(state.socket_attempts.length, 1); assert.equal(state.fixture_effects, 1);
  await adapter.sendMessage(input);
  assert.equal(state.socket_attempts.length, 2); assert.equal(state.fixture_effects, 2);
  assert.notEqual(state.socket_attempts[0].envelope.requestID, state.socket_attempts[1].envelope.requestID);
  assert.equal(state.launch_calls.length, 0); assert.equal(state.delays_ms.length, 0);
  cases.push({ id: 'fixture-effect-then-lost-reply', first_error: error, ...state });
}
{
  const { selected } = fixture('success');
  const normalize = (v) => v === undefined ? 'undefined' : v;
  const deadlines = {
    send: normalize(selected.Vr('send-message', {})),
    snapshot: normalize(selected.Vr('snapshot-messages-db', {})),
    resolve: normalize(selected.Vr('resolve-handles', {})),
    permissions_with_prompt_true: normalize(selected.Vr('check-messages-permissions', { prompt: true })),
    permissions_with_prompt_false: normalize(selected.Vr('check-messages-permissions', { prompt: false })),
  };
  assert.deepEqual(deadlines, { send: 'undefined', snapshot: 10000, resolve: 4000,
    permissions_with_prompt_true: 'undefined', permissions_with_prompt_false: 'undefined' });
  cases.push({ id: 'deadline-selector', deadlines });
}

console.log(JSON.stringify({ schema: 1, kind: 'controlled-replay-of-original-slices',
  original_application_executed: false, selected_original_js_executed: true,
  native_helper_executed: false, messages_accessed_or_sent: false,
  source: { path: sourcePath, sha256: sha(bytes), bytes: bytes.length,
    inventory_path: inventoryPath, inventory_sha256: sha(inventoryBytes), asar_sha256: inventory.asar_sha256 },
  harness_sha256: sha(readFileSync(fileURLToPath(import.meta.url))),
  parser: { name: '@babel/parser', version: require('@babel/parser/package.json').version },
  slices: slices.map(({ text, ...s }) => s), selected_program_sha256: sha(selectedProgram),
  boundaries_replaced: ['Ze', 'xr', 'rn.homedir', 'g.readFile', 'Cr', 'Br', 'kr', 'sn.setTimeout'],
  uuid_source: 'node:crypto.randomUUID', error_realm: 'host Error injected for instanceof',
  limitations: ['Only listed exact original slices execute; the complete worker never bootstraps.',
    'All service discovery, installation, launching, socket and timer boundaries are analyst fixtures.',
    'Fixture effects are counters, never Messages sends. This is not observed product duplication.',
    'Daemon B4 caches, worker_threads and native RPC implementation are not executed here.',
    'Node VM and REA process capture are not security sandbox guarantees.'],
  cases, assertions_passed: true }));
