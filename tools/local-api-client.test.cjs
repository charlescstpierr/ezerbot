const assert = require("node:assert/strict");
const { createServer } = require("node:http");
const { test } = require("node:test");
const { setImmediate } = require("node:timers/promises");
const { LaunchIdentity } = require("../app/ui/local-api-client.cjs");

async function listen(handler) {
  const server = createServer(handler);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { server, origin: `http://127.0.0.1:${server.address().port}` };
}

test("launch authentication travels as 32 bytes followed by EOF", async () => {
  const identity = new LaunchIdentity();
  const child = identity.spawn(process.execPath, ["-e", `
    const chunks = [];
    process.stdin.on("data", (chunk) => chunks.push(chunk));
    process.stdin.on("end", () => process.stdout.write(JSON.stringify(Buffer.concat(chunks).length === 32)));
  `], { stdio: ["pipe", "pipe", "pipe"] });
  const chunks = [];
  for await (const chunk of child.stdout) chunks.push(chunk);
  assert.equal(JSON.parse(Buffer.concat(chunks)), true);
  assert.equal(Object.values(identity.headers("http://127.0.0.1:43431"))
    .some((value) => child.spawnargs.some((argument) => argument.includes(value) && value.length === 64)), false);
});

test("the HTTP client refuses redirects and a different local origin", async (t) => {
  let foreignRequests = 0;
  let authenticated = false;
  const foreign = await listen((_, response) => { foreignRequests += 1; response.end(); });
  const local = await listen((request, response) => {
    authenticated = /^[a-f0-9]{64}$/.test(request.headers["x-ezerbot-launch"] || "");
    response.writeHead(302, { Location: foreign.origin });
    response.end();
  });
  t.after(() => { local.server.closeAllConnections(); local.server.close(); foreign.server.closeAllConnections(); foreign.server.close(); });
  const identity = new LaunchIdentity();
  await assert.rejects(identity.fetch(local.origin), /desktop wrapper request failed/);
  assert.equal(authenticated, true);
  assert.equal(foreignRequests, 0);
  await assert.rejects(identity.fetch(foreign.origin), /desktop wrapper origin changed/);
  assert.equal(foreignRequests, 0);
});

test("CDP adds launch authentication only to the exact origin and rejects its redirects", async () => {
  const identity = new LaunchIdentity();
  const callbacks = new Map();
  const commands = [];
  const cdp = {
    on(method, callback) { callbacks.set(method, callback); },
    async send(method, params) { commands.push({ method, params }); return {}; },
  };
  const origin = "http://127.0.0.1:43431";
  await identity.authenticateBrowser(cdp, origin);
  const pause = async (requestId, url, extra = {}) => {
    callbacks.get("Fetch.requestPaused")({ requestId, request: { url, headers: {} }, ...extra });
    await setImmediate();
    return commands.at(-1);
  };
  const local = await pause("local", origin + "/api/state", { networkId: "network-local" });
  assert.equal(local.method === "Fetch.continueRequest", true);
  assert.equal(local.params.headers.some((header) => header.name === "X-Ezerbot-Launch" && /^[a-f0-9]{64}$/.test(header.value)), true);
  for (const url of ["http://127.0.0.1:43432/api/state", "http://localhost:43431/api/state", "https://example.invalid/"]) {
    const foreign = await pause("foreign-" + url, url);
    assert.equal(foreign.method === "Fetch.continueRequest", true);
    assert.equal(foreign.params.headers.some((header) => header.name.toLowerCase() === "x-ezerbot-launch"), false);
  }
  const redirect = await pause("redirect", "http://127.0.0.1:43432/", { redirectedRequestId: "local", networkId: "network-local" });
  assert.equal(redirect.method === "Fetch.failRequest", true);
  const sameOriginRedirect = await pause("same-origin-redirect", origin + "/api/state", { redirectedRequestId: "local" });
  assert.equal(sameOriginRedirect.method === "Fetch.failRequest", true);
  const responseRedirect = await pause("local", origin + "/api/state", { responseStatusCode: 302, networkId: "network-local" });
  assert.equal(responseRedirect.method === "Fetch.failRequest", true);
  await pause("cached", origin + "/app.js");
  const cachedResponse = await pause("cached", origin + "/app.js", { responseStatusCode: 304 });
  assert.equal(cachedResponse.method === "Fetch.continueRequest", true);
  const contaminated = await pause("contaminated", "http://127.0.0.1:43432/", {
    request: { url: "http://127.0.0.1:43432/", headers: identity.headers(origin) },
  });
  assert.equal(contaminated.params.headers.some((header) => header.name.toLowerCase() === "x-ezerbot-launch"), false);
});


test("a Node pipe launches the production wrapper authentication boundary", async (t) => {
  const { mkdtempSync, rmSync } = require("node:fs");
  const { tmpdir } = require("node:os");
  const { join } = require("node:path");
  const home = mkdtempSync(join(tmpdir(), "ezerbot-node-launch-"));
  const identity = new LaunchIdentity();
  const child = identity.spawn(join(__dirname, "../target/debug/grokbot-wrapper"),
    ["--port", "0", "--db", join(home, "workspace.sqlite")],
    { env: { PATH: process.env.PATH, HOME: home, CFFIXED_USER_HOME: home }, stdio: ["pipe", "pipe", "ignore"] });
  t.after(() => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    rmSync(home, { recursive: true, force: true });
  });
  const line = await new Promise((resolve, reject) => {
    let text = "";
    child.stdout.on("data", chunk => { text += chunk; if (text.includes("\n")) resolve(text.split("\n")[0]); });
    child.on("error", () => reject(new Error("test wrapper could not start")));
    child.on("exit", () => reject(new Error("test wrapper exited before its announcement")));
  });
  const { url } = JSON.parse(line);
  assert.equal((await fetch(url + "/api/state")).status, 403);
  assert.equal((await identity.fetch(url + "/api/state")).status, 200);
});
