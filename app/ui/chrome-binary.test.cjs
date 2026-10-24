// Run with: node --test app/ui/chrome-binary.test.cjs
// dumpQaResult() against a stand-in browser script, so no real Chrome and no load dependence.
const assert = require("node:assert/strict");
const { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");
const { dumpQaResult } = require("./chrome-binary.cjs");

const REPORT = '<html><body><pre id="qa-result">{"gap":3,"name":"a &amp; b"}</pre></body></html>';

// A stand-in Chrome: `script` is the body of a /bin/sh program. The counter file gets one line per launch that got as far as running.
// launchMs is how long a launch may stay silent: short when nothing is expected to print, long
// enough for `sh` to start on a loaded Mac when something is.
function withBrowser(script, launchMs, run) {
  const directory = mkdtempSync(join(tmpdir(), "chrome-binary-test-"));
  const counter = join(directory, "launches");
  const browser = join(directory, "chrome");
  writeFileSync(counter, "");
  writeFileSync(browser, `#!/bin/sh\necho x >> "${counter}"\nN=$(wc -l < "${counter}" | tr -d ' ')\n${script}\n`);
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
  withBrowser(`if [ "$N" -lt 3 ]; then exec sleep 60; fi\ncat <<'EOF'\n${REPORT}\nEOF`, 2500, async (options, launches) => {
    const report = await dumpQaResult(options);
    assert.deepEqual(report, { gap: 3, name: "a & b" });
    assert.equal(launches(), 3);
    assert.equal(existsSync(options.directory), false, "the temp directory is removed");
  }));

test("a launch that never prints fails after three launches and says so", () =>
  withBrowser("exec sleep 60", 300, async (options) => {
    await assert.rejects(dumpQaResult(options), /chrome did not report a test report: 3 launches, none printed anything in 300 ms/);
  }));

test("a Chrome that exits without a report fails at once, with no new launch", () =>
  withBrowser("echo boom >&2; exit 3", 2500, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome exited without a test report\nboom/);
    assert.equal(launches(), 1);
  }));

test("a page printed without a report fails at once, with no new launch", () =>
  withBrowser('echo "<html><body><pre id=\\"qa-result\\"></pre></body></html>"; exec sleep 60', 2500, async (options, launches) => {
    await assert.rejects(dumpQaResult(options), /chrome printed a page without a test report/);
    assert.equal(launches(), 1);
  }));
