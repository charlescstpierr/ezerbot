// The folder button must read a granted directory on the server. webkitdirectory
// follows symlinks and never sets file.symlink, so this drives the real control.
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { LaunchIdentity, connectCdp } = require("./local-api-client.cjs");
const { mkdirSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { test } = require("node:test");
const { chromeBinary } = require("./chrome-binary.cjs");

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function debuggingPort(directory, chrome) {
  const portFile = join(directory, "profile", "DevToolsActivePort");
  const deadline = Date.now() + 10000;
  return (async () => {
    while (Date.now() < deadline) {
      if (chrome.exitCode !== null || chrome.signalCode !== null) throw new Error("chrome exited before the debugging port");
      try {
        const port = Number(readFileSync(portFile, "utf8").split(/\s/)[0]);
        if (port) return port;
      } catch { /* port file appears after startup */ }
      await sleep(20);
    }
    throw new Error("chrome did not open a debugging port");
  })();
}

async function pageTarget(port) {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then((response) => response.json());
    const found = targets.find((target) => target.type === "page" && target.webSocketDebuggerUrl);
    if (found) return found;
    await sleep(20);
  }
  throw new Error("chrome did not open a page target");
}

// First line of the child's stdout. Rejects, with the exit status, if the child ends first.
function readLine(child, details = () => "") {
  const stream = child.stdout;
  return new Promise((resolve, reject) => {
    let data = "";
    const onData = (chunk) => {
      data += chunk;
      const end = data.indexOf("\n");
      if (end >= 0) {
        stream.off("data", onData);
        resolve(data.slice(0, end));
      }
    };
    stream.on("data", onData);
    stream.on("error", reject);
    child.once("close", (code, signal) => {
      reject(new Error(`process ended (code ${code}, signal ${signal}) before printing a full line${details()}`));
    });
  });
}

// A promise that rejects as soon as `child` fails to start or ends (stdio closed) while the test still
// needs it. `settle()` marks the end of the test, so stopping the children is not a failure.
function watchChild(name, child, details = () => "") {
  let expected = false;
  const failed = new Promise((_, reject) => {
    child.once("error", (error) => reject(new Error(`${name} failed: ${error.message}`)));
    child.once("close", (code, signal) => {
      if (!expected) reject(new Error(`${name} exited (code ${code}, signal ${signal}) before the test ended${details()}`));
    });
  });
  failed.catch(() => {});
  return { failed, settle() { expected = true; } };
}

test("Importer un dossier skips a real symlink whose target is outside the folder", { timeout: 60000 }, async (t) => {
  const root = mkdtempSync(join(tmpdir(), "grokbot-folder-e2e-"));
  const dbDir = join(root, "db");
  const chosen = join(root, "choisi");
  const folder = join(chosen, "dossier");
  const outside = join(root, "dehors");
  mkdirSync(dbDir);
  mkdirSync(folder, { recursive: true });
  mkdirSync(outside, { recursive: true });
  const secret = "secret hors dossier";
  writeFileSync(join(outside, "secret.md"), secret);
  writeFileSync(join(folder, "SKILL.md"), "---\nname: Modèle\n---\nCorps\n");
  writeFileSync(join(folder, "modele.md"), "annexe interne\n");
  symlinkSync(join(outside, "secret.md"), join(folder, "lien.md"));
  symlinkSync(outside, join(chosen, "echappe"));

  const binary = join(__dirname, "../../target/debug/grokbot-wrapper");
  const identity = new LaunchIdentity();
  const child = identity.spawn(binary, ["--port", "0", "--db", join(dbDir, "app.sqlite")], {
    env: { ...process.env, HOME: dbDir, CFFIXED_USER_HOME: dbDir },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => { if (stderr.length < 4000) stderr += chunk; });
  const wrapperWatch = watchChild("grokbot-wrapper", child, () => (stderr ? `: ${stderr}` : ""));
  let chrome;
  let chromeWatch;
  let authentication;
  // Registered before anything can fail, and run by node:test when the test ends for any
  // reason, including the test timeout, where the awaited body never reaches a `finally`.
  t.after(() => {
    wrapperWatch.settle();
    chromeWatch?.settle();
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    if (chrome && chrome.exitCode === null && chrome.signalCode === null) chrome.kill("SIGKILL");
  });
  // Every wait below ends early, with a clear message, if either process fails or exits.
  const whileRunning = (work) => Promise.race([work, wrapperWatch.failed, ...(chromeWatch ? [chromeWatch.failed] : []), ...(authentication ? [authentication.failed] : [])]);
  const chromeDir = mkdtempSync(join(tmpdir(), "grokbot-folder-chrome-"));
  chrome = spawn(chromeBinary(), [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--incognito",
    "--use-mock-keychain",
    "--password-store=basic",
    "--disable-breakpad",
    "--disable-crash-reporter",
    "--remote-allow-origins=*",
    "--remote-debugging-port=0",
    `--user-data-dir=${join(chromeDir, "profile")}`,
    "--window-size=1280,900",
    "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });
  chromeWatch = watchChild("chrome", chrome);
  await whileRunning((async () => {
    const announced = JSON.parse(await readLine(child, () => (stderr ? `: ${stderr}` : "")));
    const url = announced.url;
    const headers = {
      Origin: url,
      "Content-Type": "application/json",
      "X-Grokbot-Request": "local",
    };
    const granted = await identity.fetch(`${url}/api/local/folders/add`, {
      method: "POST",
      headers,
      body: JSON.stringify({ path: chosen }),
    });
    const grantBody = await granted.json();
    assert.equal(granted.status, 200, JSON.stringify(grantBody));
    assert.equal(grantBody.folders.length, 1);

    const port = await debuggingPort(chromeDir, chrome);
    const target = await pageTarget(port);
    const cdp = await connectCdp(target.webSocketDebuggerUrl);
    const evaluate = async (expression) => {
      const result = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
      if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
      return result.result.value;
    };
    authentication = await identity.authenticateBrowser(cdp, url);
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Page.navigate", { url });
    const opened = await (async () => {
      const deadline = Date.now() + 15000;
      let last;
      while (Date.now() < deadline) {
        last = await evaluate(`(() => {
          document.getElementById("edit-bot-button")?.click();
          const button = [...document.querySelectorAll("button")].find((item) => item.textContent.trim() === "Importer un dossier");
          return { ok: Boolean(button), dialog: Boolean(document.getElementById("bot-dialog")?.open) };
        })()`);
        if (last?.ok) return last;
        await sleep(100);
      }
      throw new Error(`bouton introuvable ${JSON.stringify(last)} ${stderr}`);
    })();
    assert.equal(opened.ok, true);
    const clicked = await evaluate(`(() => {
      const button = [...document.querySelectorAll("button")].find((item) => item.textContent.trim() === "Importer un dossier");
      button.click();
      return { clicked: true, inputs: [...document.querySelectorAll("input")].map((input) => input.name).filter(Boolean) };
    })()`);
    assert.equal(clicked.inputs.includes("skills-directory"), false);
    const report = await (async () => {
      const deadline = Date.now() + 15000;
      let last;
      while (Date.now() < deadline) {
        last = await evaluate(`(() => {
          const text = document.body.innerText;
          const point = document.querySelector('[data-point="import"]');
          return {
            ok: text.includes("lien symbolique") && point?.dataset.done === "true",
            done: point?.dataset.done || null,
            lines: text.split("\\n").filter((line) => /lien|annexe|Accordez|Modèle|incert|refus/i.test(line)).slice(0, 20),
          };
        })()`);
        if (last?.ok) return last;
        await sleep(100);
      }
      throw new Error(`import non confirmé ${JSON.stringify(last)}`);
    })();
    assert.equal(report.ok, true);
    const shown = await evaluate("document.body.textContent");
    assert.equal(shown.includes(secret), false);
    await evaluate(`(() => {
      const edit = [...document.querySelectorAll("button")].find((item) => item.textContent.trim() === "Modifier / historique");
      edit.click();
      return true;
    })()`);
    const editor = await (async () => {
      const deadline = Date.now() + 10000;
      let last = "";
      while (Date.now() < deadline) {
        last = await evaluate("document.body.textContent");
        if (last.includes("annexe interne")) return last;
        await sleep(100);
      }
      throw new Error("l’annexe interne n’apparaît pas dans l’éditeur");
    })();
    assert.equal(editor.includes(secret), false);
    assert.equal(editor.includes("annexe interne"), true);

    const skills = await identity.fetch(`${url}/api/skills?bot_id=grokbot`).then((response) => response.json());
    const skillId = skills.skills[0].skill_id;
    const versions = await identity.fetch(`${url}/api/skills/versions`, {
      method: "POST",
      headers,
      body: JSON.stringify({ bot_id: "grokbot", skill_id: skillId }),
    }).then((response) => response.json());
    const stored = JSON.stringify(versions);
    assert.equal(stored.includes("annexe interne"), true);
    assert.equal(stored.includes(secret), false);
    assert.equal(stored.includes("echappe/"), false);

    const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
    writeFileSync(join(root, "import-dossier-symlink.png"), Buffer.from(shot.data, "base64"));
    cdp.close();
  })());
});
