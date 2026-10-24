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
// <pre id="qa-result">.
//
// A launch hang, measured on the loaded Mac (#387): the browser process forks, at about 135 ms
// into its start, to start its Crashpad handler (crashpad SpawnSubprocess(), called unconditionally
// for the browser process on macOS), and the fork child sits forever on a mutex inside libSystem's
// atfork handler (libxpc), while the browser waits for it (`sample`: fork -> xpc_atfork_child ->
// -[OS_xpc_object dealloc] -> __psynch_mutexwait). No Chrome switch turns that fork off (probed:
// --disable-breakpad, --disable-crash-reporter, --disable-crashpad, --no-crash-upload,
// --disable-features=Crashpad, --crash-dumps-dir and CHROME_HEADLESS all still start the handler),
// so the hang is replaced, not removed: waiting longer never helps (120 s with zero output).
//
// A launch is replaced only in the one state that proves it never started: after LAUNCH_MS it has
// printed NOTHING, neither on stdout nor on stderr, and has not exited. `--remote-debugging-port=0`
// makes that state observable: a browser that finished starting announces "DevTools listening on
// ws://..." on stderr (about 300 ms in, long before the page is navigated), so a silent stderr means
// the browser never got that far, and the page never ran. Any byte on stdout or stderr, an exit, or
// a page printed without the report fails at once with no new launch, and so does a wrong layout:
// the caller's assertions run once, on the one report we got. LAUNCH_MS stays the 20 s the tests
// always allowed, so a launch that is only slow (up to 13 s with 12 Chromes at once at load 240)
// still finishes inside it; a replaced launch gets a fresh Chrome and a fresh profile, 3 launches at
// most, the whole process group of each one killed.
const LAUNCH_ATTEMPTS = 3;
const LAUNCH_MS = 20000;
// After "exit", only the bytes already written to the pipes are still read: a descendant that keeps
// the pipes open must not delay the verdict (Node emits "close" only when they are all closed).
const EXIT_DRAIN_MS = 1000;

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
    "--remote-debugging-port=0",
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
    const timers = [];
    const settle = (action, value) => {
      if (settled) return;
      settled = true;
      timers.forEach(clearTimeout);
      stop();
      action(value);
    };
    // Never started: nothing on stdout, nothing on stderr, no "exit". The only replaceable state.
    const deadline = setTimeout(() => settle(resolve, { stalled: true, blocked: dom === "" && err === "", dom, err }), launchMs);
    timers.push(deadline);
    chrome.stdout.on("data", (chunk) => {
      dom += chunk;
      const text = qaResult(dom);
      if (text === null) return;
      try { settle(resolve, { report: JSON.parse(unescapeDom(text)) }); } catch (error) { settle(reject, error); }
    });
    chrome.stderr.on("data", (chunk) => { err += chunk; });
    chrome.on("error", (error) => settle(reject, error));
    const exited = () => settle(resolve, { exited: true, err });
    // An exit is final, whatever the pipes do: the deadline no longer applies.
    chrome.on("exit", () => { clearTimeout(deadline); stop(); timers.push(setTimeout(exited, EXIT_DRAIN_MS)); });
    chrome.on("close", () => { if (qaResult(dom) === null) exited(); });
  });
}

// launchMs may be an array of per-launch deadlines (the tests use it to stay independent of load).
async function dumpQaResult({ file, directory, width, height, label, launchMs = LAUNCH_MS }) {
  try {
    for (let attempt = 1; attempt <= LAUNCH_ATTEMPTS; attempt += 1) {
      const outcome = await dumpOnce({ file, profile: join(directory, `profile-${attempt}`), width, height, launchMs: Array.isArray(launchMs) ? launchMs[attempt - 1] : launchMs });
      if (outcome.report) return outcome.report;
      if (outcome.exited) throw new Error(`chrome exited without ${label}\n${outcome.err.slice(-500)}`);
      if (outcome.dom !== "") throw new Error(`chrome printed a page without ${label} within its deadline\n${outcome.err.slice(-500)}`);
      if (!outcome.blocked) throw new Error(`chrome started (it wrote to stderr) but did not report ${label} within its deadline\n${outcome.err.slice(-500)}`);
    }
    throw new Error(`chrome did not report ${label}: ${LAUNCH_ATTEMPTS} launches, none printed anything within ${[].concat(launchMs).join("/")} ms`);
  } finally {
    try { rmSync(directory, { recursive: true, force: true }); } catch { /* temp dir, best effort */ }
  }
}

module.exports = { chromeBinary, chromeOnPath, dumpQaResult };
