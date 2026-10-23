import test from 'node:test';
import assert from 'node:assert/strict';
import { citationPlugin, sourceNotes, openSource } from './sources.js';
const source = { n: 1, type: 'courriel', titre: 'Budget', auteur: 'Alice', date: '2026-10-23', identifiant: 'mail1', compte: 'alice@example.com' };
test('les citations deviennent des pastilles et le code reste intact', () => {
  const tree = { type: 'root', children: [{ type: 'text', value: 'Oui [1] et [7].' }, { type: 'element', tagName: 'code', children: [{ type: 'text', value: '[1]' }] }] };
  citationPlugin([{ n: 1 }, { n: 7 }])()(tree);
  assert.equal(tree.children[1].properties['data-source'], '1');
  assert.equal(tree.children[3].properties['data-source'], '7');
  assert.equal(tree.children.at(-1).children[0].value, '[1]');
});
test('copie et export incluent les notes des sources', () => {
  assert.match(sourceNotes('Oui [1]', [source]), /Oui \[1\][\s\S]*Sources[\s\S]*\[1\] Budget.*Alice.*2026-10-23/);
  assert.equal(sourceNotes('Sans source', []), 'Sans source');
});
test('ouvrir transmet le bon identifiant et signale un compte déconnecté', async () => {
  let url;
  assert.equal(await openSource(source, { open: value => { url = value; }, fetch: async () => ({ ok: true, json: async () => ({ mcp: { catalogue: [{ id: 'gmail', account: source.compte, status: 'connected' }] } }) }) }), '');
  assert.match(url, /mail1$/);
  assert.match(await openSource(source, { fetch: async () => ({ ok: false }) }), /Ce compte n'est plus connecté/);
});
test('l’export de la réponse garde les références et les notes', async () => {
  const { exportMessage } = await import('./teach.js');
  assert.match(exportMessage({ role: 'assistant', body: 'Oui [1]', payload: { sources: [source] } }).body, /Oui \[1\][\s\S]*\[1\] Budget/);
});
test('un fichier local retiré affiche une erreur sans ouvrir de fenêtre', async () => {
  assert.match(await openSource({ ...source, type: 'bibliotheque', compte: undefined }, { fetch: async () => ({ ok: false }) }), /fichier n'est plus accessible/);
});
test('ouvrir un fichier local utilise le bot de la réponse et son identifiant', async () => {
  let path;
  const result = await openSource({ n: 3, type: 'bibliotheque', identifiant: 'file3' }, { botId: 'bot1', fetch: async url => { path = url; return { ok: true, json: async () => ({ text: 'Texte local' }) }; } });
  assert.equal(path, '/api/library/source?bot_id=bot1&id=file3');
  assert.deepEqual(result, { text: 'Texte local' });
});
