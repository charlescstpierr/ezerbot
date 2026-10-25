// General UI tests find a headless Chrome here. CHROME_BIN wins; otherwise the first
// candidate that is installed (on PATH, or the macOS app bundle). Not found is a clear error.
//
// The tests that print the page with `--dump-dom` ask for `chromeBinary({ dumpDom: true })`. On
// macOS that prefers the pinned chrome-headless-shell the repository installs itself
// (`node tools/install-chrome-headless-shell.cjs`, #438): full Chrome sometimes freezes right
// after it starts there, the shell has no NSApplication and never does. The tests that drive
// Chrome over the debugging port keep the full browser.
const { spawnSync } = require("node:child_process");
const { createHash } = require("node:crypto");
const { accessSync, constants, existsSync, lstatSync, readFileSync } = require("node:fs");
const { dirname, join, resolve } = require("node:path");

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
const INSTALL_RECEIPT = ".installation.json";

function chromeOnPath(command) {
  if (command.includes("/")) return existsSync(command);
  return spawnSync("which", [command], { stdio: "ignore", timeout: 5_000, killSignal: "SIGKILL" }).status === 0;
}

// Chrome for Testing archive names for the supported fixture platforms.
function shellPlatform(platform = process.platform, arch = process.arch) {
  if (platform === "linux" && arch === "x64") return "linux64";
  if (platform !== "darwin") return null;
  if (arch === "arm64") return "mac-arm64";
  if (arch === "x64") return "mac-x64";
  return null;
}

// Where the pinned shell lives once installed (it may not be there yet), or null.
function pinnedShellPath(options = {}) {
  const platform = shellPlatform(options.platform, options.arch);
  if (!platform) return null;
  const pin = JSON.parse(readFileSync(options.pinFile || PIN_FILE, "utf8"));
  const root = options.root || SHELL_ROOT;
  return join(root, pin.version, `chrome-headless-shell-${platform}`, "chrome-headless-shell");
}

function shellVersion(binary) {
  const result = spawnSync(binary, ["--version"], {
    encoding: "utf8", timeout: 30_000, killSignal: "SIGKILL", maxBuffer: 64 * 1024,
  });
  if (result.error || result.status !== 0) {
    throw new Error(`version du shell indisponible : ${result.error?.code || result.signal || result.status}`);
  }
  const version = result.stdout.trim().match(/(?:^|\s)(\d+\.\d+\.\d+\.\d+)(?:\s|$)/)?.[1];
  if (!version) throw new Error("version du shell illisible");
  return version;
}

// Strict selection never substitutes CHROME_BIN for the verified artifact.
function pinnedFixtureShell(options = {}) {
  try {
    const platform = shellPlatform(options.platform, options.arch);
    if (!platform) throw new Error(`plateforme non prise en charge : ${options.platform || process.platform}/${options.arch || process.arch}`);
    const pin = JSON.parse(readFileSync(options.pinFile || PIN_FILE, "utf8"));
    const archiveSha256 = pin.platforms[platform]?.sha256;
    if (!archiveSha256) throw new Error(`aucune empreinte épinglée pour ${platform}`);
    const binary = pinnedShellPath(options);
    if (!lstatSync(binary).isFile()) throw new Error("le shell n'est pas un fichier exécutable régulier");
    accessSync(binary, constants.X_OK);
    const receipt = JSON.parse(readFileSync(join(dirname(binary), INSTALL_RECEIPT), "utf8"));
    if (receipt.version !== pin.version || receipt.platform !== platform || receipt.archiveSha256 !== archiveSha256) {
      throw new Error("reçu d'installation incompatible avec le pin");
    }
    const actual = createHash("sha256").update(readFileSync(binary)).digest("hex");
    if (receipt.executableSha256 !== actual) throw new Error("empreinte du shell incompatible avec le reçu d'installation");
    if (shellVersion(binary) !== pin.version) throw new Error("version du shell incompatible avec le pin");
    return binary;
  } catch (error) {
    throw new Error(`chrome-headless-shell épinglé absent ou incompatible : ${error.message} ; réinstallez avec ${INSTALL_COMMAND}`, { cause: error });
  }
}

function chromeBinary(options = {}) {
  if (process.env.CHROME_BIN) return process.env.CHROME_BIN;
  if (options.dumpDom && (options.platform || process.platform) === "darwin") {
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
  if ((options.platform || process.platform) !== "darwin") return "";
  const shell = pinnedShellPath(options);
  if (!shell || binary === shell) return "";
  return `\nChrome complet gèle parfois après son démarrage sous macOS (#438) ; installez le shell épinglé : ${INSTALL_COMMAND}`;
}

module.exports = { pinnedFixtureShell, shellPlatform, shellVersion, INSTALL_RECEIPT, chromeBinary, chromeOnPath, pinnedShellPath, freezeHint, INSTALL_COMMAND, PIN_FILE, SHELL_ROOT };
