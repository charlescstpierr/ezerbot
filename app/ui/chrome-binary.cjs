// The one place the UI tests find a headless Chrome. CHROME_BIN wins; otherwise the first
// candidate that is installed (on PATH, or the macOS app bundle). Not found is a clear error.
const { spawn, spawnSync } = require("node:child_process");
const { existsSync, rmSync } = require("node:fs");
const { join } = require("node:path");

const CHROME_CANDIDATES = [
  "google-chrome",
  "google-chrome-stable",
  "chromium",
  "chromium-browser",
  "/usr/local/bin/google-chrome",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
];

function chromeOnPath(command) {
  if (command.includes("/")) return existsSync(command);
  return spawnSync("which", [command], { stdio: "ignore" }).status === 0;
}

function chromeBinary() {
  if (process.env.CHROME_BIN) return process.env.CHROME_BIN;
  const found = CHROME_CANDIDATES.find(chromeOnPath);
  if (!found) throw new Error("chrome introuvable : installez Chrome ou définissez CHROME_BIN");
  return found;
}

// Loads `file` in a fresh headless Chrome and returns the JSON the page wrote into
// <pre id="qa-result">. A Chrome that never prints anything is a launch hang, not a slow page:
// measured on the loaded Mac (#387), the browser's own fork child sits forever on a mutex inside
// libSystem's atfork handler, so waiting longer never helps (120 s with zero output and zero
// stderr). Such a launch is killed with its whole process group after LAUNCH_MS and replaced by a
// fresh Chrome and a fresh profile, at most LAUNCH_ATTEMPTS times. LAUNCH_MS stays the 20 s the
// tests always allowed, so a launch that is only slow (up to 11 s with 12 Chromes at once at load
// 240) still finishes inside it. Only a launch that printed
// nothing is retried: a Chrome that exits or prints a page without a report, or a report with the
// wrong layout, fails at once, and the caller's assertions run once, on the one report we got.
const LAUNCH_ATTEMPTS = 3;
const LAUNCH_MS = 20000;

function unescapeDom(text) {
  return text.replaceAll("&quot;", "\"").replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">");
}

function qaResult(dom) {
  const match = dom.match(/<pre id="qa-result">([\s\S]*?)<\/pre>/);
  return match && match[1].trim() !== "" ? match[1] : null;
}

function dumpOnce({ file, profile, width, height, launchMs }) {
  const chrome = spawn(chromeBinary(), [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--force-device-scale-factor=1",
    `--user-data-dir=${profile}`,
    "--virtual-time-budget=2000",
    `--window-size=${width},${height}`,
    "--dump-dom",
    `file://${file}`,
  ], { stdio: ["ignore", "pipe", "pipe"], detached: true });
  const stop = () => { try { process.kill(-chrome.pid, "SIGKILL"); } catch { /* already gone */ } };
  let dom = "";
  let err = "";
  return new Promise((resolve, reject) => {
    let settled = false;
    const settle = (action, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      stop();
      action(value);
    };
    const timer = setTimeout(() => settle(resolve, { stalled: true, silent: dom === "", err }), launchMs);
    chrome.stdout.on("data", (chunk) => {
      dom += chunk;
      const text = qaResult(dom);
      if (text === null) return;
      try { settle(resolve, { report: JSON.parse(unescapeDom(text)) }); } catch (error) { settle(reject, error); }
    });
    chrome.stderr.on("data", (chunk) => { err += chunk; });
    chrome.on("error", (error) => settle(reject, error));
    chrome.on("close", () => {
      if (qaResult(dom) === null) settle(resolve, { exited: true, err });
    });
  });
}

// launchMs may be an array of per-launch deadlines (the tests use it to stay independent of load).
async function dumpQaResult({ file, directory, width, height, label, launchMs = LAUNCH_MS }) {
  let last = "";
  try {
    for (let attempt = 1; attempt <= LAUNCH_ATTEMPTS; attempt += 1) {
      const outcome = await dumpOnce({ file, profile: join(directory, `profile-${attempt}`), width, height, launchMs: Array.isArray(launchMs) ? launchMs[attempt - 1] : launchMs });
      if (outcome.report) return outcome.report;
      if (outcome.exited) throw new Error(`chrome exited without ${label}\n${outcome.err.slice(-500)}`);
      if (!outcome.silent) throw new Error(`chrome printed a page without ${label} within its deadline\n${outcome.err.slice(-500)}`);
      last = outcome.err;
    }
    throw new Error(`chrome did not report ${label}: ${LAUNCH_ATTEMPTS} launches, none printed anything within ${[].concat(launchMs).join("/")} ms\n${last.slice(-500)}`);
  } finally {
    try { rmSync(directory, { recursive: true, force: true }); } catch { /* temp dir, best effort */ }
  }
}

module.exports = { chromeBinary, chromeOnPath, dumpQaResult };
