// The cleanup of wrapper-chrome-fixture (#490): the fixture directory is removed only after the
// wrapper and Chrome, helper processes included, have really exited, and whatever happens to the
// test runner. Chrome's helpers outlive a SIGKILL of the browser process and keep writing into its
// profile, and Crashpad leaves the process group (setsid) but still names the profile on its command
// line. The stand-ins here do the same: a leader with a helper in its group that writes into the
// directory without pause (recreating it if it disappears), and a "Crashpad" in a session of its own
// that does the same until shortly after the leader is gone. Nothing here starts a browser.
const test = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync, spawn } = require("node:child_process");
const { createServer } = require("node:http");
const { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { createFixtureOwner } = require("./fixture-lifetime.cjs");
const { beforeDeadline, pageTarget, untilTrue } = require("./wrapper-chrome-fixture.cjs");

const LIFETIME = join(__dirname, "fixture-lifetime.cjs");

const WRITER = `
const { mkdirSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");
let n = 0;
function write(root) {
  for (let i = 0; i < 50; i++) {
    mkdirSync(join(root, "chrome", "profile"), { recursive: true });
    writeFileSync(join(root, "chrome", "profile", "file-" + n++), "x");
  }
}
`;

// In the leader's group, like Chrome's GPU and renderer helpers.
const HELPER = `${WRITER}
const root = process.argv[1];
setInterval(() => write(root), 1);
`;

// Own session, command line names the profile, ends on its own a little after its parent is gone.
const CRASHPAD = `${WRITER}
const [root, parent] = process.argv.slice(1);
setInterval(() => write(root), 1);
setInterval(() => {
  try { process.kill(Number(parent), 0); } catch { setTimeout(() => process.exit(0), 200); }
}, 5);
`;

const LEADER = `
const { spawn } = require("node:child_process");
const root = process.argv[1];
const helper = spawn(process.execPath, ["-e", ${JSON.stringify(HELPER)}, root], { stdio: "ignore" });
const crashpad = spawn(process.execPath, ["-e", ${JSON.stringify(CRASHPAD)}, root + "/chrome/profile/Crashpad", String(process.pid)], { stdio: "ignore", detached: true });
console.log(JSON.stringify({ helper: helper.pid, crashpad: crashpad.pid }));
setInterval(() => {}, 1000);
`;

function gone(pid) {
  try {
    process.kill(pid, 0);
    return false;
  } catch (error) {
    return error.code === "ESRCH";
  }
}

async function pause(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function until(read, message, ms = 10000) {
  const deadline = Date.now() + ms;
  while (!read()) {
    assert.ok(Date.now() < deadline, message);
    await pause(10);
  }
}

function firstLine(child) {
  return new Promise((resolve, reject) => {
    let output = "";
    function finish(error, line) {
      child.stdout.removeListener("data", data);
      child.removeListener("error", failed);
      child.removeListener("exit", exited);
      if (error) reject(error);
      else resolve(line);
    }
    function data(chunk) {
      output += chunk;
      const end = output.indexOf("\n");
      if (end !== -1) finish(null, output.slice(0, end).trim());
    }
    function failed(error) { finish(error); }
    function exited() { finish(new Error("the process ended before printing its line")); }
    child.stdout.on("data", data);
    child.once("error", failed);
    child.once("exit", exited);
  });
}

function lineWithin(child, ms = 15000) {
  return beforeDeadline(firstLine(child), Date.now() + ms, `no line from process ${child.pid} within ${ms} ms`);
}

function exited(child, ms = 3000) {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  return beforeDeadline(new Promise((resolve, reject) => {
    child.once("exit", resolve);
    child.once("error", reject);
  }), Date.now() + ms, `process ${child.pid} did not exit within ${ms} ms`);
}

function fixtureRoot() {
  return mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
}

async function ownedRoot(t, { root = fixtureRoot(), start } = {}) {
  const owner = await createFixtureOwner({ root, start });
  t.after(() => owner.close());
  return owner;
}

// A failed close is terminal. The harness removes the retained root only after the independent
// owner or inspection worker that caused the refusal has finished.
function retainedRoot(t, owner, release) {
  let closing;
  t.after(async () => {
    await (closing ??= owner.close()).catch(() => {});
    await release();
    rmSync(owner.root, { recursive: true, force: true });
  });
  return (options) => (closing ??= owner.close(options));
}

async function startLeader(t, root = fixtureRoot()) {
  const owner = await ownedRoot(t, { root });
  const leader = await owner.spawn(process.execPath, ["-e", LEADER, root]);
  const { helper, crashpad } = JSON.parse(await lineWithin(leader));
  await until(() => existsSync(join(root, "chrome", "profile", "file-100")), "the helpers never started writing");
  return { root, owner, leader, helper, crashpad };
}

test("the fixture directory is removed only after the helpers of a killed leader have exited, Crashpad's session included", async (t) => {
  const { root, owner, helper, crashpad } = await startLeader(t);
  assert.deepEqual(await owner.close(), { removed: true });
  assert.ok(gone(helper), "the helper of the group must be gone when the cleanup returns");
  assert.ok(gone(crashpad), "the helper of its own session must be gone when the cleanup returns");
  assert.ok(!existsSync(root), "the directory must be removed");
  await pause(150);
  assert.ok(!existsSync(root), "nothing writes into the directory after the cleanup");
});

test("the cleanup also stops the helpers when their leader already exited", async (t) => {
  const { root, owner, leader, helper, crashpad } = await startLeader(t);
  leader.kill("SIGKILL");
  await exited(leader);
  assert.ok(!gone(helper), "the helper outlives its leader");
  await owner.close();
  assert.ok(gone(helper) && gone(crashpad));
  assert.ok(!existsSync(root));
});

test("processes killed but not yet reaped, and processes already gone, are not an error", async (t) => {
  const owner = await ownedRoot(t);
  const first = await owner.spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"]);
  const second = await owner.spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"]);
  const done = await owner.spawn(process.execPath, ["-e", ""]);
  await exited(done);
  first.kill("SIGKILL");
  await owner.close();
  assert.ok(gone(first.pid) && gone(second.pid));
  assert.ok(!existsSync(owner.root));
});

test("something that names the directory and does not exit makes the cleanup fail clearly after the bound, and the directory stays", async (t) => {
  const root = fixtureRoot();
  const owner = await createFixtureOwner({ root });
  const foreign = await ownedRoot(t);
  const close = retainedRoot(t, owner, () => foreign.close());
  const stuck = await foreign.spawn(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)", join(root, "chrome", "profile")]);
  assert.equal(await lineWithin(stuck), "up");
  const started = Date.now();
  await assert.rejects(close({ stopMs: 2000 }), (error) => {
    assert.match(error.message, /directory .* kept|deletion is indeterminate/);
    assert.match(error.message, new RegExp(`process ${stuck.pid} `));
    return true;
  });
  assert.ok(Date.now() - started < 3000, "the wait is bounded");
  assert.ok(existsSync(root));
  assert.ok(!gone(stuck.pid), "an observed foreign process must not be signalled");
});

const RUNNER = `
const { mkdtempSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { createFixtureOwner } = require(${JSON.stringify(LIFETIME)});
(async () => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const owner = await createFixtureOwner({ root });
  const leader = await owner.spawn(process.execPath, ["-e", ${JSON.stringify(LEADER)}, root]);
  leader.stdout.once("data", (chunk) => {
    console.log(JSON.stringify({ root, leader: leader.pid, ...JSON.parse(String(chunk)) }));
  });
})();
setInterval(() => {}, 1000);
`;

test("when the test runner is killed, its owner stops every helper and removes the directory", async (t) => {
  const runner = spawn(process.execPath, ["-e", RUNNER], { stdio: ["ignore", "pipe", "ignore"] });
  let launched;
  t.after(async () => {
    if (runner.exitCode === null && runner.signalCode === null) runner.kill("SIGKILL");
    await exited(runner);
    if (launched) await until(() => gone(launched.leader) && gone(launched.helper) && gone(launched.crashpad) && !existsSync(launched.root), "the owner did not finish cleanup after runner death");
  });
  launched = JSON.parse(await lineWithin(runner));
  const { root, leader, helper, crashpad } = launched;
  await until(() => existsSync(join(root, "chrome", "profile", "file-100")), "the helpers never started writing");
  assert.ok(!gone(leader) && !gone(helper) && !gone(crashpad));
  runner.kill("SIGKILL");
  await exited(runner);
  await until(() => gone(leader) && gone(helper) && gone(crashpad) && !existsSync(root), "the owner did not clean up after the runner died");
  await pause(150);
  assert.ok(!existsSync(root), "nothing writes into the directory after the owner's cleanup");
});

test("an idempotent close removes its own root and leaves an independent owner and process alone", async (t) => {
  const owner = await ownedRoot(t);
  const independent = await ownedRoot(t);
  const group = await owner.spawn(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)"]);
  const witness = await independent.spawn(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)"]);
  assert.equal(await lineWithin(group), "up");
  assert.equal(await lineWithin(witness), "up");
  const first = owner.close();
  assert.equal(owner.close(), first, "concurrent close calls share one cleanup promise");
  assert.deepEqual(await first, { removed: true });
  assert.equal(owner.close(), first, "a terminal close reuses its original result");
  await pause(300);
  assert.ok(gone(group.pid) && !existsSync(owner.root));
  assert.ok(!gone(witness.pid), "closing another owner must not kill the independent process");
  assert.ok(existsSync(independent.root), "the independent root remains owned and present");
});

test("a ps slower than the bound makes the cleanup fail at the bound, is killed, and the directory stays", async (t) => {
  const root = fixtureRoot();
  const bin = join(root, "bin");
  const marker = join(root, "ps.pid");
  mkdirSync(bin);
  const quotedMarker = `'${marker.replace(/'/g, "'\\''")}'`;
  writeFileSync(join(bin, "ps"), `#!/bin/sh\nprintf '%s\\n' "$$" > ${quotedMarker}\nexec /bin/sleep 5\n`);
  chmodSync(join(bin, "ps"), 0o755);
  const owner = await createFixtureOwner({ root, start: (command, args, options) => spawn(command, args, {
    ...options, env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
  }) });
  let inspector;
  const close = retainedRoot(t, owner, async () => {
    if (existsSync(marker)) inspector = Number(readFileSync(marker, "utf8"));
    if (inspector) await until(() => gone(inspector), "the slow ps must be killed at the bound", 2000);
  });
  const started = Date.now();
  await assert.rejects(close({ stopMs: 1000 }), (error) => {
    assert.match(error.message, /directory .* kept|deletion is indeterminate/);
    assert.match(error.message, /ps|check|deadline|bound|timed out/);
    return true;
  });
  const elapsed = Date.now() - started;
  assert.ok(elapsed < 4000, `the bound holds while ps is slow (${elapsed} ms)`);
  assert.ok(existsSync(root));
  assert.ok(existsSync(marker), "the cleanup launched its slow process-table inspection");
  inspector = Number(readFileSync(marker, "utf8"));
  assert.ok(Number.isSafeInteger(inspector) && inspector > 1);
  await until(() => gone(inspector), "the slow ps must be killed at the bound", 2000);
});

test("a process that names a neighbouring directory is not waited for", async (t) => {
  const root = fixtureRoot();
  const owner = await ownedRoot(t, { root });
  const neighbour = `${root}-other`;
  mkdirSync(neighbour);
  const otherOwner = await ownedRoot(t, { root: neighbour });
  const other = await otherOwner.spawn(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)", join(neighbour, "chrome", "profile"), `/elsewhere${root}/chrome`]);
  assert.equal(await lineWithin(other), "up");
  const started = Date.now();
  await owner.close({ stopMs: 3000 });
  assert.ok(Date.now() - started < 2000, "the neighbour must not hold the cleanup until the bound");
  assert.ok(!existsSync(root));
  assert.ok(!gone(other.pid) && existsSync(neighbour), "the neighbour and its directory are left alone");
});

test("a process that names the directory through its real path is waited for", async (t) => {
  const root = fixtureRoot();
  const owner = await createFixtureOwner({ root });
  const foreign = await ownedRoot(t);
  const close = retainedRoot(t, owner, () => foreign.close());
  const stuck = await foreign.spawn(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)", `profile=${realpathSync(root)}/chrome/profile`]);
  assert.equal(await lineWithin(stuck), "up");
  await assert.rejects(close({ stopMs: 2000 }), (error) => {
    assert.match(error.message, /directory .* kept|deletion is indeterminate/);
    assert.match(error.message, new RegExp(`process ${stuck.pid} `));
    return true;
  });
  assert.ok(existsSync(root));
  assert.ok(!gone(stuck.pid), "canonical-path observation never grants signal authority");
});

const NEVER_REAPS = '$| = 1; my $pid = fork(); if ($pid == 0) { setpgrp(0, 0); exit 0 } print "$pid\\n"; sleep 60';

test("a zombie cannot keep the fixture root after its writers have stopped", async (t) => {
  const owner = await ownedRoot(t);
  const parent = await owner.spawn("perl", ["-e", NEVER_REAPS]);
  const zombie = Number(await lineWithin(parent));
  const row = () => execFileSync("ps", ["-o", "pgid=,stat=", "-p", String(zombie)], { encoding: "utf8", timeout: 5000 }).trim().split(/\s+/);
  await until(() => row()[1]?.startsWith("Z"), "the child never became a zombie");
  assert.equal(Number(row()[0]), zombie, "the unreaped child still owns its group");
  try {
    assert.equal(process.kill(-zombie, 0), true, "signal 0 still finds the zombie group");
  } catch (error) {
    assert.equal(error.code, "EPERM", "only permission denial also establishes that the group exists");
  }
  await owner.close({ stopMs: 3000 });
  assert.ok(gone(parent.pid), "the live parent is stopped before removing the root");
  assert.ok(!existsSync(owner.root));
});

test("invalid launch inputs and changed diagnostic PIDs never authorize a foreign signal", async (t) => {
  const owner = await ownedRoot(t);
  const independent = await ownedRoot(t);
  const witness = await independent.spawn(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)"]);
  assert.equal(await lineWithin(witness), "up");
  for (const command of [0, 1, -1, -42, 1.5, NaN, Infinity, undefined, null, process.pid]) {
    await assert.rejects(owner.spawn(command, []), /command|executable/, String(command));
  }
  const marker = join(owner.root, "invalid-launch-ran");
  await assert.rejects(owner.spawn(process.execPath, ["-e", "require('node:fs').writeFileSync(process.argv[1], 'ran')", marker, undefined]), /args|argument/);
  assert.ok(!existsSync(marker), "invalid launch arguments prevent useful execution");
  const stale = await owner.spawn(process.execPath, ["-e", ""]);
  await exited(stale);
  stale.pid = witness.pid;
  stale.kill("SIGKILL");
  const live = await owner.spawn(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)"]);
  assert.equal(await lineWithin(live), "up");
  const ownedPid = live.pid;
  live.pid = witness.pid;
  live.kill("SIGKILL");
  await exited(live);
  assert.ok(gone(ownedPid), "the live handle still targets only its own launch");
  await pause(150);
  assert.ok(!gone(witness.pid), "a stale or changed diagnostic PID must not target the foreign witness");
  assert.ok(existsSync(independent.root));
});

test("an exec failure still stops admitted siblings, reports the failure, and keeps the root", async (t) => {
  const owner = await createFixtureOwner({ root: fixtureRoot() });
  let next;
  const close = retainedRoot(t, owner, async () => {
    if (next) await until(() => gone(next.pid), "the sibling must be gone before the harness removes the retained root");
  });
  next = await owner.spawn(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)"]);
  assert.equal(await lineWithin(next), "up");
  let child;
  let failure;
  try {
    child = await owner.spawn(join(owner.root, "missing-executable"), []);
  } catch (error) {
    failure = error;
  }
  if (child) failure = await beforeDeadline(new Promise((resolve) => child.once("error", resolve)), Date.now() + 3000, "no launch error was reported");
  assert.match(failure.message, /exec|ENOENT|launch/);
  await assert.rejects(close({ stopMs: 5000 }), /exec|ENOENT|launch/);
  await until(() => gone(next.pid), "the sibling of the failed launch must be stopped");
  assert.ok(existsSync(owner.root));
});

test("a directory that is not a fixture directory inside the temporary directory is never removed", async (t) => {
  const owner = await ownedRoot(t);
  const nested = join(owner.root, "nested");
  mkdirSync(nested);
  await assert.rejects(createFixtureOwner({ root: nested }), /invalid fixture root/);
  assert.ok(existsSync(nested));
});

const EARLY_DEATH = `
const { writeSync } = require("node:fs");
const { spawn } = require("node:child_process");
const { createFixtureOwner } = require(${JSON.stringify(LIFETIME)});
const [root, marker] = process.argv.slice(1);
(async () => {
  const owner = await createFixtureOwner({ root, start(command, args, options) {
    const child = spawn(command, args, options);
    const send = child.send.bind(child);
    child.send = (message, ...rest) => {
      if (message.type === "spawn") {
        writeSync(1, "held\\n");
        child.disconnect();
        process.kill(process.pid, "SIGKILL");
        return false;
      }
      return send(message, ...rest);
    };
    return child;
  } });
  await owner.spawn(process.execPath, ["-e", "require('node:fs').writeFileSync(process.argv[1], 'ran')", marker]);
})();
setInterval(() => {}, 1000);
`;

test("a runner that dies before launch admission leaves no useful command running", async (t) => {
  const root = fixtureRoot();
  const elsewhere = await ownedRoot(t);
  const marker = join(elsewhere.root, "ran");
  const runner = spawn(process.execPath, ["-e", EARLY_DEATH, root, marker], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(async () => {
    if (runner.exitCode === null && runner.signalCode === null) runner.kill("SIGKILL");
    await exited(runner);
    await until(() => !existsSync(root), "the owner still removes the directory");
  });
  assert.equal(await lineWithin(runner), "held");
  await exited(runner);
  await pause(150);
  assert.ok(!existsSync(marker), "a command denied admission never executes after runner death");
  await until(() => !existsSync(root), "the owner still removes the directory");
});

test("a bounded wait holds its bound when a single read never settles", async () => {
  const started = Date.now();
  await assert.rejects(beforeDeadline(untilTrue(() => new Promise(() => {}), "never ready", 300), Date.now() + 5000, "untilTrue outlived its bound"), /^Error: never ready$/);
  assert.ok(Date.now() - started < 3000);
});

test("the wait for a page target holds its bound when the debugging port never answers", async (t) => {
  const server = createServer(() => { /* accepts and never answers */ });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => { server.closeAllConnections(); server.close(); });
  const started = Date.now();
  await assert.rejects(beforeDeadline(pageTarget(server.address().port, 300), Date.now() + 5000, "pageTarget outlived its bound"), /^Error: chrome did not open a page target$/);
  assert.ok(Date.now() - started < 3000);
});

test("a rejected launch request never releases its useful command", async (t) => {
  let rejectLaunch;
  const owner = await createFixtureOwner({ root: fixtureRoot(), start(command, args, options) {
    const child = spawn(command, args, options);
    const send = child.send.bind(child);
    rejectLaunch = () => {
      child.send = (message, ...rest) => {
        if (message.type !== "spawn") return send(message, ...rest);
        const callback = rest.find((value) => typeof value === "function");
        child.disconnect();
        if (callback) queueMicrotask(() => callback(Object.assign(new Error("launch admission failed"), { code: "EPIPE" })));
        return false;
      };
    };
    return child;
  } });
  const elsewhere = await ownedRoot(t);
  const marker = join(elsewhere.root, "ran");
  t.after(() => until(() => !existsSync(owner.root), "the disconnected owner did not finish cleanup", 3000));
  rejectLaunch();
  const started = Date.now();
  await assert.rejects(owner.spawn(process.execPath, ["-e", "require('node:fs').writeFileSync(process.argv[1], 'ran')", marker]), /admission|EPIPE|closed|disconnect|channel failed/);
  assert.ok(Date.now() - started < 3000, "a rejected launch request is bounded");
  await pause(150);
  assert.ok(!existsSync(marker), "the held command never executes after rejected admission");
  await until(() => !existsSync(owner.root), "the disconnected owner did not finish cleanup", 3000);
});

test("a closed owner rejects a new launch without executing its command", async (t) => {
  const owner = await ownedRoot(t);
  const elsewhere = await ownedRoot(t);
  const marker = join(elsewhere.root, "ran");
  await owner.close();
  await assert.rejects(owner.spawn(process.execPath, ["-e", "require('node:fs').writeFileSync(process.argv[1], 'ran')", marker]), /closed|closing|terminal/);
  await pause(150);
  assert.ok(!existsSync(marker));
  assert.ok(!existsSync(owner.root));
});
