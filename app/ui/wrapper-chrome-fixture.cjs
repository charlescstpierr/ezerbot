// Fixture for the tests that need a real page: a debug grokbot-wrapper on a throwaway database,
// answered by a scripted OpenAI on 127.0.0.1, seeded through the public HTTP API, and an isolated
// headless Chrome (own profile, own debugging port) driven over CDP. Nothing is shared with the
// person's apps: everything lives under one temporary directory and `launchFixture` registers its
// own cleanup on the test context, so the processes stop when the test ends for any reason,
// including a failed assertion or the test timeout. The directory is removed only once the wrapper
// and Chrome, helper processes included, have really exited (see `cleanUpFixture`), and a guard
// process does the same if the test runner itself dies before its cleanup runs (see `startGuard`).
const { spawn } = require("node:child_process");
const { createServer } = require("node:http");
const { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { basename, dirname, isAbsolute, join } = require("node:path");
const { chromeBinary } = require("./chrome-binary.cjs");
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

// Chrome starts helper processes (GPU, network, renderers) that outlive a SIGKILL of the browser
// process for a moment and keep writing into its profile. So each child the fixture starts leads
// its own process group (`detached`), and the group is what gets killed and awaited.
//
// A detached child outlives a runner that dies, so the guard (see `startGuard`) must know the group
// before the command runs. The child starts as /bin/sh, which waits for one line on fd 3 and only
// then execs the command, keeping its pid and group. `register(pid)` runs in between and may return
// a promise. If the runner dies before release, sh reads the end
// of the pipe and exits without running anything, so there is no window where an unregistered
// command runs.
const HOLD = 'IFS= read -r go <&3 && [ "$go" = go ] || exit 1; exec "$0" "$@" 3<&-';

function spawnGroup(command, args, options = {}, register = () => {}, start = spawn) {
  const stdio = typeof options.stdio === "string" ? [options.stdio, options.stdio, options.stdio] : [...(options.stdio || [])];
  while (stdio.length < 3) stdio.push(undefined);
  stdio[3] = "pipe";
  const child = start("/bin/sh", ["-c", HOLD, command, ...args], { ...options, stdio, detached: true });
  const hold = child.stdio[3];
  hold?.on("error", () => {});
  const go = () => hold?.end("go\n");
  if (child.pid) Promise.resolve().then(() => register(child.pid)).then(go, () => hold?.end());
  else go();
  return child;
}

// Only a group the fixture itself started may be signalled. kill(-0) would hit the caller's own
// group, kill(-1) every process of the user (their browser included), a negative or fractional
// number something else again. So anything but an integer above 1 that is not this process is
// refused before any signal.
function checkGroupPid(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 1 || pid === process.pid) {
    throw new Error(`refusing to signal process group ${String(pid)}: not a group the fixture started`);
  }
  return pid;
}

// Whether `kill(-pid, 0)` still finds the group led by `pid`. ESRCH: gone. EPERM: a member exists
// that we may not signal, which macOS answers for a group whose last member has died but is not
// reaped (a zombie); the process table below tells those apart from live members.
function groupMayExist(pid) {
  try {
    process.kill(-pid, 0);
    return true;
  } catch (error) {
    return error.code !== "ESRCH";
  }
}

// SIGKILLs the group. ESRCH: already gone. EPERM: killed but not yet reaped (macOS). Both are
// waited for like the rest; anything else is returned, so the next groups are still killed.
function killGroup(pid) {
  try {
    process.kill(-pid, "SIGKILL");
    return null;
  } catch (error) {
    return error.code === "ESRCH" || error.code === "EPERM" ? null : error;
  }
}

// The process table ({ pid, pgid, stat, command }), from one `ps`, which gets only the time left
// before `deadline`: a ps that hangs or is slow is killed (by its own pid) and the listing fails at
// the deadline instead of stretching the wait.
function processTable(deadline) {
  return new Promise((resolve, reject) => {
    const ms = deadline - Date.now();
    if (ms <= 0) {
      reject(new Error("no time left to list the processes"));
      return;
    }
    const ps = spawn("ps", ["-axww", "-o", "pid=,pgid=,stat=,command="], { stdio: ["ignore", "pipe", "ignore"] });
    let listing = "";
    ps.stdout.setEncoding("utf8");
    ps.stdout.on("data", (chunk) => { listing += chunk; });
    const timer = setTimeout(() => {
      ps.kill("SIGKILL");
      ps.stdout.destroy();
      reject(new Error(`ps did not answer within ${ms} ms`));
    }, ms);
    ps.once("error", (error) => {
      clearTimeout(timer);
      reject(new Error(`ps failed: ${error.message}`));
    });
    ps.once("close", (code, signal) => {
      clearTimeout(timer);
      if (code !== 0) {
        reject(new Error(`ps ended with code ${code}, signal ${signal}`));
        return;
      }
      resolve(listing.split("\n").map((line) => line.match(/^\s*(\d+)\s+(\d+)\s+(\S+)\s?(.*)$/)).filter(Boolean)
        .map(([, pid, pgid, stat, command]) => ({ pid: Number(pid), pgid: Number(pgid), stat, command })));
    });
  });
}

// Whether `command` names `dir` as a whole path: what precedes it starts an argument (start, blank,
// "=", ":", ",", quote) and what follows ends the directory name (end, "/", blank, ",", ":", quote).
// So "<dir>-other/..." and "/elsewhere<dir>/..." do not count, "--user-data-dir=<dir>/chrome" does.
function namesDirectory(command, dir) {
  const before = new Set([" ", "\t", "=", ":", ",", '"', "'"]);
  const after = new Set(["/", " ", "\t", ",", ":", '"', "'"]);
  for (let at = command.indexOf(dir); at >= 0; at = command.indexOf(dir, at + 1)) {
    const previous = at === 0 ? undefined : command[at - 1];
    const next = command[at + dir.length];
    if ((previous === undefined || before.has(previous)) && (next === undefined || after.has(next))) return true;
  }
  return false;
}

// `root` and the path it resolves to: macOS gives /var/folders/... for the temporary directory and a
// process may show the same directory as /private/var/folders/....
function namesOf(root) {
  try {
    return [...new Set([root, realpathSync(root)])];
  } catch {
    return [root];
  }
}

// What is still running for this fixture, from one process table: the members of the groups that
// are not zombies (an exited process its parent has not reaped, which can no longer write and which
// a parent that never reaps keeps forever), then, once no group is left, any live process but this
// one whose command line names `root` (Crashpad leaves the group with setsid but keeps naming the
// profile).
async function stillRunning({ names, groups, deadline }) {
  const present = groups.filter(({ pid }) => groupMayExist(pid));
  const live = (await processTable(deadline)).filter(({ pid, stat }) => !stat.startsWith("Z") && pid !== process.pid);
  const left = present.filter(({ pid }) => live.some((row) => row.pgid === pid)).map(({ name, pid }) => `${name} (process group ${pid})`);
  if (left.length) return left;
  return live.filter(({ command }) => names.some((dir) => namesDirectory(command, dir)))
    .map(({ pid }) => `process ${pid} (its command line names the fixture directory)`);
}

// SIGKILLs every group in `groups` ({ name, pid }), then waits until every group is gone and no
// process names `root` any more, within `ms` in all: each listing of the processes gets only the
// time left, and nothing counts as gone if it was seen after the deadline. At the deadline the
// error names what the last complete listing showed, or says that none completed. A group that is
// refused or cannot be signalled does not stop the others; the call fails once they are gone.
async function stopFixtureProcesses({ root, groups, ms = 10000 }) {
  const deadline = Date.now() + ms;
  const refused = [];
  const signalled = [];
  for (const group of groups) {
    try {
      const error = killGroup(checkGroupPid(group.pid));
      if (error) refused.push(`${group.name}: ${error.message}`);
      signalled.push(group);
    } catch (error) {
      refused.push(`${group.name}: ${error.message}`);
    }
  }
  const names = namesOf(root);
  let seen = null;
  for (;;) {
    let left;
    try {
      left = await stillRunning({ names, groups: signalled, deadline });
    } catch (error) {
      if (seen?.length) throw new Error(`still running ${ms} ms after SIGKILL: ${seen.join(", ")}`);
      throw new Error(`could not check within ${ms} ms that the processes exited: ${error.message}`);
    }
    if (Date.now() > deadline) {
      throw new Error(left.length ? `still running ${ms} ms after SIGKILL: ${left.join(", ")}` : `the exit was only seen after the ${ms} ms bound`);
    }
    if (!left.length) break;
    seen = left;
    await sleep(Math.min(20, deadline - Date.now()));
  }
  if (refused.length) throw new Error(`not signalled: ${refused.join("; ")}`);
}

// Only a directory made for a fixture, right inside the temporary directory, is ever removed.
function checkFixtureRoot(root) {
  if (typeof root !== "string" || !isAbsolute(root) || dirname(root) !== tmpdir() || basename(root) === "") {
    throw new Error(`refusing ${String(root)}: not a fixture directory inside ${tmpdir()}`);
  }
}

// The fixture's cleanup: stops the processes and waits for their exit (bounded), then removes
// `root`, once. If something does not exit in time the directory is kept, and the error says what.
async function cleanUpFixture({ root, groups, stopMs = 10000 }) {
  checkFixtureRoot(root);
  try {
    await stopFixtureProcesses({ root, groups, ms: stopMs });
  } catch (error) {
    throw new Error(`fixture directory ${root} kept: ${error.message}`);
  }
  rmSync(root, { recursive: true, force: true });
}

// `detached` children outlive their parent, and `t.after` does not run if the test runner itself
// dies (crash, SIGKILL, deadline). So a guard process of its own session holds the read end of a
// pipe from the runner: when the runner ends, the pipe closes and the guard runs the same cleanup.
// A normal cleanup tells it "done" first. The runner sends the directory, then each group it starts;
// `register` resolves once the line is in the pipe, so the guard reads it even if the runner dies
// right after.
function startGuard(root) {
  const guard = spawn(process.execPath, [__filename, "--guard"], { detached: true, stdio: ["pipe", "ignore", "ignore"] });
  guard.unref();
  guard.stdin.unref();
  guard.stdin.on("error", () => {});
  guard.on("error", () => {});
  guard.stdin.write(`root ${root}\n`);
  return {
    register(name, pid) { return new Promise((resolve, reject) => guard.stdin.write(`group ${name} ${pid}\n`, (error) => error ? reject(error) : resolve())); },
    release() { guard.stdin.end("done\n"); },
  };
}

function runGuard() {
  let root = null;
  const groups = [];
  let buffer = "";
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (chunk) => {
    buffer += chunk;
    for (let end = buffer.indexOf("\n"); end >= 0; end = buffer.indexOf("\n")) {
      const line = buffer.slice(0, end);
      buffer = buffer.slice(end + 1);
      if (line === "done") process.exit(0);
      if (line.startsWith("root ")) root = line.slice(5);
      else if (line.startsWith("group ")) {
        const [, name, pid] = line.split(" ");
        groups.push({ name, pid: Number(pid) });
      }
    }
  });
  process.stdin.on("end", async () => {
    // The runner is gone without saying "done": clean up for it.
    if (root) await cleanUpFixture({ root, groups }).catch(() => {});
    process.exit(0);
  });
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
  const processes = { wrapper: null, chrome: null };
  const watches = [];
  const guard = startGuard(root);
  let cdp;
  // Registered before anything can fail, and run by node:test when the test ends for any reason,
  // including the test timeout, where the awaited body never reaches a `finally`.
  t.after(async () => {
    watches.forEach((watch) => watch.settle());
    cdp?.close();
    fake.server?.closeAllConnections();
    fake.server?.close();
    const groups = Object.entries(processes).filter(([, child]) => child?.pid).map(([name, child]) => ({ name, pid: child.pid }));
    try {
      await cleanUpFixture({ root, groups });
    } finally {
      guard.release();
    }
  });
  const whileRunning = (work) => Promise.race([work, ...watches.map((watch) => watch.failed)]);

  const openai = await startFakeOpenAi();
  fake.server = openai.server;
  writeFakeSession(dbDir);

  let stderr = "";
  const identity = new LaunchIdentity();
  const wrapper = spawnGroup(binary, ["--port", "0", "--db", join(dbDir, "app.sqlite")], {
    env: {
      PATH: process.env.PATH,
      HOME: dbDir,
      CFFIXED_USER_HOME: dbDir,
      TMPDIR: tmpdir(),
      LANG: process.env.LANG || "C.UTF-8",
      GROKBOT_FAKE_OPENAI: openai.base,
    },
    stdio: ["ignore", "pipe", "pipe"],
  }, (pid) => guard.register("wrapper", pid), identity.spawn.bind(identity));
  processes.wrapper = wrapper;
  wrapper.stderr.on("data", (chunk) => { if (stderr.length < 4000) stderr += chunk; });
  const details = () => (stderr ? `: ${stderr}` : "");
  watches.push(watchChild("grokbot-wrapper", wrapper, details));

  const chromeDir = join(root, "chrome");
  mkdirSync(chromeDir);
  const chrome = spawnGroup(chromeBinary(), [
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
  ], { stdio: ["ignore", "ignore", "pipe"] }, (pid) => guard.register("chrome", pid));
  processes.chrome = chrome;
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
    await cdp.send("Page.navigate", { url });
    return { url, evaluate: (source) => whileRunning(evaluate(source)), details, root, pids: { wrapper: wrapper.pid, chrome: chrome.pid } };
  })());
}

if (require.main === module && process.argv[2] === "--guard") runGuard();

module.exports = { launchFixture, cleanUpFixture, spawnGroup, startGuard, checkGroupPid, beforeDeadline, untilTrue, pageTarget };
