#!/usr/bin/env node
// Extract a supplied Electron ASAR through the installed official ASAR parser.
// Never modifies or executes the source application. Keeps original byte hashes.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';

const [source, output, dependencyPackage] = process.argv.slice(2);
if (!source || !output || !dependencyPackage) {
  throw new Error('Usage: node inspect-client.mjs SOURCE_ASAR OUTPUT_ROOT ASAR_DEPENDENCY_PACKAGE_JSON');
}
const require = createRequire(path.resolve(dependencyPackage));
const asar = await import(pathToFileURL(require.resolve('@electron/asar')).href);
const sha256 = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const sourceHash = sha256(fs.readFileSync(source));
const root = path.resolve(output);
const extracted = path.join(root, 'client');
const manifestPath = path.join(root, 'manifest.json');
fs.mkdirSync(root, { recursive: true });
for (const raw of asar.listPackage(source)) {
  const relative = raw.replace(/^\//, '');
  const resolved = path.resolve(extracted, relative);
  if (!resolved.startsWith(extracted + path.sep)) throw new Error('Archive path escapes output root');
  if (asar.statFile(source, relative).link) throw new Error('This extraction recipe does not accept archive links');
}
if (fs.existsSync(extracted)) {
  if (!fs.existsSync(manifestPath) || JSON.parse(fs.readFileSync(manifestPath, 'utf8')).source.sha256 !== sourceHash) {
    throw new Error('Existing output does not identify this exact source. Choose a new output root.');
  }
} else {
  asar.extractAll(source, extracted);
}
const entries = [];
for (const raw of asar.listPackage(source).sort()) {
  const relative = raw.replace(/^\//, '');
  const stat = asar.statFile(source, relative);
  if (stat.files) continue;
  const file = path.resolve(extracted, relative);
  if (!file.startsWith(extracted + path.sep)) throw new Error('Archive path escapes output root');
  if (stat.link) {
    entries.push({ path: relative, kind: 'link', target: stat.link });
    continue;
  }
  const bytes = fs.readFileSync(file);
  const observed = sha256(bytes);
  const shipped = sha256(asar.extractFile(source, relative));
  if (observed !== shipped) throw new Error('Extracted bytes no longer match supplied source: ' + relative);
  const declared = stat.integrity?.hash ?? null;
  entries.push({ path: relative, kind: 'file', size: bytes.length, sha256: observed,
    declared_sha256: declared, unpacked: !!stat.unpacked,
    integrity: !declared ? 'not-declared' : declared === observed ? 'matches' : 'observed-untrusted' });
}
const manifest = {
  schema: 1,
  source: { path: path.resolve(source), sha256: sourceHash },
  tool: 'tools/inspect-client.mjs with @electron/asar',
  execution: 'static-only',
  entries,
  coverage: { files: entries.filter(e => e.kind === 'file').length,
    native_integrity_contradictions: entries.filter(e => e.integrity === 'observed-untrusted').length },
};
fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
const pkg = JSON.parse(fs.readFileSync(path.join(extracted, 'package.json'), 'utf8'));
console.log(JSON.stringify({ source_sha256: sourceHash, output: extracted,
  version: pkg.version, main: pkg.main, coverage: manifest.coverage,
  contradictions: entries.filter(e => e.integrity === 'observed-untrusted'),
  code_files: entries.filter(e => /\.(c?js|mjs|html|css)$/.test(e.path) && !e.path.includes('/deps/')).map(e => ({ path:e.path, size:e.size })) }, null, 2));
