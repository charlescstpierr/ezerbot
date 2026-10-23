// Run with: node --test app/ui/voice-call-header.test.cjs
// Optional: CHROME_BIN=/path/to/chrome node --test app/ui/voice-call-header.test.cjs
const assert = require("node:assert/strict");
const { spawn, spawnSync } = require("node:child_process");
const { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const { after, test } = require("node:test");

const UI = __dirname;
const CHROME_CANDIDATES = [
  "google-chrome",
  "google-chrome-stable",
  "chromium",
  "chromium-browser",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
];

function chromeBinary() {
  if (process.env.CHROME_BIN) return process.env.CHROME_BIN;
  const found = CHROME_CANDIDATES.find((command) => command.includes("/")
    ? existsSync(command)
    : spawnSync("which", [command], { stdio: "ignore" }).status === 0);
  return found || "google-chrome";
}

// The header of the app as it stands at 820 px: Notifications, Créer un groupe, Membres,
// the ChatGPT badge and the menu. With the call, the real voice-call.js mounts into it.
function page(withCall) {
  const script = withCall ? `<script>
    window.calls = [];
    window.GrokbotChat = { voiceNeedsAction: () => false };
    const context = { connected: true, bot: { id: "ezerbot", name: "Ezerbot" }, conversation: { id: "chat", messages: [] },
      state: { voice: { bots: {} }, voice_call: { available: true, debug_fake: true, active: null } } };
    window.GrokbotPanels = {
      register(panel) { panel.composer.mount(document.getElementById("composer"), context); panel.onState(context.state, context); },
      composerChanged() {}, notify() {}, async refresh() {},
      async api(path) {
        window.calls.push(path);
        if (path === "/api/voice/call/start") return { call: { id: "call_one" }, secret: "fake" };
        return { status: "listening", lines: [], transcript: "", speaking: false };
      },
    };
  </script><script src="file://${UI}/panels/voice-call.js"></script>` : "";
  return `<!doctype html>
<html lang="fr" data-theme="dark" data-accent="citron">
<head>
  <meta charset="utf-8">
  <link rel="stylesheet" href="file://${UI}/styles.css">
  <link rel="stylesheet" href="file://${UI}/panels/voice-call.css">
</head>
<body>
  <aside class="sidebar"></aside>
  <main class="main-panel">
    <header class="topbar">
      <div class="topbar-title"><button class="icon-button sidebar-toggle" type="button" aria-label="Ouvrir les conversations"></button><button class="workspace-label" type="button" disabled>Espace personnel</button><span class="breadcrumb-separator" aria-hidden="true">/</span><h1 id="conversation-title">Nouvelle conversation</h1></div>
      <div class="topbar-actions">
        <button class="tool-pill notifications-button" type="button">Notifications</button>
        <button class="tool-pill" type="button">Créer un groupe</button>
        <button class="tool-pill" type="button">Membres</button>
        <button class="icon-button" id="conversation-menu-button" type="button" aria-label="Actions de la conversation" aria-expanded="false">…</button>
      </div>
      <div class="popover conversation-menu" id="conversation-menu" hidden><button id="rename-button">Renommer</button></div>
    </header>
    <div id="composer"></div>
  </main>
  ${script}
</body>
</html>`;
}

const REPORT = `(async () => {
  const box = (node) => node.getBoundingClientRect();
  const title = document.getElementById("conversation-title");
  const header = [...document.querySelectorAll(".topbar-actions > *")].map((node) => ({
    text: node.textContent.trim() || node.getAttribute("aria-label"), width: box(node).width,
  }));
  const report = { width: innerWidth, titleClient: title.clientWidth, titleScroll: title.scrollWidth, header };
  const toggle = document.querySelector(".topbar-actions .voice-call-toggle");
  if (!toggle) return report;
  report.toggle = { width: box(toggle).width, label: toggle.getAttribute("aria-label") };
  document.getElementById("conversation-menu-button").click();
  document.getElementById("conversation-menu").hidden = false;
  const item = document.querySelector("#conversation-menu .voice-call-menu-item");
  report.item = item && { width: box(item).width, height: box(item).height, name: item.textContent, pressed: item.getAttribute("aria-pressed"), disabled: item.disabled };
  const visible = item && getComputedStyle(item).display !== "none" ? item : toggle;
  visible.click();
  for (let i = 0; i < 50 && !window.calls.includes("/api/voice/call/debug"); i += 1) await new Promise((resolve) => setTimeout(resolve, 20));
  report.calls = window.calls.slice();
  report.pressedAfter = [toggle.getAttribute("aria-pressed"), item?.getAttribute("aria-pressed")];
  report.menuHidden = document.getElementById("conversation-menu").hidden;
  return report;
})()`;

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// One headless Chrome for the file, killed with its process group when the file ends.
let shared = null;

async function chrome() {
  if (shared) return shared;
  const directory = mkdtempSync(join(tmpdir(), "voice-call-header-"));
  const child = spawn(chromeBinary(), [
    "--headless=new", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
    "--force-device-scale-factor=1", "--remote-allow-origins=*", "--remote-debugging-port=0",
    `--user-data-dir=${join(directory, "profile")}`, "--window-size=1440,900", "about:blank",
  ], { stdio: "ignore", detached: true });
  child.on("error", () => {});
  shared = { directory, child };
  const portFile = join(directory, "profile", "DevToolsActivePort");
  for (const deadline = Date.now() + 10000; Date.now() < deadline; await sleep(20)) {
    if (child.exitCode !== null) throw new Error("chrome exited before the debugging port");
    if (existsSync(portFile)) {
      const port = Number(readFileSync(portFile, "utf8").split(/\s/)[0]);
      if (port) { shared.port = port; return shared; }
    }
  }
  throw new Error("chrome did not open a debugging port");
}

function stopChrome() {
  if (!shared) return;
  try { process.kill(-shared.child.pid, "SIGKILL"); } catch { /* already gone */ }
  rmSync(shared.directory, { recursive: true, force: true });
  shared = null;
}
process.on("exit", stopChrome);
after(stopChrome);

function cdp(url) {
  const ws = new WebSocket(url);
  const pending = new Map();
  let next = 0;
  ws.addEventListener("message", (event) => {
    const message = JSON.parse(String(event.data));
    const waiting = pending.get(message.id);
    if (!waiting) return;
    pending.delete(message.id);
    if (message.error) waiting.reject(new Error(message.error.message));
    else waiting.resolve(message.result || {});
  });
  return new Promise((resolve, reject) => {
    ws.addEventListener("open", () => resolve({
      send: (method, params = {}) => new Promise((ok, ko) => {
        const id = ++next;
        pending.set(id, { resolve: ok, reject: ko });
        ws.send(JSON.stringify({ id, method, params }));
      }),
      close: () => ws.close(),
    }));
    ws.addEventListener("error", () => reject(new Error("devtools websocket failed")));
  });
}

async function measure(width, withCall) {
  const { directory, port } = await chrome();
  const file = join(directory, `header-${width}-${withCall}.html`);
  writeFileSync(file, page(withCall));
  const target = await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" }).then((response) => response.json());
  const session = await cdp(target.webSocketDebuggerUrl);
  try {
    await session.send("Emulation.setDeviceMetricsOverride", { width, height: 560, deviceScaleFactor: 1, mobile: false, screenWidth: width, screenHeight: 560 });
    await session.send("Page.enable");
    await session.send("Page.navigate", { url: pathToFileURL(file).href });
    let ready = false;
    for (let attempt = 0; attempt < 100 && !ready; attempt += 1) {
      const loaded = await session.send("Runtime.evaluate", {
        expression: `document.readyState === "complete" && ${withCall ? "Boolean(document.querySelector('.voice-call-toggle'))" : "true"}`,
        returnByValue: true,
      });
      ready = loaded.result?.value === true;
      if (!ready) await sleep(20);
    }
    assert.ok(ready, `page not ready at ${width}`);
    const evaluated = await session.send("Runtime.evaluate", { expression: REPORT, awaitPromise: true, returnByValue: true });
    if (evaluated.exceptionDetails) throw new Error(JSON.stringify(evaluated.exceptionDetails));
    return evaluated.result.value;
  } finally {
    session.close();
    await fetch(`http://127.0.0.1:${port}/json/close/${target.id}`).catch(() => {});
    rmSync(file, { force: true });
  }
}

test("at 820 px the call costs the header no width, and stays reachable from the conversation menu", { timeout: 60000 }, async () => {
  const without = await measure(820, false);
  const withCall = await measure(820, true);
  const where = JSON.stringify({ without, withCall });
  assert.equal(without.width, 820, where);
  assert.equal(without.titleScroll <= without.titleClient + 1, true, `le titre tient sans l’appel : ${where}`);
  assert.equal(withCall.titleClient, without.titleClient, `même largeur de titre avec l’appel : ${where}`);
  assert.equal(withCall.titleScroll <= withCall.titleClient + 1, true, where);
  assert.deepEqual(withCall.header.filter((node) => node.width > 0), without.header, `le reste de l’en-tête est identique : ${where}`);
  assert.equal(withCall.toggle.width, 0, where);
  assert.deepEqual([withCall.item.name, withCall.item.pressed, withCall.item.disabled], ["Appel vocal", "false", false], where);
  assert.ok(withCall.item.width > 0 && withCall.item.height > 0, where);
  assert.ok(withCall.calls.includes("/api/voice/call/start"), `le menu lance le même appel : ${where}`);
  assert.deepEqual(withCall.pressedAfter, ["true", "true"], where);
  assert.equal(withCall.menuHidden, true, where);
});

test("above 900 px the call stays an icon in the header", { timeout: 60000 }, async () => {
  const withCall = await measure(1140, true);
  const where = JSON.stringify(withCall);
  assert.equal(withCall.toggle.label, "Appel vocal", where);
  assert.ok(withCall.toggle.width > 0, where);
  assert.equal(withCall.item.width, 0, where);
  assert.ok(withCall.calls.includes("/api/voice/call/start"), where);
  assert.deepEqual(withCall.pressedAfter, ["true", "true"], where);
});
