// Fixture for the tests that need a real page: a debug grokbot-wrapper on a throwaway database,
// answered by a scripted OpenAI on 127.0.0.1, seeded through the public HTTP API, and an isolated
// headless Chrome (own profile, own debugging port) driven over CDP. Nothing is shared with the
// person's apps: everything lives under one temporary directory and `launchFixture` registers its
// own cleanup on the test context, so the processes stop when the test ends for any reason,
// including a failed assertion or the test timeout.
const { spawn } = require("node:child_process");
const { LaunchIdentity, connectCdp } = require("./local-api-client.cjs");
const { createServer } = require("node:http");
const { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { chromeBinary } = require("./chrome-binary.cjs");

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// The scripted model. A user message "create_document:{json}" or "write_page:{json}" calls that tool with the
// JSON as arguments; once the tool output is back (or for anything else: titles, memory) it
// answers with one short text. The tool call and the catalogue follow the shapes the real
// provider parses, as in crates/wrapper/tests/support and .cursor/skills/verify-ezerbot.
const MODELS = {
  models: [{
    slug: "gpt-test",
    display_name: "GPT Test",
    description: "Modèle factice",
    visibility: "list",
    default_reasoning_level: "medium",
    supported_reasoning_levels: [{ effort: "low", description: "Léger" }, { effort: "medium", description: "Équilibré" }],
    service_tiers: [],
  }],
};

function sse(events) {
  return events.map((event) => `event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`).join("");
}

function strings(value, found = []) {
  if (typeof value === "string") found.push(value);
  else if (Array.isArray(value)) value.forEach((item) => strings(item, found));
  else if (value && typeof value === "object") Object.values(value).forEach((item) => strings(item, found));
  return found;
}

function scriptedReply(raw) {
  let payload = {};
  try { payload = JSON.parse(raw); } catch { /* an unreadable body gets the plain answer */ }
  const input = Array.isArray(payload.input) ? payload.input : [];
  const answered = input.some((item) => item?.type === "function_call_output");
  const lastUser = [...input].reverse().find((item) => item?.role === "user");
  const toolName = ["create_document", "write_page"].find((name) => strings(lastUser).some((text) => text.startsWith(`${name}:`)));
  const request = strings(lastUser).find((text) => toolName && text.startsWith(`${toolName}:`));
  const offered = (payload.tools || []).some((tool) => tool?.name === toolName);
  if (request && offered && !answered) {
    const call = {
      id: "fc-fixture",
      type: "function_call",
      status: "completed",
      call_id: `fixture-${Buffer.from(request).toString("base64url").slice(-24)}`,
      name: toolName,
      arguments: request.slice(toolName.length + 1),
    };
    return sse([{ type: "response.output_item.done", item: call }, { type: "response.completed" }]);
  }
  return sse([
    { type: "response.created" },
    { type: "response.output_text.delta", delta: "Voici votre document." },
    { type: "response.completed", response: { usage: { input_tokens: 1, output_tokens: 2 } } },
  ]);
}

function startFakeOpenAi() {
  const server = createServer((request, response) => {
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => {
      const path = request.url.split("?")[0];
      const send = (status, type, body) => {
        response.writeHead(status, { "Content-Type": type, "Cache-Control": "no-store", Connection: "close" });
        response.end(body);
      };
      if (request.method === "GET" && (path === "/v1/models" || path === "/backend-api/codex/models")) send(200, "application/json", JSON.stringify(MODELS));
      else if (request.method === "GET" && path === "/jwks") send(200, "application/json", '{"keys":[]}');
      else if (request.method === "GET") send(200, "application/json", "{}");
      else if (path === "/v1/responses" || path === "/backend-api/codex/responses") send(200, "text/event-stream", scriptedReply(Buffer.concat(chunks).toString("utf8")));
      else if (path === "/token") send(200, "application/json", JSON.stringify({ access_token: "fake-access", refresh_token: "fake-refresh", token_type: "Bearer", expires_in: 3600 }));
      else send(200, "application/json", "{}");
    });
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve({ server, base: `http://127.0.0.1:${server.address().port}` }));
  });
}

function fakeJwt() {
  const part = (data) => Buffer.from(JSON.stringify(data)).toString("base64url");
  const claims = {
    email: "vous@exemple.com",
    "https://api.openai.com/auth": { chatgpt_account_id: "account-fake", chatgpt_plan_type: "plus", chatgpt_data_residency: "eu" },
  };
  return `${part({ alg: "none", typ: "JWT" })}.${part(claims)}.fake`;
}

// The same fake session the verification skill writes next to the database.
function writeFakeSession(dbDir) {
  const session = {
    host_id: "urn:uuid:00000000-0000-4000-8000-0000000000e2",
    account_id: "account-fake",
    client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
    access_token: fakeJwt(),
    refresh_token: "fake-refresh",
    expires_at: Math.floor(Date.now() / 1000) + 86400,
    scopes: "openid profile email offline_access",
    email: "vous@exemple.com",
    plan: "plus",
    welcomed: true,
  };
  writeFileSync(join(dbDir, "app.codex-auth.json"), JSON.stringify(session), { mode: 0o600 });
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

// A promise that rejects as soon as `child` fails to start or ends while the test still needs it.
// `settle()` marks the end of the test, so stopping the children is not a failure.
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

function alive(child) {
  return child && child.exitCode === null && child.signalCode === null;
}

async function untilTrue(read, message, ms = 20000) {
  const deadline = Date.now() + ms;
  let last;
  while (Date.now() < deadline) {
    last = await read();
    if (last) return last;
    await sleep(50);
  }
  throw new Error(message);
}

// Starts the scripted model, the wrapper and Chrome; creates one conversation per entry of
// `documents` ({ title, name, format, content }) through /api/conversations and /api/send and
// waits for each create_document result; opens the app in Chrome. Resolves with the app's URL
// and `evaluate(source)`, which runs `source` (an expression, usually a function) in the page.
async function launchFixture(t, { documents = [], viewport, headless = true, navigate = true, binary = join(__dirname, "../../target/debug/grokbot-wrapper") }) {
  const root = mkdtempSync(join(tmpdir(), "grokbot-fixture-"));
  const dbDir = join(root, "db");
  mkdirSync(dbDir);
  const privateEnv = { PATH: process.env.PATH, LANG: "C.UTF-8", TZ: "UTC" };
  for (const [key, directory] of Object.entries({ HOME: "home", XDG_CONFIG_HOME: "config", XDG_CACHE_HOME: "cache", XDG_DATA_HOME: "data", XDG_RUNTIME_DIR: "runtime", TMPDIR: "tmp" })) {
    privateEnv[key] = join(root, directory);
    mkdirSync(privateEnv[key], { mode: 0o700 });
  }
  const fake = { server: null };
  const processes = { wrapper: null, chrome: null };
  const watches = [];
  let cdp;
  let disposed = false;
  // Registered before anything can fail, and run by node:test when the test ends for any reason,
  // including the test timeout, where the awaited body never reaches a `finally`.
  t.after(async () => {
    disposed = true;
    watches.forEach((watch) => watch.settle());
    cdp?.close();
    await Promise.all(Object.values(processes).map(child => {
      if (!alive(child)) return;
      return new Promise(resolve => {
        child.once("close", resolve);
        if (process.platform === "linux") {
          try { process.kill(-child.pid, "SIGKILL"); } catch (error) { if (error.code !== "ESRCH") throw error; }
        } else child.kill("SIGKILL");
      });
    }));
    fake.server?.closeAllConnections();
    if (fake.server) await new Promise(resolve => fake.server.close(resolve));
    rmSync(root, { recursive: true, force: true });
  });
  const whileRunning = (work) => Promise.race([work, ...watches.map((watch) => watch.failed)]);

  const openai = await startFakeOpenAi();
  if (disposed) {
    openai.server.closeAllConnections();
    await new Promise(resolve => openai.server.close(resolve));
    throw new Error("fixture closed during startup");
  }
  fake.server = openai.server;
  writeFakeSession(dbDir);

  let stderr = "";
  const identity = new LaunchIdentity();
  const wrapper = identity.spawn(binary, ["--port", "0", "--db", join(dbDir, "app.sqlite")], {
    env: {
      ...privateEnv,
      CFFIXED_USER_HOME: dbDir,
      GROKBOT_FAKE_OPENAI: openai.base,
    },
    detached: process.platform === "linux",
    stdio: ["ignore", "pipe", "pipe"],
  });
  processes.wrapper = wrapper;
  wrapper.stderr.on("data", (chunk) => { if (stderr.length < 4000) stderr += chunk; });
  const details = () => (stderr ? `: ${stderr}` : "");
  watches.push(watchChild("grokbot-wrapper", wrapper, details));

  const chromeDir = join(root, "chrome");
  mkdirSync(chromeDir);
  const chrome = spawn(chromeBinary(), [
    ...(headless ? ["--headless=new"] : []),
    "--no-sandbox",
    "--no-first-run",
    "--disable-background-networking",
    "--disable-sync",
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
  ], {
    env: { ...privateEnv, ...(process.env.DISPLAY ? { DISPLAY: process.env.DISPLAY } : {}), ...(process.env.XAUTHORITY ? { XAUTHORITY: process.env.XAUTHORITY } : {}) },
    detached: process.platform === "linux",
    stdio: ["ignore", "ignore", "pipe"],
  });
  processes.chrome = chrome;
  watches.push(watchChild("chrome", chrome));

  return whileRunning((async () => {
    const url = JSON.parse(await readLine(wrapper, details)).url;
    const headers = { Origin: url, "Content-Type": "application/json", "X-Grokbot-Request": "local" };
    const api = async (path, body) => {
      const response = await identity.fetch(`${url}${path}`, body === undefined ? { headers } : { method: "POST", headers, body: JSON.stringify(body) });
      const text = await response.text();
      if (!response.ok) throw new Error(`${path} ${response.status}: ${text}${details()}`);
      return JSON.parse(text);
    };
    await untilTrue(async () => (await api("/api/state")).provider?.models_ready, `le catalogue du faux modèle n’est pas chargé${details()}`);
    for (const [index, document] of documents.entries()) {
      const { conversation } = await api("/api/conversations", { title: document.title });
      await api("/api/send", {
        conversation_id: conversation.id,
        message_id: `fixture-${index}`,
        body: `create_document:${JSON.stringify({ name: document.name, format: document.format, content: document.content })}`,
      });
      await untilTrue(async () => {
        const found = (await api("/api/state")).conversations.find((item) => item.id === conversation.id);
        return found?.messages?.some((message) => message.payload?.outcome?.output?.id?.startsWith("generated_"))
          && found.messages.at(-1).role === "assistant";
      }, `le document ${document.format} n’a pas été créé${details()}`);
    }

    const port = await debuggingPort(chromeDir, chrome);
    const target = await pageTarget(port);
    cdp = await connectCdp(target.webSocketDebuggerUrl);
    if (disposed) { cdp.close(); throw new Error("fixture closed during startup"); }
    const evaluate = async (expression) => {
      const result = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
      if (result.exceptionDetails) {
        const exception = result.exceptionDetails.exception;
        throw new Error(exception?.description || exception?.value || result.exceptionDetails.text);
      }
      return result.result.value;
    };
    watches.push(await identity.authenticateBrowser(cdp, url));
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    if (viewport) await cdp.send("Emulation.setDeviceMetricsOverride", { ...viewport, deviceScaleFactor: 1, mobile: false });
    if (navigate) await cdp.send("Page.navigate", { url });
    return { url, cdp, evaluate: (source) => whileRunning(evaluate(source)), details, database: join(dbDir, "app.sqlite") };
  })());
}

module.exports = { launchFixture };
