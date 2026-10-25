#!/usr/bin/env node
// Installe la version ÉPINGLÉE de chrome-headless-shell (Chrome for Testing) que les tests UI
// de faisabilité utilisent sur macOS et Linux x86_64, et --dump-dom sur macOS (#438). Si le binaire est
// déjà là et que son empreinte correspond, rien n'est téléchargé. Le zip est vérifié par son
// SHA-256 (tools/chrome-headless-shell-pin.json) avant d'être extrait ; un zip qui ne
// correspond pas n'est jamais installé.
//
//   node tools/install-chrome-headless-shell.cjs
//
// Dossier d'installation : tools/chrome-headless-shell/<version>/ (ignoré par git).
// Dernière ligne de la sortie : le chemin absolu du binaire.
const { createHash } = require("node:crypto");
const { spawnSync } = require("node:child_process");
const { createWriteStream, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync, accessSync, constants } = require("node:fs");
const { join } = require("node:path");
const { Readable } = require("node:stream");
const { pipeline } = require("node:stream/promises");
const { PIN_FILE, SHELL_ROOT, pinnedShellPath, pinnedFixtureShell, shellPlatform, shellVersion, INSTALL_RECEIPT } = require("../app/ui/chrome-binary.cjs");

function isExecutable(file) {
  try { accessSync(file, constants.X_OK); return statSync(file).isFile(); } catch { return false; }
}

function installedAndVerified(options = {}) {
  try {
    pinnedFixtureShell(options);
    return true;
  } catch {
    return false;
  }
}

// A run killed half-way leaves its .install-* work folder behind: the next run clears the old ones.
function clearStaleWork(versionDir) {
  for (const name of readdirSync(versionDir)) {
    if (!name.startsWith(".install-")) continue;
    const folder = join(versionDir, name);
    if (Date.now() - statSync(folder).mtimeMs > 30 * 60 * 1000) rmSync(folder, { recursive: true, force: true });
  }
}

async function download(url, destination) {
  const response = await fetch(url, { signal: AbortSignal.timeout(10 * 60 * 1000) });
  if (!response.ok || !response.body) throw new Error(`téléchargement refusé (${response.status}) : ${url}`);
  const hash = createHash("sha256");
  const out = createWriteStream(destination);
  const source = Readable.fromWeb(response.body);
  source.on("data", (chunk) => hash.update(chunk));
  await pipeline(source, out);
  return hash.digest("hex");
}

async function main() {
  const pin = JSON.parse(readFileSync(PIN_FILE, "utf8"));
  const platform = shellPlatform();
  if (!platform) throw new Error(`plateforme non prise en charge : ${process.platform}/${process.arch}`);
  const sha256 = pin.platforms[platform]?.sha256;
  if (!sha256) throw new Error(`aucune empreinte épinglée pour ${platform}`);
  const binary = pinnedShellPath();
  const versionDir = join(SHELL_ROOT, pin.version);

  if (installedAndVerified()) {
    console.log(`chrome-headless-shell ${pin.version} (${platform}) déjà installé, rien à télécharger`);
    console.log(binary);
    return;
  }

  mkdirSync(versionDir, { recursive: true });
  clearStaleWork(versionDir);
  const work = mkdtempSync(join(versionDir, ".install-"));
  try {
    const zip = join(work, "shell.zip");
    const url = `${pin.urlBase}/${pin.version}/${platform}/chrome-headless-shell-${platform}.zip`;
    console.log(`téléchargement de ${url}`);
    const actual = await download(url, zip);
    if (actual !== sha256) throw new Error(`empreinte SHA-256 inattendue : ${actual} (attendue ${sha256}) ; rien n'est installé`);

    const unpacked = join(work, "unpacked");
    mkdirSync(unpacked);
    const unzip = spawnSync("unzip", ["-q", zip, "-d", unpacked], { stdio: "inherit", timeout: 120_000, killSignal: "SIGKILL" });
    if (unzip.error || unzip.status !== 0) throw new Error(`unzip a échoué : ${unzip.error?.code || unzip.signal || unzip.status}`);
    const extracted = join(unpacked, `chrome-headless-shell-${platform}`);
    const executable = join(extracted, "chrome-headless-shell");
    if (!isExecutable(executable)) throw new Error("le zip ne contient pas chrome-headless-shell exécutable");
    if (shellVersion(executable) !== pin.version) throw new Error("la version extraite ne correspond pas au pin");
    const executableSha256 = createHash("sha256").update(readFileSync(executable)).digest("hex");
    writeFileSync(join(extracted, INSTALL_RECEIPT), `${JSON.stringify({
      version: pin.version, platform, url, archiveSha256: actual, executableSha256,
    }, null, 2)}\n`);
    const destination = join(versionDir, `chrome-headless-shell-${platform}`);
    rmSync(destination, { recursive: true, force: true });
    renameSync(extracted, destination);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }

  if (!installedAndVerified()) throw new Error(`installation incomplète : ${binary}`);
  console.log(`installé : chrome-headless-shell ${pin.version}`);
  console.log(binary);
}

if (require.main === module) main().catch((error) => {
  console.error(`install-chrome-headless-shell : ${error.message}`);
  process.exit(1);
});

module.exports = { installedAndVerified };
