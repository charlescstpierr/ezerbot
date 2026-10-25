const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createServer } = require("node:http");
const { spawn } = require("node:child_process");
const { mkdtempSync, readFileSync, rmSync, existsSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { chromeBinary } = require("../chrome-binary.cjs");
const { connectCdp } = require("../local-api-client.cjs");

const pause = () => new Promise(done => setTimeout(done, 20));
const clone = value => JSON.parse(JSON.stringify(value));
const gate = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const page = (id, text, version, previous = false) => ({
  id, title: `Page ${id}`, bot_id: "grokbot", conversation_id: `fil-${id}`, status: "ready",
  source_version: version, preview: text, has_previous: previous, anchors: [],
  document: { states: [], view: { type: "element", tag: "p", children: [{ type: "text", text }] } },
});

async function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "ezerbot-pages-app-"));
  const pages = { a: page("a", "Ancien passage. Passage récent.", "v2", true), b: page("b", "Autre Page.", "b1") };
  const state = {
    account: { name: "Espace personnel" }, mode: "simulated", state_revision: 1,
    bots: [{ id: "grokbot", name: "Ezerbot", color: "citron", shape: "galet", is_default: true }],
    conversations: ["a", "b", "vide"].map(id => ({ id: `fil-${id}`, title: `Fil ${id}`, bot_id: "grokbot", kind: "direct", messages: [], member_ids: ["grokbot"] })),
    preferences: { theme: "light", accent: "citron" },
    provider: { connected: false, signin: "idle", backend: "sharing", models: [], connections: {} },
    pages: { by_bot: { grokbot: Object.values(pages).map(({ id, title, bot_id, conversation_id }) => ({ id, title, bot_id, conversation_id })) } },
  };
  const calls = [];
  const reads = [];
  let revert, send, chrome, cdp;
  const server = createServer(async (request, response) => {
    const json = (data, status = 200) => { response.writeHead(status, { "Content-Type": "application/json" }); response.end(JSON.stringify(data)); };
    const url = request.url;
    if (!url.startsWith("/api/")) {
      const file = url === "/" ? "index.html" : url === "/panels.js" ? "panels/pages.js" : url.slice(1);
      const path = resolve(__dirname, "..", file);
      if (!path.startsWith(resolve(__dirname, "..") + "/") || !existsSync(path)) { response.writeHead(404); response.end(); return; }
      response.setHeader("Content-Type", file.endsWith(".js") ? "text/javascript" : file.endsWith(".css") ? "text/css" : "text/html");
      let content = readFileSync(path);
      if (file === "index.html") content = Buffer.from(content.toString().replace("</head>", `<link rel="stylesheet" href="/panels/pages.css"><script>
        const realFetch = window.fetch.bind(window);
        window.fetch = (path, options = {}) => realFetch(path, {
          ...options, headers: { ...options.headers, 'X-Test-Reader': /\\brestore\\b/.test(new Error().stack) ? 'composer' : 'panel' }
        });
      </script></head>`));
      response.end(content); return;
    }
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    calls.push({ url, reader: request.headers["x-test-reader"], body: chunks.length ? JSON.parse(Buffer.concat(chunks)) : null });
    if (url === "/api/state") { json(state); return; }
    if (url === "/api/ping") { json({ revision: state.state_revision }); return; }
    if (url === "/api/send") {
      if (send) { const held = send; send = null; held.resolve(() => json({ error: "Envoi refusé pour le test." }, 400)); }
      else json({ error: "Envoi refusé pour le test." }, 400);
      return;
    }
    if (url === "/api/pages/revert") {
      const held = revert; revert = null;
      held.resolve((refused = false) => {
        if (refused) { json({ error: "Restauration refusée pour le test." }, 409); return; }
        pages.a = page("a", "Ancien passage.", "v1");
        json({ page: pages.a });
      });
      return;
    }
    if (/^\/api\/pages\/[ab]$/.test(url)) {
      const snapshot = clone(pages[url.at(-1)]);
      const index = reads.findIndex(held => held.reader === request.headers["x-test-reader"]);
      const held = index < 0 ? null : reads.splice(index, 1)[0];
      if (held) held.resolve((failed = false) => json(failed ? { error: "Lecture refusée pour le test." } : { page: snapshot }, failed ? 503 : 200));
      else json({ page: snapshot });
      return;
    }
    json({});
  });
  t.after(async () => {
    cdp?.close();
    if (chrome && chrome.exitCode === null) { chrome.kill("SIGTERM"); await new Promise(done => chrome.once("exit", done)); }
    server.closeAllConnections();
    await new Promise(done => server.close(done));
    rmSync(root, { recursive: true, force: true });
    t.diagnostic(JSON.stringify(calls.map(({ url, reader }) => ({ url, reader }))));
  });
  await new Promise(done => server.listen(0, "127.0.0.1", done));
  chrome = spawn(chromeBinary(), ["--headless=new", "--no-sandbox", "--disable-gpu", "--use-mock-keychain", "--password-store=basic", "--remote-debugging-port=0", `--user-data-dir=${root}`, "about:blank"], { stdio: "ignore" });
  let port;
  for (let n = 0; n < 500 && !port; n++) {
    try { port = Number(readFileSync(join(root, "DevToolsActivePort"), "utf8").split("\n")[0]); } catch { await pause(); }
  }
  assert.ok(port, "Chrome ouvre son propre port CDP");
  const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then(response => response.json());
  cdp = await connectCdp(targets.find(target => target.type === "page").webSocketDebuggerUrl);
  await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1140, height: 780, deviceScaleFactor: 1, mobile: false });
  const run = async source => {
    const result = await cdp.send("Runtime.evaluate", { expression: `(${source})()`, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result.value;
  };
  const until = async source => { for (let n = 0; n < 300; n++) { if (await run(source)) return; await pause(); } throw new Error(`État absent: ${source}`); };
  await cdp.send("Page.navigate", { url: `http://127.0.0.1:${server.address().port}/` });
  await until(() => window.GrokbotPanels?.connected && document.querySelector(".page-link"));
  await run(() => { window.button = text => [...document.querySelectorAll("button")].find(el => el.textContent === text); document.querySelector('[data-page-id="a"]').click(); });
  await until(() => document.querySelector(".page-canvas").textContent.includes("Passage récent"));
  return {
    run, until, pages, state, calls,
    holdRead(reader = "panel") { const held = { ...gate(), reader }; reads.push(held); return held.promise; },
    holdSend() { send = gate(); return send.promise; },
    async startRevert() { revert = gate(); const waiting = revert.promise; await run(() => button("Revenir à avant").click()); return waiting; },
    async quote(text) {
      await run(`() => {
        const node = document.querySelector('.page-canvas p').firstChild;
        const start = node.textContent.indexOf(${JSON.stringify(text)});
        const range = document.createRange(); range.setStart(node, start); range.setEnd(node, start + ${text.length});
        getSelection().removeAllRanges(); getSelection().addRange(range); document.dispatchEvent(new Event('selectionchange'));
        document.querySelector('.page-comment').click();
      }`);
    },
    async refusedQuoteRead() {
      await this.quote("Passage récent");
      const waiting = this.holdSend();
      await run(() => {
        const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
        editor.focus(); const range = document.createRange(); range.selectNodeContents(editor); range.collapse(false);
        getSelection().removeAllRanges(); getSelection().addRange(range);
        if (!document.execCommand("insertText", false, "Commentaire refusé")) throw new Error("Saisie refusée");
      });
      await until(() => !document.getElementById("send-button").disabled);
      await run(() => document.getElementById("send-button").click());
      const releaseSend = await waiting;
      const reading = this.holdRead("composer");
      releaseSend();
      return reading;
    },
  };
}

for (const refused of [false, true]) test(`navigation-perdue, retour depuis un fil sans Page pendant une restauration ${refused ? "refusée" : "réussie"}`, { timeout: 30000 }, async t => {
  const app = await fixture(t);
  const release = await app.startRevert();
  await app.run(() => GrokbotPanels.openConversation("fil-vide"));
  assert.equal(await app.run(() => document.querySelector(".page-stage").hidden), true);
  await app.run(() => GrokbotPanels.openConversation("fil-a"));
  const visibleDuring = await app.run(() => !document.querySelector(".page-stage").hidden);
  release(refused);
  await app.until(() => !button("Restauration…"));
  await app.run(() => GrokbotPanels.refresh());
  assert.equal(visibleDuring, true, "la Page redevient visible avant la réponse de restauration");
  assert.equal(await app.run(() => document.querySelector(".page-stage").hidden), false, "le panneau reste visible sans nouvelle révision");
  assert.equal(await app.run(() => document.querySelector(".page-canvas").textContent), refused ? "Ancien passage. Passage récent." : "Ancien passage.");
});

test("reponse-perimee, la relecture d’un envoi refusé ne réinstalle pas la version annulée ni son passage", { timeout: 30000 }, async t => {
  const app = await fixture(t);
  const releaseRead = await app.refusedQuoteRead();
  const releaseRevert = await app.startRevert();
  releaseRevert();
  await app.until(() => document.querySelector(".page-canvas").textContent === "Ancien passage.");
  await app.run(() => {
    window.observed = [];
    new MutationObserver(() => observed.push(document.querySelector(".page-canvas").textContent))
      .observe(document.querySelector(".page-canvas"), { childList: true, subtree: true });
  });
  releaseRead();
  await app.until(() => !GrokbotPanels.busy);
  assert.equal(await app.run(() => observed.some(text => text.includes("Passage récent"))), false, "la version annulée ne doit jamais être repeinte");
  assert.equal(await app.run(() => document.querySelector(".page-canvas").textContent), "Ancien passage.", "une réponse antérieure ne remplace pas la source restaurée");
  assert.equal(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").hidden), true, "le passage disparu n’est pas repris");
});
