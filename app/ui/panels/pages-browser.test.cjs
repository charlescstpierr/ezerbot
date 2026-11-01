const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createServer } = require("node:http");
const { spawn } = require("node:child_process");
const { mkdtempSync, readFileSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { chromeBinary, pageTarget } = require("../chrome-binary.cjs");
const { connectCdp } = require("../local-api-client.cjs");

const pause = () => new Promise(resolve => setTimeout(resolve, 20));
const clone = value => JSON.parse(JSON.stringify(value));
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const element = (tag, ...children) => ({ type: "element", tag, children: children.map(child => typeof child === "string" ? { type: "text", text: child } : child) });
const original = {
  id: "a", title: "Forfaits", bot_id: "grokbot", conversation_id: "deleted", status: "ready",
  source_version: "v1", preview: "Forfaits Confort 19 € Famille recommandée", has_previous: false, anchors: [],
  document: { states: [], view: element("article", element("h1", "Forfaits"), element("table", element("tbody", element("tr", element("td", "Confort"), element("td", "19 €")))), element("p", element("strong", "Famille"), " recommandée")) },
};

test("le panneau réel conserve les sélections, annote les passages et isole les restaurations concurrentes", { timeout: 60000 }, async t => {
  const root = mkdtempSync(join(tmpdir(), "ezerbot-pages-ui-"));
  let chrome, cdp;
  let current = clone(original);
  // Un contenu distinct : une restauration tardive de « a » peinte sur « b » se verrait dans le canevas.
  const other = { ...clone(original), id: "b", title: "Autre Page", source_version: "b1", anchors: [], document: { states: [], view: element("article", element("h1", "Autre Page"), element("p", "Contenu distinct")) } };
  const state = { conversations: [{ id: "review" }, { id: "other" }], pages: { by_bot: { grokbot: [current, other].map(page => ({ id: page.id, title: page.title, conversation_id: page.conversation_id, bot_id: page.bot_id })) } } };
  const calls = [];
  let heldRead = null, heldReadPath = "/api/pages/a", heldRevert = null, refuse = false;
  const html = `<!doctype html><html data-theme="light"><head><meta charset="utf-8"><link rel="stylesheet" href="/styles.css"><link rel="stylesheet" href="/pages.css"><style>body{display:block}aside.sidebar{width:200px}.main-panel{height:600px;width:850px}.page-stage{min-height:300px}[hidden]{display:none!important}</style></head><body><aside class="sidebar"><nav id="conversation-list"></nav></aside><main class="main-panel"><div class="topbar"></div><div class="connection-banner"></div><section id="chat-scroll" class="chat-scroll"></section><div class="composer-dock"><div class="composer-area"><div id="composer-input"><div contenteditable="true"></div></div></div></div></main><script>
    window.notices=[]; window.opened=[]; window.reverted=0;
    window.GrokbotPanels={
      register(panel){this.panel=panel},
      async api(path,body){const response=await fetch(path,body===undefined?{}:{method:'POST',body:JSON.stringify(body)});const data=await response.json();if(path==='/api/pages/revert')window.reverted++;if(!response.ok)throw new Error(data.error);return data},
      async refresh(){this.state=await this.api('/state');await this.panel.onState(this.state,{bot:{id:'grokbot'},conversation:{id:'other'}})},
      notify(message){window.notices.push(message)},
      openConversation(id,messageId){if(window.refuseNavigation)return false;window.opened.push([id,messageId]);return this.state.conversations.some(c=>c.id===id)}
    };
    window.repaint=()=>window.GrokbotPanels.refresh();
    window.button=text=>[...document.querySelectorAll('button')].find(b=>b.textContent===text);
  </script><script src="/pages.js"></script></body></html>`;
  const server = createServer(async (request, response) => {
    const send = (data, status = 200) => { response.writeHead(status, { "Content-Type": "application/json" }); response.end(JSON.stringify(data)); };
    if (request.url === "/") { response.end(html); return; }
    const assets = { "/pages.js": "pages.js", "/pages.css": "pages.css", "/styles.css": "../styles.css" };
    if (assets[request.url]) { response.setHeader("Content-Type", request.url.endsWith("js") ? "text/javascript" : "text/css"); response.end(readFileSync(join(__dirname, assets[request.url]))); return; }
    if (request.url === "/state") { send(state); return; }
    const body = [];
    for await (const chunk of request) body.push(chunk);
    calls.push({ path: request.url, body: body.length ? JSON.parse(Buffer.concat(body)) : null });
    if (request.url === "/api/pages/revert") {
      if (heldRevert) { const gate = heldRevert; heldRevert = null; gate.resolve(() => { current = { ...clone(original), anchors: clone(current.anchors) }; send({ page: current }); }); return; }
      if (refuse) { current.has_previous = false; send({ error: "Cette Page n’a pas de version d’avant." }, 400); return; }
      current = { ...clone(original), anchors: clone(current.anchors) };
    }
    const page = request.url === "/api/pages/b" ? other : current;
    if (heldRead && request.url === heldReadPath) { const gate = heldRead; heldRead = null; const snapshot = clone(page); gate.resolve(() => send({ page: snapshot })); return; }
    send({ page });
  });
  t.after(async () => {
    cdp?.close();
    if (chrome && chrome.exitCode === null) { chrome.kill("SIGTERM"); await new Promise(resolve => chrome.once("exit", resolve)); }
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
    rmSync(root, { recursive: true, force: true, maxRetries: 10 });
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  chrome = spawn(chromeBinary(), ["--headless=new", "--no-sandbox", "--disable-gpu", "--use-mock-keychain", "--password-store=basic", "--remote-debugging-port=0", `--user-data-dir=${root}`, "about:blank"], { stdio: "ignore" });
  const port = await (async () => {
    for (let n = 0; n < 500; n++) {
      try { return Number(readFileSync(join(root, "DevToolsActivePort"), "utf8").split("\n")[0]); } catch { await pause(); }
    }
    throw new Error("Chrome n’a pas ouvert de port CDP");
  })();
  cdp = await connectCdp((await pageTarget(port)).webSocketDebuggerUrl);
  await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1140, height: 780, deviceScaleFactor: 1, mobile: false });
  const run = async source => {
    const result = await cdp.send("Runtime.evaluate", { expression: `(${source})()`, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result.value;
  };
  const until = async predicate => { for (let n = 0; n < 300; n++) { if (await run(predicate)) return; await pause(); } throw new Error(`État attendu absent: ${predicate}`); };
  await cdp.send("Page.navigate", { url: `http://127.0.0.1:${server.address().port}/` });
  await until(() => window.GrokbotPanels?.panel);
  await run(async () => { await repaint(); GrokbotPanels.panel.openPage("a"); });
  await until(() => document.querySelector(".page-canvas td"));
  assert.deepEqual(await run(() => [...document.querySelectorAll(".page-stage-bar button")].filter(b => !b.hidden).map(b => b.textContent)), ["Ouvrir en pleine page"]);
  await run(() => {
    window.initialCanvas = document.querySelector(".page-canvas").innerHTML;
    window.selectedNode = document.querySelector(".page-canvas td").firstChild;
    const range = document.createRange(); range.selectNodeContents(selectedNode); getSelection().removeAllRanges(); getSelection().addRange(range);
  });
  current.anchors = [{ conversation_id: "review", message_id: "one", passage: "Confort\t19 €", state: "open" }, { conversation_id: "other", message_id: "two", passage: "Famille", state: "open" }];
  await run(() => repaint());
  assert.deepEqual(await run(() => ({ text: getSelection().toString(), same: getSelection().anchorNode === selectedNode, html: document.querySelector(".page-canvas").innerHTML === initialCanvas, badges: document.querySelectorAll(".page-anchor-number").length })), { text: "Confort", same: true, html: true, badges: 2 });
  assert.equal(await run(() => document.querySelectorAll(".page-anchor-line").length >= 2), true);
  assert.equal(await run(() => { document.querySelector(".main-panel").style.width = "700px"; return document.querySelector(".page-rename").getBoundingClientRect().width >= 140; }), true, "le titre reste lisible quand les commentaires ajoutent un bouton");
  await run(() => { document.querySelector(".main-panel").style.width = "850px"; });
  await run(() => { document.dispatchEvent(new Event("selectionchange")); button("Commenter").click(); });
  assert.match(await run(() => document.querySelector(".composer-area > .gb-quote-chip").textContent), /Confort/);
  await run(() => document.querySelector(".gb-quote-remove").click());
  assert.equal(await run(() => document.querySelector(".composer-area > .gb-quote-chip").hidden), true);
  await run(() => button("Commentaires (2)").click());
  assert.deepEqual(await run(() => [...document.querySelectorAll(".page-comment-state")].map(el => el.textContent)), ["Ouvert", "Ouvert"]);
  await run(() => button("Ouvrir en pleine page").click());
  await run(() => button("Voir dans le fil").click());
  assert.deepEqual(await run(() => opened.at(-1)), ["review", "one"]);
  assert.equal(await run(() => document.querySelector("#chat-scroll").hidden), false);
  await run(() => { window.refuseNavigation = true; button("Voir dans le fil").click(); });
  assert.match(await run(() => notices.at(-1)), /message n’est pas accessible/);
  current.anchors[0].state = "done";
  current.document.view.children[2].children[0].children[0].text = "Duo";
  current.source_version = "v2";
  current.has_previous = true;
  await run(() => repaint());
  assert.deepEqual(await run(() => [...document.querySelectorAll(".page-comment-state")].map(el => el.textContent)), ["Traité", "Traité"]);
  assert.equal(await run(() => document.querySelectorAll(".page-anchor-number").length), 1);

  const read = heldRead = deferred();
  await run(() => { window.staleRead = repaint(); });
  const releaseRead = await read.promise;
  const revert = heldRevert = deferred();
  await run(() => button("Revenir à avant").click());
  const releaseRevert = await revert.promise;
  const readsBefore = calls.filter(call => call.path === "/api/pages/a").length;
  await run(async () => { await repaint(); button("Restauration…").click(); });
  assert.equal(calls.filter(call => call.path === "/api/pages/a").length, readsBefore);
  assert.equal(calls.filter(call => call.path === "/api/pages/revert").length, 1);
  releaseRevert();
  await until(() => !button("Restauration…") && document.querySelector(".page-canvas").textContent.includes("Famille"));
  releaseRead();
  await run(async () => { await staleRead; });
  assert.equal(await run(() => document.querySelector(".page-canvas").innerHTML === initialCanvas), true);
  assert.equal(await run(() => button("Revenir à avant").hidden), true);

  current = { ...clone(original), source_version: "v3", has_previous: true };
  await run(() => repaint());
  refuse = true;
  await run(() => button("Revenir à avant").click());
  await until(() => notices.some(message => message.includes("version d’avant")) && button("Revenir à avant").hidden);
  assert.equal(await run(() => document.querySelector(".page-canvas").innerHTML === initialCanvas), true);
  refuse = false;

  current = { ...clone(original), source_version: "v4", has_previous: true };
  await run(() => repaint());
  const late = heldRevert = deferred();
  await run(() => button("Revenir à avant").click());
  const releaseLate = await late.promise;
  await run(() => { window.refuseNavigation = false; GrokbotPanels.panel.openPage("b"); });
  // Le titre s’affiche avant la lecture de la Page : on attend aussi son contenu.
  await until(() => document.querySelector(".page-rename input").value === "Autre Page" && document.querySelector(".page-canvas h1")?.textContent === "Autre Page");
  const otherCanvas = await run(() => document.querySelector(".page-canvas").innerHTML);
  releaseLate();
  await until(() => reverted === 3);
  assert.equal(await run(() => document.querySelector(".page-rename input").value), "Autre Page");
  assert.equal(await run(() => document.querySelector(".page-canvas").innerHTML), otherCanvas);

  const lateLoad = heldRead = deferred();
  await run(() => { GrokbotPanels.panel.openPage("a"); });
  const releaseLoad = await lateLoad.promise;
  await run(() => GrokbotPanels.panel.openPage("b"));
  // Le titre s’affiche avant la lecture de la Page : on attend aussi son contenu.
  await until(() => document.querySelector(".page-rename input").value === "Autre Page" && document.querySelector(".page-canvas h1")?.textContent === "Autre Page");
  releaseLoad();
  await pause();
  assert.equal(await run(() => document.querySelector(".page-rename input").value), "Autre Page");
  assert.equal(await run(() => document.querySelector(".page-canvas h1")?.textContent), "Autre Page");

  current = { ...clone(original), source_version: "v5", has_previous: true };
  await run(() => GrokbotPanels.panel.openPage("a"));
  await until(() => button("Revenir à avant") && !button("Revenir à avant").hidden);
  const returning = heldRevert = deferred();
  await run(() => button("Revenir à avant").click());
  const releaseReturning = await returning.promise;
  const crossRead = heldRead = deferred(); heldReadPath = "/api/pages/b";
  await run(() => GrokbotPanels.panel.openPage("b"));
  const releaseCrossRead = await crossRead.promise;
  await run(() => GrokbotPanels.panel.openPage("a"));
  releaseCrossRead();
  await pause();
  assert.equal(await run(() => document.querySelector(".page-canvas").textContent), "");
  releaseReturning();
  await until(() => reverted === 4 && document.querySelector(".page-canvas td"));
  assert.equal(await run(() => document.querySelector(".page-rename input").value), "Forfaits");
});
