// Run with: node --test app/ui/composer-bar.test.cjs
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { mkdtempSync, readFileSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");

const UI = __dirname;
const FRAMES = [[1140, 780], [820, 560]];

function page(name) {
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
          <span class="composer-tools" id="composer-tools"><button class="tool-pill" type="button"><small>Modèle</small><b>GPT-6 Sol</b><svg class="icon"><use href="#i-chevron"/></svg></button><button class="tool-pill" type="button"><small>Effort</small><b>Moyen</b><svg class="icon"><use href="#i-chevron"/></svg></button></span>
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
    const measure = (theme) => {
      document.documentElement.dataset.theme = theme;
      const name = document.getElementById("composer-bot-name");
      const voice = document.querySelector(".voice-controls").getBoundingClientRect();
      const send = document.getElementById("send-button").getBoundingClientRect();
      const nameBox = name.getBoundingClientRect();
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

function measure(width, height, name) {
  const directory = mkdtempSync(join(tmpdir(), "composer-bar-"));
  const file = join(directory, "bar.html");
  writeFileSync(file, page(name).replace("FRAMES_PLACEHOLDER", "null"));
  const chrome = spawn("google-chrome", [
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
