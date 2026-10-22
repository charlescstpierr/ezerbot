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
  const bars = memoPaths.flatMap((path) => path.split(/(?=[Mm])/)).filter((part) => /^[Mm][\d\s.,+-]*[Vv][\d\s.,+-]*$/.test(part));
  assert.equal(bars.length, 3, `le mémo vocal doit montrer trois barres d'onde dans la bulle, pas ${bars.length}`);
  assert.ok(memoPaths.some((path) => /[aA]/.test(path)), "la bulle du mémo vocal reste le contour");
  assert.match(session, /setAttribute\("aria-label","Mode voix"\)/);
  assert.match(session, /voice-session-label","Mode voix"/);
  assert.match(voice, /setAttribute\("aria-label", "Mémo vocal"\)/);
  assert.match(voice, /createTextNode\("Mémo vocal"\)/);
  assert.match(voiceCss, /\.voice-mode \{[^}]*font-size:\s*0/);
  const overlap = await measure(stroked(mode), stroked(memo));
  assert.equal(overlap.error, undefined, overlap.error);
  assert.ok(overlap.modeInk > 40, `mode voix icon is empty (${overlap.modeInk})`);
  assert.ok(overlap.memoInk > 40, `mémo vocal icon is empty (${overlap.memoInk})`);
  assert.ok(overlap.iou < 0.3, `icons still overlap (IoU ${overlap.iou.toFixed(2)}, ink ${overlap.modeInk}/${overlap.memoInk})`);
});

function setRootAttr(markup, name, value) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const existing = new RegExp(`(<svg\\b[^>]*\\s)${escaped}\\s*=\\s*("[^"]*"|'[^']*')`);
  if (existing.test(markup)) return markup.replace(existing, `$1${name}="${value}"`);
  return markup.replace("<svg", `<svg ${name}="${value}"`);
}

function paintRoot(markup, attrs) {
  return Object.entries(attrs).reduce((svg, [name, value]) => setRootAttr(svg, name, value), markup);
}

function paintIcon(markup, size) {
  return paintRoot(markup.replace(/\sclass="icon"/, ""), {
    xmlns: "http://www.w3.org/2000/svg",
    width: String(size),
    height: String(size),
    fill: "none",
    stroke: "black",
    "stroke-width": "1.55",
    "stroke-linecap": "round",
    "stroke-linejoin": "round",
  });
}

function paintMemo(markup, size) {
  return paintRoot(markup, {
    xmlns: "http://www.w3.org/2000/svg",
    width: String(size),
    height: String(size),
  });
}

function assertPaintableSvg(svg) {
  const open = svg.match(/^<svg\b([^>]*)>/);
  assert.ok(open, "racine svg");
  const names = [...open[1].matchAll(/(?:^|\s)([^\s=]+)\s*=/g)].map((match) => match[1]);
  const duplicates = names.filter((name, index) => names.indexOf(name) !== index);
  assert.deepEqual(duplicates, [], `attributs SVG dupliqués: ${duplicates.join(", ")}`);
}

function glyphPage(icons) {
  return `<!doctype html>
<meta charset="utf-8">
<pre id="qa-result"></pre>
<script>
  const icons = ${JSON.stringify(icons)};
  function ink(svg, size) {
    return new Promise((resolve, reject) => {
      const scale = 8;
      const image = new Image();
      image.onload = () => {
        const canvas = document.createElement("canvas");
        canvas.width = size * scale;
        canvas.height = size * scale;
        const context = canvas.getContext("2d");
        context.drawImage(image, 0, 0, canvas.width, canvas.height);
        const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
        let minX = canvas.width, minY = canvas.height, maxX = -1, maxY = -1;
        for (let y = 0; y < canvas.height; y += 1) {
          for (let x = 0; x < canvas.width; x += 1) {
            if (data[(y * canvas.width + x) * 4 + 3] <= 40) continue;
            if (x < minX) minX = x;
            if (y < minY) minY = y;
            if (x > maxX) maxX = x;
            if (y > maxY) maxY = y;
          }
        }
        const width = (maxX - minX + 1) / scale;
        const height = (maxY - minY + 1) / scale;
        resolve({
          width,
          height,
          dx: (minX + maxX) / 2 / scale - size / 2,
          dy: (minY + maxY) / 2 / scale - size / 2,
        });
      };
      image.onerror = () => reject(new Error("svg"));
      image.src = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
    });
  }
  Promise.all(icons.map((icon) => ink(icon.svg, icon.size).then((box) => ({ name: icon.name, size: icon.size, ...box }))))
    .then((glyphs) => { document.getElementById("qa-result").textContent = JSON.stringify({ glyphs }); })
    .catch((error) => { document.getElementById("qa-result").textContent = JSON.stringify({ error: String(error) }); });
</script>`;
}

function measureGlyphs(icons) {
  const directory = mkdtempSync(join(tmpdir(), "voice-glyphs-"));
  const file = join(directory, "glyphs.html");
  writeFileSync(file, glyphPage(icons));
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
      reject(new Error(`chrome did not measure glyphs\n${err.slice(-400)}`));
    }, 20000);
    chrome.stdout.on("data", (chunk) => {
      dom += chunk;
      const match = dom.match(/<pre id="qa-result">([\s\S]*?)<\/pre>/);
      if (!match || match[1].trim() === "") return;
      clearTimeout(timer);
      chrome.kill("SIGKILL");
      resolve(JSON.parse(match[1].replaceAll("&quot;", "\"").replaceAll("&amp;", "&")));
    });
    chrome.stderr.on("data", (chunk) => { err += chunk; });
    chrome.on("error", (error) => { clearTimeout(timer); reject(error); });
  });
}

test("voice memo glyph is centered and as large as the neighbouring icons", async () => {
  const voice = readFileSync(join(UI, "panels/voice.js"), "utf8");
  const session = readFileSync(join(UI, "panels/voice-conversation.js"), "utf8");
  const voiceCss = readFileSync(join(UI, "panels/voice.css"), "utf8");
  const memoRule = voiceCss.match(/\.voice-mode::before \{([^}]*)\}/);
  assert.ok(memoRule, "memo icon rule");
  assert.match(memoRule[1], /width:\s*16px/);
  assert.match(memoRule[1], /height:\s*16px/);
  const mic = voice.match(/<svg class="icon" aria-hidden="true" viewBox="0 0 24 24">[\s\S]*?<\/svg>/);
  assert.ok(mic, "dictation microphone");
  const icons = [
    { name: "memo", size: 16, svg: paintMemo(memoSvg(voiceCss), 16) },
    { name: "mode", size: 16, svg: paintIcon(modeMarkup(session), 16) },
    { name: "mic", size: 19, svg: paintIcon(mic[0], 19) },
  ];
  icons.forEach((icon) => assertPaintableSvg(icon.svg));
  const report = await measureGlyphs(icons);
  assert.equal(report.error, undefined, report.error);
  const glyphs = Object.fromEntries(report.glyphs.map((glyph) => [glyph.name, glyph]));
  const memo = glyphs.memo;
  const mode = glyphs.mode;
  assert.ok(Math.abs(memo.dx) <= 0.5 && Math.abs(memo.dy) <= 0.5, `mémo décentré de ${memo.dx.toFixed(2)} px, ${memo.dy.toFixed(2)} px`);
  assert.ok(memo.width >= mode.width * 0.9 && memo.height >= mode.height * 0.9, `mémo ${memo.width.toFixed(2)}×${memo.height.toFixed(2)} plus petit que le casque ${mode.width.toFixed(2)}×${mode.height.toFixed(2)}`);
  assert.ok(Math.max(memo.width, memo.height) >= Math.min(glyphs.mic.width, glyphs.mic.height), `mémo ${memo.width.toFixed(2)}×${memo.height.toFixed(2)} plus petit que le micro ${glyphs.mic.width.toFixed(2)}×${glyphs.mic.height.toFixed(2)}`);
});
