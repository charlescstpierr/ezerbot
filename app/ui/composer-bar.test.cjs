// Run with: node --test app/ui/composer-bar.test.cjs
// Optional: CHROME_BIN=/path/to/chrome node --test app/ui/composer-bar.test.cjs
const assert = require("node:assert/strict");
const { spawn, spawnSync } = require("node:child_process");
const { existsSync, mkdtempSync, readFileSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
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

function page(name, { pills = true, model = "GPT-6.1 Sol", effort = "medium", speed = "", workspace = "" } = {}) {
  const speedPill = speed
    ? `<button class="tool-pill" type="button" data-picker="speed"><small>Vitesse</small><b>${speed}</b><svg class="icon"><use href="#i-chevron"/></svg></button>`
    : "";
  const tools = pills
    ? `<button class="tool-pill" type="button" data-picker="model"><small>Modèle</small><b>${model}</b><svg class="icon"><use href="#i-chevron"/></svg></button><button class="tool-pill" type="button" data-picker="effort"><small>Effort</small><b>${effort}</b><svg class="icon"><use href="#i-chevron"/></svg></button>${speedPill}`
    : "";
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
    </defs>
  </svg>
  <aside class="sidebar"></aside>
  <main class="main-panel">
    <header class="topbar">
      <div class="topbar-title">${workspace ? `<button class="workspace-label" type="button" disabled>${workspace}</button><span class="breadcrumb-separator" aria-hidden="true">/</span>` : ""}<h1 id="conversation-title">Nouvelle conversation</h1></div>
      <div class="topbar-actions">
        <button class="tool-pill notifications-button" type="button">Notifications</button>
        <button class="tool-pill" type="button">Créer un groupe</button>
        <button class="tool-pill" type="button">Membres</button>
      </div>
    </header>
    <div class="composer-dock"><div class="composer-area"><form class="composer" id="composer-form">
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
            <span class="voice-session-icon" aria-hidden="true"><svg class="icon" viewBox="0 0 24 24"><path d="M4 10v4m4-7v10m4-14v18m4-14v10m4-7v4"/></svg></span>
            <span class="voice-session-label">Mode voix</span>
          </button>
          <label class="voice-mode" title="Mémo vocal"><input type="checkbox">Mémo vocal</label>
          <button class="icon-button voice-mic" type="button" title="Dicter un message"></button>
        </div>
        <button class="send-button" id="send-button" type="button" aria-label="Envoyer le message" title="Envoyer le message"><svg class="icon"><use href="#i-send"/></svg></button>
      </div>
    </form></div></div>
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
      const speedLabel = document.querySelector('[data-picker="speed"] b');
      const modelBox = modelLabel ? boxOf(modelLabel) : { width: 0 };
      const modelButtonBox = modelButton ? boxOf(modelButton) : null;
      const effortButtonBox = effortButton ? boxOf(effortButton) : null;
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
        toolsClear: document.querySelector(".composer-tools").getBoundingClientRect().right <= voice.left + 0.5,
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
        titleText: document.getElementById("conversation-title").textContent,
        titleClient: document.getElementById("conversation-title").clientWidth,
        titleScroll: document.getElementById("conversation-title").scrollWidth,
        titleFits: document.getElementById("conversation-title").scrollWidth <= document.getElementById("conversation-title").clientWidth + 1,
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
    `--user-data-dir=${join(directory, "profile")}`,
    "--virtual-time-budget=2000",
    `--window-size=${width},${height}`,
    "--dump-dom",
    `file://${file}`,
  ], { stdio: ["ignore", "pipe", "ignore"] });
  let dom = "";
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      chrome.kill("SIGKILL");
      reject(new Error(`chrome did not report layout at ${width}×${height}`));
    }, 20000);
    chrome.stdout.on("data", (chunk) => {
      dom += chunk;
      const match = dom.match(/<pre id="qa-result">([\s\S]*?)<\/pre>/);
      if (!match) return;
      clearTimeout(timer);
      chrome.kill("SIGKILL");
      resolve(JSON.parse(match[1].replaceAll("&quot;", "\"").replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">")));
    });
    chrome.on("error", (error) => { clearTimeout(timer); reject(error); });
    chrome.on("exit", () => {
      if (!dom.includes("qa-result")) {
        clearTimeout(timer);
        reject(new Error(`chrome exited without a layout report at ${width}×${height}`));
      }
    });
  });
}

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
