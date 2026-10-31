const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createServer } = require("node:http");
const { spawn } = require("node:child_process");
const { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } = require("node:fs");
const { rm } = require("node:fs/promises");
const { join, resolve, extname, isAbsolute } = require("node:path");
const { chromeBinary } = require("./chrome-binary.cjs");
const { connectCdp } = require("./local-api-client.cjs");

const clone = value => JSON.parse(JSON.stringify(value));
const pause = () => new Promise(done => setTimeout(done, 20));
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const created = "2026-10-25T12:00:00.000Z";
const traces = ["trace-command-565", "/fixture/cwd-565", "trace-output-565", "opaque-json-565", "trace-tool-565", "trace-server-565", "Préparation d’un outil", "Lecture automatique de démonstration", "Actions disponibles", "Tâche sur votre Mac"];
const user = (id, body) => ({ id, role: "user", kind: "message", body, state: "echoed", created_at: created });
const reply = (body, state = "streaming", parts = [{ body, before: "tool-a" }]) => ({
  id: "reply-main", role: "assistant", kind: "reply", body, state, created_at: created,
  payload: { run_id: "run-main", parts },
});
const run = (status = "running", run_id = "run-main", conversation_id = "parent", bot_id = "grokbot") => ({ run_id, conversation_id, bot_id, status });
const tool = (id, name = "trace-tool-565", outcome = { status: "done", output: { stdout: "trace-output-565", data: "opaque-json-565" } }, run_id = "run-main") => ({
  id, role: "assistant", kind: "tool", body: "", state: "echoed", created_at: created,
  payload: { run_id, run_status: "running", authorized_by: { kind: "yolo" },
    call: { call_id: `call-${id}`, name, arguments: { description: "Lecture automatique de démonstration", command: "printf trace-command-565", cwd: "/fixture/cwd-565", data: "opaque-json-565", timeout_ms: 10000, output_limit: 2000 } },
    ...(name === "trace-tool-565" ? { connector: { server: "trace-server-565", tool: name } } : {}), outcome },
});
const approval = (id, status = "pending", run_id = "run-main") => ({
  id, role: "assistant", kind: "approval", body: "", state: "echoed", created_at: created,
  payload: { id: `request-${id}`, entry_id: `entry-${id}`, run_id,
    kind: { kind: "approval", tool: "run_shell", args_digest: `digest-${id}`, expires_at: Date.now() + 3600000 },
    arguments: { command: "printf trace-command-565", cwd: "/fixture/cwd-565", timeout_ms: 10000, output_limit: 2000 },
    status: { status }, route: { target_conversation_id: "parent", requesting_bot_id: "grokbot" } },
});
const form = () => ({
  id: "form-child", role: "assistant", kind: "form", body: "", state: "echoed", created_at: created,
  payload: { id: "request-child", entry_id: "entry-child", run_id: "run-child", status: { status: "pending" },
    route: { target_conversation_id: "parent", requesting_bot_id: "helper" },
    kind: { kind: "form", fields: [{ name: "city", label: "Ville demandée", required: true, field_type: { kind: "text" } }] } },
});

async function fixture(t, phone) {
  assert.ok(process.env.TMPDIR, "le lanceur fournit TMPDIR pour le profil isolé");
  const root = mkdtempSync(join(resolve(process.env.TMPDIR), "thread-conversation-"));
  const uiRoot = resolve(__dirname);
  const state = {
    account: { name: "Espace de test" }, mode: "simulated", state_revision: 1,
    bots: [{ id: "grokbot", name: "Ezerbot", color: "citron", shape: "galet", is_default: true }, { id: "helper", name: "Clément", color: "menthe", shape: "galet" }],
    conversations: [{ id: "parent", title: "Lecture du dossier", bot_id: "grokbot", kind: "chat", member_ids: ["grokbot"], messages: [user("question", "Lis l’issue #557."), reply("Je consulte le dossier."), tool("tool-a")] }],
    preferences: { theme: phone ? "dark" : "light", accent: "citron", model: "gpt-test" },
    provider: { connected: true, signed_in: true, welcomed: true, signin: "idle", backend: "sharing", models_ready: true, models: [{ slug: "gpt-test", name: "Modèle de test", efforts: [], tiers: [] }], connections: {} },
    engine: { runs: [run()] }, forms: { requests: [] }, delegation: { delegations: [], active_by_conversation: {} }, pages: { by_bot: {} },
  };
  const calls = [], held = new Map(), pendingHolds = new Set();
  let chrome, cdp, failState = false;
  const parent = () => state.conversations.find(item => item.id === "parent");
  const panelBundle = extension => readdirSync(join(uiRoot, "panels")).filter(name => extname(name) === `.${extension}`).sort().map(name => {
    const source = readFileSync(join(uiRoot, "panels", name), "utf8");
    return extension === "js" ? `;(function () {\n${source}\n})();\n` : `${source}\n`;
  }).join("");
  const server = createServer(async (request, response) => {
    const path = new URL(request.url, "http://fixture.test").pathname;
    const json = (data, status = 200) => { response.writeHead(status, { "Content-Type": "application/json", "Cache-Control": "no-store" }); response.end(JSON.stringify(data)); };
    if (path.startsWith("/api/generated/") || path.startsWith("/api/attachments/")) {
      response.writeHead(200, { "Content-Type": "image/svg+xml" });
      response.end('<svg xmlns="http://www.w3.org/2000/svg" width="80" height="50"><rect width="80" height="50" fill="#a4b62c"/></svg>'); return;
    }
    if (!path.startsWith("/api/")) {
      if (["/panels.js", "/panels.css"].includes(path)) {
        const extension = path.endsWith(".js") ? "js" : "css";
        response.writeHead(200, { "Content-Type": extension === "js" ? "text/javascript" : "text/css" }); response.end(panelBundle(extension)); return;
      }
      const publicFile = ["/sw.js", "/manifest.webmanifest", "/icon-192.png", "/icon-512.png"].includes(path);
      const base = publicFile ? resolve(uiRoot, "../web/public") : uiRoot;
      const file = resolve(base, path === "/" ? "index.html" : path.slice(1));
      if (!file.startsWith(`${base}/`) || !existsSync(file)) { response.writeHead(404); response.end(); return; }
      const types = { ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".woff2": "font/woff2", ".webmanifest": "application/manifest+json" };
      response.writeHead(200, { "Content-Type": types[extname(file)] || "text/html" }); response.end(readFileSync(file)); return;
    }
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = chunks.length ? JSON.parse(Buffer.concat(chunks)) : null;
    const call = { path, body }; calls.push(call);
    const holding = held.get(path);
    if (holding) {
      held.delete(path); holding.seen.resolve(call);
      const result = await holding.release.promise;
      if (result) { json(result.body, result.status); return; }
    }
    if (path === "/api/state") { if (failState) response.destroy(); else json(state); return; }
    if (path === "/api/ping") { json({ revision: state.state_revision }); return; }
    if (path === "/api/preferences") { Object.assign(state.preferences, body); json({ preferences: state.preferences }); return; }
    if (path === "/api/approvals/resolve" || path === "/api/forms/submit") {
      const request = state.forms.requests.find(item => item.entry_id === body.entry_id);
      if (!request) { json({ error: "Demande inconnue." }, 404); return; }
      request.status = { status: body.decision === "deny" ? "denied" : "resolved" };
      request.decision = body.decision;
      request.receipt = { resolution_id: `resolution-${request.entry_id}`, wake: { status: "delivered", run_id: request.run_id, first_event_seq: 1 } };
      state.state_revision += 1;
      json({ accepted: true, receipt: request.receipt }); return;
    }
    if (path === "/api/local/tasks/stop" || path === "/api/stop") {
      const message = parent().messages.find(item => item.kind === "tool" && item.payload.run_id === (body.run_id || body.message_id));
      if (message) { message.payload.outcome = { status: "done", output: { background: true, status: "arrêtée", stdout: "trace-output-565" } }; message.payload.run_status = "cancelled"; }
      for (const reply of parent().messages.filter(item => item.kind === "reply" && item.payload?.run_id === body.message_id)) reply.state = "stopped";
      state.engine.runs = state.engine.runs.filter(item => item.run_id !== (body.run_id || body.message_id));
      state.state_revision += 1;
      json({ conversation: parent() }); return;
    }
    if (path === "/api/send") {
      const conversation = state.conversations.find(item => item.id === body.conversation_id);
      conversation.messages.push(user(body.message_id, body.body)); state.state_revision += 1;
      json({ conversation }); return;
    }
    if (path === "/api/notifications") { json({ unread: 0, items: [], settings: {} }); return; }
    json({});
  });
  t.after(async () => {
    for (const holding of pendingHolds) holding.release.resolve();
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
    if (server.listening) await new Promise(done => server.close(done));
    await rm(root, { recursive: true, force: true, maxRetries: 20, retryDelay: 50 });
  });
  await new Promise(done => server.listen(0, "127.0.0.1", done));
  chrome = spawn(chromeBinary(), ["--headless=new", "--no-sandbox", "--disable-gpu", "--disable-background-networking", "--disable-sync", "--no-first-run", "--use-mock-keychain", "--password-store=basic", "--no-proxy-server", "--host-resolver-rules=MAP thread-fixture.ts.net 127.0.0.1", `--unsafely-treat-insecure-origin-as-secure=http://thread-fixture.ts.net:${server.address().port}`, "--remote-debugging-port=0", `--user-data-dir=${root}`, "about:blank"], { stdio: "ignore" });
  let port, startupError;
  chrome.once("error", error => { startupError = error; });
  for (let n = 0; n < 500 && !port && !startupError; n += 1) {
    try { port = Number(readFileSync(join(root, "DevToolsActivePort"), "utf8").split("\n")[0]); } catch {}
    if (!port) await pause();
  }
  if (startupError) throw startupError;
  assert.ok(port, "Chrome ouvre un port CDP dans son profil isolé");
  const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then(response => response.json());
  cdp = await connectCdp(targets.find(target => target.type === "page").webSocketDebuggerUrl);
  const browserErrors = [];
  cdp.on("Runtime.exceptionThrown", ({ exceptionDetails }) => browserErrors.push(exceptionDetails.exception?.description || exceptionDetails.text));
  await cdp.send("Runtime.enable");
  await cdp.send("Emulation.setDeviceMetricsOverride", { width: phone ? 390 : 1280, height: phone ? 844 : 900, deviceScaleFactor: 1, mobile: phone });
  const evaluate = async source => {
    const result = await cdp.send("Runtime.evaluate", { expression: `(${source})()`, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result.value;
  };
  const until = async source => {
    for (let n = 0; n < 300; n += 1) { if (await evaluate(`async () => Boolean(await (${source})())`)) return; await pause(); }
    throw new Error(`État absent : ${source}`);
  };
  const refresh = async () => {
    state.state_revision += 1;
    await evaluate(() => GrokbotPanels.refresh());
    await evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));
  };
  await cdp.send("Page.navigate", { url: `http://${phone ? "thread-fixture.ts.net" : "127.0.0.1"}:${server.address().port}/` });
  try {
    await until(() => window.GrokbotPanels?.connected && document.querySelector("#conversation-list .conversation-item"));
    await evaluate(() => document.querySelector("#conversation-list .conversation-item").click());
    await until(() => window.GrokbotPanels?.connected && window.GrokbotEngine && window.GrokbotForms && window.GrokbotDelegation && document.querySelector(".gb-thread .gb-markdown"));
  } catch (error) {
    const observed = await evaluate(() => ({ connected: window.GrokbotPanels?.connected, engine: !!window.GrokbotEngine, forms: !!window.GrokbotForms, delegation: !!window.GrokbotDelegation, title: document.title, text: document.body.innerText.slice(0, 1600) }));
    throw new Error(`${error.message}\n${JSON.stringify({ observed, browserErrors, requests: calls.map(call => call.path) })}`);
  }
  assert.equal(await evaluate(() => EzerbotIPhone.remote), phone, "iphone.js choisit le vrai parcours distant selon l’adresse");
  assert.equal(await evaluate(() => typeof crypto.randomUUID), "function", "le serveur fixture dispose des API du contexte HTTPS de l’iPhone");
  assert.equal(await evaluate(() => document.documentElement.dataset.theme), phone ? "dark" : "light");
  return {
    state, calls, run: evaluate, until, refresh, parent,
    setMessages(messages, runs = []) {
      state.conversations = [{ ...parent(), messages }]; state.engine.runs = runs;
      state.delegation = { delegations: [], active_by_conversation: {} };
      state.forms.requests = messages.filter(item => ["approval", "form", "secret"].includes(item.kind)).map(item => item.payload);
    },
    hold(path) {
      const holding = { seen: deferred(), release: deferred() }; held.set(path, holding);
      pendingHolds.add(holding);
      const seen = (async () => {
        let timer;
        try {
          return await Promise.race([holding.seen.promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Requête absente : ${path}`)), 5000); })]);
        } finally { clearTimeout(timer); }
      })();
      return { seen, release: result => { pendingHolds.delete(holding); holding.release.resolve(result); } };
    },
    async type(selector, text) {
      await evaluate(`() => document.querySelector(${JSON.stringify(selector)}).focus()`);
      await cdp.send("Input.insertText", { text });
    },
    async enter(selector) {
      await evaluate(`() => document.querySelector(${JSON.stringify(selector)}).focus()`);
      assert.equal(await evaluate(`() => document.activeElement.matches(${JSON.stringify(selector)})`), true, "le contrôle reçoit le focus clavier");
      await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: "\r" });
      await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    },
    async thinking(expected) {
      await until(`() => [...document.querySelectorAll('.gb-thread [role="status"]')].filter(node => node.textContent.trim() === 'Réfléchit').length === ${expected ? 1 : 0}`);
      assert.equal(await evaluate(() => GrokbotPanels.busy), false, "l’activité ne dépend pas d’un envoi HTTP en cours");
    },
    async cleanThread() {
      const dom = await evaluate(() => {
        const surface = document.querySelector(".gb-conversation").cloneNode(true);
        surface.querySelectorAll('[data-card-kind="approval"]').forEach(request => request.remove());
        return { html: surface.outerHTML, tools: surface.querySelectorAll('[data-card-kind="tool"]:not(.gb-generated-image-card), .gb-local-command, .gb-engine-data').length };
      });
      assert.equal(dom.tools, 0, "aucune carte ni donnée d’exécution automatique ne subsiste dans toute la conversation");
      for (const trace of traces) assert.equal(dom.html.includes(trace), false, `le DOM de la conversation, hors consentement canonique, ne contient pas ${trace}`);
      const document = await cdp.send("DOM.getDocument");
      const { nodeId } = await cdp.send("DOM.querySelector", { nodeId: document.root.nodeId, selector: ".gb-conversation" });
      const ax = await cdp.send("Accessibility.queryAXTree", { nodeId });
      assert.ok(ax.nodes.some(node => node.name?.value), "le contrôle lit un arbre d’accessibilité non vide");
      const requests = await cdp.send("DOM.querySelectorAll", { nodeId, selector: '[data-card-kind="approval"]' });
      const canonical = new Set();
      for (const request of requests.nodeIds) {
        const requestAX = await cdp.send("Accessibility.queryAXTree", { nodeId: request });
        for (const node of requestAX.nodes) canonical.add(node.nodeId);
      }
      const accessible = JSON.stringify(ax.nodes.filter(node => !canonical.has(node.nodeId)));
      for (const trace of traces) assert.equal(accessible.includes(trace), false, `l’accessibilité de toute la conversation, hors consentement canonique, ne contient pas ${trace}`);
    },
    async capture(label) {
      if (!phone || !process.env.I565_SCREENSHOTS) return;
      const directory = process.env.I565_SCREENSHOTS;
      assert.ok(isAbsolute(directory), "I565_SCREENSHOTS désigne un dossier de preuves absolu");
      mkdirSync(directory, { recursive: true });
      for (const theme of ["light", "dark"]) {
        await evaluate(() => document.getElementById("settings-button").click());
        await until(() => document.getElementById("settings-dialog").open);
        await evaluate(`() => document.querySelector('[data-theme-choice="${theme}"]').click()`);
        await until(`() => document.documentElement.dataset.theme === '${theme}' && !document.querySelector('[data-theme-choice="${theme}"]').disabled`);
        await evaluate(() => document.querySelector('#settings-dialog [data-close-dialog]').click());
        await evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));
        const { data } = await cdp.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
        const name = `iphone-${label}-${theme}`;
        writeFileSync(join(directory, `${name}.png`), Buffer.from(data, "base64"));
        writeFileSync(join(directory, `${name}.txt`), await evaluate(() => document.body.innerText));
        writeFileSync(join(directory, `${name}.action.json`), JSON.stringify({ surface: "Chromium mobile émulé, vrai iphone.js distant, sans appareil physique ni jumelage réel", width: 390, height: 844, theme, scenario: label, action: "Paramètres, Apparence, choix du thème, fermeture des paramètres", state_revision: state.state_revision }, null, 2));
      }
    },
    async offline() { failState = true; await evaluate(() => GrokbotPanels.refresh().catch(() => {})); await until(() => !GrokbotPanels.connected); },
    async online() { failState = false; await refresh(); },
  };
}

for (const phone of [false, true]) test(`conversation réelle ${phone ? "iPhone distant 390 × 844 sombre" : "Mac 1280 × 900 clair"}`, { timeout: 120000 }, async t => {
  const app = await fixture(t, phone);
  await t.test("les phrases restent ordonnées pendant les appels et les transitions du moteur", async () => {
    const first = "Je consulte le dossier.", second = "Je recoupe les informations.", final = "Voici le résultat demandé.";
    const internalTools = ["read_file", "list_directory", "write_file", "document_action", "computer_screenshot", "computer_move", "computer_click", "computer_scroll", "computer_type", "computer_key", "create_document", "generate_image"];
    const saved = [user("question", "Lis l’issue #557."), reply(first), tool("tool-a"), tool("tool-b", "run_shell"), approval("approval-old", "resolved"),
      ...internalTools.map((name, index) => tool(`internal-${index}`, name, { status: "done", output: { text: "trace-output-565", data: "opaque-json-565" } })),
      { id: "summary", role: "assistant", kind: "summary", payload: { run_id: "run-main", text: "trace-output-565" } },
      { id: "task", role: "assistant", kind: "task", payload: { run_id: "run-main", status: "running", task_status: { status: "working" } } }];
    saved[1].notice = "Préparation d’un outil : trace-tool-565";
    app.setMessages(saved, [run()]); await app.refresh();
    await app.thinking(true); await app.cleanThread();
    assert.match(await app.run(() => document.querySelector(".gb-thread").textContent), /Lis l’issue #557\./);
    assert.deepEqual(await app.run(() => [...document.querySelectorAll(".gb-thread .gb-markdown")].map(node => node.textContent.trim())), [first]);
    saved[1] = reply(`${first}\n${second}`, "streaming", [{ body: first, before: "tool-a" }, { body: second, before: "tool-b" }]);
    await app.refresh(); await app.thinking(true);
    const pending = tool("tool-c", "run_shell"); delete pending.payload.outcome;
    saved.push(pending); await app.refresh(); await app.thinking(true); await app.cleanThread();
    assert.equal(await app.run(() => document.querySelector(".gb-interventions")), null, "un appel automatique au premier plan ne devient pas une liste d’actions voisine");
    assert.deepEqual(await app.run(() => [...document.querySelectorAll(".gb-thread .gb-markdown")].map(node => node.textContent.trim())), [first, second], "plusieurs appels sans phrase intermédiaire gardent les ancres des fragments");
    const revised = "Je consulte maintenant le dossier.";
    saved[1] = reply(`${revised}\n${second}\n${final}`, "echoed", [{ body: revised, before: "tool-a" }, { body: second, before: "tool-b" }, { body: final }]);
    await app.refresh(); await app.thinking(true);
    assert.deepEqual(await app.run(() => [...document.querySelectorAll(".gb-thread .gb-markdown")].map(node => node.textContent.trim())), [revised, second, final], "les révisions remplacent leur fragment sans déplacer le résultat final");
    assert.deepEqual(await app.run(() => GrokbotPanels.state.conversations.find(item => item.id === "parent").messages), clone(saved), "la présentation conserve les appels et résultats dans le snapshot source");
    await app.capture("active");
    for (const status of ["finished", "failed", "cancelled", "awaiting_approval", "awaiting_human_input", "queued", "interrupted"]) {
      const terminal = ["finished", "failed", "cancelled"].includes(status);
      app.state.engine.runs = terminal ? [] : [run(status)];
      saved[1].state = status === "failed" ? "failed" : status === "cancelled" ? "stopped" : status === "queued" ? "queued" : "echoed";
      if (status === "failed") saved[1].error = "Je n’ai pas pu terminer cette lecture.";
      else delete saved[1].error;
      await app.refresh(); await app.thinking(false); await app.cleanThread();
      assert.match(await app.run(() => document.querySelector(".gb-thread").textContent), /Voici le résultat demandé\./, status);
      if (status === "failed") assert.match(await app.run(() => document.querySelector(".gb-thread").textContent), /La réponse n’a pas abouti\. Réessayez\./);
      if (status === "cancelled") assert.match(await app.run(() => document.querySelector(".gb-thread").textContent), /Réponse arrêtée/);
    }
    saved[1].state = "failed";
    saved[1].error = '{"command":"trace-command-565","cwd":"/fixture/cwd-565","output":"trace-output-565","data":"opaque-json-565"}';
    app.state.engine.runs = [];
    await app.refresh(); await app.thinking(false); await app.cleanThread();
    assert.match(await app.run(() => document.querySelector(".gb-thread").textContent), /La réponse n’a pas abouti\. Réessayez\./, "l’erreur technique devient un problème lisible");
    assert.equal(await app.run(() => GrokbotPanels.state.conversations[0].messages.find(item => item.id === "reply-main").error), saved[1].error, "le diagnostic brut reste dans les données internes");
    await app.capture("failed");
    for (const error_kind of ["signin", "signin_expired", "limit", "local_connection", "context_budget"]) {
      saved[1].error_kind = error_kind;
      await app.refresh(); await app.cleanThread();
      if (error_kind === "limit") assert.equal(await app.run(() => document.querySelector('.gb-thread a[href="https://chatgpt.com/codex/settings/usage"]').textContent), "Gérer l’utilisation");
      if (error_kind === "context_budget") assert.match(await app.run(() => document.querySelector(".gb-thread").textContent), /Raccourcissez le texte ou joignez moins de documents/);
    }
    delete saved[1].error_kind;
    const failures = ["run_shell", "trace-tool-565", ...internalTools].map((name, index) => tool(`failure-${index}`, name, { status: "failed", reason: saved[1].error }));
    app.setMessages([user("question", "Explique les échecs."), ...failures]);
    await app.refresh(); await app.cleanThread();
    assert.equal(await app.run(() => document.querySelectorAll(".gb-conversation-problem").length), failures.length, "chaque chemin d’exécution en échec garde une phrase humaine");
    const interrupted = tool("unknown", "run_shell", { status: "failed", reason: "Résultat inconnu après interruption. Action non relancée ; vérifiez son effet avant de réessayer." });
    interrupted.payload.code = "uncertain";
    const exhausted = { id: "task-budget", role: "assistant", kind: "task", created_at: created, payload: { run_id: "run-main", event_kind: "turn_ended", status: "finished", turn_end: "budget_exhausted", routine: true, task_status: { status: "working" } } };
    app.setMessages([user("question", "Range le dossier."), interrupted, exhausted]);
    await app.refresh(); await app.cleanThread();
    const warnings = await app.run(() => [...document.querySelectorAll(".gb-conversation-problem")].map(note => note.textContent));
    assert.equal(warnings.length, 2);
    assert.match(warnings[0], /son résultat est inconnu\. Vérifiez son effet avant de réessayer\./, "le fil n’affirme pas un échec qu’il ignore");
    assert.match(warnings[1], /Le budget de ce tour est épuisé/, "une routine arrêtée par le budget ne laisse pas un fil muet");
    app.setMessages(saved);
    delete saved[1].error;
    app.state.engine.runs = [run()]; saved[1].state = "streaming"; await app.refresh(); await app.thinking(true);
    await app.offline(); await app.thinking(false); await app.cleanThread();
    assert.equal(await app.run(() => GrokbotPanels.state.conversations[0].messages.find(item => item.id === "reply-main").state), "streaming", "la perte du moteur ne réécrit pas le message enregistré");
    await app.online(); await app.thinking(true);
    app.state.provider.connected = false; await app.refresh(); await app.thinking(true);
    for (const status of ["queued", "interrupted"]) { app.state.engine.runs = [run(status)]; await app.refresh(); await app.thinking(false); }
    app.state.provider.connected = true;
    const document = tool("document", "create_document", { status: "done", output: { id: "generated_565", name: "Synthèse.pdf", format: "pdf", byte_length: 100, pages: 1, preview_url: "/api/generated/generated_565/preview" } });
    const image = tool("image", "generate_image", { status: "done", output: {} });
    image.attachments = [{ id: "image_565", name: "Illustration demandée", caption: "Illustration demandée", mime_type: "image/png", url: "/api/attachments/image_565" }];
    app.setMessages([user("question", "Lis l’issue #557."), reply("Je ne peux pas effectuer cette action.", "echoed", [{ body: "Je ne peux pas effectuer cette action." }]), document, image,
      { id: "memory", role: "assistant", kind: "memory_auto", state: "echoed", created_at: created, payload: { fact_id: "fact-565", active: true, text: "Je préfère une synthèse courte." } }]);
    await app.refresh(); await app.thinking(false); await app.cleanThread();
    await app.until(() => document.querySelector('.gb-thread [data-card-kind="generated-document"]') && document.querySelector(".gb-thread .gb-generated-image-card img") && document.querySelector('.gb-thread [data-card-kind="memory_auto"]'));
    assert.deepEqual(await app.run(() => { const image = document.querySelector(".gb-thread .gb-generated-image-card img"); return { source: new URL(image.src).pathname, caption: image.alt }; }), { source: "/api/attachments/image_565", caption: "Illustration demandée" });
    const retained = await app.run(() => document.querySelector(".gb-thread").textContent);
    assert.match(retained, /Je ne peux pas effectuer cette action\./); assert.match(retained, /Synthèse\.pdf/); assert.match(retained, /Je préfère une synthèse courte\./);
    const completion = { id: "task-done", role: "assistant", kind: "task", payload: { run_id: "run-main", event_kind: "task_changed", status: "running", task_status: { status: "done", summary: "Le complément demandé est prêt." } } };
    app.setMessages([user("question", "Prépare le complément."), completion, { ...clone(completion), id: "task-ended", payload: { ...clone(completion.payload), event_kind: "turn_ended", status: "finished" } }]);
    await app.refresh(); await app.thinking(false); await app.cleanThread();
    assert.equal(await app.run(() => document.querySelectorAll(".gb-conversation-completion").length), 1, "le résultat déclaré reste visible une seule fois sans carte d’état");
    assert.match(await app.run(() => document.querySelector(".gb-thread").textContent), /Le complément demandé est prêt\./);
    completion.payload.status = "failed";
    await app.refresh(); await app.thinking(false); await app.cleanThread();
    assert.equal(await app.run(() => document.querySelectorAll(".gb-conversation-completion").length), 1);
    assert.match(await app.run(() => document.querySelector(".gb-thread").textContent), /Le dernier échange n’a pas abouti\./, "un échec du tour ne fait pas perdre son résultat déclaré");
    assert.equal(await app.run(() => { const ids = [...document.querySelectorAll(".gb-thread article[data-message-id]")].map(node => node.dataset.messageId); return new Set(ids).size === ids.length; }), true);
  });

  await t.test("les interventions gardent le consentement et les identités des contrôles", async () => {
    for (const decision of ["deny", "allow_once"]) {
      const request = approval(`approval-${decision}`, "pending", "run-approval");
      app.setMessages([user("question", "Continue la lecture."), reply("Je poursuis la vérification.", "echoed", [{ body: "Je poursuis la vérification." }]), request], [run(), run("awaiting_approval", "run-approval")]);
      await app.refresh(); await app.thinking(true); await app.cleanThread();
      const card = `.gb-interventions [data-message-id="${request.id}"]`;
      await app.until(`() => document.querySelector(${JSON.stringify(card)})`);
      assert.equal(await app.run(() => document.querySelector(".gb-interventions").parentElement === document.querySelector(".gb-thread").parentElement), true, "les interventions sont à côté du fil");
      assert.match(await app.run(`() => document.querySelector(${JSON.stringify(card)}).textContent`), /printf trace-command-565/, "le consentement montre encore la commande exacte");
      const selector = `${card} fieldset button${decision === "deny" ? ":nth-child(2)" : ":first-child"}`;
      const holding = app.hold("/api/approvals/resolve"); await app.enter(selector);
      const call = await holding.seen;
      assert.deepEqual(call.body, { entry_id: request.payload.entry_id, submission_id: `ui:${request.payload.entry_id}:forms.decide:${JSON.stringify(decision)}`, conversation_id: "parent", decision }, "la décision transmet l’entrée canonique sans reconstruire les arguments");
      assert.equal(await app.run(`() => [...document.querySelectorAll(${JSON.stringify(`${card} fieldset button`)})].every(button => button.matches(':disabled'))`), true, "une décision en cours ne peut pas être soumise deux fois");
      holding.release(); await app.until(`() => !document.querySelector(${JSON.stringify(card)})`);
      await app.until(() => document.activeElement !== document.body && !document.activeElement.matches(":disabled"));
      await app.thinking(true); await app.cleanThread();
    }
    const background = tool("background", "run_shell", { status: "done", output: { background: true, status: "en cours", stdout: "trace-output-565", stderr: "opaque-json-565" } }, "run-shell");
    app.setMessages([user("question", "Lance ce programme."), reply("Le programme attend votre réponse.", "echoed", [{ body: "Le programme attend votre réponse." }]), background]);
    await app.refresh(); await app.thinking(false); await app.cleanThread();
    await app.until(() => document.querySelector('.gb-interventions [data-message-id="background"] .gb-local-stop'));
    assert.equal(await app.run(() => Boolean(document.querySelector(".gb-local-input"))), !phone, "la saisie du programme conserve son périmètre Mac");
    assert.equal(await app.run(() => document.querySelector(".gb-interventions").textContent.includes("trace-command-565")), false);
    if (!phone) {
      await app.type('.gb-local-input input[aria-label="Saisie pour la tâche"]', "oui");
      const refusal = app.hold("/api/local/tasks/input"); await app.enter(".gb-local-input button");
      assert.deepEqual((await refusal.seen).body, { run_id: "run-shell", call_id: "call-background", line: "oui" });
      refusal.release({ status: 403, body: { error: "Le programme n’accepte plus de saisie." } });
      await app.until(() => document.querySelector(".gb-local-failure")?.textContent.includes("Le programme n’accepte plus de saisie.") && !document.querySelector(".gb-local-input button").disabled);
      assert.equal(await app.run(() => document.querySelector(".gb-local-input input").value), "oui", "une saisie refusée garde le brouillon et l’explication");
      const holding = app.hold("/api/local/tasks/input"); await app.enter(".gb-local-input button");
      assert.deepEqual((await holding.seen).body, { run_id: "run-shell", call_id: "call-background", line: "oui" });
      assert.equal(await app.run(() => document.querySelector(".gb-local-input button").disabled), true);
      holding.release(); await app.until(() => document.querySelector(".gb-local-input input").value === "" && !document.querySelector(".gb-local-input button").disabled);
    }
    const stop = app.hold("/api/local/tasks/stop"); await app.enter(".gb-interventions .gb-local-stop");
    assert.deepEqual((await stop.seen).body, { run_id: "run-shell", call_id: "call-background" });
    assert.equal(await app.run(() => document.querySelector(".gb-interventions .gb-local-stop").disabled), true);
    stop.release(); await app.until(() => !document.querySelector(".gb-local-stop, .gb-local-input"));
    await app.until(() => document.activeElement !== document.body && !document.activeElement.matches(":disabled"));
    await app.cleanThread();
    const foreground = tool("foreground", "run_shell", undefined, "run-shell-live");
    delete foreground.payload.outcome;
    const foregroundReply = { ...reply("Je lis le dossier."), id: "reply-run-shell-live", payload: { run_id: "run-shell-live", parts: [{ body: "Je lis le dossier." }] } };
    app.setMessages([user("question", "Lis le dossier."), foregroundReply, foreground], [run("running", "run-shell-live")]);
    await app.refresh(); await app.thinking(true); await app.cleanThread();
    assert.equal(await app.run(() => document.querySelector(".gb-interventions")), null);
    await app.until(() => document.getElementById("send-button").dataset.mode === "stop" && !document.getElementById("send-button").disabled);
    foregroundReply.state = "echoed";
    await app.refresh(); await app.thinking(true); await app.cleanThread();
    assert.equal(await app.run(() => document.getElementById("send-button").dataset.mode), "stop", "un run actif reste arrêtable après la dernière phrase du bot");
    const cancel = app.hold("/api/stop"); await app.enter("#send-button");
    assert.deepEqual((await cancel.seen).body, { conversation_id: "parent", message_id: "run-shell-live" });
    cancel.release(); await app.until(() => !document.querySelector(".gb-local-stop")); await app.thinking(false);
  });

  await t.test("un enfant actif ou en attente humaine laisse le parent envoyer", async () => {
    const edge = { id: "delegated-565", parent_conversation_id: "parent", conversation_id: "child", parent_run_id: "run-main", child_run_id: "run-child", parent_bot: "grokbot", child_bot: "helper", objective: "Préparer le complément", status: { status: "running" } };
    app.setMessages([user("question", "Prépare une synthèse."), reply("Clément prépare le complément.", "echoed", [{ body: "Clément prépare le complément." }]), { id: "delegation", role: "assistant", kind: "delegation", body: "", state: "echoed", created_at: created, payload: edge }], [run("awaiting_children"), run("running", "run-child", "child", "helper")]);
    app.state.conversations.push({ id: "child", parent_id: "parent", title: "Complément de Clément", bot_id: "helper", member_ids: ["helper"], kind: "task", messages: [user("child-question", "Prépare le complément.")] });
    app.state.delegation = { delegations: [edge], active_by_conversation: { parent: 1 } };
    await app.refresh();
    for (const waiting of [false, true]) {
      if (waiting) {
        const request = form(); app.parent().messages.push(request); app.state.forms.requests = [request.payload];
        app.state.engine.runs = [run("awaiting_children"), run("awaiting_human_input", "run-child", "child", "helper")];
        await app.refresh();
        await app.until(() => document.querySelector('[data-message-id="form-child"]'));
        assert.match(await app.run(() => document.querySelector('[data-message-id="form-child"]').textContent), /Demande de Clément/);
      }
      assert.match(await app.run(() => document.querySelector(".gb-delegation-bar").textContent), /En attente de 1 sous-tâche/);
      assert.match(await app.run(() => document.querySelector('.gb-thread [data-card-kind="delegation"]').textContent), /Sous-tâche · Clément/, "la carte conserve l’identité du bot enfant");
      assert.equal(await app.run(() => document.querySelector(".gb-delegation-bar").hidden), false);
      assert.equal(await app.run(() => document.querySelectorAll('.gb-thread .gb-assistant[aria-busy="true"]').length), 0, "l’enfant ne fabrique aucun streaming du parent");
      await app.type('.gb-editor[aria-label="Votre message"]', waiting ? "Je précise aussi la ville." : "Ajoute une conclusion courte.");
      await app.until(() => document.getElementById("send-button").dataset.mode === "send" && !document.getElementById("send-button").disabled);
      const count = app.calls.filter(call => call.path === "/api/send").length;
      await app.enter("#send-button");
      await app.until(() => !GrokbotPanels.busy);
      const sends = app.calls.filter(call => call.path === "/api/send");
      assert.equal(sends.length, count + 1); assert.equal(sends.at(-1).body.conversation_id, "parent");
      assert.equal(sends.at(-1).body.body, waiting ? "Je précise aussi la ville." : "Ajoute une conclusion courte.");
      assert.equal(app.state.engine.runs.find(item => item.run_id === "run-main").status, "awaiting_children");
      assert.equal(await app.run(() => document.getElementById("send-button").dataset.mode), "send");
      await app.cleanThread();
    }
    await app.run(() => GrokbotPanels.openConversation("child"));
    await app.until(() => document.querySelector(".gb-delegation-bar").textContent.includes("Fil de tâche · Clément"));
    await app.run(() => [...document.querySelectorAll(".gb-delegation-bar button")].find(button => button.textContent === "Répondre à la demande").click());
    await app.until(() => document.getElementById("conversation-title").textContent === "Lecture du dossier" && document.querySelector('[data-message-id="form-child"] input'));
    await app.type('[data-message-id="form-child"] input[name="field-0"]', "Montréal");
    const holding = app.hold("/api/forms/submit"); await app.enter('[data-message-id="form-child"] button[type="submit"]');
    const body = (await holding.seen).body;
    assert.equal(body.entry_id, "entry-child"); assert.equal(body.conversation_id, "parent"); assert.deepEqual(body.values, { city: "Montréal" });
    holding.release();
    await app.until(() => document.querySelector('[data-message-id="form-child"]').dataset.requestStatus === "resolved");
    assert.equal(app.state.delegation.delegations[0].child_run_id, "run-child");
  });
});
