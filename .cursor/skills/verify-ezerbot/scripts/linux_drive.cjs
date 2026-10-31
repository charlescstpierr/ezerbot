// Linux driver for verify-ezerbot: reuses tools/linux-captures.cjs (isolated wrapper, own Chrome
// profile, local fake provider) and drives three entries of the feature map under Xvfb.
// Usage: node --experimental-websocket linux_drive.cjs <sha40> <binary> <captures-directory>
const { readdirSync, writeFileSync } = require("node:fs");
const { join, resolve } = require("node:path");
const { VIEWS, capture } = require(resolve(__dirname, "../../../../tools/linux-captures.cjs"));

// View id from the capture registry -> feature file of features/.
const FEATURES = Object.freeze([
  { view: "home-dark", feature: "accueil" },
  { view: "conversation-dark", feature: "reponse-fil" },
  { view: "settings-dark", feature: "parametres-connecteurs" },
]);

function selectedViews() {
  return FEATURES.map(({ view, feature }) => {
    const found = VIEWS.find(item => item.id === view);
    if (!found) throw new Error(`View ${view} is missing from tools/linux-captures.cjs.`);
    return { ...found, feature };
  });
}

async function main([sha, binary, output]) {
  const views = selectedViews();
  const controller = new AbortController();
  for (const name of ["SIGINT", "SIGTERM"]) process.once(name, () => controller.abort());
  let error = null;
  try {
    await capture(sha, binary, output, controller.signal, views);
  } catch (failure) {
    error = failure.message;
  }
  const files = new Set(readdirSync(output));
  const features = views.map(view => ({
    feature: view.feature,
    view: view.id,
    file: `${view.id}.png`,
    status: files.has(`${view.id}.png`) && (error === null || !error.startsWith(`${view.id} `)) ? "pass" : "fail",
  }));
  // A failure before any view (launch, readiness) fails every feature that has no capture.
  const result = { sha, error, features, status: error === null && features.every(item => item.status === "pass") ? "pass" : "fix" };
  writeFileSync(join(output, "result.json"), `${JSON.stringify(result, null, 2)}\n`);
  console.log(`linux-drive: ${result.status}${error ? ` (${error})` : ""}`);
  return result.status === "pass" ? 0 : 1;
}

if (require.main === module) {
  if (process.argv.length !== 5) {
    console.error("Usage: node --experimental-websocket linux_drive.cjs <sha40> <binary> <captures-directory>");
    process.exitCode = 2;
  } else main(process.argv.slice(2)).then(code => { process.exitCode = code; }, failure => { console.error(`linux-drive: ${failure.message}`); process.exitCode = 2; });
}

module.exports = { FEATURES, selectedViews };
