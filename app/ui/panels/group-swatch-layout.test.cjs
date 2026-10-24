// Run with: node --test app/ui/panels/group-swatch-layout.test.cjs
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { mkdtempSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");
const { chromeBinary } = require("../chrome-binary.cjs");

const UI = join(__dirname, "..");
const COLORS = [
  ["citron", "Jaune citron", "#E6FF3D"],
  ["orange", "Orange vif", "#FF5C1F"],
  ["ambre", "Ambre", "#FFB224"],
  ["menthe", "Vert menthe", "#35E0A1"],
  ["cyan", "Cyan", "#22D3EE"],
  ["magenta", "Rose magenta", "#FF3D8B"],
  ["vermillon", "Rouge vermillon", "#FF4530"],
  ["violet", "Violet", "#9B7BFF"],
  ["blanc", "Sans couleur", "#F5F5F5"],
];

function swatches(id) {
  return COLORS.map(([accent, label, swatch]) => `<button type="button" data-avatar-color="${accent}" aria-label="${label}"><i style="--swatch:${swatch}" aria-hidden="true"></i><span>${label}</span></button>`).join("");
}

function page() {
  return `<!doctype html>
<html lang="fr" data-theme="light" data-accent="citron">
<head>
  <meta charset="utf-8">
  <link rel="stylesheet" href="file://${UI}/styles.css">
  <link rel="stylesheet" href="file://${UI}/panels/multibot.css">
  <style>body { display: block; height: auto; overflow: visible; }</style>
</head>
<body>
  <dialog open class="dialog small-dialog" id="rename-dialog">
    <p class="form-label" id="rename-avatar-label">Avatar</p>
    <div class="accent-options" id="rename-avatar-colors" role="group" aria-labelledby="rename-avatar-label">${swatches("rename")}</div>
  </dialog>
  <dialog open class="dialog group-dialog" id="group-dialog">
    <section class="setting-section group-profile">
      <div class="group-avatar-row">
        <span class="group-mark group-mark-preview" aria-hidden="true">A</span>
        <div class="accent-options" id="group-avatar-colors" role="group">${swatches("group")}</div>
      </div>
    </section>
  </dialog>
  <pre id="qa-result"></pre>
  <script>
    const measure = (root) => [...root.querySelectorAll("button")].map((button) => {
      const span = button.querySelector("span");
      return {
        label: span.textContent,
        client: span.clientWidth,
        scroll: span.scrollWidth,
        fits: span.scrollWidth <= span.clientWidth + 1,
      };
    });
    document.getElementById("qa-result").textContent = JSON.stringify({
      viewport: [window.innerWidth, window.innerHeight],
      rename: measure(document.getElementById("rename-avatar-colors")),
      group: measure(document.getElementById("group-avatar-colors")),
    });
  </script>
</body>
</html>`;
}

function measure() {
  const directory = mkdtempSync(join(tmpdir(), "group-swatches-"));
  const file = join(directory, "swatches.html");
  writeFileSync(file, page());
  const chrome = spawn(chromeBinary(), [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--force-device-scale-factor=1",
    `--user-data-dir=${join(directory, "profile")}`,
    "--virtual-time-budget=2000",
    "--window-size=1140,780",
    "--dump-dom",
    `file://${file}`,
  ], { stdio: ["ignore", "pipe", "ignore"] });
  let dom = "";
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      chrome.kill("SIGKILL");
      reject(new Error("chrome did not report swatch layout"));
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
        reject(new Error("chrome exited without a swatch layout report"));
      }
    });
  });
}

test("à 1140 px, chaque nom de pastille du groupe s’affiche en entier", async () => {
  const report = await measure();
  assert.ok(report.viewport[0] >= 1140, JSON.stringify(report.viewport));
  for (const [where, rows] of [["rename", report.rename], ["group", report.group]]) {
    assert.equal(rows.length, COLORS.length, where);
    for (const [, label] of COLORS) {
      const row = rows.find((item) => item.label === label);
      assert.ok(row, `${where} ${label}`);
      assert.equal(row.fits, true, `${where} « ${label} » coupé (${row.client}/${row.scroll})`);
    }
  }
});
