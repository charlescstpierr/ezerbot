const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createServer } = require("node:http");
const { spawn } = require("node:child_process");
const { mkdtempSync, readFileSync, existsSync } = require("node:fs");
const { rm } = require("node:fs/promises");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { chromeBinary, pageTarget } = require("../chrome-binary.cjs");
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
  let revert, send, heldState, chrome, cdp;
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
        window.parsed = [];
        window.fetch = async (path, options = {}) => {
          const reader = /\\brestore\\b/.test(new Error().stack) ? 'composer' : 'panel';
          const response = await realFetch(path, { ...options, headers: { ...options.headers, 'X-Test-Reader': reader } });
          const json = response.json.bind(response);
          response.json = async () => { const data = await json(); parsed.push({ path, reader }); return data; };
          return response;
        };
      </script></head>`));
      response.end(content); return;
    }
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    calls.push({ url, reader: request.headers["x-test-reader"], body: chunks.length ? JSON.parse(Buffer.concat(chunks)) : null });
    if (url === "/api/state") {
      if (heldState) { const held = heldState; heldState = null; const snapshot = clone(state); held.resolve((failed = false) => json(failed ? { error: "État indisponible pour le test." } : snapshot, failed ? 503 : 200)); }
      else json(state);
      return;
    }
    if (url === "/api/ping") { json({ revision: state.state_revision }); return; }
    if (url === "/api/pages/rename") {
      const { id, title } = calls.at(-1).body;
      pages[id] = { ...pages[id], title };
      state.pages.by_bot.grokbot.find(page => page.id === id).title = title;
      json({ page: pages[id] }); return;
    }
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
    if (chrome && chrome.exitCode === null && chrome.signalCode === null) {
      const exited = new Promise(done => chrome.once("exit", done));
      chrome.kill("SIGTERM");
      let timer;
      await Promise.race([exited, new Promise(done => { timer = setTimeout(done, 3000); })]);
      clearTimeout(timer);
      if (chrome.exitCode === null && chrome.signalCode === null) { chrome.kill("SIGKILL"); await exited; }
    }
    server.closeAllConnections();
    await new Promise(done => server.close(done));
    await rm(root, { recursive: true, force: true, maxRetries: 20, retryDelay: 50 });
    t.diagnostic(JSON.stringify(calls.map(({ url, reader }) => ({ url, reader }))));
  });
  await new Promise(done => server.listen(0, "127.0.0.1", done));
  chrome = spawn(chromeBinary(), ["--headless=new", "--no-sandbox", "--disable-gpu", "--use-mock-keychain", "--password-store=basic", "--remote-debugging-port=0", `--user-data-dir=${root}`, "about:blank"], { stdio: "ignore" });
  let port;
  for (let n = 0; n < 500 && !port; n++) {
    try { port = Number(readFileSync(join(root, "DevToolsActivePort"), "utf8").split("\n")[0]); } catch {}
    if (!port) await pause();
  }
  assert.ok(port, "Chrome ouvre son propre port CDP");
  cdp = await connectCdp((await pageTarget(port)).webSocketDebuggerUrl);
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
    holdState() { heldState = gate(); return heldState.promise; },
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
    async refusedQuoteRead(failedRefresh = false) {
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
      const stateReading = failedRefresh ? this.holdState() : null;
      releaseSend();
      if (stateReading) (await stateReading)(true);
      return reading;
    },
  };
}

for (const from of ["vide", "b"]) for (const refused of [false, true]) test(`navigation-perdue, retour depuis le fil ${from} pendant une restauration ${refused ? "refusée" : "réussie"}`, { timeout: 60000 }, async t => {
  const app = await fixture(t);
  const release = await app.startRevert();
  await app.run(`() => GrokbotPanels.openConversation('fil-${from}')`);
  if (from === "vide") assert.equal(await app.run(() => document.querySelector(".page-stage").hidden), true);
  else await app.until(() => document.querySelector(".page-rename input").value === "Page b");
  await app.run(() => GrokbotPanels.openConversation("fil-a"));
  const visibleDuring = await app.run(() => !document.querySelector(".page-stage").hidden);
  const pendingDuring = await app.run(() => ({ hidden: button("Restauration…").hidden, disabled: button("Restauration…").disabled }));
  const titleDuring = await app.run(() => document.querySelector(".page-rename input").value);
  release(refused);
  await app.until(() => !button("Restauration…"));
  await app.run(() => GrokbotPanels.refresh());
  assert.equal(visibleDuring, true, "la Page redevient visible avant la réponse de restauration");
  assert.deepEqual(pendingDuring, { hidden: false, disabled: true }, "la restauration en cours reste visible et ne peut pas être relancée");
  assert.equal(titleDuring, "Page a", "le titre identifie la Page rejointe pendant la mutation");
  assert.equal(await app.run(() => document.querySelector(".page-stage").hidden), false, "le panneau reste visible sans nouvelle révision");
  assert.equal(await app.run(() => document.querySelector(".page-canvas").textContent), refused ? "Ancien passage. Passage récent." : "Ancien passage.");
});

for (const [during, refused, failedRead] of [[false, false, false], [false, false, true], [true, false, false], [true, false, true], [false, true, false], [false, true, true]])
test(`reponse-perimee, lecture ${failedRead ? "refusée" : "réussie"} partie ${during ? "pendant" : "avant"} une restauration ${refused ? "refusée" : "réussie"}`, { timeout: 60000 }, async t => {
  const app = await fixture(t);
  let releaseRevert = during ? await app.startRevert() : null;
  const releaseRead = await app.refusedQuoteRead();
  if (!during) releaseRevert = await app.startRevert();
  releaseRevert(refused);
  await app.until(() => !button("Restauration…"));
  if (!refused) await app.until(() => document.querySelector(".page-canvas").textContent === "Ancien passage.");
  await app.run(() => {
    window.observed = [];
    new MutationObserver(() => observed.push(document.querySelector(".page-canvas").textContent))
      .observe(document.querySelector(".page-canvas"), { childList: true, subtree: true });
  });
  releaseRead(failedRead);
  await app.until(() => !GrokbotPanels.busy);
  if (!refused) assert.equal(await app.run(() => observed.some(text => text.includes("Passage récent"))), false, "la version annulée ne doit jamais être repeinte");
  assert.equal(await app.run(() => document.querySelector(".page-canvas").textContent), refused ? "Ancien passage. Passage récent." : "Ancien passage.", "une réponse antérieure ne remplace pas la source courante");
  assert.equal(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").hidden), !refused, "seul un passage encore valide reste cité");
});

for (const edit of ["remove", "same", "valid"]) test(`reponse-perimee, un choix récent ${edit} prime sur la lecture retenue`, { timeout: 60000 }, async t => {
  const app = await fixture(t);
  const releaseRead = await app.refusedQuoteRead();
  if (edit === "same") await app.quote("Passage récent");
  else if (edit === "remove") await app.run(() => document.querySelector(".gb-quote-remove").click());
  else {
    const releaseRevert = await app.startRevert(); releaseRevert();
    await app.until(() => document.querySelector(".page-canvas").textContent === "Ancien passage.");
    await app.quote("Ancien passage");
  }
  releaseRead();
  await app.until(() => !GrokbotPanels.busy);
  assert.equal(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").hidden), edit === "remove");
  if (edit !== "remove") assert.match(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").textContent), edit === "same" ? /Passage récent/ : /Ancien passage/);
});

test("reponse-perimee, un polling identique conserve la citation valide et la sélection récente", { timeout: 60000 }, async t => {
  const app = await fixture(t);
  const releaseRead = await app.refusedQuoteRead();
  await app.run(() => {
    const range = document.createRange(); range.selectNodeContents(document.querySelector(".page-canvas p"));
    getSelection().removeAllRanges(); getSelection().addRange(range); window.selectionNode = getSelection().anchorNode;
  });
  await app.run(() => GrokbotPanels.refresh());
  assert.equal(await app.run(() => getSelection().anchorNode === selectionNode), true, "le polling garde la sélection avant la fin de l’envoi");
  releaseRead();
  await app.until(() => !GrokbotPanels.busy);
  assert.equal(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").hidden), false);
  assert.equal(await app.run(() => selectionNode.isConnected), true, "la fin de l’envoi n’a pas reconstruit le canevas");
  assert.equal(await app.run(() => document.activeElement.matches('.gb-editor[aria-label="Votre message"]')), true, "le composeur reprend le focus après le refus");
});

for (const failedRead of [false, true]) test(`navigation-perdue, lecture tardive ${failedRead ? "refusée" : "réussie"} d’une Page quittée`, { timeout: 60000 }, async t => {
  const app = await fixture(t);
  const waiting = app.holdRead();
  await app.run(() => GrokbotPanels.refresh());
  const releaseRead = await waiting;
  await app.run(() => GrokbotPanels.openConversation("fil-b"));
  await app.until(() => document.querySelector(".page-canvas").textContent === "Autre Page.");
  const count = await app.run(() => parsed.filter(read => read.path === "/api/pages/a").length);
  releaseRead(failedRead);
  await app.until(`() => parsed.filter(read => read.path === '/api/pages/a').length > ${count}`);
  assert.deepEqual(await app.run(() => ({ hidden: document.querySelector(".page-stage").hidden, text: document.querySelector(".page-canvas").textContent, title: document.querySelector(".page-rename input").value })), { hidden: false, text: "Autre Page.", title: "Page b" });
});

for (const refused of [false, true]) test(`navigation-perdue, une restauration tardive ${refused ? "refusée" : "réussie"} ne touche pas la Page courante`, { timeout: 60000 }, async t => {
  const app = await fixture(t);
  const releaseRevert = await app.startRevert();
  await app.run(() => GrokbotPanels.openConversation("fil-b"));
  await app.until(() => document.querySelector(".page-canvas").textContent === "Autre Page.");
  releaseRevert(refused);
  await app.until(() => parsed.some(read => read.path === "/api/pages/revert"));
  assert.deepEqual(await app.run(() => ({ hidden: document.querySelector(".page-stage").hidden, text: document.querySelector(".page-canvas").textContent })), { hidden: false, text: "Autre Page." });
});

for (const failedRead of [false, true]) test(`reponse-perimee, réponse ${failedRead ? "refusée" : "réussie"} reçue pendant la mutation`, { timeout: 60000 }, async t => {
  const app = await fixture(t);
  const releaseRead = await app.refusedQuoteRead();
  const releaseRevert = await app.startRevert();
  releaseRead(failedRead);
  await app.until(() => !GrokbotPanels.busy);
  assert.equal(await app.run(() => document.querySelector(".page-stage").hidden), false);
  releaseRevert();
  await app.until(() => document.querySelector(".page-canvas").textContent === "Ancien passage.");
  assert.equal(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").hidden), true);
});

test("reponse-perimee, le renommage confirmé prime sur une ancienne relecture de citation", { timeout: 60000 }, async t => {
  const app = await fixture(t);
  const releaseRead = await app.refusedQuoteRead(true);
  const waiting = app.holdState();
  await app.run(() => { const input = document.querySelector(".page-rename input"); input.value = "Nouveau titre"; input.dispatchEvent(new Event("change")); });
  const releaseState = await waiting;
  await app.run(() => {
    window.titles = [];
    new MutationObserver(() => titles.push(document.querySelector(".composer-area > .gb-quote-chip").textContent))
      .observe(document.querySelector(".composer-area > .gb-quote-chip"), { childList: true, subtree: true });
  });
  releaseRead();
  await app.until(() => !GrokbotPanels.busy);
  releaseState();
  assert.equal(await app.run(() => titles.some(title => title.includes("Page a"))), false, "le titre confirmé ne doit jamais revenir à son ancienne valeur");
  assert.match(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").textContent), /Nouveau titre/);
});

for (const context of ["deleted", "task", "bot"]) test(`reponse-perimee, le contexte ${context} retire la citation pendant la lecture`, { timeout: 60000 }, async t => {
  const app = await fixture(t);
  const releaseRead = await app.refusedQuoteRead();
  if (context === "deleted") {
    app.state.conversations = app.state.conversations.filter(conversation => conversation.id !== "fil-a");
    app.state.pages.by_bot.grokbot = [];
  } else if (context === "task") app.state.conversations[0].kind = "task";
  else {
    app.state.bots.push({ id: "other", name: "Autre bot", color: "cyan", shape: "galet" });
    app.state.conversations[0].bot_id = "other";
  }
  await app.run(() => GrokbotPanels.refresh());
  assert.equal(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").hidden), true);
  releaseRead();
  await app.until(() => !GrokbotPanels.busy);
  assert.equal(await app.run(() => document.querySelector(".composer-area > .gb-quote-chip").hidden), true, "l’ancienne lecture ne réinstalle pas une citation d’un contexte quitté");
  if (context === "deleted") assert.equal(await app.run(() => document.querySelector(".page-stage").hidden), true);
});
