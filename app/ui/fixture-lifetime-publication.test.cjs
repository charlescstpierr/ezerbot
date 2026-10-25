const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const { mkdtempSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { setImmediate: nextTurn } = require("node:timers/promises");
const { createFixtureOwner } = require("./fixture-lifetime.cjs");

test("await spawn publishes its handle before a batched launch error, exit and close", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-publication-"));
  t.after(() => rmSync(root, { recursive: true, force: false, maxRetries: 0 }));
  const transport = new EventEmitter();
  transport.connected = true;
  transport.disconnect = () => { transport.connected = false; transport.emit("disconnect"); };
  transport.send = (request, callback) => {
    queueMicrotask(() => {
      const deliver = (message) => transport.emit("message", message);
      if (request.type === "init") deliver({ type: "ready" });
      else if (request.type === "spawn") {
        const launch = request.launch;
        deliver({ type: "spawned", request: request.request, launch, pid: 12345 });
        deliver({ type: "data", launch, stream: "stdout", chunk: Buffer.from("ready\n").toString("base64") });
        deliver({ type: "target-error", launch });
        deliver({ type: "target-exit", launch, code: 127, signal: null });
        deliver({ type: "stream-end", launch, stream: "stdout" });
        deliver({ type: "stream-end", launch, stream: "stderr" });
      } else if (request.type === "close") deliver({ type: "response", request: request.request, ok: true });
      else assert.fail("this transport must never receive a signal request");
      callback?.();
    });
  };
  const owner = await createFixtureOwner({ root, start: () => transport });
  t.after(() => owner.close());
  const child = await owner.spawn("unexecuted-transport-fixture", []);
  const events = [];
  let output = "";
  child.on("error", (error) => { assert.match(error.message, /launch/); events.push("error"); });
  child.on("exit", (code, signal) => { assert.equal(code, 127); assert.equal(signal, null); events.push("exit"); });
  child.on("close", (code, signal) => { assert.equal(code, 127); assert.equal(signal, null); events.push("close"); });
  child.stdout.on("data", (chunk) => { output += chunk; });
  assert.equal(child.exitCode, 127, "terminal state is available even before notifications are delivered");
  assert.equal(child.kill(), false, "an unpublished terminal launch cannot regain signal authority");
  await nextTurn();
  assert.deepEqual(events, ["error", "exit", "close"]);
  assert.equal(output, "ready\n");
});
