const test = require("node:test");
const assert = require("node:assert/strict");
const { spawn, execFile } = require("node:child_process");
const { existsSync, mkdtempSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { promisify } = require("node:util");
const { nativeBinary } = require("./fixture-lifetime.cjs");
const { LaunchIdentity } = require("./local-api-client.cjs");

const exec = promisify(execFile);
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function within(work, ms, label) {
  let timer;
  try {
    return await Promise.race([work, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(label)), ms);
    })]);
  } finally { clearTimeout(timer); }
}

async function until(read, label, ms = 3000) {
  const deadline = performance.now() + ms;
  while (!(await read())) {
    assert.ok(performance.now() < deadline, label);
    await pause(10);
  }
}

async function processRow(pid) {
  const { stdout, stderr } = await exec("ps", ["-ax", "-o", "pid=,stat="], { timeout: 1000 });
  assert.equal(stderr, "");
  return stdout.split("\n").map((line) => line.trim().split(/\s+/)).find(([found]) => Number(found) === pid);
}

async function launch(t, args, start = spawn) {
  const child = start(await nativeBinary(), args, { detached: true, stdio: ["ignore", "pipe", "pipe", "pipe", "pipe"] });
  const events = [];
  let buffer = "";
  let output = "";
  child.stdout.on("data", (chunk) => { output += chunk; });
  child.stderr.resume();
  child.stdio[3].on("error", () => {});
  child.stdio[4].on("data", (chunk) => {
    buffer += chunk;
    let end;
    while ((end = buffer.indexOf("\n")) >= 0) {
      events.push(JSON.parse(buffer.slice(0, end)));
      buffer = buffer.slice(end + 1);
    }
  });
  const closed = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal }));
  });
  const owner = {
    child, events, closed,
    send(line) { child.stdio[3].write(`${line}\n`); },
    async event(name) {
      await until(() => events.some((entry) => entry.event === name), `owner did not report ${name}`);
      return events.find((entry) => entry.event === name);
    },
    async line() {
      await until(() => output.includes("\n"), "payload did not report readiness");
      return output.slice(0, output.indexOf("\n"));
    },
    async finish() {
      if (child.exitCode === null && child.signalCode === null) child.stdio[3].end("finish\n");
      return within(closed, 4000, "native owner did not finish");
    },
  };
  t.after(() => owner.finish());
  return owner;
}

for (const frame of ["", "go", "invalid\n"]) {
  test(`native owner refuses incomplete launch control ${JSON.stringify(frame)}`, async (t) => {
    const root = mkdtempSync(join(tmpdir(), "fixture-native-held-"));
    const marker = join(root, "ran");
    const owner = await launch(t, [process.execPath, "-e", "require('node:fs').writeFileSync(process.argv[1], 'ran')", marker]);
    await owner.event("ready");
    owner.child.stdio[3].end(frame);
    await within(owner.closed, 4000, "rejected launcher did not close");
    assert.ok(!owner.events.some((entry) => entry.event === "started"));
    assert.ok(!existsSync(marker), "no useful command may run on partial control or EOF");
    rmSync(root, { recursive: true, force: true, maxRetries: 0 });
  });
}

test("native owner acknowledges cancellation before useful launch", async (t) => {
  const owner = await launch(t, [process.execPath, "-e", "process.exit(93)"]);
  const result = await owner.finish();
  assert.equal(result.code, 0);
  assert.ok(owner.events.some((entry) => entry.event === "finished" && entry.value === 0));
  assert.ok(!owner.events.some((entry) => entry.event === "started"));
});

test("launch identity crosses the owner only on stdin", async (t) => {
  const identity = new LaunchIdentity();
  const source = `
    const fs = require('node:fs');
    const bytes = fs.readFileSync(0);
    console.log(JSON.stringify({ validLength: bytes.length === 32 }));
  `;
  const owner = await launch(t, [process.execPath, "-e", source], identity.spawn.bind(identity));
  await owner.event("ready");
  owner.send("go");
  assert.deepEqual(JSON.parse(await owner.line()), { validLength: true });
  await owner.event("exited");
  assert.equal((await owner.finish()).code, 0);
});

test("ownership control descriptors do not cross exec", async (t) => {
  const owner = await launch(t, ["/bin/sh", "-c", 'if ( : <&3 ) 2>/dev/null || ( : <&4 ) 2>/dev/null; then exit 91; fi; printf "closed\\n"']);
  await owner.event("ready");
  owner.send("go");
  assert.equal(await owner.line(), "closed");
  assert.equal((await owner.event("exited")).value, 0);
  assert.equal((await owner.finish()).code, 0);
});

test("reaping a killed useful child does not expire the still-owned group", async (t) => {
  const owner = await launch(t, [process.execPath, "-e", "console.log('up'); setInterval(() => {}, 1000)"]);
  await owner.event("ready");
  owner.send("go");
  await owner.line();
  owner.send("kill-target");
  await owner.event("exited");
  await owner.event("target-signal");
  await pause(3100);
  assert.equal(owner.child.exitCode, null, "only cleanup, not useful-child exit, retires group ownership");
  owner.send("kill-target");
  await owner.event("target-terminal");
  assert.equal((await owner.finish()).code, 0);
});

test("a dead unreaped anchor still reserves the immortal helper's group", async (t) => {
  const source = `
    const child = require('node:child_process').spawn(process.execPath,
      ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
    console.log(JSON.stringify({ helper: child.pid }));
    setInterval(() => {}, 1000);
  `;
  const owner = await launch(t, [process.execPath, "-e", source]);
  const anchor = (await owner.event("ready")).pid;
  owner.send("go");
  const { helper } = JSON.parse(await owner.line());
  owner.send("kill-anchor");
  await owner.event("anchor-killed");
  await until(async () => (await processRow(anchor))?.[1].startsWith("Z"), "anchor must remain unreaped");
  assert.ok((await processRow(helper)) && !(await processRow(helper))[1].startsWith("Z"));
  owner.send("kill-target");
  await owner.event("exited");
  assert.ok((await processRow(helper)) && !(await processRow(helper))[1].startsWith("Z"));
  assert.equal((await owner.finish()).code, 0);
  await until(async () => {
    const row = await processRow(helper);
    return !row || row[1].startsWith("Z");
  }, "the immortal helper must be terminal");
});

test("numeric target injection is refused while an independent witness survives", async (t) => {
  const witness = await launch(t, [process.execPath, "-e", "console.log('up'); setInterval(() => {}, 1000)"]);
  await witness.event("ready");
  witness.send("go");
  await witness.line();
  const witnessPid = (await witness.event("started")).pid;
  const owner = await launch(t, [process.execPath, "-e", "setInterval(() => {}, 1000)"]);
  await owner.event("ready");
  owner.send(`kill-target ${witnessPid}`);
  assert.notEqual((await within(owner.closed, 4000, "invalid command did not finish")).code, 0);
  const row = await processRow(witnessPid);
  assert.ok(row && !row[1].startsWith("Z"));
  assert.ok(!owner.events.some((entry) => entry.event === "started"));
});

test("failed exec reports failure after its owned children are reaped", async (t) => {
  const owner = await launch(t, [join(__dirname, "fixture-command-that-does-not-exist")]);
  await owner.event("ready");
  owner.send("go");
  const result = await within(owner.closed, 4000, "failed exec did not finish");
  assert.notEqual(result.code, 0);
  assert.ok(owner.events.some((entry) => entry.event === "exec-failed"));
  assert.ok(owner.events.some((entry) => entry.event === "exited"));
  assert.equal(owner.events.filter((entry) => entry.event === "stopped").length, 1);
  assert.ok(owner.events.some((entry) => entry.event === "finished"));
});


test("loss of the private event channel prevents useful execution", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-native-events-"));
  const marker = join(root, "ran");
  const owner = await launch(t, [process.execPath, "-e", "require('node:fs').writeFileSync(process.argv[1], 'ran')", marker]);
  await owner.event("ready");
  await new Promise((resolve) => {
    owner.child.stdio[4].once("close", resolve);
    owner.child.stdio[4].destroy();
  });
  owner.send("go");
  const result = await within(owner.closed, 4000, "owner did not close after event-channel loss");
  assert.notEqual(result.code, 0);
  assert.ok(!existsSync(marker), "unacknowledged execution is never released");
  rmSync(root, { recursive: true, force: true, maxRetries: 0 });
});

test("the anchor lease stops the admitted group when its native supervisor dies", async (t) => {
  const source = `
    const child = require('node:child_process').spawn(process.execPath,
      ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
    console.log(JSON.stringify({ helper: child.pid }));
    setInterval(() => {}, 1000);
  `;
  const owner = await launch(t, [process.execPath, "-e", source]);
  const anchor = (await owner.event("ready")).pid;
  owner.send("go");
  const { helper } = JSON.parse(await owner.line());
  const target = (await owner.event("started")).pid;
  assert.ok(owner.child.exitCode === null && owner.child.signalCode === null);
  owner.child.kill("SIGKILL");
  assert.equal((await within(owner.closed, 4000, "native owner did not close")).signal, "SIGKILL");
  for (const pid of [anchor, target, helper]) {
    await until(async () => {
      const row = await processRow(pid);
      return !row || row[1].startsWith("Z");
    }, "the private lease must terminate every admitted member");
  }
});
