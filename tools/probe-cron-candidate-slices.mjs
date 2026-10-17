// Execute exact, manifest-bound pure cron slices. Never bootstrap the renderer.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import vm from 'node:vm';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
assert(process.argv[2], 'Pass the existing REA package.json; no dependency is installed.');
const require = createRequire(path.resolve(process.argv[2]));
const parser = require('@babel/parser');
const sha = (bytes) => createHash('sha256').update(bytes).digest('hex');
const memberPath = 'dist/renderer/assets/index.eager-app-BfVhIMYf.js';
const sourcePath = `artifacts/client/${memberPath}`;
const bytes = readFileSync(path.join(root, sourcePath));
const source = bytes.toString('utf8');
assert(Buffer.from(source).equals(bytes));
const manifestPath = 'artifacts/manifest.json';
const manifestRaw = readFileSync(path.join(root, manifestPath));
const manifest = JSON.parse(manifestRaw);
const member = manifest.entries.find((e) => e.path === memberPath);
assert.equal(sha(bytes), member.sha256);
assert.equal(manifest.source.sha256, '9fe94a66942f8511281f6d7c80a42432d749284c46e1e26f5b6d2f99b87652b0');

const wanted = new Set(['qs', 'i0', 'ane', 'Fi', 'lne', 'pR', 'cne', 'hR', 'a0', 'gR',
  'fne', 'pne', '_v', 'mR', 'mi', 'mne', 'yne', 'sne', 'Mf', 'rne', 'one', 'ine', 'une', 'wv', 'dne', '_r']);
const slices = [];
for (const node of parser.parse(source, { sourceType: 'module' }).program.body) {
  for (const selected of node.type === 'VariableDeclaration' ? node.declarations : [node]) {
    const name = selected.id?.name;
    if (!wanted.has(name)) continue;
    assert(['FunctionDeclaration', 'VariableDeclarator'].includes(selected.type));
    const text = source.slice(selected.start, selected.end);
    slices.push({ name, type: selected.type, start_utf16: selected.start, end_utf16: selected.end,
      start_byte: Buffer.byteLength(source.slice(0, selected.start)),
      end_byte: Buffer.byteLength(source.slice(0, selected.end)), raw_line: selected.loc.start.line,
      sha256: sha(text), text });
  }
}
assert.equal(slices.length, wanted.size);
assert.equal(new Set(slices.map((s) => s.name)).size, wanted.size);
const selectedProgram = slices.map((s) => `${s.type === 'VariableDeclarator' ? 'var ' : ''}${s.text};`).join('\n')
  + '\nthis.selected = { mne, mi, mR };';
const context = vm.createContext({});
new vm.Script(selectedProgram, { filename: 'original-cron-candidate-slices.js' }).runInContext(context, { timeout: 1000 });
const { mne } = context.selected;
const cases = [];
function next(id, schedule, reference, expected) {
  const referenceMs = Date.parse(reference);
  assert(Number.isFinite(referenceMs));
  const nextMs = mne(schedule, referenceMs);
  const observed = nextMs === null ? null : new Date(nextMs).toISOString();
  assert.equal(observed, expected, id);
  cases.push({ id, schedule, reference, observed, expected });
}

next('spring-gap', 'CRON_TZ=America/New_York 30 2 * * *', '2026-03-08T06:59:00Z', '2026-03-09T06:30:00.000Z');
next('fall-first', 'CRON_TZ=America/New_York 30 1 * * *', '2026-11-01T04:59:00Z', '2026-11-01T05:30:00.000Z');
next('fall-second', 'CRON_TZ=America/New_York 30 1 * * *', '2026-11-01T05:30:00Z', '2026-11-01T06:30:00.000Z');
next('leap-beyond-horizon', 'CRON_TZ=UTC 0 0 29 2 *', '2026-01-01T00:00:00Z', null);
next('leap-in-horizon', 'CRON_TZ=UTC 0 0 29 2 *', '2028-01-01T00:00:00Z', '2028-02-29T00:00:00.000Z');
next('restricted-days-or-date', 'CRON_TZ=UTC 0 0 13 * 1', '2026-10-12T00:00:00Z', '2026-10-13T00:00:00.000Z');
next('restricted-days-or-weekday', 'CRON_TZ=UTC 0 0 13 * 1', '2026-10-13T00:00:00Z', '2026-10-19T00:00:00.000Z');
next('sunday-seven', 'CRON_TZ=UTC 0 0 * * 7', '2026-10-17T12:00:00Z', '2026-10-18T00:00:00.000Z');
next('strict-next-minute', 'CRON_TZ=UTC * * * * *', '2026-10-17T10:00:00Z', '2026-10-17T10:01:00.000Z');
next('next-minute-fraction', 'CRON_TZ=UTC * * * * *', '2026-10-17T10:00:59.999Z', '2026-10-17T10:01:00.000Z');
next('interval-decimal', '@every 0.5s', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00.500Z');
next('interval-phase', '@every 1h/15m', '2026-01-01T00:15:00Z', '2026-01-01T01:15:00.000Z');
next('interval-timezone-prefix-rejected', 'CRON_TZ=UTC @every 1h', '2026-01-01T00:00:00Z', null);
next('invalid-timezone-rejected', 'CRON_TZ=Fixture/Invalid 0 0 * * *', '2026-01-01T00:00:00Z', null);
next('alias-case-insensitive', 'CRON_TZ=UTC @DAILY', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00.000Z');
next('seconds-field-rejected', 'CRON_TZ=UTC 0 0 0 * * *', '2026-01-01T00:00:00Z', null);

console.log(JSON.stringify({ schema: 1, kind: 'controlled-replay-of-original-cron-candidate-slices',
  source: { path: sourcePath, bytes: bytes.length, sha256: sha(bytes), manifest_path: manifestPath,
    manifest_sha256: sha(manifestRaw), asar_sha256: manifest.source.sha256 },
  harness_sha256: sha(readFileSync(fileURLToPath(import.meta.url))),
  parser: { name: '@babel/parser', version: require('@babel/parser/package.json').version },
  environment: { node: process.version, v8: process.versions.v8, icu: process.versions.icu,
    tz: process.versions.tz, host_time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone },
  slices: slices.map(({ text, ...s }) => s), selected_program_sha256: sha(selectedProgram),
  original_renderer_bootstrapped: false, selected_original_javascript_executed: true,
  scheduler_executed: false, routine_created_or_run: false, cases, assertions_passed: true,
  limitations: ['Pure original functions execute in Node VM with Node Date/Intl, not Electron Chromium Intl.',
    'No app, renderer bootstrap, account, gateway, server scheduler or native helper is executed.',
    'No clock or network fixture is used; all reference times are explicit constants.',
    'The source proves only an interval-description caller; active cron-branch use is not established.',
    'Observed candidate instants are not proof of actual routine dispatch, deduplication or DST policy.',
    'Node VM and REA process capture are not security sandbox guarantees.'] }));
