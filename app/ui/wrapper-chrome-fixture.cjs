// A private wrapper and the pinned headless shell share one lifetime owner. The owner
// stops their reserved process groups and waits for writers before removing the root.
// Runner death closes its lease and enters that same cleanup path.
const { createServer } = require("node:http");
const { mkdirSync, mkdtempSync, readFileSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { pinnedFixtureShell } = require("./chrome-binary.cjs");
const { createFixtureOwner } = require("./fixture-lifetime.cjs");
const { LaunchIdentity, connectCdp } = require("./local-api-client.cjs");

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// The scripted model. A user message "create_document:{json}" makes it call that tool with the
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
  const request = strings(lastUser).find((text) => text.startsWith("create_document:"));
  const offered = (payload.tools || []).some((tool) => tool?.name === "create_document");
  if (request && offered && !answered) {
    const call = {
      id: "fc-fixture",
      type: "function_call",
      status: "completed",
      call_id: `fixture-${Buffer.from(request).toString("base64url").slice(-24)}`,
      name: "create_document",
      arguments: request.slice("create_document:".length),
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

async function pageTarget(port, ms = 10000) {
  const deadline = Date.now() + ms;
  const message = "chrome did not open a page target";
  while (Date.now() < deadline) {
    const request = fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(Math.max(1, deadline - Date.now())) })
      .then((response) => response.json()).catch((error) => { throw timedOut(error) ? new Error(message) : error; });
    const targets = await beforeDeadline(request, deadline, message);
    const found = targets.find((target) => target.type === "page" && target.webSocketDebuggerUrl);
    if (found) return found;
    await sleep(20);
  }
  throw new Error(message);
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

// Whether a fetch failed because its AbortSignal.timeout ran out.
function timedOut(error) {
  return error?.name === "TimeoutError" || error?.name === "AbortError";
}

// Settles like `work`, or rejects with `message` once `deadline` passes, whichever comes first, so
// a slow step cannot stretch a bounded wait.
function beforeDeadline(work, deadline, message) {
  let timer;
  const expired = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), Math.max(0, deadline - Date.now()));
  });
  return Promise.race([work, expired]).finally(() => clearTimeout(timer));
}

// Polls `read(deadline)` until it returns something truthy, within `ms` in all, a slow read included.
async function untilTrue(read, message, ms = 20000) {
  const deadline = Date.now() + ms;
  for (;;) {
    const last = await beforeDeadline(read(deadline), deadline, message);
    if (last) return last;
    if (Date.now() + 50 >= deadline) throw new Error(message);
    await sleep(50);
  }
}

// Starts the scripted model, the wrapper and Chrome; creates one conversation per entry of
// `documents` ({ title, name, format, content }) through /api/conversations and /api/send and
// waits for each create_document result; opens the app in Chrome. Resolves with the app's URL
// and `evaluate(source)`, which runs `source` (an expression, usually a function) in the page.
async function launchFixture(t, { documents, binary = join(__dirname, "../../target/debug/grokbot-wrapper") }) {
  const root = mkdtempSync(join(tmpdir(), "grokbot-fixture-"));
  const dbDir = join(root, "db");
  mkdirSync(dbDir);
  const fake = { server: null };
  const watches = [];
  const identity = new LaunchIdentity();
  const owner = await createFixtureOwner({ root, start: identity.spawn.bind(identity) });
  let cdp;
  // node:test also closes the owner when a test times out before its body can finish.
  t.after(async () => {
    watches.forEach((watch) => watch.settle());
    cdp?.close();
    fake.server?.closeAllConnections();
    fake.server?.close();
    await owner.close();
  });
  const whileRunning = (work) => Promise.race([work, ...watches.map((watch) => watch.failed)]);

  const openai = await startFakeOpenAi();
  fake.server = openai.server;
  writeFakeSession(dbDir);

  let stderr = "";
  const wrapper = await owner.spawn(binary, ["--port", "0", "--db", join(dbDir, "app.sqlite")], {
    env: {
      PATH: process.env.PATH,
      HOME: dbDir,
      CFFIXED_USER_HOME: dbDir,
      TMPDIR: tmpdir(),
      LANG: process.env.LANG || "C.UTF-8",
      GROKBOT_FAKE_OPENAI: openai.base,
    },
    authenticated: true,
  });
  wrapper.stderr.on("data", (chunk) => { if (stderr.length < 4000) stderr += chunk; });
  const details = () => (stderr ? `: ${stderr}` : "");
  watches.push(watchChild("grokbot-wrapper", wrapper, details));

  const chromeDir = join(root, "chrome");
  mkdirSync(chromeDir);
  const chrome = await owner.spawn(pinnedFixtureShell(), [
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
  ]);
  watches.push(watchChild("chrome", chrome));

  return whileRunning((async () => {
    const url = JSON.parse(await beforeDeadline(readLine(wrapper, details), Date.now() + 20000, `grokbot-wrapper did not print its address within 20 s${details()}`)).url;
    const headers = { Origin: url, "Content-Type": "application/json", "X-Grokbot-Request": "local" };
    // Each call gets the time left before `deadline` (20 s by default), the reading of the answer
    // included.
    const api = (path, body, deadline = Date.now() + 20000) => {
      const late = `${path} did not answer in time${details()}`;
      return beforeDeadline((async () => {
        const signal = AbortSignal.timeout(Math.max(1, deadline - Date.now()));
        const response = await identity.fetch(`${url}${path}`, body === undefined ? { headers, signal } : { method: "POST", headers, signal, body: JSON.stringify(body) });
        const text = await response.text();
        if (!response.ok) throw new Error(`${path} ${response.status}: ${text}${details()}`);
        return JSON.parse(text);
      })().catch((error) => { throw timedOut(error) ? new Error(late) : error; }), deadline, late);
    };
    await untilTrue(async (deadline) => (await api("/api/state", undefined, deadline)).provider?.models_ready, `le catalogue du faux modèle n’est pas chargé${details()}`);
    for (const [index, document] of documents.entries()) {
      const { conversation } = await api("/api/conversations", { title: document.title });
      await api("/api/send", {
        conversation_id: conversation.id,
        message_id: `fixture-${index}`,
        body: `create_document:${JSON.stringify({ name: document.name, format: document.format, content: document.content })}`,
      });
      await untilTrue(async (deadline) => {
        const found = (await api("/api/state", undefined, deadline)).conversations.find((item) => item.id === conversation.id);
        return found?.messages?.some((message) => message.payload?.outcome?.output?.id?.startsWith("generated_"))
          && found.messages.at(-1).role === "assistant";
      }, `le document ${document.format} n’a pas été créé${details()}`);
    }

    const port = await debuggingPort(chromeDir, chrome);
    const target = await pageTarget(port);
    cdp = await beforeDeadline(connectCdp(target.webSocketDebuggerUrl), Date.now() + 10000, "the devtools websocket did not open within 10 s");
    const pageEvents = { domContentLoaded: 0, loaded: 0, runtimeExceptions: 0, failedResponses: 0 };
    cdp.on("Page.domContentEventFired", () => { pageEvents.domContentLoaded++; });
    cdp.on("Page.loadEventFired", () => { pageEvents.loaded++; });
    cdp.on("Runtime.exceptionThrown", () => { pageEvents.runtimeExceptions++; });
    cdp.on("Fetch.requestPaused", (event) => {
      if (event.responseErrorReason || event.responseStatusCode >= 400) pageEvents.failedResponses++;
    });
    const evaluate = async (expression) => {
      const result = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
      if (result.exceptionDetails) {
        const exception = result.exceptionDetails.exception;
        throw new Error(`${exception?.description || exception?.value || result.exceptionDetails.text}; browser events ${JSON.stringify(pageEvents)}`);
      }
      return result.result.value;
    };
    watches.push(await identity.authenticateBrowser(cdp, url));
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Page.navigate", { url });
    return { url, evaluate: (source) => whileRunning(evaluate(source)), details, root, pids: { wrapper: wrapper.pid, chrome: chrome.pid } };
  })());
}

module.exports = { launchFixture, beforeDeadline, untilTrue, pageTarget };
