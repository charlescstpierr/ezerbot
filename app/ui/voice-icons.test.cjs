// Run with: node --test app/ui/voice-icons.test.cjs
const assert = require("node:assert/strict");
const { spawn, spawnSync } = require("node:child_process");
const { mkdtempSync, readFileSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");

const UI = __dirname;
const CHROME_CANDIDATES = [
  "google-chrome",
  "google-chrome-stable",
  "chromium",
  "chromium-browser",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
];

function chromeOnPath(command) {
  if (command.includes("/")) return require("node:fs").existsSync(command);
  return spawnSync("which", [command], { stdio: "ignore" }).status === 0;
}

function chromeBinary() {
  if (process.env.CHROME_BIN) return process.env.CHROME_BIN;
  return CHROME_CANDIDATES.find(chromeOnPath) || "google-chrome";
}

function commands(path) {
  return [...path.matchAll(/[a-z]/gi)].map((match) => match[0].toLowerCase());
}

function isBarWave(path) {
  const cmds = commands(path);
  const vertical = cmds.filter((command) => command === "v").length;
  const curves = cmds.filter((command) => "acsqt".includes(command)).length;
  return vertical >= 4 && curves === 0;
}

function modeMarkup(source) {
  const html = source.match(/icon\.innerHTML = '([^']+)'/);
  assert.ok(html, "mode voix icon markup");
  return html[1];
}

function memoSvg(css) {
  const encoded = css.match(/data:image\/svg\+xml,([^"]+)/);
  assert.ok(encoded, "mémo vocal icon mask");
  return decodeURIComponent(encoded[1]);
}

function pathsOf(markup) {
  return [...markup.matchAll(/\bd=(?:"([^"]+)"|'([^']+)')/g)].map((match) => match[1] || match[2]);
}

function stroked(markup) {
  const paths = pathsOf(markup);
  assert.ok(paths.length > 0, markup);
  const body = paths.map((path) => `<path d="${path}"/>`).join("");
  return `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;
}

function page(modeSvg, memoSvgText) {
  return `<!doctype html>
<meta charset="utf-8">
<pre id="qa-result"></pre>
<script>
  const svgs = ${JSON.stringify({ mode: modeSvg, memo: memoSvgText })};
  function raster(svg) {
    return new Promise((resolve, reject) => {
      const image = new Image();
      image.onload = () => {
        const canvas = document.createElement("canvas");
        canvas.width = 64; canvas.height = 64;
        const context = canvas.getContext("2d");
        context.drawImage(image, 0, 0, 64, 64);
        const data = context.getImageData(0, 0, 64, 64).data;
        const bits = [];
        for (let index = 3; index < data.length; index += 4) bits.push(data[index] > 40 ? 1 : 0);
        resolve(bits);
      };
      image.onerror = () => reject(new Error("svg"));
      image.src = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
    });
  }
  Promise.all([raster(svgs.mode), raster(svgs.memo)]).then(([mode, memo]) => {
    let intersection = 0, union = 0;
    for (let index = 0; index < mode.length; index += 1) {
      const either = mode[index] | memo[index];
      union += either;
      intersection += mode[index] & memo[index];
    }
    const ink = (bits) => bits.reduce((sum, bit) => sum + bit, 0);
    document.getElementById("qa-result").textContent = JSON.stringify({
      intersection, union, iou: union ? intersection / union : 1, modeInk: ink(mode), memoInk: ink(memo),
    });
  }).catch((error) => {
    document.getElementById("qa-result").textContent = JSON.stringify({ error: String(error) });
  });
</script>`;
}

function measure(modeSvg, memoSvgText) {
  const directory = mkdtempSync(join(tmpdir(), "voice-icons-"));
  const file = join(directory, "icons.html");
  writeFileSync(file, page(modeSvg, memoSvgText));
  const chrome = spawn(chromeBinary(), [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--force-device-scale-factor=1",
    `--user-data-dir=${join(directory, "profile")}`,
    "--virtual-time-budget=2000",
    "--window-size=200,200",
    "--dump-dom",
    `file://${file}`,
  ], { stdio: ["ignore", "pipe", "pipe"] });
  let dom = "";
  let err = "";
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      chrome.kill("SIGKILL");
      reject(new Error(`chrome did not compare icons\n${err.slice(-400)}`));
    }, 20000);
    chrome.stdout.on("data", (chunk) => {
      dom += chunk;
      const match = dom.match(/<pre id="qa-result">([\s\S]*?)<\/pre>/);
      if (!match || match[1].trim() === "") return;
      clearTimeout(timer);
      chrome.kill("SIGKILL");
      const json = match[1].replaceAll("&quot;", "\"").replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">");
      resolve(JSON.parse(json));
    });
    chrome.stderr.on("data", (chunk) => { err += chunk; });
    chrome.on("error", (error) => { clearTimeout(timer); reject(error); });
    chrome.on("exit", () => {
      if (!dom.includes("qa-result")) {
        clearTimeout(timer);
        reject(new Error(`chrome exited without an icon report\n${err.slice(-400)}`));
      }
    });
  });
}

test("voice mode and voice memo icons differ and keep accessible names", async () => {
  const voice = readFileSync(join(UI, "panels/voice.js"), "utf8");
  const session = readFileSync(join(UI, "panels/voice-conversation.js"), "utf8");
  const voiceCss = readFileSync(join(UI, "panels/voice.css"), "utf8");
  const mode = modeMarkup(session);
  const memo = memoSvg(voiceCss);
  const modePaths = pathsOf(mode);
  const memoPaths = pathsOf(memo);
  assert.equal(modePaths.some(isBarWave), false, `mode voix is still a bar wave: ${modePaths.join(" | ")}`);
  assert.equal(memoPaths.some(isBarWave), false, `mémo vocal is still a bar wave: ${memoPaths.join(" | ")}`);
  assert.match(session, /setAttribute\("aria-label","Mode voix"\)/);
  assert.match(session, /voice-session-label","Mode voix"/);
  assert.match(voice, /setAttribute\("aria-label", "Mémo vocal"\)/);
  assert.match(voice, /createTextNode\("Mémo vocal"\)/);
  assert.match(voiceCss, /\.voice-mode \{[^}]*font-size:\s*0/);
  const overlap = await measure(stroked(mode), stroked(memo));
  assert.equal(overlap.error, undefined, overlap.error);
  assert.ok(overlap.modeInk > 40, `mode voix icon is empty (${overlap.modeInk})`);
  assert.ok(overlap.memoInk > 40, `mémo vocal icon is empty (${overlap.memoInk})`);
  assert.ok(overlap.iou < 0.35, `icons still overlap (IoU ${overlap.iou.toFixed(2)}, ink ${overlap.modeInk}/${overlap.memoInk})`);
});
