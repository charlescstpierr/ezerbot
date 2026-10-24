// Run with: node --test app/ui/chrome-binary.test.cjs
// dumpQaResult() against a stand-in browser script and a stand-in DevTools port, so no real Chrome
// and no load dependence.
const assert = require("node:assert/strict");
const { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const http = require("node:http");
const net = require("node:net");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");
const { dumpQaResult } = require("./chrome-binary.cjs");

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
  const options = { file: join(work, "page.html"), directory: work, width: 500, height: 780, label: "a test report", launchMs, probeMs: 300 };
  const launches = () => readFileSync(counter, "utf8").split("\n").filter(Boolean).length;
  return run(options, launches).finally(() => {
    if (previous === undefined) delete process.env.CHROME_BIN;
    else process.env.CHROME_BIN = previous;
    rmSync(directory, { recursive: true, force: true });
    rmSync(work, { recursive: true, force: true });
  });
}

// A stand-in DevTools port. `targets` null = the browser's main thread is stuck: it accepts the
// connection and never answers; otherwise it answers /json/list with those targets.
function withDevtools(targets, run) {
  const sockets = new Set();
  const server = targets === null ? net.createServer((socket) => { sockets.add(socket); socket.on("error", () => {}); }) : http.createServer((request, response) => {
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify(targets));
  });
  server.on("connection", (socket) => { sockets.add(socket); socket.on("error", () => {}); });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", resolve)).then(() => run(server.address().port)).finally(() => {
    sockets.forEach((socket) => socket.destroy());
    server.close();
  });
}

const announce = (port) => `echo 'DevTools listening on ws://127.0.0.1:${port}/devtools/browser/00000000-0000-0000-0000-000000000001' >&2`;
const secondLaunchReports = (stall) => `case "$*" in\n*profile-2*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) ${stall};;\nesac`;

test("a launch that prints nothing is replaced by a fresh Chrome and the report is returned once", () =>
  withBrowser(`case "$*" in\n*profile-3*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) exec sleep 60;;\nesac`, [300, 300, 30000], async (options) => {
    const report = await dumpQaResult(options);
    assert.deepEqual(report, { gap: 3, name: "a & b" });
    assert.equal(existsSync(options.directory), false, "the temp directory is removed");
  }));

test("a launch that never prints fails after three launches and says so", () =>
  withBrowser("exec sleep 60", 300, async (options) => {
    await assert.rejects(dumpQaResult(options), /chrome did not report a test report: 3 launches, none began loading the page within 300 ms/);
  }));

test("a Chrome that exits without a report fails at once, with no new launch", () =>
  withBrowser("echo boom >&2; exit 3", 30000, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report\nboom/);
    assert.equal(launches(), 1);
  }));

test("a launch that wrote an error to stderr and then stalls fails after its deadline, with no new launch", () =>
  withBrowser(`echo boom >&2\nexec sleep 60`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome wrote to stderr and did not report a test report within its deadline\nboom/);
    assert.equal(launches(), 1);
  }));

test("a launch that began printing the page and then stalls fails after its deadline, with no new launch", () =>
  withBrowser(`echo '<html><body>' \nexec sleep 60`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome printed a page without a test report within its deadline/);
    assert.equal(launches(), 1);
  }));

// The descendant leaves the process group (setsid) before the stand-in exits, and keeps the pipes
// open for 20 s: "close" comes long after the deadline, only "exit" tells this Chrome is gone.
test("a Chrome that exited while a descendant keeps its pipes open fails at once, with no new launch", () =>
  withBrowser(`perl -MPOSIX -e 'POSIX::setsid(); open(F, ">", shift); close(F); exec "sleep", "20"' "$0.ready" &\nwhile [ ! -e "$0.ready" ]; do sleep 0.05; done\nexit 3`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report/);
    assert.equal(launches(), 1);
  }));

test("Chrome is started with a DevTools port, so that a browser that never started is told from one that is running", () =>
  withBrowser(`case "$*" in\n*--remote-debugging-port=0*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) exec sleep 60;;\nesac`, [300, 300, 30000], async (options, launches) => {
    assert.deepEqual(await dumpQaResult(options), { gap: 3, name: "a & b" });
    assert.equal(launches(), 1);
  }));

test("a browser whose DevTools port does not answer (stuck main thread) is replaced; its start-up lines on stderr are not errors", () =>
  withDevtools(null, (port) =>
    withBrowser(`${announce(port)}\necho '${DISPLAY_LINK}' >&2\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      assert.deepEqual(await dumpQaResult(options), { gap: 3, name: "a & b" });
      assert.equal(launches(), 2);
    })));

test("a browser that answers but never left about:blank is replaced", () =>
  withDevtools([{ type: "page", url: "about:blank" }, { type: "iframe", url: "about:blank" }], (port) =>
    withBrowser(`${announce(port)}\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      assert.deepEqual(await dumpQaResult(options), { gap: 3, name: "a & b" });
      assert.equal(launches(), 2);
    })));

test("a browser that began loading the page and then stalls fails after its deadline, with no new launch", () =>
  withDevtools([{ type: "page", url: "file:///work/page.html" }], (port) =>
    withBrowser(`${announce(port)}\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult(options), /chrome began loading the page but did not report a test report within its deadline/);
      assert.equal(launches(), 1);
    })));

test("a browser that wrote an error next to its start-up lines and then stalls is not replaced even if its DevTools port is stuck", () =>
  withDevtools(null, (port) =>
    withBrowser(`${announce(port)}\necho 'ERROR: the page crashed' >&2\n${secondLaunchReports("exec sleep 60")}`, [STARTED_MS, 30000, 30000], async (options, launches) => {
      await assert.rejects(dumpQaResult(options), /chrome wrote to stderr and did not report a test report within its deadline\n.*the page crashed/s);
      assert.equal(launches(), 1);
    })));
