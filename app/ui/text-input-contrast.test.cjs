// Run with: node --test app/ui/text-input-contrast.test.cjs
// Optional: CHROME_BIN=/path/to/chrome node --test app/ui/text-input-contrast.test.cjs
const assert = require("node:assert/strict");
const { spawn, spawnSync } = require("node:child_process");
const { mkdtempSync, readdirSync, readFileSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");

const UI = __dirname;
const AA = 4.5;
const FIELDS = ["bot-name", "bot-description", "bot-instructions"];
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

function panelsCss() {
  return readdirSync(join(UI, "panels"))
    .filter((name) => name.endsWith(".css"))
    .sort()
    .map((name) => readFileSync(join(UI, "panels", name), "utf8"))
    .join("\n");
}

function page() {
  return `<!doctype html>
<html lang="fr" data-theme="light" data-accent="citron">
<head>
  <meta charset="utf-8">
  <link rel="stylesheet" href="file://${UI}/styles.css">
  <link rel="stylesheet" href="file://${UI}/chat.css">
  <link rel="stylesheet" href="file://${UI}/panels.bundle.css">
  <style>
    body { display: block !important; height: auto !important; overflow: visible !important; }
  </style>
</head>
<body>
  <div class="dialog bot-dialog" id="bot-dialog">
    <form id="bot-form">
      <label class="form-label" for="bot-name">Nom</label>
      <input class="text-input" id="bot-name" value="Relecteur">
      <label class="form-label bot-field" for="bot-description">Présentation <small>Ce que les autres voient</small></label>
      <textarea class="text-input" id="bot-description">Relit vos textes et propose des tournures plus claires.</textarea>
      <label class="form-label bot-field" for="bot-instructions">Personnalité <small>Comment il parle et se comporte. Privé.</small></label>
      <textarea class="text-input" id="bot-instructions">Tu parles avec douceur, tu tutoies, tu réponds en trois phrases maximum.</textarea>
      <label class="form-label bot-field" for="skill-name">Nom de la skill</label>
      <input class="text-input" id="skill-name" value="Relire un texte" disabled>
      <label class="form-label bot-field" for="routine-notes">Notes de la routine</label>
      <textarea class="text-input" id="routine-notes" disabled>Tous les matins, résume les messages.</textarea>
    </form>
  </div>
  <pre id="qa-result"></pre>
  <script>
    const channel = (value) => {
      const s = value / 255;
      return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    const parse = (value) => {
      const match = value.match(/rgba?\\(([^)]+)\\)/);
      if (!match) throw new Error(value);
      const parts = match[1].split(/[,/]/).map((part) => part.trim()).filter((part) => part && part !== "");
      const [r, g, b, a] = parts.map(Number);
      return { r, g, b, a: a === undefined || Number.isNaN(a) ? 1 : a };
    };
    const composite = (fg, bg) => ({
      r: fg.r * fg.a + bg.r * (1 - fg.a),
      g: fg.g * fg.a + bg.g * (1 - fg.a),
      b: fg.b * fg.a + bg.b * (1 - fg.a),
      a: 1,
    });
    const luminance = (color) => 0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b);
    const contrast = (fg, bg) => {
      const lighter = Math.max(luminance(fg), luminance(bg));
      const darker = Math.min(luminance(fg), luminance(bg));
      return (lighter + 0.05) / (darker + 0.05);
    };
    const opaqueBackground = (node) => {
      let color = { r: 255, g: 255, b: 255, a: 1 };
      const chain = [];
      for (let current = node; current && current.nodeType === 1; current = current.parentElement) chain.push(current);
      for (const current of chain.reverse()) {
        const background = parse(getComputedStyle(current).backgroundColor);
        if (background.a === 0) continue;
        color = composite(background, color);
        if (background.a === 1) color = background;
      }
      return color;
    };
    const measure = (theme) => {
      document.documentElement.dataset.theme = theme;
      return {
        theme,
        fields: ${JSON.stringify(FIELDS)}.map((id) => {
          const node = document.getElementById(id);
          const style = getComputedStyle(node);
          const ink = composite(parse(style.color), opaqueBackground(node));
          const ratio = contrast(ink, opaqueBackground(node));
          return {
            id,
            color: style.color,
            background: style.backgroundColor,
            fontSize: style.fontSize,
            ratio: Math.round(ratio * 100) / 100,
          };
        }),
        disabled: ["skill-name", "routine-notes"].map((id) => {
          const node = document.getElementById(id);
          const style = getComputedStyle(node);
          return { id, opacity: Number(style.opacity), cursor: style.cursor };
        }),
      };
    };
    document.getElementById("qa-result").textContent = JSON.stringify({ themes: ["light", "dark"].map(measure) });
  </script>
</body>
</html>`;
}

function measure() {
  const directory = mkdtempSync(join(tmpdir(), "text-input-contrast-"));
  writeFileSync(join(directory, "panels.bundle.css"), panelsCss());
  const file = join(directory, "form.html");
  const html = page().replaceAll(`file://${UI}/panels.bundle.css`, `file://${directory}/panels.bundle.css`);
  writeFileSync(file, html);
  const chrome = spawn(chromeBinary(), [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--force-device-scale-factor=1",
    `--user-data-dir=${join(directory, "profile")}`,
    "--virtual-time-budget=2000",
    "--window-size=900,800",
    "--dump-dom",
    `file://${file}`,
  ], { stdio: ["ignore", "pipe", "pipe"] });
  let dom = "";
  let err = "";
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      chrome.kill("SIGKILL");
      reject(new Error(`chrome did not report contrast\n${err.slice(-500)}`));
    }, 20000);
    chrome.stdout.on("data", (chunk) => {
      dom += chunk;
      const match = dom.match(/<pre id="qa-result">([\s\S]*?)<\/pre>/);
      if (!match) return;
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
        reject(new Error(`chrome exited without a contrast report\n${err.slice(-500)}`));
      }
    });
  });
}

test("typed text in the bot form stays WCAG AA on both themes", async () => {
  const report = await measure();
  for (const theme of report.themes) {
    assert.deepEqual(theme.fields.map((field) => field.id), FIELDS);
    for (const field of theme.fields) {
      assert.equal(field.fontSize, "12px", `${theme.theme} ${field.id}`);
      assert.ok(
        field.ratio >= AA,
        `${theme.theme} #${field.id} contraste ${field.ratio}:1 (seuil AA ${AA}:1) encre ${field.color} sur ${field.background}`,
      );
    }
  }
});

test("disabled text inputs and textareas read as inactive in both themes", async () => {
  const report = await measure();
  for (const theme of report.themes) {
    assert.deepEqual(theme.disabled.map((field) => field.id), ["skill-name", "routine-notes"]);
    for (const field of theme.disabled) {
      assert.ok(field.opacity < 1, `${theme.theme} #${field.id} opacité ${field.opacity} (curseur ${field.cursor})`);
    }
  }
});
