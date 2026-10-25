// Which Chrome the UI tests launch (#438): the pinned chrome-headless-shell for the --dump-dom
// tests when it is installed, an explicit CHROME_BIN first of all, the full browser otherwise.
// Uses fake binaries; nothing here starts a browser.
const test = require("node:test");
const assert = require("node:assert/strict");
const { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { chromeBinary, freezeHint, pinnedShellPath, INSTALL_COMMAND, PIN_FILE, SHELL_ROOT } = require("./chrome-binary.cjs");

const PIN = JSON.parse(readFileSync(PIN_FILE, "utf8"));

function withEnv(chromeBin, run) {
  const previous = process.env.CHROME_BIN;
  if (chromeBin === undefined) delete process.env.CHROME_BIN;
  else process.env.CHROME_BIN = chromeBin;
  try {
    return run();
  } finally {
    if (previous === undefined) delete process.env.CHROME_BIN;
    else process.env.CHROME_BIN = previous;
  }
}

function fakeInstall(platform = "mac-x64") {
  const root = mkdtempSync(join(tmpdir(), "chrome-binary-"));
  const expected = join(root, PIN.version, `chrome-headless-shell-${platform}`, "chrome-headless-shell");
  return { root, expected };
}

function installFake({ root, expected }) {
  mkdirSync(join(expected, ".."), { recursive: true });
  writeFileSync(expected, "#!/bin/sh\n");
  chmodSync(expected, 0o755);
  return root;
}

test("the pin names one exact version and a SHA-256 for each Mac architecture", () => {
  assert.match(PIN.version, /^\d+\.\d+\.\d+\.\d+$/);
  for (const platform of ["mac-x64", "mac-arm64"]) {
    assert.match(PIN.platforms[platform].sha256, /^[0-9a-f]{64}$/, platform);
  }
});

test("the pinned shell path is per version and per Mac architecture, and only exists for macOS", () => {
  assert.equal(
    pinnedShellPath({ platform: "darwin", arch: "x64", root: "/r" }),
    `/r/${PIN.version}/chrome-headless-shell-mac-x64/chrome-headless-shell`,
  );
  assert.equal(
    pinnedShellPath({ platform: "darwin", arch: "arm64", root: "/r" }),
    `/r/${PIN.version}/chrome-headless-shell-mac-arm64/chrome-headless-shell`,
  );
  assert.equal(pinnedShellPath({ platform: "linux", arch: "x64", root: "/r" }), null);
  assert.ok(pinnedShellPath({ platform: "darwin", arch: "x64" }).startsWith(SHELL_ROOT));
});

test("--dump-dom tests get the pinned shell when it is installed", () => {
  const fake = fakeInstall();
  try {
    installFake(fake);
    withEnv(undefined, () => {
      assert.equal(chromeBinary({ dumpDom: true, platform: "darwin", arch: "x64", root: fake.root }), fake.expected);
    });
  } finally {
    rmSync(fake.root, { recursive: true, force: true });
  }
});

test("without the shell installed the --dump-dom tests fall back to the usual Chrome", () => {
  const fake = fakeInstall();
  try {
    withEnv(undefined, () => {
      const plain = (() => { try { return chromeBinary(); } catch (error) { return error.message; } })();
      const fallback = (() => { try { return chromeBinary({ dumpDom: true, platform: "darwin", arch: "x64", root: fake.root }); } catch (error) { return error.message; } })();
      assert.equal(fallback, plain);
      assert.notEqual(fallback, fake.expected);
    });
  } finally {
    rmSync(fake.root, { recursive: true, force: true });
  }
});

test("an explicit CHROME_BIN wins over the pinned shell", () => {
  const fake = fakeInstall();
  try {
    installFake(fake);
    withEnv("/opt/chrome/custom", () => {
      assert.equal(chromeBinary({ dumpDom: true, platform: "darwin", arch: "x64", root: fake.root }), "/opt/chrome/custom");
    });
  } finally {
    rmSync(fake.root, { recursive: true, force: true });
  }
});

test("the tests that drive Chrome over the debugging port never get the shell", () => {
  const fake = fakeInstall();
  try {
    installFake(fake);
    withEnv(undefined, () => {
      const chosen = (() => { try { return chromeBinary({ platform: "darwin", arch: "x64", root: fake.root }); } catch (error) { return error.message; } })();
      assert.notEqual(chosen, fake.expected);
    });
  } finally {
    rmSync(fake.root, { recursive: true, force: true });
  }
});

test("a freeze says how to install the shell unless the shell is what froze", () => {
  const options = { platform: "darwin", arch: "x64", root: "/r" };
  const shell = pinnedShellPath(options);
  assert.equal(freezeHint(shell, options), "");
  const hint = freezeHint("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", options);
  assert.ok(hint.includes(INSTALL_COMMAND), hint);
  assert.equal(freezeHint("google-chrome", { platform: "linux", arch: "x64", root: "/r" }), "");
});
