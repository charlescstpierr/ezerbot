// The one place the UI tests find a headless Chrome. CHROME_BIN wins; otherwise the first
// candidate that is installed (on PATH, or the macOS app bundle). Not found is a clear error.
//
// The tests that print the page with `--dump-dom` ask for `chromeBinary({ dumpDom: true })`. On
// macOS that prefers the pinned chrome-headless-shell the repository installs itself
// (`node tools/install-chrome-headless-shell.cjs`, #438): full Chrome sometimes freezes right
// after it starts there, the shell has no NSApplication and never does. The tests that drive
// Chrome over the debugging port keep the full browser.
const { spawnSync } = require("node:child_process");
const { existsSync } = require("node:fs");
const { join, resolve } = require("node:path");

const CHROME_CANDIDATES = [
  "google-chrome",
  "google-chrome-stable",
  "chromium",
  "chromium-browser",
  "/usr/local/bin/google-chrome",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
];

const TOOLS = resolve(__dirname, "..", "..", "tools");
const PIN_FILE = join(TOOLS, "chrome-headless-shell-pin.json");
const SHELL_ROOT = join(TOOLS, "chrome-headless-shell");
const INSTALL_COMMAND = "node tools/install-chrome-headless-shell.cjs";

function chromeOnPath(command) {
  if (command.includes("/")) return existsSync(command);
  return spawnSync("which", [command], { stdio: "ignore" }).status === 0;
}

// "mac-arm64" or "mac-x64" on macOS, null elsewhere (the shell is only pinned for macOS).
function shellPlatform(platform = process.platform, arch = process.arch) {
  if (platform !== "darwin") return null;
  if (arch === "arm64") return "mac-arm64";
  if (arch === "x64") return "mac-x64";
  return null;
}

// Where the pinned shell lives once installed (it may not be there yet), or null.
function pinnedShellPath(options = {}) {
  const platform = shellPlatform(options.platform, options.arch);
  if (!platform) return null;
  const pin = require(options.pinFile || PIN_FILE);
  const root = options.root || SHELL_ROOT;
  return join(root, pin.version, `chrome-headless-shell-${platform}`, "chrome-headless-shell");
}

function chromeBinary(options = {}) {
  if (process.env.CHROME_BIN) return process.env.CHROME_BIN;
  if (options.dumpDom) {
    const shell = pinnedShellPath(options);
    if (shell && existsSync(shell)) return shell;
  }
  const found = CHROME_CANDIDATES.find(chromeOnPath);
  if (!found) throw new Error("chrome introuvable : installez Chrome ou définissez CHROME_BIN");
  return found;
}

// Appended to a "chrome did not report ..." timeout: when the browser that froze is not the
// pinned shell, say how to get it.
function freezeHint(binary, options = {}) {
  const shell = pinnedShellPath(options);
  if (!shell || binary === shell) return "";
  return `\nChrome complet gèle parfois après son démarrage sous macOS (#438) ; installez le shell épinglé : ${INSTALL_COMMAND}`;
}

module.exports = { chromeBinary, chromeOnPath, pinnedShellPath, freezeHint, INSTALL_COMMAND, PIN_FILE, SHELL_ROOT };
