// Run with: node --test app/ui/chrome-binary.test.cjs
// dumpQaResult() against a stand-in browser script, so no real Chrome and no load dependence.
const assert = require("node:assert/strict");
const { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");
const { dumpQaResult } = require("./chrome-binary.cjs");

const REPORT = '<html><body><pre id="qa-result">{"gap":3,"name":"a &amp; b"}</pre></body></html>';

// A stand-in Chrome: `script` is the body of a /bin/sh program, started with the real Chrome's
// arguments (so `$*` names the profile of the launch). The counter file gets one line per launch
// that got as far as running. launchMs are the deadlines of the three launches: tiny for a launch
// that must stay silent (a slower Mac only makes it more silent), long for one that must print, so no
// test depends on how loaded the Mac is. A launch that must print and then stall is the one place a
// deadline must be long enough for the stand-in to start: STARTED_MS, vs ~1 ms of work it needs.
const STARTED_MS = 2500;
const DEVTOOLS = "DevTools listening on ws://127.0.0.1:9/devtools/browser/test";
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

test("a launch that prints nothing is replaced by a fresh Chrome and the report is returned once", () =>
  withBrowser(`case "$*" in\n*profile-3*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) exec sleep 60;;\nesac`, [300, 300, 30000], async (options) => {
    const report = await dumpQaResult(options);
    assert.deepEqual(report, { gap: 3, name: "a & b" });
    assert.equal(existsSync(options.directory), false, "the temp directory is removed");
  }));

test("a launch that never prints fails after three launches and says so", () =>
  withBrowser("exec sleep 60", 300, async (options) => {
    await assert.rejects(dumpQaResult(options), /chrome did not report a test report: 3 launches, none printed anything within 300 ms/);
  }));

test("a Chrome that exits without a report fails at once, with no new launch", () =>
  withBrowser("echo boom >&2; exit 3", 30000, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report\nboom/);
    assert.equal(launches(), 1);
  }));

test("a launch that wrote to stderr and then stalls fails after its deadline, with no new launch", () =>
  withBrowser(`echo '${DEVTOOLS}' >&2\nexec sleep 60`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome started \(it wrote to stderr\) but did not report a test report within its deadline\nDevTools listening/);
    assert.equal(launches(), 1);
  }));

test("a launch that began printing the page and then stalls fails after its deadline, with no new launch", () =>
  withBrowser(`echo '<html><body>' \nexec sleep 60`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome printed a page without a test report within its deadline/);
    assert.equal(launches(), 1);
  }));

// The descendant leaves the process group (setsid) before the stand-in exits, and keeps the pipes
// open for 6 s: "close" comes long after the 2.5 s deadline, only "exit" tells this Chrome is gone.
test("a Chrome that exited while a descendant keeps its pipes open fails at once, with no new launch", () =>
  withBrowser(`perl -MPOSIX -e 'POSIX::setsid(); open(F, ">", shift); close(F); exec "sleep", "6"' "$0.ready" &\nwhile [ ! -e "$0.ready" ]; do sleep 0.05; done\nexit 3`, STARTED_MS, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report/);
    assert.equal(launches(), 1);
  }));

test("Chrome is started with a DevTools port, so that a browser that never started is told from one that is running", () =>
  withBrowser(`case "$*" in\n*--remote-debugging-port=0*) cat <<'EOF'\n${REPORT}\nEOF\n;;\n*) exec sleep 60;;\nesac`, [300, 300, 30000], async (options, launches) => {
    assert.deepEqual(await dumpQaResult(options), { gap: 3, name: "a & b" });
    assert.equal(launches(), 1);
  }));
