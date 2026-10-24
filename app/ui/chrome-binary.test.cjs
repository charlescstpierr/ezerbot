// Run with: node --test app/ui/chrome-binary.test.cjs
// dumpQaResult() against a stand-in browser script (its processes are listed by the real `ps`, as for a
// real Chrome; a stand-in DevTools port where a test needs one), so no real Chrome and no load
// dependence.
const assert = require("node:assert/strict");
const { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const http = require("node:http");
const net = require("node:net");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");
const { dumpQaResult, rendererState } = require("./chrome-binary.cjs");

const REPORT = '<html><body><pre id="qa-result">{"gap":3,"name":"a &amp; b"}</pre></body></html>';
const DISPLAY_LINK = "[1:2:1024/130000.000000:ERROR:ui/display/mac/cv_display_link_mac.mm:188] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670";

// A stand-in Chrome: `script` is the body of a /bin/sh program, started with the real Chrome's
// arguments (so `$*` names the profile of the launch). The counter file gets one line per launch
// that got as far as running. launchMs are the deadlines of the launches: tiny for a launch that
// must stay silent (a slower Mac only makes it more silent), long for one that must print, so no
// test depends on how loaded the Mac is. A launch that must print to stderr and then stall is the one
// place a deadline must be long enough for the stand-in to start: STARTED_MS, against the few ms of
// work it needs (2.1 s at most was measured with the whole UI suite running at load 130).
const STARTED_MS = 5000;

function withBrowser(script, launchMs, run) {
  const directory = mkdtempSync(join(tmpdir(), "chrome-binary-test-"));
  const counter = join(directory, "launches");
  const browser = join(directory, "chrome");
  writeFileSync(counter, "");
  writeFileSync(browser, `#!/bin/sh\necho x >> "${counter}"\n${script}\n`);
  chmodSync(browser, 0o755);
  const previous = process.env.CHROME_BIN;
  process.env.CHROME_BIN = browser;
  const work = mkdtempSync(join(tmpdir(), "chrome-binary-work-"));
  const options = { file: join(work, "page.html"), directory: work, width: 500, height: 780, label: "a test report", launchMs };
  const launches = () => readFileSync(counter, "utf8").split("\n").filter(Boolean).length;
  return run(options, launches).finally(() => {
    if (previous === undefined) delete process.env.CHROME_BIN;
    else process.env.CHROME_BIN = previous;
    rmSync(directory, { recursive: true, force: true });
    rmSync(work, { recursive: true, force: true });
  });
}

// A stand-in DevTools port. `targets` is the list /json/list answers, or a function of the call
// number (1, 2, ...) returning that list; null = the browser's main thread is stuck: the connection
// is accepted and never answered. chrome-binary.cjs no longer asks the browser anything: a launch
// that announced DevTools and did not report fails at its deadline whatever this port would say.
// The port is there to put the stand-in in the situation where the previous contract waited for it
// and replaced the launch, so that these tests fail on that contract.
function withDevtools(targets, run) {
  const sockets = new Set();
  let calls = 0;
  const server = http.createServer((request, response) => {
    calls += 1;
    const answer = typeof targets === "function" ? targets(calls) : targets;
    if (answer === null) return;
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify(answer));
  });
  server.on("connection", (socket) => { sockets.add(socket); socket.on("error", () => {}); });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", resolve)).then(() => run(server.address().port)).finally(() => {
    sockets.forEach((socket) => socket.destroy());
    server.close();
  });
}

const announce = (port) => `echo 'DevTools listening on ws://127.0.0.1:${port}/devtools/browser/00000000-0000-0000-0000-000000000001' >&2`;
const secondLaunchReports = (stall) => `case "$*" in\n*profile-2*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) ${stall};;\nesac`;

// A launch is replaced ONLY when, at its deadline, it has not announced "DevTools listening", has no
// renderer process, and has not exited nor written anything but start-up lines.
test("a launch that prints nothing is replaced by a fresh Chrome and the report is returned once", () =>
  withBrowser(`case "$*" in\n*profile-3*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) exec sleep 60;;\nesac`, [300, 300, 30000], async (options) => {
    const report = await dumpQaResult(options);
    assert.deepEqual(report, { gap: 3, name: "a & b" });
    assert.equal(existsSync(options.directory), false, "the temp directory is removed");
  }));

test("a launch that never prints fails after three launches and says so", () =>
  withBrowser("exec sleep 60", 300, async (options) => {
    await assert.rejects(dumpQaResult(options), /chrome did not report a test report: 3 launches, none announced DevTools within 300 ms/);
  }));

test("a launch that never announced DevTools, whatever start-up line it printed, is replaced and the report of the second launch is accepted", () =>
  withBrowser(`case "$*" in\n*profile-2*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) echo '${DISPLAY_LINK}' >&2; exec sleep 60;;\nesac`, [300, 30000, 30000], async (options) => {
    // Only the second launch reports; the first one may be killed before it ran a single line.
    assert.deepEqual(await dumpQaResult(options), { gap: 3, name: "a & b" });
  }));

test("Chrome is started with a DevTools port, so that a browser that never started is told from one that is running", () =>
  withBrowser(`case "$*" in\n*--remote-debugging-port=0*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) exec sleep 60;;\nesac`, [300, 300, 30000], async (options, launches) => {
    assert.deepEqual(await dumpQaResult(options), { gap: 3, name: "a & b" });
    assert.equal(launches(), 1);
  }));


// The renderer table. A stand-in Chrome has a renderer when one of its descendants has `--type=renderer`
// in its command line (argv[0] of a `sleep`, so that `ps` shows it as a real Chrome's renderer is).
const PROCESS = (type, seconds, { leaveGroup = false } = {}) =>
  `perl ${leaveGroup ? "-MPOSIX " : ""}-e '${leaveGroup ? "POSIX::setsid(); " : ""}exec {"sleep"} "--type=${type}", "${seconds}"' &`;
const NO_PROCESS_TABLE = "could not be read to prove that no renderer began";

test("a launch with other Chrome processes but no renderer is replaced, and the report of the second launch is accepted", () =>
  withBrowser(`case "$*" in\n*profile-2*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) ${PROCESS("gpu-process", 60)}\n${PROCESS("utility", 60)}\nexec sleep 60;;\nesac`, [STARTED_MS, 30000, 30000], async (options, launches) => {
    assert.deepEqual(await dumpQaResult(options), { gap: 3, name: "a & b" });
    assert.equal(launches(), 2);
  }));

// A renderer is the page: once one exists, the freeze is a real failure, not a launch to replace.
test("a launch with a renderer that then stalls, silent on stdout and stderr, fails after its deadline, with no new launch", () =>
  withBrowser(`${secondLaunchReports(`${PROCESS("renderer", 60)}\nexec sleep 60`)}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome started a renderer \(the page began\) but did not report a test report within its deadline/);
    assert.equal(launches(), 1);
  }));

test("a renderer that left the process group of the Chrome still counts: the launch fails after its deadline, with no new launch", () =>
  withBrowser(`${secondLaunchReports(`${PROCESS("renderer", 7, { leaveGroup: true })}\nexec sleep 60`)}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome started a renderer \(the page began\) but did not report a test report within its deadline/);
    assert.equal(launches(), 1);
  }));

// The list of the processes is the proof: a list that cannot be read, or that does not show this Chrome
// at the head of its group, proves nothing, and then the launch fails instead of being replaced.
const UNREADABLE_TABLES = {
  "ps fails": () => Promise.reject(new Error("ps failed")),
  "ps throws": () => { throw new Error("ps missing"); },
  "an empty table": () => Promise.resolve(""),
  "a table that is not one": () => Promise.resolve("this is not a process table\nat all"),
  "a table without this Chrome": () => Promise.resolve("    1     0     1 /sbin/launchd\n  100     1   100 /usr/bin/something"),
  "a table where this Chrome does not lead its group": (pid) => Promise.resolve(`${pid} 1 1 /bin/chrome\n`),
};
for (const [name, listProcesses] of Object.entries(UNREADABLE_TABLES)) {
  test(`${name}: a silent launch with no renderer to be seen fails after its deadline, with no new launch`, () =>
    withBrowser(secondLaunchReports("exec sleep 60"), [300, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult({ ...options, listProcesses }), new RegExp(NO_PROCESS_TABLE));
      assert.ok(launches() <= 1, "no second launch"); // the 300 ms deadline may kill the first one before it ran
    }));
}

test("without a `ps` to run, a silent launch fails after its deadline, with no new launch", () => {
  const empty = mkdtempSync(join(tmpdir(), "chrome-binary-nops-"));
  const path = process.env.PATH;
  // `sleep` by its full path: the stand-in inherits this PATH.
  return withBrowser(`case "$*" in\n*profile-2*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) exec /bin/sleep 60;;\nesac`, [300, 30000, 30000], async (options, launches) => {
    process.env.PATH = empty;
    try {
      await assert.rejects(dumpQaResult(options), new RegExp(NO_PROCESS_TABLE));
    } finally {
      process.env.PATH = path;
    }
    assert.ok(launches() <= 1, "no second launch");
  }).finally(() => rmSync(empty, { recursive: true, force: true }));
});

test("rendererState reads a ps table: renderer in the group or among the descendants, nothing else", () => {
  const table = (...rows) => rows.join("\n");
  const head = "  500   400   500 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --headless=new";
  assert.equal(rendererState(table(head), 500), "none");
  assert.equal(rendererState(table(head, "  501   500   500 Helper --type=gpu-process --foo"), 500), "none");
  assert.equal(rendererState(table(head, "  501   500   500 Helper --type=utility", "  502   500   500 Helper --type=renderer --lang=en"), 500), "present");
  // A renderer that left the group but descends from the Chrome (through another process).
  assert.equal(rendererState(table(head, "  501   500   500 Helper --type=utility", "  502   501   502 Helper --type=renderer"), 500), "present");
  // A renderer in the group whose parent died.
  assert.equal(rendererState(table(head, "  502     1   500 Helper --type=renderer"), 500), "present");
  // Not this Chrome's: another group, not descending from it.
  assert.equal(rendererState(table(head, "  900     1   900 Chrome --type=renderer"), 500), "none");
  // Only the whole word counts, and only in the command line.
  assert.equal(rendererState(table(head, "  501   500   500 Helper --type=renderer-extra --x=--type=renderer"), 500), "none");
  // A command line that spans lines belongs to its row.
  assert.equal(rendererState(table(head, "  501   500   500 sh -c 'echo", "x' --type=renderer"), 500), "present");
  assert.equal(rendererState(table(head, "  501   500   500 sh -c 'echo", "x' --type=utility"), 500), "none");
  // Nothing is proven by a table that does not show the Chrome at the head of its group.
  assert.equal(rendererState("", 500), "unknown");
  assert.equal(rendererState(undefined, 500), "unknown");
  assert.equal(rendererState("garbage", 500), "unknown");
  assert.equal(rendererState("  501   500   500 Helper", 500), "unknown");
  assert.equal(rendererState("  500   400   499 Chrome", 500), "unknown");
});

// Once DevTools is announced, the browser has started: every freeze is a real failure, whatever its
// DevTools port does (silent, about:blank, a page that began), and whatever the stand-in says next.
test("a browser that announced DevTools and then stalls fails after its deadline, with no new launch, even if its DevTools port does not answer", () =>
  withDevtools(() => null, (port) =>
    withBrowser(`${announce(port)}\necho '${DISPLAY_LINK}' >&2\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult(options), /chrome started \(it announced DevTools\) but did not report a test report within its deadline\n/);
      assert.equal(launches(), 1);
    })));

test("a browser that announced DevTools and then stalls fails after its deadline, with no new launch, even if its DevTools port shows only about:blank", () =>
  withDevtools(() => [{ type: "page", url: "about:blank" }], (port) =>
    withBrowser(`${announce(port)}\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult(options), /chrome started \(it announced DevTools\) but did not report a test report within its deadline/);
      assert.equal(launches(), 1);
    })));

test("a browser that announced DevTools and then stalls fails after its deadline, with no new launch, even if no port answers at all", () =>
  withBrowser(`echo 'DevTools listening on ws://127.0.0.1:9/devtools/browser/00000000-0000-0000-0000-000000000001' >&2\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome started \(it announced DevTools\) but did not report a test report within its deadline/);
    assert.equal(launches(), 1);
  }));

// Exits.
test("a Chrome that exits without a report fails at once, with no new launch", () =>
  withBrowser("echo boom >&2; exit 3", 30000, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report\nboom/);
    assert.equal(launches(), 1);
  }));

// The descendant leaves the process group (setsid) before the stand-in exits, and keeps the pipes
// open for 20 s: "close" comes long after the deadline, only "exit" tells this Chrome is gone.
const HOLD_PIPES = `perl -MPOSIX -e 'POSIX::setsid(); open(F, ">", shift); close(F); exec "sleep", "20"' "$0.ready" &\nwhile [ ! -e "$0.ready" ]; do sleep 0.05; done`;

test("a Chrome that exited while a descendant keeps its pipes open fails at once, with no new launch", () =>
  withBrowser(`${HOLD_PIPES}\nexit 3`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report/);
    assert.equal(launches(), 1);
  }));

// The stand-in waits for an absolute time (the test's start plus `afterMs`), so how long it took to
// start does not matter, only that it is not later than the margin.
const waitUntil = (epochMs) => `perl -MTime::HiRes=time,sleep -e 'my $d = $ARGV[0] - time(); sleep($d) if $d > 0' ${epochMs / 1000}`;

test("a silent Chrome that exits just before its deadline, pipes held open, fails at the exit, with no new launch", () => {
  const exitAt = Date.now() + 3500;
  return withBrowser(`${HOLD_PIPES}\n${waitUntil(exitAt)}\nexit 3`, 4000, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report/);
    assert.equal(launches(), 1);
  });
});

// The test process is blocked (asleep, not spinning) from 0.1 s to 6 s, so the stand-in's exit (at
// 3 s) and the 1 s deadline are both overdue when the event loop runs again, the deadline first. The
// exit still counts: a Chrome that is gone is never replaced. (The block starts in a check-phase
// callback, so the next turn of the loop runs the overdue timers before it reads the pipes and signals.)
test("an exit that was not yet delivered when the deadline fired still fails the launch, with no new launch", () => {
  const exitAt = Date.now() + 3000;
  setTimeout(() => setImmediate(() => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5900)), 100);
  return withBrowser(`${waitUntil(exitAt)}\nexit 3`, [1000, 30000, 30000], async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report/);
    assert.equal(launches(), 1);
  });
});

// The deadline is absolute: an exit before it does not move it. The stand-in announces DevTools, exits
// 0.5 s before its deadline, and a descendant that left the group (so it survives the exit and keeps the
// pipes open) writes the complete report 0.3 s after it, while the pipes are still being drained.
const AFTER = (seconds) => `perl -MTime::HiRes=time,sleep -e 'my $d = $ARGV[0] + $ARGV[1] - time(); sleep($d) if $d > 0' "$START" ${seconds}`;
const PRINT_REPORT_AT = (seconds) => `QA='${REPORT}' perl -MPOSIX -MTime::HiRes=time,sleep -e 'POSIX::setsid(); open(F, ">", shift); close(F); my $d = $ARGV[0] + $ARGV[1] - time(); sleep($d) if $d > 0; print $ENV{QA}' "$0.ready" "$START" ${seconds} &\nwhile [ ! -e "$0.ready" ]; do sleep 0.05; done`;

test("a report that comes after the deadline is not accepted, even when the Chrome exited before the deadline", () =>
  withBrowser(`START=$(perl -MTime::HiRes=time -e 'printf "%.3f", time()')\n${announce(9)}\n${PRINT_REPORT_AT((4000 + 300) / 1000)}\n${AFTER((4000 - 500) / 1000)}\nexit 3`, 4000, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome (exited without|started \(it announced DevTools\) but did not report) a test report/);
    assert.equal(launches(), 1);
  }));

test("a report that comes after the deadline is not accepted, even when a silent Chrome exited before the deadline", () =>
  withBrowser(`START=$(perl -MTime::HiRes=time -e 'printf "%.3f", time()')\n${PRINT_REPORT_AT((4000 + 300) / 1000)}\n${AFTER((4000 - 500) / 1000)}\nexit 3`, 4000, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report/);
    // A stand-in that a loaded Mac starts more than 0.5 s late exits after the deadline, and a silent
    // Chrome with no renderer is then replaced (by the same stand-in, which exits again): never a success.
    assert.ok(launches() >= 1, "the stand-in ran");
  }));

// An exit after the deadline came too late to matter: the launch already failed, and nothing is waited for.
test("a browser that announced DevTools and exits after its deadline, pipes held open, has already failed, with no new launch", () => {
  const go = join(tmpdir(), `chrome-binary-go-${process.pid}-${Date.now()}`);
  return withDevtools((call) => { if (call >= 2) writeFileSync(go, ""); return null; }, (port) =>
    withBrowser(`${announce(port)}\n${HOLD_PIPES}\nn=0; while [ ! -e "${go}" ] && [ $n -lt 160 ]; do sleep 0.1; n=$((n+1)); done\nsleep 2.2\nexit 3`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult(options), /chrome started \(it announced DevTools\) but did not report a test report within its deadline/);
      assert.equal(launches(), 1);
    })).finally(() => rmSync(go, { force: true }));
});

// The deadline is the deadline: a report that comes after it is not accepted. The stand-in prints its
// report only once the module has asked its DevTools port a second time (the old last look, at the
// deadline), or at 16 s at the latest.
test("a report that arrives after the deadline is not accepted", () => {
  const go = join(tmpdir(), `chrome-binary-go-${process.pid}-${Date.now()}`);
  return withDevtools((call) => { if (call >= 2) writeFileSync(go, ""); return null; }, (port) =>
    withBrowser(`${announce(port)}\nn=0; while [ ! -e "${go}" ] && [ $n -lt 160 ]; do sleep 0.1; n=$((n+1)); done\nsleep 0.2\ncat <<'EOF'\n${REPORT}\nEOF`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult(options), /chrome started \(it announced DevTools\) but did not report a test report within its deadline/);
      assert.equal(launches(), 1);
    })).finally(() => rmSync(go, { force: true }));
});

// Anything on stderr beyond the two start-up lines, or on stdout without a complete report, fails the
// launch at its deadline with no new launch, announced or not.
test("a launch that wrote an error to stderr and then stalls fails after its deadline, with no new launch", () =>
  withBrowser(`echo boom >&2\nexec sleep 60`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome wrote to stderr and did not report a test report within its deadline\nboom/);
    assert.equal(launches(), 1);
  }));

test("a browser that wrote an error next to its start-up lines and then stalls fails after its deadline, with no new launch, even if its DevTools port is stuck", () =>
  withDevtools(() => null, (port) =>
    withBrowser(`${announce(port)}\necho 'ERROR: the page crashed' >&2\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult(options), /chrome wrote to stderr and did not report a test report within its deadline\n.*the page crashed/s);
      assert.equal(launches(), 1);
    })));

test("an unexpected stderr line before the announcement, then a stall, fails after its deadline, with no new launch", () =>
  withBrowser(`echo 'ERROR: boom' >&2\necho 'DevTools listening on ws://127.0.0.1:9/devtools/browser/00000000-0000-0000-0000-000000000001' >&2\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome wrote to stderr and did not report a test report within its deadline/);
    assert.equal(launches(), 1);
  }));

test("a launch that began printing the page and then stalls fails after its deadline, with no new launch", () =>
  withBrowser(`echo '<html><body>' \nexec sleep 60`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome printed a page without a test report within its deadline/);
    assert.equal(launches(), 1);
  }));

test("a browser that announced DevTools, printed part of the page and then stalls fails after its deadline, with no new launch, even if its DevTools port is stuck", () =>
  withDevtools(() => null, (port) =>
    withBrowser(`${announce(port)}\necho '<html><body>'\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult(options), /chrome printed a page without a test report within its deadline/);
      assert.equal(launches(), 1);
    })));
