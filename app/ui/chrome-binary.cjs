// The one place the UI tests find a headless Chrome. CHROME_BIN wins; otherwise the first
// candidate that is installed (on PATH, or the macOS app bundle). Not found is a clear error.
const { spawn, spawnSync } = require("node:child_process");
const http = require("node:http");
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
// Two hangs of Chrome's own browser process, both measured on the loaded Mac (#387), both with the
// page never reached and zero output on stdout. (1) At about 135 ms into its start the browser forks
// to start its Crashpad handler (crashpad SpawnSubprocess(), called unconditionally for the browser
// process on macOS) and the fork child sits forever on a mutex in libSystem's atfork handler (libxpc)
// while the browser waits for it: `sample` = fork -> xpc_atfork_child -> -[OS_xpc_object dealloc] ->
// __psynch_mutexwait; stderr stays empty. (2) After a normal start (DevTools announced) the browser's
// main thread blocks for good in rbs_acquire_appnap_assertion -> -[RBSConnection _connection] ->
// _dispatch_async_and_wait_f (RunningBoard, a macOS service), 0% CPU. No Chrome switch removes
// either: Chrome already registers NSAppSleepDisabled itself (content/app/mac_init.mm), and
// --disable-breakpad, --disable-crash-reporter, --disable-crashpad, --no-crash-upload,
// --disable-features=Crashpad, --crash-dumps-dir and CHROME_HEADLESS all still start the Crashpad
// handler. So the hang is replaced, not removed: waiting longer never helps (120 s without output).
//
// A launch is replaced only when, after LAUNCH_MS, it printed nothing on stdout, has not exited,
// wrote nothing to stderr beyond Chrome's own start-up lines (the "DevTools listening" announcement
// that --remote-debugging-port=0 adds, and the CVDisplayLink error every headless launch prints on
// this Mac), AND the browser process proves that the page never began: no DevTools announcement at
// all (it never started), or, asked at the deadline itself and not seen navigating PROBE_MS before,
// its DevTools endpoint does not answer (the main thread is stuck) or lists no page that left
// about:blank. A page that is stuck lives in the renderer process and cannot stop the browser from
// answering, so a stuck page never looks like this. Any other byte on stderr, any stdout, an exit,
// or a navigated page fails at once with no new launch, and so does a wrong layout: the caller's
// assertions run once, on the one report we got. LAUNCH_MS stays the 20 s the tests always allowed,
// so a launch that is only slow (up to 13 s with 12 Chromes at once at load 240) still finishes
// inside it; only the verdict on a launch that is already stuck waits for the DevTools answer, at
// most PROBE_MS more. A replaced launch gets a fresh Chrome and a fresh profile, 3 launches at
// most, the whole process group of each one killed.
const LAUNCH_ATTEMPTS = 3;
const LAUNCH_MS = 20000;
const PROBE_MS = 3000;
// After "exit", only the bytes already written to the pipes are still read: a descendant that keeps
// the pipes open must not delay the verdict (Node emits "close" only when they are all closed).
const EXIT_DRAIN_MS = 1000;
const START_UP_LINES = [
  /^DevTools listening on ws:\/\/127\.0\.0\.1:\d+\/devtools\/browser\/[0-9a-f-]+$/,
  /^\[\d+:\d+:\d+\/\d+\.\d+:ERROR:ui\/display\/mac\/cv_display_link_mac\.mm:\d+\] CVDisplayLinkCreateWithCGDisplay failed\. CVReturn: -?\d+$/,
];

function unescapeDom(text) {
  return text.replaceAll("&quot;", "\"").replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">");
}

function qaResult(dom) {
  const match = dom.match(/<pre id="qa-result">([\s\S]*?)<\/pre>/);
  return match && match[1].trim() !== "" ? match[1] : null;
}

const onlyStartUpLines = (err) => err.split("\n").map((line) => line.trim()).filter(Boolean).every((line) => START_UP_LINES.some((pattern) => pattern.test(line)));
const devtoolsPort = (err) => (err.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)\//) || [])[1];

// Asks the browser process, through its DevTools port, what it sees: "unanswered" (nothing within
// `ms`), "navigated" (a page target left about:blank), "no-page", or "error" (not a proof of anything).
function askBrowser(port, ms) {
  return new Promise((resolve) => {
    const request = http.get({ host: "127.0.0.1", port, path: "/json/list", timeout: ms }, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => { body += chunk; });
      response.on("error", () => resolve("error"));
      response.on("end", () => {
        try { resolve(JSON.parse(body).some((target) => target.type === "page" && target.url !== "about:blank") ? "navigated" : "no-page"); } catch { resolve("error"); }
      });
    });
    request.on("timeout", () => { request.destroy(); resolve("unanswered"); });
    request.on("error", () => resolve("error"));
  });
}

function dumpOnce({ file, profile, width, height, launchMs, probeMs }) {
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
    // The browser is asked twice: just before the deadline, where a page seen navigating disqualifies
    // the launch for good, and again AT the deadline, where the answer is fresh (an earlier "no page"
    // proves nothing about a page that began since).
    let navigated = false;
    const look = async () => {
      const port = devtoolsPort(err);
      if (!port) return null;
      const seen = await askBrowser(port, Math.max(probeMs - 100, 1));
      if (seen === "navigated") navigated = true;
      return seen;
    };
    timers.push(setTimeout(look, Math.max(launchMs - probeMs, 0)));
    const deadline = setTimeout(async () => {
      const stalled = (replace, why) => settle(resolve, { stalled: true, replace, why, err });
      // What the launch printed disqualifies it, now and again after the (slow) last look at the browser.
      const printed = () => (dom !== "" ? "page" : onlyStartUpLines(err) ? null : "stderr");
      if (printed()) return stalled(false, printed());
      if (!devtoolsPort(err)) return stalled(err.trim() === "", "stderr"); // silent: never started
      const seen = await look();
      if (printed()) return stalled(false, printed());
      return stalled(!navigated && (seen === "unanswered" || seen === "no-page"), navigated ? "navigated" : "unproven");
    }, launchMs);
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

const WHY = {
  page: "chrome printed a page without",
  stderr: "chrome wrote to stderr and did not report",
  navigated: "chrome began loading the page but did not report",
  unproven: "chrome started but its DevTools endpoint could not show that the page never began, and it did not report",
};

// launchMs may be an array of per-launch deadlines (the tests use it to stay independent of load).
async function dumpQaResult({ file, directory, width, height, label, launchMs = LAUNCH_MS, probeMs = PROBE_MS }) {
  try {
    for (let attempt = 1; attempt <= LAUNCH_ATTEMPTS; attempt += 1) {
      const outcome = await dumpOnce({ file, profile: join(directory, `profile-${attempt}`), width, height, launchMs: Array.isArray(launchMs) ? launchMs[attempt - 1] : launchMs, probeMs });
      if (outcome.report) return outcome.report;
      if (outcome.exited) throw new Error(`chrome exited without ${label}\n${outcome.err.slice(-500)}`);
      if (!outcome.replace) throw new Error(`${WHY[outcome.why]} ${label} within its deadline\n${outcome.err.slice(-500)}`);
    }
    throw new Error(`chrome did not report ${label}: ${LAUNCH_ATTEMPTS} launches, none began loading the page within ${[].concat(launchMs).join("/")} ms`);
  } finally {
    try { rmSync(directory, { recursive: true, force: true }); } catch { /* temp dir, best effort */ }
  }
}

module.exports = { chromeBinary, chromeOnPath, dumpQaResult };
