// The one place the UI tests find a headless Chrome. CHROME_BIN wins; otherwise the first
// candidate that is installed (on PATH, or the macOS app bundle). Not found is a clear error.
const { execFile, spawn, spawnSync } = require("node:child_process");
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
// The contract is the narrowest one that is proven. A launch is replaced ONLY when, at LAUNCH_MS
// after its start, ALL of these hold: it has not exited; it has written nothing on stdout; it has
// written nothing on stderr but the two start-up lines Chrome always prints (the "DevTools listening"
// announcement and the CVDisplayLink error of this Mac); it has NOT announced DevTools (with
// --remote-debugging-port=0 the announcement is the first line Chrome writes to stderr: 300 launches
// out of 300, at most 3.1 s after the start); and NO process of it is a renderer. The last one is the
// proof that the page never began: every page is loaded by a renderer process (--type=renderer), which
// Chrome starts before the page can run anything, and in 100 launches of a real Chrome it appeared
// after the browser process, the GPU and utility processes, and before any output.
// The announcement alone proved nothing about the page (Chromium prints it on a separate thread that
// the navigation does not wait for), the renderer table does. The table is taken from `ps` on the
// process group of this Chrome (it is launched detached, so it leads its own group) and on the
// descendants of its pid, after the whole group has been stopped (SIGSTOP), so that nothing can start
// a renderer between the freeze and the listing; the group is killed right after. A table that cannot
// be read, that does not list this Chrome as the leader of its group, or that is not parseable, proves
// nothing: the launch fails and is not replaced.
// Every other outcome fails at once, with no new launch, because it could hide a real failure of the
// page or of the test: a renderer exists (the page began), the announcement was seen and nothing was
// reported (this includes hang (2), which is therefore a real, visible failure, followed separately),
// any byte on stdout without a complete report, any other line on stderr, an exit, whenever it
// arrives, and a table that proves nothing. The verdict is taken in one step at the deadline and
// nothing is waited for but the listing: LAUNCH_MS is absolute and stays the 20 s the tests always
// allowed (no report is accepted after it, whatever happened before, an exit included), so a launch
// that is only slow (up to 13 s with 12 Chromes at once at load 240) still finishes inside it. A
// replaced launch gets a fresh Chrome and a fresh profile, 3 launches at most, the whole process
// group of each one killed.
const LAUNCH_ATTEMPTS = 3;
const LAUNCH_MS = 20000;
// After "exit", only the bytes already written to the pipes are still read: a descendant that keeps
// the pipes open must not delay the verdict (Node emits "close" only when they are all closed).
const EXIT_DRAIN_MS = 1000;
// The process listing at the deadline may take a while on a loaded Mac; past this it proves nothing.
const PROCESS_LIST_MS = 10000;
const RENDERER_PROCESS = /(^|\s)--type=renderer(\s|$)/;
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

// `ps` rows: "pid ppid pgid command". The command may span lines (an argument with a newline): a line
// that does not start a row continues the previous one.
function psTable() {
  return new Promise((resolve, reject) => {
    execFile("ps", ["-axww", "-o", "pid=,ppid=,pgid=,command="], { maxBuffer: 64 << 20, timeout: PROCESS_LIST_MS }, (error, stdout) => (error ? reject(error) : resolve(stdout)));
  });
}

function parseProcesses(text) {
  const rows = [];
  for (const line of text.split("\n")) {
    const match = line.match(/^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\S.*)$/);
    if (match) rows.push({ pid: Number(match[1]), ppid: Number(match[2]), pgid: Number(match[3]), command: match[4] });
    else if (line.trim() !== "") {
      if (rows.length === 0) return null;
      rows[rows.length - 1].command += `\n${line}`;
    }
  }
  return rows;
}

// "present" | "none" | "unknown" for the Chrome whose pid is `root` and that leads its own group.
// "none" is only said of a table that lists `root` as the leader of its group; a renderer counts if it
// is in that group or descends from `root` (a descendant may have left the group).
function rendererState(text, root) {
  const rows = typeof text === "string" ? parseProcesses(text) : null;
  const leader = rows && rows.find((row) => row.pid === root);
  if (!leader || leader.pgid !== root) return "unknown";
  const mine = new Set([root]);
  for (let grew = true; grew;) {
    grew = false;
    for (const row of rows) if (!mine.has(row.pid) && mine.has(row.ppid)) { mine.add(row.pid); grew = true; }
  }
  return rows.some((row) => (row.pgid === root || mine.has(row.pid)) && RENDERER_PROCESS.test(row.command)) ? "present" : "none";
}

function dumpOnce({ file, profile, width, height, launchMs, listProcesses }) {
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
    // Judges the launch, in one step, on what it printed and on the process table taken at the
    // deadline. `replace` is true only for a Chrome that never announced DevTools, wrote nothing but
    // start-up lines and has no renderer.
    const judge = (snapshot) => {
      if (exited) return settle(resolve, { exited: true, err });
      if (dom !== "") return settle(resolve, { replace: false, why: qaResult(dom) === null ? "page" : "late", err });
      if (!onlyStartUpLines(err)) return settle(resolve, { replace: false, why: "stderr", err });
      if (announcedDevtools(err)) return settle(resolve, { replace: false, why: "started", err });
      const state = snapshot && "text" in snapshot ? rendererState(snapshot.text, chrome.pid) : "unknown";
      if (state === "present") return settle(resolve, { replace: false, why: "renderer", err });
      if (state === "unknown") return settle(resolve, { replace: false, why: "unproven", err });
      return settle(resolve, { replace: true, why: "silent", err });
    };
    const deadline = setTimeout(() => {
      over = true;
      // An exit that happened before the deadline (but that a busy test process has not delivered yet)
      // is remembered in the turn of the event loop that follows, before the launch is judged.
      if (exited) return void setImmediate(() => judge(null));
      // Freeze the whole group, then list: nothing of this Chrome can start a renderer in between.
      try { process.kill(-chrome.pid, "SIGSTOP"); } catch { /* already gone: its exit says so */ }
      Promise.resolve()
        .then(() => listProcesses(chrome.pid))
        .then((text) => ({ text }), (error) => ({ error }))
        .then((snapshot) => setImmediate(() => judge(snapshot)));
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
    // An exit is final, whatever the pipes do, and it does not move the deadline: no report is accepted
    // after LAUNCH_MS, so one that a descendant writes while the pipes drain is refused all the same.
    chrome.on("exit", () => { exited = true; stop(); timers.push(setTimeout(gone, EXIT_DRAIN_MS)); });
    chrome.on("close", () => { if (qaResult(dom) === null) gone(); });
  });
}

const WHY = {
  page: (label) => `chrome printed a page without ${label} within its deadline`,
  stderr: (label) => `chrome wrote to stderr and did not report ${label} within its deadline`,
  started: (label) => `chrome started (it announced DevTools) but did not report ${label} within its deadline`,
  renderer: (label) => `chrome started a renderer (the page began) but did not report ${label} within its deadline`,
  unproven: (label) => `chrome did not report ${label} within its deadline and its processes could not be read to prove that no renderer began, so it was not replaced`,
  late: (label) => `chrome reported ${label} only after its deadline, which is not accepted`,
};

// launchMs may be an array of per-launch deadlines (the tests use it to stay independent of load), and
// listProcesses (pid -> Promise of the `ps` table) is replaceable so that a test can make it fail.
async function dumpQaResult({ file, directory, width, height, label, launchMs = LAUNCH_MS, listProcesses = psTable }) {
  try {
    for (let attempt = 1; attempt <= LAUNCH_ATTEMPTS; attempt += 1) {
      const outcome = await dumpOnce({ file, profile: join(directory, `profile-${attempt}`), width, height, launchMs: Array.isArray(launchMs) ? launchMs[attempt - 1] : launchMs, listProcesses });
      if (outcome.report) return outcome.report;
      if (outcome.exited) throw new Error(`chrome exited without ${label}\n${outcome.err.slice(-500)}`);
      if (!outcome.replace) throw new Error(`${WHY[outcome.why](label)}\n${outcome.err.slice(-500)}`);
    }
    throw new Error(`chrome did not report ${label}: ${LAUNCH_ATTEMPTS} launches, none announced DevTools within ${[].concat(launchMs).join("/")} ms`);
  } finally {
    try { rmSync(directory, { recursive: true, force: true }); } catch { /* temp dir, best effort */ }
  }
}

module.exports = { chromeBinary, chromeOnPath, dumpQaResult, rendererState };
