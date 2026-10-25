#!/usr/bin/env node
// Installe la version ÉPINGLÉE de chrome-headless-shell (Chrome for Testing) que les tests UI
// --dump-dom utilisent sur macOS (#438). Idempotent : si le binaire de la version épinglée est
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
const { PIN_FILE, SHELL_ROOT, pinnedShellPath } = require("../app/ui/chrome-binary.cjs");

const STAMP = ".sha256";

function isExecutable(file) {
  try { accessSync(file, constants.X_OK); return statSync(file).isFile(); } catch { return false; }
}

function platformName() {
  if (process.platform !== "darwin") throw new Error("chrome-headless-shell n'est épinglé que pour macOS ; ailleurs les tests gardent Chrome");
  if (process.arch === "arm64") return "mac-arm64";
  if (process.arch === "x64") return "mac-x64";
  throw new Error(`architecture non prise en charge : ${process.arch}`);
}

function installedAndVerified(binary, versionDir, sha256) {
  try {
    return isExecutable(binary) && readFileSync(join(versionDir, STAMP), "utf8").trim() === sha256;
  } catch {
    return false;
  }
}

// A run killed half-way leaves its .install-* work folder behind: the next run clears the old ones.
function clearStaleWork() {
  for (const name of readdirSync(SHELL_ROOT)) {
    if (!name.startsWith(".install-")) continue;
    const folder = join(SHELL_ROOT, name);
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
  const platform = platformName();
  const sha256 = pin.platforms[platform]?.sha256;
  if (!sha256) throw new Error(`aucune empreinte épinglée pour ${platform}`);
  const binary = pinnedShellPath();
  const versionDir = join(SHELL_ROOT, pin.version);

  if (installedAndVerified(binary, versionDir, sha256)) {
    console.log(`chrome-headless-shell ${pin.version} (${platform}) déjà installé, rien à télécharger`);
    console.log(binary);
    return;
  }

  mkdirSync(SHELL_ROOT, { recursive: true });
  clearStaleWork();
  const work = mkdtempSync(join(SHELL_ROOT, ".install-"));
  try {
    const zip = join(work, "shell.zip");
    const url = `${pin.urlBase}/${pin.version}/${platform}/chrome-headless-shell-${platform}.zip`;
    console.log(`téléchargement de ${url}`);
    const actual = await download(url, zip);
    if (actual !== sha256) throw new Error(`empreinte SHA-256 inattendue : ${actual} (attendue ${sha256}) ; rien n'est installé`);

    const unpacked = join(work, "unpacked");
    mkdirSync(unpacked);
    const unzip = spawnSync("unzip", ["-q", zip, "-d", unpacked], { stdio: "inherit" });
    if (unzip.status !== 0) throw new Error("unzip a échoué");
    const extracted = join(unpacked, `chrome-headless-shell-${platform}`);
    if (!isExecutable(join(extracted, "chrome-headless-shell"))) throw new Error("le zip ne contient pas chrome-headless-shell exécutable");

    writeFileSync(join(unpacked, STAMP), `${sha256}\n`);
    rmSync(versionDir, { recursive: true, force: true });
    renameSync(unpacked, versionDir);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }

  if (!installedAndVerified(binary, versionDir, sha256)) throw new Error(`installation incomplète : ${binary}`);
  const version = spawnSync(binary, ["--version"], { encoding: "utf8" });
  console.log(`installé : ${(version.stdout || "").trim()}`);
  console.log(binary);
}

main().catch((error) => {
  console.error(`install-chrome-headless-shell : ${error.message}`);
  process.exit(1);
});
