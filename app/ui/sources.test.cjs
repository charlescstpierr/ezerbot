// Run with: node --test app/ui/sources.test.cjs (CHROME_BIN optional).
const assert = require('node:assert/strict');
const { execFile } = require('node:child_process');
const { mkdtempSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { pathToFileURL } = require('node:url');
const { promisify } = require('node:util');
const { test } = require('node:test');

// Exercise the real SourcesProse component and stylesheet, without a manual server
// or a separate Playwright installation.
const probe = async () => {
  const wait = async selector => {
    for (let i = 0; i < 200; i++) {
      const node = document.querySelector(selector);
      if (node) return node;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    throw new Error(`Élément absent : ${selector}`);
  };
  try {
    const pill = await wait('button.gb-citation');
    const style = getComputedStyle(pill);
    const report = { pill: { text: pill.textContent, visible: pill.getBoundingClientRect().width > 0, border: style.borderTopStyle, verticalAlign: style.verticalAlign } };
    pill.click();
    const card = await wait('[role=dialog]');
    report.card = card.textContent;
    let connected = true;
    window.fetch = async () => ({ ok: true, json: async () => ({ mcp: { catalogue: connected ? [{ id: 'gmail', account: 'alice@example.com', status: 'connected' }] : [] } }) });
    window.open = url => { report.opened = url; };
    const open = [...card.querySelectorAll('button')].find(node => node.textContent === 'Ouvrir la source');
    open.click();
    for (let i = 0; i < 100 && !report.opened; i++) await new Promise(resolve => setTimeout(resolve, 10));
    connected = false;
    open.click();
    report.error = (await wait('[role=status]')).textContent;
    card.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await new Promise(resolve => setTimeout(resolve, 25));
    report.focusRestored = document.activeElement === pill;
    const missing = await wait('[title="Source introuvable"]');
    report.missing = { text: missing.textContent, decoration: getComputedStyle(missing).textDecorationLine, title: missing.title };
    document.body.innerHTML = '<pre id="qa-result"></pre>';
    document.querySelector('pre').textContent = JSON.stringify(report);
  } catch (error) {
    document.body.innerHTML = '<pre id="qa-result"></pre>';
    document.querySelector('pre').textContent = JSON.stringify({ failure: String(error) });
  }
};

test('sources : pastille, carte, ouverture et référence introuvable dans Chromium', async t => {
  const { build } = await import('../web/node_modules/vite/dist/node/index.js');
  const output = mkdtempSync(join(tmpdir(), 'sources-build-'));
  t.after(() => rmSync(output, { recursive: true, force: true }));
  await build({
    root: join(__dirname, '../web'), base: './', logLevel: 'error',
    build: { lib: false, outDir: output, rollupOptions: { input: join(__dirname, '../web/sources-preview.html') } },
    plugins: [{ name: 'sources-ui-test', transformIndexHtml: { order: 'pre', handler(html) {
      return html.replace("'Le budget est confirmé [1].", "'Source absente [7]. Le budget est confirmé [1].")
        + `<script type="module">(${probe.toString()})()</script>`;
    } } }],
  });
  for (const theme of ['', '?dark']) {
    const { stdout, stderr } = await promisify(execFile)(process.env.CHROME_BIN || 'google-chrome', [
      '--headless=new', '--allow-file-access-from-files', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage',
      '--virtual-time-budget=15000', '--window-size=820,560',
      '--dump-dom', `${pathToFileURL(join(output, 'sources-preview.html'))}${theme}`,
    ], { timeout: 30000, maxBuffer: 4 * 1024 * 1024 });
    const match = stdout.match(/<pre id="qa-result">([\s\S]*?)<\/pre>/);
    assert.ok(match, `Chromium doit produire le résultat du test : ${stdout.slice(-2000)} ${stderr.slice(-2000)}`);
    const report = JSON.parse(match[1].replaceAll('&quot;', '"').replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&'));
    assert.equal(report.failure, undefined);
    assert.deepEqual(report.pill, { text: '[1]', visible: true, border: 'solid', verticalAlign: 'super' });
    assert.match(report.card, /Budget du trimestre.*Alice Martin.*2026-10-23/);
    assert.equal(report.opened, 'https://mail.google.com/mail/u/alice%40example.com/#all/mail1');
    assert.match(report.error, /Ce compte n'est plus connecté/);
    assert.equal(report.focusRestored, true);
    assert.deepEqual(report.missing, { text: '[7]', decoration: 'line-through', title: 'Source introuvable' });
  }
});
