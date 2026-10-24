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
// Two hangs of Chrome's own browser process, both measured on the loaded Mac (#387), both with the
// page never reached and zero output on stdout. (1) At about 135 ms into its start the browser forks
// to start its Crashpad handler (crashpad SpawnSubprocess(), called unconditionally for the browser
// process on macOS) and the fork child sits forever on a mutex in libSystem's atfork handler (libxpc)
// while the browser waits for it: `sample` = fork -> xpc_atfork_child -> -[OS_xpc_object dealloc] ->
// __psynch_mutexwait; stderr stays empty, Chrome never gets as far as announcing DevTools. (2) After
// a normal start the browser's main thread blocks for good in rbs_acquire_appnap_assertion ->
// -[RBSConnection _connection] -> _dispatch_async_and_wait_f (RunningBoard, a macOS service), 0% CPU.
// No Chrome switch removes either: Chrome already registers NSAppSleepDisabled itself
// (content/app/mac_init.mm), and --disable-breakpad, --disable-crash-reporter, --disable-crashpad,
// --no-crash-upload, --disable-features=Crashpad, --crash-dumps-dir and CHROME_HEADLESS all still
// start the Crashpad handler. So waiting longer never helps (120 s without output): a stuck launch
// has to be replaced.
//
// The contract is the narrowest one that is proven: a launch is replaced ONLY when, at LAUNCH_MS
// after its start, Chrome has not announced "DevTools listening" (--remote-debugging-port=0 makes it
// the first line it writes to stderr: 300 launches out of 300, at most 3.1 s after the start). No
// announcement means the browser never started, which is hang (1), and nothing of the page ran.
// Every other outcome fails at once, with no new launch, because it could hide a real failure of the
// page or of the test: the announcement was seen and nothing was reported (this includes hang (2),
// which is therefore a real, visible failure, followed separately), any byte on stdout without a
// complete report, any line on stderr other than the two start-up lines Chrome always prints (the
// announcement and the CVDisplayLink error of this Mac), and an exit, whenever it arrives. The
// verdict is taken in one synchronous step at the deadline and nothing is waited for: no report
// is accepted after LAUNCH_MS, which stays the 20 s the tests always allowed, so a launch that is
// only slow (up to 13 s with 12 Chromes at once at load 240) still finishes inside it. A replaced
// launch gets a fresh Chrome and a fresh profile, 3 launches at most, the whole process group of
// each one killed.
const LAUNCH_ATTEMPTS = 3;
const LAUNCH_MS = 20000;
// After "exit", only the bytes already written to the pipes are still read: a descendant that keeps
// the pipes open must not delay the verdict (Node emits "close" only when they are all closed).
const EXIT_DRAIN_MS = 1000;
const DEVTOOLS_ANNOUNCEMENT = /^DevTools listening on ws:\/\/127\.0\.0\.1:\d+\/devtools\/browser\/[0-9a-f-]+$/;
const START_UP_LINES = [
  DEVTOOLS_ANNOUNCEMENT,
  /^\[\d+:\d+:\d+\/\d+\.\d+:ERROR:ui\/display\/mac\/cv_display_link_mac\.mm:\d+\] CVDisplayLinkCreateWithCGDisplay failed\. CVReturn: -?\d+$/,
];

function unescapeDom(text) {
  return text.replaceAll("&quot;", "\"").replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">");
}

function qaResult(dom) {
  const match = dom.match(/<pre id="qa-result">([\s\S]*?)<\/pre>/);
  return match && match[1].trim() !== "" ? match[1] : null;
}

const linesOf = (err) => err.split("\n").map((line) => line.trim()).filter(Boolean);
const onlyStartUpLines = (err) => linesOf(err).every((line) => START_UP_LINES.some((pattern) => pattern.test(line)));
const announcedDevtools = (err) => linesOf(err).some((line) => DEVTOOLS_ANNOUNCEMENT.test(line));

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
    let over = false; // the deadline has passed: no report is accepted any more
    let exited = false; // the exit is remembered whenever it arrives, and no new launch follows it
    const timers = [];
    const settle = (action, value) => {
      if (settled) return;
      settled = true;
      timers.forEach(clearTimeout);
      stop();
      action(value);
    };
    // Judges the launch on what it printed, in one step. `replace` is true only for a Chrome that never
    // announced DevTools and wrote nothing but start-up lines.
    const judge = () => {
      if (exited) return settle(resolve, { exited: true, err });
      if (dom !== "") return settle(resolve, { replace: false, why: "page", err });
      if (!onlyStartUpLines(err)) return settle(resolve, { replace: false, why: "stderr", err });
      if (announcedDevtools(err)) return settle(resolve, { replace: false, why: "started", err });
      return settle(resolve, { replace: true, why: "silent", err });
    };
    const deadline = setTimeout(() => {
      over = true;
      // One turn of the event loop lets an exit that happened before the deadline (but that a busy
      // test process has not delivered yet) be remembered before the launch is judged.
      setImmediate(judge);
    }, launchMs);
    timers.push(deadline);
    chrome.stdout.on("data", (chunk) => {
      dom += chunk;
      if (over) return;
      const text = qaResult(dom);
      if (text === null) return;
      try { settle(resolve, { report: JSON.parse(unescapeDom(text)) }); } catch (error) { settle(reject, error); }
    });
    chrome.stderr.on("data", (chunk) => { err += chunk; });
    chrome.on("error", (error) => settle(reject, error));
    const gone = () => settle(resolve, { exited: true, err });
    // An exit is final, whatever the pipes do: the deadline no longer applies.
    chrome.on("exit", () => { exited = true; clearTimeout(deadline); stop(); timers.push(setTimeout(gone, EXIT_DRAIN_MS)); });
    chrome.on("close", () => { if (qaResult(dom) === null) gone(); });
  });
}

const WHY = {
  page: "chrome printed a page without",
  stderr: "chrome wrote to stderr and did not report",
  started: "chrome started (it announced DevTools) but did not report",
};

// launchMs may be an array of per-launch deadlines (the tests use it to stay independent of load).
async function dumpQaResult({ file, directory, width, height, label, launchMs = LAUNCH_MS }) {
  try {
    for (let attempt = 1; attempt <= LAUNCH_ATTEMPTS; attempt += 1) {
      const outcome = await dumpOnce({ file, profile: join(directory, `profile-${attempt}`), width, height, launchMs: Array.isArray(launchMs) ? launchMs[attempt - 1] : launchMs });
      if (outcome.report) return outcome.report;
      if (outcome.exited) throw new Error(`chrome exited without ${label}\n${outcome.err.slice(-500)}`);
      if (!outcome.replace) throw new Error(`${WHY[outcome.why]} ${label} within its deadline\n${outcome.err.slice(-500)}`);
    }
    throw new Error(`chrome did not report ${label}: ${LAUNCH_ATTEMPTS} launches, none announced DevTools within ${[].concat(launchMs).join("/")} ms`);
  } finally {
    try { rmSync(directory, { recursive: true, force: true }); } catch { /* temp dir, best effort */ }
  }
}

module.exports = { chromeBinary, chromeOnPath, dumpQaResult };
