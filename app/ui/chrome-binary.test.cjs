// Which Chrome the UI tests launch (#438): the pinned chrome-headless-shell for the --dump-dom
// tests when it is installed, an explicit CHROME_BIN first of all, the full browser otherwise.
// Uses fake binaries; nothing here starts a browser.
const test = require("node:test");
const assert = require("node:assert/strict");
const { createHash } = require("node:crypto");
const { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { dirname, join } = require("node:path");
const { pinnedFixtureShell, INSTALL_RECEIPT, chromeBinary, freezeHint, pinnedShellPath, INSTALL_COMMAND, PIN_FILE, SHELL_ROOT } = require("./chrome-binary.cjs");

const { installedAndVerified } = require("../../tools/install-chrome-headless-shell.cjs");

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
  return { root, expected, platform };
}

function installFake({ root, expected }) {
  mkdirSync(join(expected, ".."), { recursive: true });
  writeFileSync(expected, "#!/bin/sh\n");
  chmodSync(expected, 0o755);
  return root;
}

test("the pin names one exact version and a SHA-256 for macOS and Linux x86_64", () => {
  assert.match(PIN.version, /^\d+\.\d+\.\d+\.\d+$/);
  for (const platform of ["mac-x64", "mac-arm64", "linux64"]) {
    assert.match(PIN.platforms[platform].sha256, /^[0-9a-f]{64}$/, platform);
  }
});

test("the pinned shell path is per version and supported platform", () => {
  assert.equal(
    pinnedShellPath({ platform: "darwin", arch: "x64", root: "/r" }),
    `/r/${PIN.version}/chrome-headless-shell-mac-x64/chrome-headless-shell`,
  );
  assert.equal(
    pinnedShellPath({ platform: "darwin", arch: "arm64", root: "/r" }),
    `/r/${PIN.version}/chrome-headless-shell-mac-arm64/chrome-headless-shell`,
  );
  assert.equal(
    pinnedShellPath({ platform: "linux", arch: "x64", root: "/r" }),
    `/r/${PIN.version}/chrome-headless-shell-linux64/chrome-headless-shell`,
  );
  assert.equal(pinnedShellPath({ platform: "linux", arch: "arm64", root: "/r" }), null);
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

function installVerifiedFake(fake, version = PIN.version) {
  installFake(fake);
  writeFileSync(fake.expected, `#!/bin/sh\nprintf 'Google Chrome for Testing ${version}\\n'\n`);
  writeReceipt(fake);
}

function writeReceipt(fake, overrides = {}) {
  writeFileSync(join(dirname(fake.expected), INSTALL_RECEIPT), JSON.stringify({
    version: PIN.version,
    platform: fake.platform,
    archiveSha256: PIN.platforms[fake.platform].sha256,
    executableSha256: createHash("sha256").update(readFileSync(fake.expected)).digest("hex"),
    ...overrides,
  }));
}

for (const [platform, arch, archive] of [["darwin", "x64", "mac-x64"], ["darwin", "arm64", "mac-arm64"], ["linux", "x64", "linux64"]]) {
  test(`the fixture selects only its verified ${archive} shell, ignoring CHROME_BIN`, () => {
    const fake = fakeInstall(archive);
    try {
      installVerifiedFake(fake);
      const options = { platform, arch, root: fake.root };
      withEnv("/opt/chrome/custom", () => assert.equal(pinnedFixtureShell(options), fake.expected));
      assert.equal(installedAndVerified(options), true);
    } finally {
      rmSync(fake.root, { recursive: true, force: true });
    }
  });
}

test("Linux --dump-dom retains the full-browser selection even when the fixture shell exists", () => {
  const fake = fakeInstall("linux64");
  try {
    installVerifiedFake(fake);
    withEnv(undefined, () => {
      const select = (options) => { try { return chromeBinary(options); } catch (error) { return error.message; } };
      assert.equal(select({ dumpDom: true, platform: "linux", arch: "x64", root: fake.root }), select({}));
    });
  } finally {
    rmSync(fake.root, { recursive: true, force: true });
  }
});

test("the fixture fails closed without a shell or with only the legacy ZIP stamp", () => {
  const fake = fakeInstall();
  const options = { platform: "darwin", arch: "x64", root: fake.root };
  try {
    withEnv("/opt/chrome/custom", () => assert.throws(() => pinnedFixtureShell(options), /absent ou incompatible.*réinstallez/));
    installFake(fake);
    writeFileSync(join(fake.root, PIN.version, ".sha256"), PIN.platforms[fake.platform].sha256);
    assert.equal(installedAndVerified(options), false);
    assert.throws(() => pinnedFixtureShell(options), /installation.json/);
  } finally {
    rmSync(fake.root, { recursive: true, force: true });
  }
});

test("changing the installed executable invalidates the installer's cache and fixture receipt", () => {
  const fake = fakeInstall();
  const options = { platform: "darwin", arch: "x64", root: fake.root };
  try {
    installVerifiedFake(fake);
    assert.equal(installedAndVerified(options), true);
    writeFileSync(fake.expected, `${readFileSync(fake.expected, "utf8")}# modified executable\n`);
    assert.equal(installedAndVerified(options), false);
    assert.throws(() => pinnedFixtureShell(options), /empreinte du shell incompatible/);
  } finally {
    rmSync(fake.root, { recursive: true, force: true });
  }
});

for (const overrides of [{ version: "1.2.3.4" }, { platform: "linux64" }, { archiveSha256: "0".repeat(64) }]) {
  test(`the fixture rejects a receipt with a mismatching ${Object.keys(overrides)[0]}`, () => {
    const fake = fakeInstall();
    try {
      installVerifiedFake(fake);
      writeReceipt(fake, overrides);
      assert.throws(() => pinnedFixtureShell({ platform: "darwin", arch: "x64", root: fake.root }), /reçu d'installation incompatible/);
    } finally {
      rmSync(fake.root, { recursive: true, force: true });
    }
  });
}

test("a matching executable digest cannot substitute for the pinned runtime version", () => {
  const fake = fakeInstall();
  try {
    installVerifiedFake(fake, "1.2.3.4");
    const options = { platform: "darwin", arch: "x64", root: fake.root };
    assert.equal(installedAndVerified(options), false);
    assert.throws(() => pinnedFixtureShell(options), /version du shell incompatible/);
  } finally {
    rmSync(fake.root, { recursive: true, force: true });
  }
});

test("the fixture rejects unsupported architectures without falling back to Chrome", () => {
  withEnv("/opt/chrome/custom", () => {
    assert.throws(() => pinnedFixtureShell({ platform: "linux", arch: "arm64" }), /plateforme non prise en charge/);
  });
});
