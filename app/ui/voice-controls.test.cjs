// Run with: node --test app/ui/voice-controls.test.cjs
// Optional: CHROME_BIN=/path/to/chrome node --test app/ui/voice-controls.test.cjs
const assert = require("node:assert/strict");
const { spawn, spawnSync } = require("node:child_process");
const { existsSync, mkdtempSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");

const UI = __dirname;
const WIDTHS = [500, 820, 901, 1140];
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

function page() {
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
  <aside class="sidebar"></aside>
  <main class="main-panel">
    <div class="composer-dock"><div class="composer-area"><form class="composer" id="composer-form">
      <div class="composer-bottom">
        <span class="composer-model" id="composer-model"></span>
        <div class="voice-controls">
          <button class="voice-session-toggle" type="button" aria-label="Mode voix" aria-pressed="false" title="Mode voix · parler et écouter les réponses sur ce Mac">
            <span class="voice-session-icon" aria-hidden="true"><svg class="icon" viewBox="0 0 24 24"><path d="M4 10v4m4-7v10m4-14v18m4-14v10m4-7v4"/></svg></span>
            <span class="voice-session-label">Mode voix</span>
          </button>
          <label class="voice-mode" title="Mémo vocal"><input type="checkbox">Mémo vocal</label>
          <button class="icon-button voice-mic" type="button" aria-pressed="false" title="Dicter un message"></button>
        </div>
        <button class="send-button" id="send-button" type="button" aria-label="Envoyer le message"></button>
      </div>
    </form></div></div>
  </main>
  <pre id="qa-result"></pre>
  <script>
    const NAMES = { "voice-session-toggle": "Mode voix", "voice-mode": "Mémo vocal", "voice-mic": "Dictée" };
    const measure = (theme) => {
      document.documentElement.dataset.theme = theme;
      const nodes = [...document.querySelector(".voice-controls").children];
      const controls = nodes.map((node) => {
        const style = getComputedStyle(node);
        const box = node.getBoundingClientRect();
        const name = Object.keys(NAMES).find((key) => node.classList.contains(key));
        return {
          name: NAMES[name],
          left: box.left,
          right: box.right,
          border: ["Top", "Right", "Bottom", "Left"].map((side) => parseFloat(style["border" + side + "Width"]) || 0),
          borderStyle: style.borderTopStyle,
        };
      });
      const gaps = controls.slice(1).map((control, index) => ({
        between: controls[index].name + " / " + control.name,
        gap: control.left - controls[index].right,
      }));
      return { theme, viewport: innerWidth, controls, gaps };
    };
    const report = { themes: ["dark", "light"].map(measure) };
    document.getElementById("qa-result").textContent = JSON.stringify(report);
  </script>
</body>
</html>`;
}

function measure(width, height) {
  const directory = mkdtempSync(join(tmpdir(), "voice-controls-"));
  const file = join(directory, "voice.html");
  writeFileSync(file, page());
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

test("Mode voix, Mémo vocal and Dictée have no border at rest in both themes", async () => {
  for (const width of WIDTHS) {
    const report = await measure(width, 780);
    for (const theme of report.themes) {
      assert.deepEqual(theme.controls.map((control) => control.name), ["Mode voix", "Mémo vocal", "Dictée"]);
      for (const control of theme.controls) {
        assert.deepEqual(
          control.border,
          [0, 0, 0, 0],
          `${control.name} has a ${control.borderStyle} border ${JSON.stringify(control.border)} at rest at ${width}px ${theme.theme}`,
        );
      }
    }
  }
});

test("voice controls keep at least 2px between them at 500, 820, 901 and 1140px in both themes", async () => {
  for (const width of WIDTHS) {
    const report = await measure(width, 780);
    for (const theme of report.themes) {
      for (const { between, gap } of theme.gaps) {
        assert.ok(gap >= 2, `${between} are ${gap}px apart at ${width}px ${theme.theme}`);
      }
    }
  }
});
