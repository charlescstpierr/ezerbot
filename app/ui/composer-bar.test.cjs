// Run with: node --test app/ui/composer-bar.test.cjs
// Optional: CHROME_BIN=/path/to/chrome node --test app/ui/composer-bar.test.cjs
const assert = require("node:assert/strict");
const { spawn, spawnSync } = require("node:child_process");
const { existsSync, mkdtempSync, readFileSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");
const { test } = require("node:test");

const UI = __dirname;
const FRAMES = [[1140, 780], [820, 560]];
const CHROME_CANDIDATES = [
  "google-chrome",
  "google-chrome-stable",
  "chromium",
  "chromium-browser",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
];

function chromeOnPath(command) {
  if (command.includes("/")) return existsSync(command);
  return spawnSync("which", [command], { stdio: "ignore" }).status === 0;
}

function chromeBinary() {
  if (process.env.CHROME_BIN) return process.env.CHROME_BIN;
  return CHROME_CANDIDATES.find(chromeOnPath) || "google-chrome";
}

function page(name, { pills = true, model = "GPT-6.1 Sol", effort = "medium", speed = "", workspace = "", welcome = false, title = "Nouvelle conversation", header = "bench" } = {}) {
  const speedPill = speed
    ? `<button class="tool-pill" type="button" data-picker="speed"><small>Vitesse</small><b>${speed}</b><svg class="icon"><use href="#i-chevron"/></svg></button>`
    : "";
  const tools = pills
    ? `<button class="tool-pill" type="button" data-picker="model"><small>Modèle</small><b>${model}</b><svg class="icon"><use href="#i-chevron"/></svg></button><button class="tool-pill" type="button" data-picker="effort"><small>Effort</small><b>${effort}</b><svg class="icon"><use href="#i-chevron"/></svg></button>${speedPill}`
    : "";
  const welcomeBlock = welcome ? `<section class="chat-scroll" id="chat-scroll"><div class="welcome" id="welcome"><div class="mascot-stage"><button class="mascot-greeting" type="button"></button></div><p class="welcome-eyebrow">UN PETIT COMPAGNON, DE GRANDES IDÉES</p><h2>Une idée en tête<span> ?</span></h2><p class="welcome-description">Posez-la ici. On lui fait une place.</p><div class="plan-banner" id="plan-banner"><span>Connectez votre compte ChatGPT via Codex pour obtenir de vraies réponses.</span><button class="chatgpt-button" type="button">Se connecter via Codex</button></div><div class="suggestion-grid"><button class="suggestion" type="button"><span class="suggestion-icon"></span><span><strong>Faire grandir une idée</strong><small>Du premier déclic au prochain pas</small></span></button><button class="suggestion" type="button"><span class="suggestion-icon"></span><span><strong>Trouver les bons mots</strong><small>Un texte clair, à votre manière</small></span></button><button class="suggestion" type="button"><span class="suggestion-icon"></span><span><strong>Explorer du code</strong><small>Comprendre, construire, affiner</small></span></button><button class="suggestion" type="button"><span class="suggestion-icon"></span><span><strong>Suivre sa curiosité</strong><small>Une bonne question ouvre des portes</small></span></button></div></div></section>` : "";
  return `<!doctype html>
<html lang="fr" data-theme="dark" data-accent="citron">
<head>
  <meta charset="utf-8">
  <link rel="stylesheet" href="file://${UI}/styles.css">
  <link rel="stylesheet" href="file://${UI}/panels/attachments.css">
  <link rel="stylesheet" href="file://${UI}/panels/meetings.css">
  <link rel="stylesheet" href="file://${UI}/panels/voice-conversation.css">
  <link rel="stylesheet" href="file://${UI}/panels/voice.css">
</head>
<body>
  <svg class="icon-definitions" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
    <defs>
      <symbol id="i-plus" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></symbol>
      <symbol id="i-chevron" viewBox="0 0 24 24"><path d="m7 10 5 5 5-5"/></symbol>
      <symbol id="i-send" viewBox="0 0 24 24"><path d="M12 19V5m-6 6 6-6 6 6"/></symbol>
      <symbol id="i-menu" viewBox="0 0 24 24"><path d="M4 6h16M4 12h16M4 18h16"/></symbol>
      <symbol id="i-more" viewBox="0 0 24 24"><circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/></symbol>
    </defs>
  </svg>
  <aside class="sidebar"></aside>
  <main class="main-panel">
    <header class="topbar">
      ${header === "app" ? `<div class="topbar-title"><button class="icon-button sidebar-toggle" type="button" aria-label="Ouvrir les conversations"><svg class="icon"><use href="#i-menu"/></svg></button><button class="workspace-label" type="button" disabled>Espace personnel</button><span class="breadcrumb-separator" aria-hidden="true">/</span><h1 id="conversation-title">${title}</h1></div>
      <div class="topbar-actions">
        <button class="tool-pill notifications-button" type="button">Notifications</button>
        <button class="tool-pill" type="button">Créer un groupe</button>
        <span class="mode-badge"><span class="status-dot"></span><span>Local</span></span>
        <button class="icon-button" type="button" disabled aria-label="Actions de la conversation"><svg class="icon"><use href="#i-more"/></svg></button>
      </div>` : `<div class="topbar-title">${workspace ? `<button class="workspace-label" type="button" disabled>${workspace}</button><span class="breadcrumb-separator" aria-hidden="true">/</span>` : ""}<h1 id="conversation-title">${title}</h1></div>
      <div class="topbar-actions">
        <button class="tool-pill notifications-button" type="button">Notifications</button>
        <button class="tool-pill" type="button">Créer un groupe</button>
        <button class="tool-pill" type="button">Membres</button>
      </div>`}
    </header>
    ${welcomeBlock}
    <div class="composer-dock"><div class="composer-area"><form class="composer" id="composer-form">
      ${welcome ? `<div class="composer-input"></div>` : ""}
      <div class="composer-bottom">
        <span class="composer-model" id="composer-model">
          <button class="attachment-choose" type="button" aria-label="Joindre un fichier" title="Joindre un fichier"><svg class="icon" aria-hidden="true"><use href="#i-plus"/></svg></button>
          <button class="attachment-choose meeting-start" type="button" aria-label="Enregistrer une réunion" title="Enregistrer une réunion">● Enregistrer une réunion</button>
          <button class="composer-bot" id="composer-bot" type="button" title="Choisir le bot">
            <span class="composer-mascot" aria-hidden="true"></span>
            <span class="composer-bot-name" id="composer-bot-name">${name}</span>
            <svg class="icon composer-chevron" aria-hidden="true"><use href="#i-chevron"/></svg>
          </button>
          <span class="composer-tools" id="composer-tools">${tools}</span>
        </span>
        <div class="voice-controls">
          <button class="voice-session-toggle" type="button" aria-label="Mode voix" title="Mode voix · parler et écouter les réponses sur ce Mac">
            <span class="voice-session-icon" aria-hidden="true"><svg class="icon" viewBox="0 0 24 24"><path d="M3 18v-6a9 9 0 0 1 18 0v6"/><path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3z"/><path d="M3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z"/></svg></span>
            <span class="voice-session-label">Mode voix</span>
          </button>
          <label class="voice-mode" title="Mémo vocal"><input type="checkbox">Mémo vocal</label>
          <button class="icon-button voice-mic" type="button" title="Dicter un message"></button>
        </div>
        <button class="send-button" id="send-button" type="button" aria-label="Envoyer le message" title="Envoyer le message"><svg class="icon"><use href="#i-send"/></svg></button>
      </div>
    </form>${welcome ? `<div class="composer-caption"><span id="composer-hint">Entrée pour envoyer</span><span class="simulation-note">Connexion ChatGPT requise</span></div>` : ""}</div></div>
  </main>
  <pre id="qa-result"></pre>
  <script>
    const icon = (selector) => {
      const node = document.querySelector(selector);
      if (!node) return { present: false };
      const box = node.getBoundingClientRect();
      const style = getComputedStyle(node);
      const strokeWidth = parseFloat(style.strokeWidth) || 0;
      return {
        present: true,
        width: box.width,
        height: box.height,
        stroke: style.stroke,
        strokeWidth,
        opacity: parseFloat(style.opacity),
        visibility: style.visibility,
        display: style.display,
        color: style.color,
      };
    };
    const visible = (info) => info.present && info.width >= 8 && info.height >= 8
      && info.display !== "none" && info.visibility !== "hidden" && info.opacity > 0
      && info.stroke !== "none" && info.strokeWidth > 0 && info.color !== "rgba(0, 0, 0, 0)";
    const boxOf = (node) => {
      const box = node.getBoundingClientRect();
      return { left: box.left, right: box.right, top: box.top, bottom: box.bottom, width: box.width, height: box.height };
    };
    const contained = (child, parent) => child.width > 0 && child.height > 0
      && child.left >= parent.left - 0.5 && child.right <= parent.right + 0.5
      && child.top >= parent.top - 0.5 && child.bottom <= parent.bottom + 0.5;
    const measure = (theme) => {
      document.documentElement.dataset.theme = theme;
      const name = document.getElementById("composer-bot-name");
      const voice = document.querySelector(".voice-controls").getBoundingClientRect();
      const send = document.getElementById("send-button").getBoundingClientRect();
      const nameBox = boxOf(name);
      const botBox = boxOf(document.getElementById("composer-bot"));
      const chevronBox = boxOf(document.querySelector("#composer-bot .composer-chevron"));
      const modelLabel = document.querySelector('[data-picker="model"] b');
      const modelButton = document.querySelector('[data-picker="model"]');
      const effortButton = document.querySelector('[data-picker="effort"]');
      const effortLabel = document.querySelector('[data-picker="effort"] b');
      const speedButton = document.querySelector('[data-picker="speed"]');
      const speedLabel = document.querySelector('[data-picker="speed"] b');
      const modelBox = modelLabel ? boxOf(modelLabel) : { width: 0 };
      const modelButtonBox = modelButton ? boxOf(modelButton) : null;
      const effortButtonBox = effortButton ? boxOf(effortButton) : null;
      const speedBox = speedButton ? boxOf(speedButton) : null;
      const headsetBox = boxOf(document.querySelector(".voice-session-toggle"));
      const overlap = (a, b) => {
        if (!a || !b) return { width: 0, height: 0 };
        return {
          width: Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left)),
          height: Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top)),
        };
      };
      const toolsBox = boxOf(document.querySelector(".composer-tools"));
      return {
        theme,
        viewport: [innerWidth, innerHeight],
        plus: icon("#composer-model .attachment-choose:not(.meeting-start) .icon"),
        chevron: icon(".composer-chevron"),
        name: name.textContent,
        nameClient: name.clientWidth,
        nameScroll: name.scrollWidth,
        nameFits: name.scrollWidth <= name.clientWidth + 1,
        nameEllipsed: name.scrollWidth > name.clientWidth + 1,
        clear: nameBox.width > 0 && nameBox.right <= voice.left + 0.5 && nameBox.right <= send.left + 0.5,
        toolsClear: toolsBox.right <= voice.left + 0.5,
        toolsGap: voice.left - toolsBox.right,
        speedGap: speedBox ? voice.left - speedBox.right : null,
        speedTop: speedBox ? speedBox.top : null,
        modelTop: modelButtonBox ? modelButtonBox.top : null,
        effortTop: effortButtonBox ? effortButtonBox.top : null,
        modelGap: modelButtonBox ? voice.left - modelButtonBox.right : null,
        effortGap: effortButtonBox ? voice.left - effortButtonBox.right : null,
        headsetLeft: headsetBox.left,
        headsetBox,
        speedHeadset: overlap(speedBox, headsetBox),
        voiceTop: voice.top,
        pillLabel: document.querySelector(".composer-tools .tool-pill small")
          ? getComputedStyle(document.querySelector(".composer-tools .tool-pill small")).display
          : "",
        titleMinWidth: getComputedStyle(document.getElementById("conversation-title")).minWidth,
        nameInside: contained(nameBox, botBox),
        chevronInside: contained(chevronBox, botBox),
        modelLabelWidth: modelBox.width,
        modelText: modelLabel ? modelLabel.textContent : "",
        effortText: effortLabel ? effortLabel.textContent : "",
        modelClient: modelLabel ? modelLabel.clientWidth : 0,
        modelScroll: modelLabel ? modelLabel.scrollWidth : 0,
        modelFits: !modelLabel || modelLabel.scrollWidth <= modelLabel.clientWidth + 1,
        effortClient: effortLabel ? effortLabel.clientWidth : 0,
        effortScroll: effortLabel ? effortLabel.scrollWidth : 0,
        effortFits: !effortLabel || effortLabel.scrollWidth <= effortLabel.clientWidth + 1,
        speedClient: speedLabel ? speedLabel.clientWidth : 0,
        speedScroll: speedLabel ? speedLabel.scrollWidth : 0,
        speedFits: !speedLabel || speedLabel.scrollWidth <= speedLabel.clientWidth + 1,
        modelInside: !modelLabel || contained(modelBox, modelButtonBox),
        modelBeforeEffort: !modelButtonBox || !effortButtonBox || modelButtonBox.right <= effortButtonBox.left + 0.5,
        welcome: (() => {
          const scroll = document.getElementById("chat-scroll");
          const banner = document.getElementById("plan-banner");
          if (!scroll || !banner) return null;
          const parent = boxOf(scroll);
          const suggestions = [...document.querySelectorAll(".suggestion")];
          const last = suggestions.at(-1);
          const composer = document.querySelector(".composer");
          const lastBox = last ? boxOf(last) : null;
          const composerBox = composer ? boxOf(composer) : null;
          return {
            banner: contained(boxOf(banner), parent),
            suggestions: suggestions.map((node) => contained(boxOf(node), parent)),
            below: lastBox && composerBox ? composerBox.top - lastBox.bottom : null,
            above: boxOf(document.getElementById("welcome")).top - parent.top,
          };
        })(),
        titleText: document.getElementById("conversation-title").textContent,
        titleClient: document.getElementById("conversation-title").clientWidth,
        titleScroll: document.getElementById("conversation-title").scrollWidth,
        titleFits: document.getElementById("conversation-title").scrollWidth <= document.getElementById("conversation-title").clientWidth + 1,
        titleBox: boxOf(document.getElementById("conversation-title")),
        actionsBox: boxOf(document.querySelector(".topbar-actions")),
        titleClear: boxOf(document.getElementById("conversation-title")).right <= boxOf(document.querySelector(".topbar-actions")).left + 0.5,
        notificationsLeft: document.querySelector(".notifications-button") ? boxOf(document.querySelector(".notifications-button")).left : null,
        titleGap: document.querySelector(".notifications-button")
          ? boxOf(document.querySelector(".notifications-button")).left - boxOf(document.getElementById("conversation-title")).right
          : null,
        workspaceText: document.querySelector(".workspace-label")?.textContent || "",
        workspaceClient: document.querySelector(".workspace-label")?.clientWidth || 0,
        workspaceScroll: document.querySelector(".workspace-label")?.scrollWidth || 0,
        headerHeights: [...document.querySelectorAll(".topbar-actions .tool-pill")].map((node) => ({
          text: node.textContent,
          height: node.getBoundingClientRect().height,
        })),
        composerWidth: document.querySelector(".composer").getBoundingClientRect().width,
        areaWidth: document.querySelector(".composer-area").getBoundingClientRect().width,
        pieces: {
          bottom: document.querySelector(".composer-bottom").getBoundingClientRect().width,
          plus: document.querySelector("#composer-model .attachment-choose:not(.meeting-start)").getBoundingClientRect().width,
          meeting: document.querySelector(".meeting-start").getBoundingClientRect().width,
          bot: document.getElementById("composer-bot").getBoundingClientRect().width,
          tools: document.querySelector(".composer-tools").getBoundingClientRect().width,
          model: (document.querySelector('[data-picker="model"]') || { getBoundingClientRect: () => ({ width: 0 }) }).getBoundingClientRect().width,
          effort: (document.querySelector('[data-picker="effort"]') || { getBoundingClientRect: () => ({ width: 0 }) }).getBoundingClientRect().width,
          voice: voice.width,
          send: send.width,
          mascot: getComputedStyle(document.querySelector(".composer-mascot")).display,
        },
        nameBox,
        chevronBox,
        botBox,
      };
    };
    const report = { frames: FRAMES_PLACEHOLDER, themes: ["dark", "light"].map(measure) };
    for (const theme of report.themes) {
      theme.plusVisible = visible(theme.plus);
      theme.chevronVisible = visible(theme.chevron);
    }
    document.getElementById("qa-result").textContent = JSON.stringify(report);
  </script>
</body>
</html>`;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// --window-size fixes the window, not the layout viewport. Headless Chrome keeps
// its own chrome, so innerHeight is short by a platform-specific amount: about
// 87px here, 143px on macOS (a requested 560px window is 417px tall). Device
// metrics emulation is what the page and the media queries actually see.
async function debuggingPort(directory, chrome) {
  const portFile = join(directory, "profile", "DevToolsActivePort");
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    if (chrome.exitCode !== null) throw new Error("chrome exited before the debugging port");
    if (existsSync(portFile)) {
      const port = Number(readFileSync(portFile, "utf8").split(/\s/)[0]);
      if (port) return port;
    }
    await sleep(20);
  }
  throw new Error("chrome did not open a debugging port");
}

function connectCdp(url) {
  const ws = new WebSocket(url);
  const pending = new Map();
  let next = 0;
  ws.addEventListener("message", (event) => {
    const message = JSON.parse(String(event.data));
    if (!message.id || !pending.has(message.id)) return;
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(message.error.message));
    else resolve(message.result || {});
  });
  ws.addEventListener("close", () => {
    for (const { reject } of pending.values()) reject(new Error("devtools closed"));
    pending.clear();
  });
  return new Promise((resolve, reject) => {
    ws.addEventListener("open", () => resolve({
      send(method, params = {}) {
        return new Promise((resolveSend, rejectSend) => {
          const id = ++next;
          pending.set(id, { resolve: resolveSend, reject: rejectSend });
          ws.send(JSON.stringify({ id, method, params }));
        });
      },
      close() { ws.close(); },
    }));
    ws.addEventListener("error", () => reject(new Error("devtools websocket failed")));
  });
}

async function pageTarget(port) {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then((response) => response.json());
    const found = targets.find((target) => target.type === "page" && target.webSocketDebuggerUrl);
    if (found) return found;
    await sleep(20);
  }
  throw new Error("chrome did not open a page target");
}

function measure(width, height, name, options) {
  const directory = mkdtempSync(join(tmpdir(), "composer-bar-"));
  const file = join(directory, "bar.html");
  writeFileSync(file, page(name, options).replace("FRAMES_PLACEHOLDER", "null"));
  const chrome = spawn(chromeBinary(), [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--force-device-scale-factor=1",
    "--remote-allow-origins=*",
    `--remote-debugging-port=0`,
    `--user-data-dir=${join(directory, "profile")}`,
    `--window-size=${width},${height}`,
    "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  chrome.on("error", () => {});
  let stderr = "";
  chrome.stderr.on("data", (chunk) => {
    if (stderr.length < 4000) stderr += chunk;
  });
  const stop = () => {
    if (chrome.exitCode === null) chrome.kill("SIGKILL");
  };
  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (error, report) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      stop();
      if (error) reject(error);
      else resolve(report);
    };
    const timer = setTimeout(() => finish(new Error(`chrome did not report layout at ${width}×${height}`)), 20000);
    (async () => {
      const port = await debuggingPort(directory, chrome);
      const target = await pageTarget(port);
      const cdp = await connectCdp(target.webSocketDebuggerUrl);
      try {
        await cdp.send("Emulation.setDeviceMetricsOverride", {
          width,
          height,
          deviceScaleFactor: 1,
          mobile: false,
          screenWidth: width,
          screenHeight: height,
        });
        await cdp.send("Page.enable");
        await cdp.send("Page.navigate", { url: pathToFileURL(file).href });
        let text = "";
        for (let attempt = 0; attempt < 50 && !text; attempt += 1) {
          const evaluated = await cdp.send("Runtime.evaluate", {
            expression: "document.getElementById('qa-result')?.textContent || ''",
            returnByValue: true,
          });
          text = evaluated.result?.value || "";
          if (!text) await sleep(40);
        }
        if (!text) throw new Error(`missing layout report at ${width}×${height}`);
        const report = JSON.parse(text);
        for (const theme of report.themes) {
          if (theme.viewport[0] !== width || theme.viewport[1] !== height) {
            throw new Error(`viewport is ${theme.viewport.join("×")}, expected ${width}×${height}`);
          }
        }
        finish(null, report);
      } finally {
        cdp.close();
      }
    })().catch((error) => {
      const detail = error instanceof Error ? error.message : String(error);
      finish(new Error(`${detail}${stderr ? `\n${stderr.trim()}` : ""}`));
    });
  });
}

function forcedComposerWidth(width) {
  const source = readFileSync(join(UI, "../../tools/verify-app.py"), "utf8");
  const start = source.indexOf("const applyFrame = (width) => {");
  assert.notEqual(start, -1, "applyFrame");
  const snippet = source.slice(start, source.indexOf("style.textContent", start));
  const value = (name) => {
    const found = snippet.match(new RegExp(`const ${name} = ([^;]+);`));
    assert.ok(found, name);
    return found[1];
  };
  return new Function("width", `const sidebar = ${value("sidebar")}; const main = ${value("main")}; const area = ${value("area")}; return area;`)(width);
}

test("applyFrame matches the measured composer width", async () => {
  const frames = [[820, 560], [1140, 780]];
  for (const [width, height] of frames) {
    const report = await measure(width, height, "Ezerbot", { pills: false });
    const forced = forcedComposerWidth(width);
    for (const theme of report.themes) {
      const where = `${theme.theme} ${width} applyFrame=${forced} composer=${theme.composerWidth} area=${theme.areaWidth} viewport=${theme.viewport}`;
      assert.equal(theme.viewport[0], width, where);
      assert.ok(Math.abs(theme.composerWidth - theme.areaWidth) <= 0.5, where);
      assert.ok(Math.abs(forced - theme.composerWidth) <= 0.5, where);
      if (width === 820) assert.ok(Math.abs(theme.composerWidth - 530) <= 0.5, where);
    }
  }
});

test("composer rules do not restate a min-width of zero", () => {
  const css = readFileSync(`${UI}/styles.css`, "utf8");
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)];
  const body = (selector) => rules.find(([, found]) => found.trim() === selector)?.[2] || "";
  assert.equal(body(".tool-pill").includes("min-width: 0"), true);
  assert.equal(body(".composer-tools").includes("min-width: 0"), true);
  assert.equal(body(".composer .tool-pill b").includes("min-width: 0"), true);
  assert.doesNotMatch(body(".composer .tool-pill"), /min-width:\s*0/);
  assert.doesNotMatch(body(".composer .composer-tools"), /min-width:\s*0/);
  assert.equal(body('.composer [data-picker="model"] b'), "");
  assert.match(body(".composer .composer-bot"), /min-width:\s*0/);
});

test("CHROME_BIN selects the browser and otherwise falls back to an installed Chrome", () => {
  const previous = process.env.CHROME_BIN;
  process.env.CHROME_BIN = "/opt/chrome/custom";
  try {
    assert.equal(chromeBinary(), "/opt/chrome/custom");
    delete process.env.CHROME_BIN;
    const fallback = chromeBinary();
    assert.notEqual(fallback, "/opt/chrome/custom");
    assert.ok(chromeOnPath(fallback) || fallback === "google-chrome", fallback);
  } finally {
    if (previous === undefined) delete process.env.CHROME_BIN;
    else process.env.CHROME_BIN = previous;
  }
});

test("voice composer controls keep a name when they are icon-only", () => {
  const voice = readFileSync(`${UI}/panels/voice.js`, "utf8");
  const session = readFileSync(`${UI}/panels/voice-conversation.js`, "utf8");
  const voiceCss = readFileSync(`${UI}/panels/voice.css`, "utf8");
  const sessionCss = readFileSync(`${UI}/panels/voice-conversation.css`, "utf8");
  assert.match(voice, /aria-label", "Mémo vocal"/);
  assert.match(voice, /label\.title = "Mémo vocal"/);
  assert.match(session, /aria-label","Mode voix"/);
  assert.match(session, /Mode voix · parler et écouter les réponses sur ce Mac/);
  const memo = voiceCss.match(/\.voice-mode \{([^}]*)\}/);
  assert.ok(memo, "the memo control has a base rule");
  assert.match(memo[1], /font-size:\s*0/);
  assert.match(sessionCss, /\.voice-session-icon \{[^}]*display:\s*flex/);
  assert.match(sessionCss, /\.voice-session-label \{[^}]*overflow:\s*hidden/);
  assert.doesNotMatch(voiceCss, /@media \(max-width: 1100px\)[^{]*\{[^}]*\.voice-mode/);
});

const MEDIUM_NAME = "Coach sportif du matin";
const LONG_NAME = "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMN";

test("a medium composer name stays whole and a 40-character name ellipsizes clear of the voice controls", async () => {
  for (const [width, height] of FRAMES) {
    const medium = await measure(width, height, MEDIUM_NAME);
    const long = await measure(width, height, LONG_NAME);
    for (const theme of medium.themes) {
      assert.equal(theme.name, MEDIUM_NAME);
      assert.equal(theme.nameFits, true, `medium name truncated at ${width}×${height} ${theme.theme}`);
      assert.equal(theme.clear, true, `medium name overlaps controls at ${width}×${height} ${theme.theme}`);
      assert.equal(theme.toolsClear, true, `model tools overlap voice controls at ${width}×${height} ${theme.theme}`);
    }
    for (const theme of long.themes) {
      assert.equal(theme.name, LONG_NAME);
      assert.equal(theme.nameEllipsed, true, `40-character name is not ellipsed at ${width}×${height} ${theme.theme}`);
      assert.equal(theme.clear, true, `40-character name overlaps voice controls or Send at ${width}×${height} ${theme.theme}`);
      assert.equal(theme.toolsClear, true, `model tools overlap voice controls at ${width}×${height} ${theme.theme}`);
    }
  }
});

const SHORT_NAMES = ["Ezerbot", "Rédacteur", MEDIUM_NAME];

test("short names stay whole at 901px and at 500px, except a medium name beside the real model", async () => {
  const cases = [
    [500, 780, true],
    [500, 780, false],
    [901, 780, true],
  ];
  for (const label of SHORT_NAMES) {
    for (const [width, height, pills] of cases) {
      const report = await measure(width, height, label, { pills });
      for (const theme of report.themes) {
        const where = `${label} at ${width}×${height} ${theme.theme} pills=${pills} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} area=${theme.areaWidth} composer=${theme.composerWidth} ${JSON.stringify(theme.pieces)}`;
        // À 500 px, GPT-6.1 Sol et medium tiennent en entier : le nom moyen s'ellipse dans son bouton.
        const phoneWithRealModel = width === 500 && pills && label === MEDIUM_NAME;
        if (!phoneWithRealModel) assert.equal(theme.nameFits, true, where);
        assert.equal(theme.nameInside, true, where);
        assert.equal(theme.chevronInside, true, where);
        assert.equal(theme.toolsClear, true, where);
        if (pills) {
          assert.equal(theme.modelFits, true, where);
          assert.equal(theme.modelInside, true, where);
          assert.equal(theme.modelBeforeEffort, true, where);
          assert.ok(theme.modelLabelWidth >= 40, where);
        }
      }
    }
  }
});

test("a 40-character name still ellipsizes inside the phone composer", async () => {
  const report = await measure(500, 780, LONG_NAME, { pills: true });
  for (const theme of report.themes) {
    assert.equal(theme.nameEllipsed, true, `${theme.theme} ${theme.nameClient}/${theme.nameScroll}`);
    assert.equal(theme.nameInside, true);
    assert.equal(theme.chevronInside, true);
    assert.equal(theme.clear, true, JSON.stringify(theme.pieces));
    assert.equal(theme.toolsClear, true, JSON.stringify(theme.pieces));
    assert.equal(theme.modelFits, true, `${theme.theme} model=${theme.modelClient}/${theme.modelScroll}`);
    assert.equal(theme.modelInside, true);
    assert.equal(theme.modelBeforeEffort, true);
  }
});

test("the bot name and chevron stay inside the bot button and the model label stays readable", async () => {
  for (const label of ["Ezerbot", MEDIUM_NAME, LONG_NAME]) {
    for (const [width, height] of FRAMES) {
      const report = await measure(width, height, label);
      for (const theme of report.themes) {
        assert.equal(theme.nameInside, true, `name leaves #composer-bot at ${width}×${height} ${theme.theme} (${label}): ${JSON.stringify({ name: theme.nameBox, bot: theme.botBox })}`);
        assert.equal(theme.chevronInside, true, `chevron leaves #composer-bot at ${width}×${height} ${theme.theme} (${label}): ${JSON.stringify({ chevron: theme.chevronBox, bot: theme.botBox })}`);
        assert.ok(theme.modelLabelWidth >= 40, `model label is ${theme.modelLabelWidth}px at ${width}×${height} ${theme.theme} (${label})`);
      }
    }
  }
});

const DEFAULT_MODEL = "GPT-6.1 Sol";
const TERRA_MODEL = "GPT-5.6 Terra";
const DEFAULT_EFFORT = "medium";

test("GPT-6.1 Sol and medium stay whole inside the model button, left of the effort", async () => {
  const cases = [
    [820, 560, MEDIUM_NAME, DEFAULT_MODEL],
    [820, 560, LONG_NAME, DEFAULT_MODEL],
    [1140, 780, MEDIUM_NAME, DEFAULT_MODEL],
    [1140, 780, LONG_NAME, DEFAULT_MODEL],
    [1140, 780, MEDIUM_NAME, TERRA_MODEL],
    [901, 780, MEDIUM_NAME, DEFAULT_MODEL],
    [901, 780, LONG_NAME, DEFAULT_MODEL],
  ];
  for (const [width, height, bot, model] of cases) {
    const report = await measure(width, height, bot, { model, effort: DEFAULT_EFFORT });
    for (const theme of report.themes) {
      const where = `${bot} / ${model} / ${DEFAULT_EFFORT} at ${width}×${height} ${theme.theme} viewport=${theme.viewport} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} ${JSON.stringify(theme.pieces)}`;
      assert.equal(theme.modelText, model, where);
      assert.equal(theme.effortText, DEFAULT_EFFORT, where);
      assert.equal(theme.modelFits, true, where);
      assert.equal(theme.modelInside, true, where);
      assert.equal(theme.modelBeforeEffort, true, where);
      assert.equal(theme.nameInside, true, where);
      assert.equal(theme.toolsClear, true, where);
      if (bot === MEDIUM_NAME) assert.equal(theme.nameFits, true, where);
    }
  }
});

test("Nouvelle conversation stays whole at 820px beside a long workspace", async () => {
  const report = await measure(820, 560, "Ezerbot", { workspace: MEDIUM_NAME });
  for (const theme of report.themes) {
    const where = `${theme.theme} title=${theme.titleClient}/${theme.titleScroll} viewport=${theme.viewport} heights=${JSON.stringify(theme.headerHeights)}`;
    assert.equal(theme.titleText, "Nouvelle conversation", where);
    assert.equal(theme.titleFits, true, where);
    assert.deepEqual(theme.headerHeights.map((item) => item.text), ["Notifications", "Créer un groupe", "Membres"], where);
  }
});

test("Notifications, Créer un groupe and Membres keep their height at 820, 910 and 960px", async () => {
  const wide = await measure(1140, 780, "Ezerbot", { workspace: MEDIUM_NAME });
  const frames = [
    [820, 560],
    [901, 780],
    [910, 780],
    [960, 780],
    [500, 780],
  ];
  const reports = [];
  for (const [width, height] of frames) reports.push(await measure(width, height, "Ezerbot", { workspace: MEDIUM_NAME }));
  for (const theme of ["dark", "light"]) {
    const reference = wide.themes.find((item) => item.theme === theme);
    assert.equal(reference.titleFits, true, `${theme} title=${reference.titleClient}/${reference.titleScroll} at 1140`);
    for (const report of reports) {
      const actual = report.themes.find((item) => item.theme === theme);
      assert.equal(actual.titleFits, true, `${theme} title=${actual.titleClient}/${actual.titleScroll} at ${actual.viewport}`);
      assert.deepEqual(actual.headerHeights.map((item) => item.text), ["Notifications", "Créer un groupe", "Membres"]);
      for (let index = 0; index < actual.headerHeights.length; index += 1) {
        const seen = actual.headerHeights[index];
        const expected = reference.headerHeights[index];
        assert.ok(
          Math.abs(seen.height - expected.height) <= 0.5,
          `${seen.text} is ${seen.height}px at ${actual.viewport[0]}px and ${expected.height}px at ${reference.viewport[0]}px (${theme})`,
        );
      }
    }
  }
});

test("the connection card and suggestions stay visible at 820×560", async () => {
  const report = await measure(820, 560, "Ezerbot", { pills: false, welcome: true });
  for (const theme of report.themes) {
    const where = `${theme.theme} viewport=${theme.viewport} banner=${theme.welcome?.banner} suggestions=${JSON.stringify(theme.welcome?.suggestions)}`;
    assert.equal(theme.welcome.banner, true, where);
    assert.equal(theme.welcome.suggestions.length, 4, where);
    assert.equal(theme.welcome.suggestions.every(Boolean), true, where);
  }
});

test("a medium name keeps the speed pill at least 6px clear of the voice controls", async () => {
  const frames = [[820, 560], [960, 780], [1140, 780]];
  for (const model of [DEFAULT_MODEL, TERRA_MODEL]) {
    for (const [width, height] of frames) {
      const report = await measure(width, height, MEDIUM_NAME, { model, effort: DEFAULT_EFFORT, speed: "Standard" });
      for (const theme of report.themes) {
        const where = `${MEDIUM_NAME} / ${model} / Effort+Vitesse at ${width}×${height} ${theme.theme} viewport=${theme.viewport} speedGap=${theme.speedGap} toolsGap=${theme.toolsGap} composer=${theme.composerWidth} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} effort=${theme.effortClient}/${theme.effortScroll} speed=${theme.speedClient}/${theme.speedScroll} ${JSON.stringify(theme.pieces)}`;
        assert.equal(theme.name, MEDIUM_NAME, where);
        assert.equal(theme.modelText, model, where);
        assert.equal(theme.viewport[0], width, where);
        assert.ok(theme.speedGap >= 6, where);
        assert.equal(theme.modelFits, true, where);
        assert.equal(theme.effortFits, true, where);
        assert.equal(theme.speedFits, true, where);
        assert.equal(theme.chevronInside, true, where);
        assert.equal(theme.nameInside, true, where);
      }
    }
  }
});

test("a short name and its chevron stay whole when speed is shown", async () => {
  const cases = [
    [820, 560, "GPT-5.6 Terra"],
    [1140, 780, "GPT-6.1 Sol"],
    [1140, 780, "GPT-5.6 Terra"],
  ];
  for (const [width, height, model] of cases) {
    const report = await measure(width, height, "Ezerbot", { model, effort: "Moyen", speed: "Standard" });
    for (const theme of report.themes) {
      const where = `${model} ${theme.theme} at ${width} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} effort=${theme.effortClient}/${theme.effortScroll} speed=${theme.speedClient}/${theme.speedScroll}`;
      assert.equal(theme.nameFits, true, where);
      assert.equal(theme.chevronInside, true, where);
      assert.equal(theme.toolsClear, true, where);
      assert.equal(theme.modelFits, true, where);
      assert.ok(theme.effortClient >= 40, where);
      assert.equal(theme.speedFits, true, where);
    }
  }
});

test("GPT-5.6 Terra keeps a medium bot name whole at 820px", async () => {
  const report = await measure(820, 560, MEDIUM_NAME, { model: TERRA_MODEL, effort: "medium" });
  for (const theme of report.themes) {
    const where = `${theme.theme} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} effort=${theme.effortClient}/${theme.effortScroll} ${JSON.stringify(theme.pieces)}`;
    assert.equal(theme.modelText, TERRA_MODEL, where);
    assert.equal(theme.nameFits, true, where);
    assert.equal(theme.modelFits, true, where);
    assert.equal(theme.effortFits, true, where);
    assert.equal(theme.chevronInside, true, where);
    assert.equal(theme.toolsClear, true, where);
    assert.ok(theme.modelLabelWidth >= 40, where);
  }
});

test("a medium bot name keeps a width floor at 500px beside the real model", async () => {
  const report = await measure(500, 780, MEDIUM_NAME, { model: "GPT-6.1 Sol", effort: "medium" });
  for (const theme of report.themes) {
    const where = `${theme.theme} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} ${JSON.stringify(theme.pieces)}`;
    assert.ok(theme.nameClient >= 110, where);
    assert.equal(theme.nameEllipsed, true, where);
    assert.equal(theme.nameInside, true, where);
    assert.equal(theme.chevronInside, true, where);
    assert.equal(theme.toolsClear, true, where);
    assert.equal(theme.modelFits, true, where);
    assert.ok(theme.modelLabelWidth >= 40, where);
  }
});

test("effort and speed labels keep a 40px floor at 500px", async () => {
  const crowded = await measure(500, 780, "Ezerbot", { model: "GPT-6.1 Sol", effort: "Moyen", speed: "Standard" });
  const long = await measure(500, 780, LONG_NAME, { model: "GPT-6.1 Sol", effort: "medium", speed: "Standard" });
  for (const theme of crowded.themes) {
    const where = `${theme.theme} effort=${theme.effortClient}/${theme.effortScroll} speed=${theme.speedClient}/${theme.speedScroll} name=${theme.nameClient}/${theme.nameScroll}`;
    assert.equal(theme.effortText, "Moyen", where);
    assert.ok(theme.effortClient >= 40, where);
    assert.equal(theme.effortFits, true, where);
    assert.ok(theme.speedClient >= 40, where);
    assert.equal(theme.speedFits, true, where);
    assert.equal(theme.nameFits, true, where);
    assert.equal(theme.toolsClear, true, where);
    assert.equal(theme.chevronInside, true, where);
  }
  for (const theme of long.themes) {
    const where = `${theme.theme} effort=${theme.effortClient}/${theme.effortScroll} speed=${theme.speedClient}/${theme.speedScroll} name=${theme.nameClient}/${theme.nameScroll}`;
    assert.equal(theme.nameEllipsed, true, where);
    assert.ok(theme.effortClient >= 40, where);
    assert.equal(theme.effortFits, true, where);
    assert.equal(theme.speedFits, true, where);
    assert.equal(theme.modelFits, true, where);
    assert.equal(theme.toolsClear, true, where);
    assert.equal(theme.chevronInside, true, where);
  }
});

test("pills yield before a short bot name at 500px", async () => {
  const report = await measure(500, 780, "Ezerbot", { model: "GPT-6.1 Sol", effort: "Moyen", speed: "Standard" });
  for (const theme of report.themes) {
    const where = `${theme.theme} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} ${JSON.stringify(theme.pieces)}`;
    assert.equal(theme.name, "Ezerbot", where);
    assert.equal(theme.nameFits, true, where);
    assert.ok(theme.nameClient >= 45, where);
    assert.equal(theme.chevronInside, true, where);
    assert.equal(theme.toolsClear, true, where);
    assert.equal(theme.modelFits, true, where);
    assert.ok(theme.modelLabelWidth >= 40, where);
  }
});

const THREAD_TITLE = "Préparer la revue de lancement avec l'équipe";
const CODEX_SPARK = "Codex Spark";
// « Codex Spark » tient encore dans le bouton avec Noto (Linux). Le plancher de
// 137 px pousse le nom dehors dès que le modèle est un peu plus large, ce que
// « Codex Spark » fait déjà sur macOS. Le libellé suivant reproduit ce débordement ici.
const CODEX_SPARK_WIDE = "Codex Spark High";

test("a thread title stays clear of Notifications at 820px and the workspace keeps 56px", async () => {
  const report = await measure(820, 560, MEDIUM_NAME, {
    workspace: MEDIUM_NAME,
    title: THREAD_TITLE,
    model: CODEX_SPARK,
    effort: DEFAULT_EFFORT,
  });
  for (const theme of report.themes) {
    const where = `${theme.theme} viewport=${theme.viewport} title=${theme.titleClient}/${theme.titleScroll} clear=${theme.titleClear} ws=${theme.workspaceClient}/${theme.workspaceScroll} titleBox=${JSON.stringify(theme.titleBox)} actions=${JSON.stringify(theme.actionsBox)}`;
    assert.equal(theme.titleText, THREAD_TITLE, where);
    assert.equal(theme.workspaceText, MEDIUM_NAME, where);
    assert.equal(theme.titleClear, true, where);
    assert.ok(theme.titleClient >= 120, where);
    assert.ok(theme.workspaceClient >= 56, where);
  }
});

test("a bot name stays inside its button at 820px when the model has no speed pill", async () => {
  for (const model of [CODEX_SPARK, CODEX_SPARK_WIDE]) {
    const report = await measure(820, 560, MEDIUM_NAME, { model, effort: DEFAULT_EFFORT, speed: "" });
    for (const theme of report.themes) {
      const where = `${model} ${theme.theme} viewport=${theme.viewport} name=${theme.nameClient}/${theme.nameScroll} bot=${theme.pieces.bot} nameBox=${JSON.stringify(theme.nameBox)} botBox=${JSON.stringify(theme.botBox)} chevron=${JSON.stringify(theme.chevronBox)}`;
      assert.equal(theme.speedGap, null, where);
      assert.equal(theme.modelText, model, where);
      assert.equal(theme.nameInside, true, where);
      assert.equal(theme.chevronInside, true, where);
    }
  }
});

test("the header yields and a model without speed does not force a 137px bot name", () => {
  const css = readFileSync(`${UI}/styles.css`, "utf8");
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)];
  const body = (selector) => rules.find(([, found]) => found.trim() === selector)?.[2] || "";
  assert.match(body(".topbar h1"), /flex:\s*0 1 auto/);
  assert.match(body(".topbar h1"), /min-width:\s*min\(120px,\s*100%\)/);
  assert.match(body(".workspace-label"), /flex:\s*0 1 auto/);
  assert.match(body(".workspace-label"), /min-width:\s*56px/);
  assert.doesNotMatch(css, /min-width:\s*137px/);
});

test("speed stays on the composer row and at least 6px clear of the voice controls at 500 and 600px", async () => {
  for (const width of [500, 600]) {
    const report = await measure(width, 780, MEDIUM_NAME, { model: CODEX_SPARK, effort: DEFAULT_EFFORT, speed: "Standard" });
    for (const theme of report.themes) {
      const where = `${width} ${theme.theme} viewport=${theme.viewport} speedGap=${theme.speedGap} toolsGap=${theme.toolsGap} speedTop=${theme.speedTop} voiceTop=${theme.voiceTop} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} speed=${theme.speedClient}/${theme.speedScroll} ${JSON.stringify(theme.pieces)}`;
      assert.equal(theme.speedFits, true, where);
      assert.equal(theme.effortFits, true, where);
      assert.equal(theme.toolsClear, true, where);
      assert.equal(theme.nameInside, true, where);
      assert.equal(theme.chevronInside, true, where);
      assert.ok(Math.abs(theme.speedTop - theme.voiceTop) <= 1, where);
      assert.ok(theme.speedGap >= 6, where);
    }
  }
});

test("a long bot name at 960 and 1140 is at least as wide as at 910 without a speed pill", async () => {
  const widths = [910, 960, 1140];
  const reports = new Map();
  for (const width of widths) {
    reports.set(width, await measure(width, 780, LONG_NAME, { model: CODEX_SPARK, effort: DEFAULT_EFFORT, speed: "" }));
  }
  for (const theme of ["dark", "light"]) {
    const at = (width) => reports.get(width).themes.find((item) => item.theme === theme);
    const base = at(910);
    assert.equal(base.speedGap, null, `${theme} 910 still has a speed pill`);
    for (const width of [960, 1140]) {
      const actual = at(width);
      const where = `${theme} 910=${base.nameClient}/${base.nameScroll} ${width}=${actual.nameClient}/${actual.nameScroll} viewport=${actual.viewport} pieces=${JSON.stringify(actual.pieces)}`;
      assert.equal(actual.speedGap, null, where);
      assert.equal(actual.nameInside, true, where);
      assert.equal(actual.chevronInside, true, where);
      assert.ok(actual.nameClient + 0.5 >= base.nameClient, where);
    }
  }
});

function sweep(start, end, step, extra = []) {
  const widths = new Set(extra);
  for (let width = start; width <= end; width += step) widths.add(width);
  return [...widths].sort((a, b) => a - b);
}

test("layout holds at every 10px from 360 to 1140", { timeout: 300000 }, async () => {
  const widths = sweep(360, 1140, 10, [390, 417, 479, 480, 760, 761, 910, 911, 1100, 1101]);
  const names = new Map();
  for (const width of widths) {
    if (width < 900) continue;
    names.set(width, await measure(width, 780, LONG_NAME, { model: CODEX_SPARK, effort: DEFAULT_EFFORT, speed: "" }));
  }
  const speedReports = new Map();
  const headerReports = new Map();
  for (const width of widths) {
    speedReports.set(width, await measure(width, 780, MEDIUM_NAME, { model: CODEX_SPARK, effort: DEFAULT_EFFORT, speed: "Standard" }));
    headerReports.set(width, await measure(width, 780, MEDIUM_NAME, {
      workspace: MEDIUM_NAME,
      title: THREAD_TITLE,
      model: CODEX_SPARK,
      effort: DEFAULT_EFFORT,
    }));
  }
  for (const theme of ["dark", "light"]) {
    const at = (reports, width) => reports.get(width).themes.find((item) => item.theme === theme);
    const base = at(names, 910);
    for (const width of widths) {
      if (width >= 911) {
        const actual = at(names, width);
        const where = `${theme} name 910=${base.nameClient} ${width}=${actual.nameClient}/${actual.nameScroll} viewport=${actual.viewport}`;
        assert.equal(actual.nameInside, true, where);
        assert.equal(actual.chevronInside, true, where);
        assert.ok(actual.nameClient + 0.5 >= base.nameClient, where);
      }
      const speed = at(speedReports, width);
      const speedWhere = `${theme} ${width} speedGap=${speed.speedGap} speedTop=${speed.speedTop} voiceTop=${speed.voiceTop} bot=${speed.pieces.bot} name=${speed.nameClient}/${speed.nameScroll} nameBox=${JSON.stringify(speed.nameBox)} botBox=${JSON.stringify(speed.botBox)} chevron=${JSON.stringify(speed.chevronBox)}`;
      assert.equal(speed.nameInside, true, speedWhere);
      assert.equal(speed.chevronInside, true, speedWhere);
      assert.ok(speed.pieces.bot >= 45, speedWhere);
      if (width < 480) {
        assert.ok(speed.speedTop >= speed.voiceTop + 16, speedWhere);
      }
      const header = at(headerReports, width);
      const headerWhere = `${theme} ${width} clear=${header.titleClear} title=${header.titleClient}/${header.titleScroll} ws=${header.workspaceClient}/${header.workspaceScroll} titleBox=${JSON.stringify(header.titleBox)} actions=${JSON.stringify(header.actionsBox)}`;
      assert.equal(header.titleClear, true, headerWhere);
      if (width <= 760) assert.equal(header.titleMinWidth, "0px", headerWhere);
      if (width >= 761) {
        assert.ok(header.titleClient >= 120, headerWhere);
        assert.ok(header.workspaceClient >= 56, headerWhere);
      }
    }
  }
});

const MODELS = ["GPT-6.1 Sol", CODEX_SPARK, CODEX_SPARK_WIDE];
const BOTS = ["Ezerbot", MEDIUM_NAME, LONG_NAME];

test("without speed, model and effort stay on the voice row at 360, 430, 480 and 520", { timeout: 300000 }, async () => {
  const widths = [360, 370, 430, 480, 520];
  for (const width of widths) {
    for (const model of MODELS) {
      for (const bot of BOTS) {
        const report = await measure(width, 780, bot, { model, effort: DEFAULT_EFFORT, speed: "" });
        for (const theme of report.themes) {
          const where = `${width} ${model} / ${bot} ${theme.theme} viewport=${theme.viewport} modelGap=${theme.modelGap} effortGap=${theme.effortGap} toolsGap=${theme.toolsGap} modelTop=${theme.modelTop} effortTop=${theme.effortTop} voiceTop=${theme.voiceTop} name=${theme.nameClient}/${theme.nameScroll} pieces=${JSON.stringify(theme.pieces)} nameBox=${JSON.stringify(theme.nameBox)} botBox=${JSON.stringify(theme.botBox)} chevron=${JSON.stringify(theme.chevronBox)}`;
          assert.equal(theme.speedGap, null, where);
          assert.equal(theme.nameInside, true, where);
          assert.equal(theme.chevronInside, true, where);
          assert.ok(Math.abs(theme.modelTop - theme.voiceTop) <= 2, where);
          assert.ok(Math.abs(theme.effortTop - theme.voiceTop) <= 2, where);
          assert.ok(theme.modelGap >= 0, where);
          assert.ok(theme.effortGap >= 0, where);
          if (bot === "Ezerbot") assert.equal(theme.nameFits, true, where);
        }
      }
    }
  }
});

test("from 1101 to 1440 the medium name stays whole with speed across three models and three bots", { timeout: 300000 }, async () => {
  const widths = sweep(1101, 1440, 40, [1140, 1440]);
  for (const width of widths) {
    for (const model of MODELS) {
      for (const bot of BOTS) {
        const report = await measure(width, 780, bot, { model, effort: DEFAULT_EFFORT, speed: "Standard" });
        for (const theme of report.themes) {
          const where = `${width} ${model} / ${bot} ${theme.theme} viewport=${theme.viewport} name=${theme.nameClient}/${theme.nameScroll} pill=${theme.pillLabel} speedGap=${theme.speedGap} speedTop=${theme.speedTop} voiceTop=${theme.voiceTop} bot=${theme.pieces.bot}`;
          assert.equal(theme.pillLabel, "none", where);
          assert.equal(theme.nameInside, true, where);
          assert.equal(theme.chevronInside, true, where);
          assert.ok(Math.abs(theme.speedTop - theme.voiceTop) <= 2, where);
          assert.ok(theme.speedGap >= 6, where);
          if (bot === "Ezerbot" || bot === MEDIUM_NAME) assert.equal(theme.nameFits, true, where);
        }
      }
    }
  }
});

test("a long name with speed stays clear of the headset below 400px", { timeout: 120000 }, async () => {
  for (const width of [360, 390]) {
    const report = await measure(width, 780, LONG_NAME, { model: CODEX_SPARK, effort: DEFAULT_EFFORT, speed: "Standard" });
    for (const theme of report.themes) {
      const nameOver = theme.nameBox.right - theme.headsetLeft;
      const where = `${width} ${theme.theme} viewport=${theme.viewport} headset=${theme.headsetLeft} nameOver=${nameOver} bot=${theme.pieces.bot} name=${theme.nameClient}/${theme.nameScroll} nameBox=${JSON.stringify(theme.nameBox)} botBox=${JSON.stringify(theme.botBox)} chevron=${JSON.stringify(theme.chevronBox)} speedTop=${theme.speedTop} voiceTop=${theme.voiceTop}`;
      assert.equal(theme.nameInside, true, where);
      assert.equal(theme.chevronInside, true, where);
      assert.ok(theme.botBox.right <= theme.headsetLeft + 0.5, where);
      assert.ok(theme.nameBox.right <= theme.headsetLeft + 0.5, where);
      assert.ok(theme.speedTop >= theme.voiceTop + 16, where);
    }
  }
});

test("with Codex Spark High and speed, the chevron stays inside the bot from 480 to 500", { timeout: 180000 }, async () => {
  for (let width = 480; width <= 500; width += 1) {
    const report = await measure(width, 780, "Ezerbot", { model: CODEX_SPARK_WIDE, effort: DEFAULT_EFFORT, speed: "Standard" });
    for (const theme of report.themes) {
      const over = theme.chevronBox.right - theme.botBox.right;
      const where = `${width} ${theme.theme} over=${over} name=${theme.nameClient}/${theme.nameScroll} bot=${theme.pieces.bot} speedTop=${theme.speedTop} voiceTop=${theme.voiceTop} chevron=${JSON.stringify(theme.chevronBox)} botBox=${JSON.stringify(theme.botBox)}`;
      assert.equal(theme.chevronInside, true, where);
      assert.equal(theme.nameInside, true, where);
      assert.ok(Math.abs(theme.speedTop - theme.voiceTop) <= 2, where);
      assert.ok(over <= 0.5, where);
    }
  }
});

test("from 360 to 371 the speed pill does not cover the headset", { timeout: 120000 }, async () => {
  for (let width = 360; width <= 371; width += 1) {
    const report = await measure(width, 780, "Ezerbot", { model: CODEX_SPARK_WIDE, effort: DEFAULT_EFFORT, speed: "Standard" });
    for (const theme of report.themes) {
      const where = `${width} ${theme.theme} overlap=${theme.speedHeadset.width}×${theme.speedHeadset.height} speedGap=${theme.speedGap} speedTop=${theme.speedTop} voiceTop=${theme.voiceTop} headset=${JSON.stringify(theme.headsetBox)} effort=${theme.effortClient}/${theme.effortScroll}`;
      assert.equal(theme.speedHeadset.height, 0, where);
      assert.ok(theme.speedTop >= theme.headsetBox.bottom - 0.5, where);
      assert.ok(theme.speedGap >= -0.5, where);
      assert.equal(theme.nameInside, true, where);
      assert.equal(theme.chevronInside, true, where);
    }
  }
});

test("the real header keeps the title clear of Notifications from 761 to 785", { timeout: 180000 }, async () => {
  for (const title of ["Nouvelle conversation", "Créer un groupe"]) {
    for (let width = 761; width <= 785; width += 1) {
      const report = await measure(width, 780, "Ezerbot", { header: "app", title, speed: "" });
      for (const theme of report.themes) {
        const where = `${width} « ${title} » ${theme.theme} titleGap=${theme.titleGap} title=${theme.titleClient}/${theme.titleScroll} min=${theme.titleMinWidth} ws=${theme.workspaceClient} titleBox=${JSON.stringify(theme.titleBox)} actions=${JSON.stringify(theme.actionsBox)}`;
        assert.equal(theme.titleText, title, where);
        assert.equal(theme.workspaceText, "Espace personnel", where);
        assert.ok(theme.titleGap >= -0.5, where);
        assert.equal(theme.titleClear, true, where);
      }
    }
  }
});

test("effort text stays visible from 360 to 600 without speed", { timeout: 300000 }, async () => {
  const widths = sweep(360, 600, 10, [371, 400, 480, 500, 501, 509, 600]);
  for (const width of widths) {
    for (const model of MODELS) {
      const report = await measure(width, 780, "Ezerbot", { model, effort: DEFAULT_EFFORT, speed: "" });
      for (const theme of report.themes) {
        const where = `${width} ${model} ${theme.theme} effort=${theme.effortClient}/${theme.effortScroll} model=${theme.modelClient}/${theme.modelScroll} modelGap=${theme.modelGap} effortGap=${theme.effortGap} bot=${theme.pieces.bot}`;
        assert.equal(theme.effortFits, true, where);
        assert.ok(theme.effortClient >= 40, where);
        assert.ok(theme.modelClient >= 40, where);
        assert.ok(theme.effortGap >= 0, where);
        assert.ok(theme.modelGap >= 0, where);
        assert.equal(theme.nameFits, true, where);
        assert.equal(theme.chevronInside, true, where);
      }
    }
  }
});

test("from 360 to 911 the name yields before the model, with and without speed", { timeout: 600000 }, async () => {
  const widths = sweep(360, 911, 10, [761, 764, 819, 820, 910, 911]);
  const names = ["Ezerbot", MEDIUM_NAME, LONG_NAME];
  for (const width of widths) {
    for (const model of MODELS) {
      for (const name of names) {
        for (const speed of ["", "Standard"]) {
          const report = await measure(width, 780, name, { model, effort: DEFAULT_EFFORT, speed });
          for (const theme of report.themes) {
            const sameRow = Boolean(speed) && Math.abs(theme.speedTop - theme.voiceTop) <= 2;
            const where = `${width} ${speed ? "speed" : "no-speed"} ${model} / ${name} ${theme.theme} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} effort=${theme.effortClient}/${theme.effortScroll} speedGap=${theme.speedGap} speedTop=${theme.speedTop} voiceTop=${theme.voiceTop} bot=${theme.pieces.bot}`;
            assert.ok(theme.nameClient >= 40, where);
            assert.equal(theme.nameInside, true, where);
            assert.equal(theme.chevronInside, true, where);
            assert.ok(theme.modelClient >= 40, where);
            assert.equal(theme.effortFits, true, where);
            assert.ok(theme.effortClient >= 40, where);
            if (!speed) {
              assert.ok(theme.modelGap >= 0, where);
              assert.ok(theme.effortGap >= 0, where);
            } else if (sameRow) {
              assert.ok(theme.speedGap >= 6, where);
              assert.equal(theme.speedFits, true, where);
            } else {
              assert.equal(theme.speedHeadset.height, 0, where);
              assert.ok(theme.speedTop >= theme.voiceTop + 16, where);
            }
          }
        }
      }
    }
  }
});

test("Ezerbot stays visible from 761 to 764 beside the wide model and speed", { timeout: 120000 }, async () => {
  for (let width = 761; width <= 764; width += 1) {
    const report = await measure(width, 780, "Ezerbot", { model: CODEX_SPARK_WIDE, effort: DEFAULT_EFFORT, speed: "Standard" });
    for (const theme of report.themes) {
      const where = `${width} ${theme.theme} name=${theme.nameClient}/${theme.nameScroll} model=${theme.modelClient}/${theme.modelScroll} speedGap=${theme.speedGap} bot=${theme.pieces.bot} nameBox=${JSON.stringify(theme.nameBox)} botBox=${JSON.stringify(theme.botBox)}`;
      assert.ok(theme.nameClient >= 40, where);
      assert.equal(theme.nameInside, true, where);
      assert.equal(theme.chevronInside, true, where);
      assert.ok(theme.speedGap >= 6, where);
      assert.ok(Math.abs(theme.speedTop - theme.voiceTop) <= 2, where);
    }
  }
});

test("the bot button focus ring sits inside the control", () => {
  const css = readFileSync(`${UI}/styles.css`, "utf8");
  assert.match(css, /\.composer-bot:focus-visible\s*\{[^}]*outline-offset:\s*-2px/);
});

test("suggestions sit against the composer at 820×560 instead of leaving a void", async () => {
  const report = await measure(820, 560, "Ezerbot", { pills: false, welcome: true });
  for (const theme of report.themes) {
    const where = `${theme.theme} viewport=${theme.viewport} below=${theme.welcome.below} above=${theme.welcome.above} banner=${theme.welcome.banner} suggestions=${JSON.stringify(theme.welcome.suggestions)}`;
    assert.equal(theme.welcome.banner, true, where);
    assert.equal(theme.welcome.suggestions.length, 4, where);
    assert.equal(theme.welcome.suggestions.every(Boolean), true, where);
    assert.ok(theme.welcome.below <= 32, where);
    assert.ok(theme.welcome.above > theme.welcome.below, where);
  }
});

test("attach plus and composer chevron are present and painted at both frames", async () => {
  for (const [width, height] of FRAMES) {
    const report = await measure(width, height, "Ezerbot");
    for (const theme of report.themes) {
      assert.equal(theme.plus.present, true, `plus missing at ${width}×${height} ${theme.theme}`);
      assert.equal(theme.chevron.present, true, `chevron missing at ${width}×${height} ${theme.theme}`);
      assert.equal(theme.plusVisible, true, `plus invisible at ${width}×${height} ${theme.theme}: ${JSON.stringify(theme.plus)}`);
      assert.equal(theme.chevronVisible, true, `chevron invisible at ${width}×${height} ${theme.theme}: ${JSON.stringify(theme.chevron)}`);
    }
  }
});
