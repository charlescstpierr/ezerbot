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
const { chmodSync, existsSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { beforeDeadline, checkGroupPid, cleanUpFixture, pageTarget, spawnGroup, startGuard, untilTrue } = require("./wrapper-chrome-fixture.cjs");

const FIXTURE = join(__dirname, "wrapper-chrome-fixture.cjs");

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
    child.stdout.once("data", (chunk) => resolve(String(chunk).trim()));
    child.once("error", reject);
    child.once("exit", () => reject(new Error("the process ended before printing its line")));
  });
}

// The leader and its writers, running. The teardown goes through the same bounded wait as the
// fixture, so it cannot bring the race back, and so does the removal of the directory.
async function startLeader(t, root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"))) {
  const leader = spawnGroup(process.execPath, ["-e", LEADER, root], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(() => cleanUpFixture({ root, groups: [{ name: "leader", pid: leader.pid }] }));
  const { helper, crashpad } = JSON.parse(await firstLine(leader));
  await until(() => existsSync(join(root, "chrome", "profile", "file-100")), "the helpers never started writing");
  return { root, leader, helper, crashpad };
}

test("the fixture directory is removed only after the helpers of a killed leader have exited, Crashpad's session included", async (t) => {
  const { root, leader, helper, crashpad } = await startLeader(t);
  await cleanUpFixture({ root, groups: [{ name: "chrome", pid: leader.pid }] });
  assert.ok(gone(helper), "the helper of the group must be gone when the cleanup returns");
  assert.ok(gone(crashpad), "the helper of its own session must be gone when the cleanup returns");
  assert.ok(!existsSync(root), "the directory must be removed");
  await pause(150);
  assert.ok(!existsSync(root), "nothing writes into the directory after the cleanup");
});

test("the cleanup also stops the helpers when their leader already exited", async (t) => {
  const { root, leader, helper, crashpad } = await startLeader(t);
  leader.kill("SIGKILL");
  await new Promise((resolve) => leader.once("exit", resolve));
  assert.ok(!gone(helper), "the helper outlives its leader");
  await cleanUpFixture({ root, groups: [{ name: "chrome", pid: leader.pid }] });
  assert.ok(gone(helper) && gone(crashpad));
  assert.ok(!existsSync(root));
});

test("groups killed but not yet reaped, and groups already gone, are not an error", async () => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const first = spawnGroup(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
  const second = spawnGroup(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
  const done = spawnGroup(process.execPath, ["-e", ""], { stdio: "ignore" });
  await new Promise((resolve) => done.once("exit", resolve));
  await cleanUpFixture({ root, groups: [first, done, second].map((child, index) => ({ name: `group-${index}`, pid: child.pid })) });
  assert.ok(gone(first.pid) && gone(second.pid));
  assert.ok(!existsSync(root));
});

test("something that names the directory and does not exit makes the cleanup fail clearly after the bound, and the directory stays", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const stuck = spawnGroup(process.execPath, ["-e", "setInterval(() => {}, 1000)", join(root, "chrome", "profile")], { stdio: "ignore" });
  t.after(() => cleanUpFixture({ root, groups: [{ name: "stuck", pid: stuck.pid }] }));
  const started = Date.now();
  await assert.rejects(
    cleanUpFixture({ root, groups: [], stopMs: 2000 }),
    new RegExp(`fixture directory ${root} kept: still running 2000 ms after SIGKILL: process ${stuck.pid} `),
  );
  assert.ok(Date.now() - started < 3000, "the wait is bounded");
  assert.ok(existsSync(root));
});

// The runner of the tests can die without running t.after (crash, SIGKILL, deadline); the guard
// the fixture starts cleans up for it.
const RUNNER = `
const { mkdtempSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { spawn } = require("node:child_process");
const { spawnGroup, startGuard } = require(${JSON.stringify(join(__dirname, "wrapper-chrome-fixture.cjs"))});
const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
const guard = startGuard(root);
const leader = spawnGroup(process.execPath, ["-e", ${JSON.stringify(LEADER)}, root], { stdio: ["ignore", "pipe", "ignore"] }, (pid) => guard.register("chrome", pid));
leader.stdout.once("data", (chunk) => {
  console.log(JSON.stringify({ root, leader: leader.pid, ...JSON.parse(String(chunk)) }));
});
setInterval(() => {}, 1000);
`;

test("when the test runner is killed, the guard stops every helper and removes the directory", async (t) => {
  const runner = spawn(process.execPath, ["-e", RUNNER], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(() => { try { runner.kill("SIGKILL"); } catch { /* already gone */ } });
  const { root, leader, helper, crashpad } = JSON.parse(await firstLine(runner));
  t.after(() => cleanUpFixture({ root, groups: [{ name: "leader", pid: leader }] }));
  await until(() => existsSync(join(root, "chrome", "profile", "file-100")), "the helpers never started writing");
  assert.ok(!gone(leader) && !gone(helper) && !gone(crashpad));
  runner.kill("SIGKILL");
  await new Promise((resolve) => runner.once("exit", resolve));
  await until(() => gone(leader) && gone(helper) && gone(crashpad) && !existsSync(root), "the guard did not clean up after the runner died");
  await pause(150);
  assert.ok(!existsSync(root), "nothing writes into the directory after the guard's cleanup");
});

test("after \"done\" the guard leaves what it was given alone", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const group = spawnGroup(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
  t.after(() => cleanUpFixture({ root, groups: [{ name: "group", pid: group.pid }] }));
  const guard = startGuard(root);
  guard.register("group", group.pid);
  guard.release();
  await pause(300);
  assert.ok(!gone(group.pid), "a released guard must not kill the group");
  assert.ok(existsSync(root));
});

// The first line `child` prints, or a failure after `ms`, so a regression cannot hang the test.
function lineWithin(child, ms = 15000) {
  return beforeDeadline(firstLine(child), Date.now() + ms, `no line from process ${child.pid} within ${ms} ms`);
}

// Runs cleanUpFixture({ root, groups, stopMs }) in a process of its own group, with `env`, and
// resolves with { ok, message, ms }. A cleanup that signalled its own group kills only that process.
const SANDBOX = `
const { cleanUpFixture } = require(${JSON.stringify(FIXTURE)});
const [root, groups, stopMs] = [process.argv[1], JSON.parse(process.argv[2]), Number(process.argv[3])];
const started = Date.now();
cleanUpFixture({ root, groups, stopMs }).then(() => ({ ok: true }), (error) => ({ ok: false, message: error.message }))
  .then((result) => { console.log(JSON.stringify({ ...result, ms: Date.now() - started })); process.exit(0); });
`;

async function cleanUpInSandbox(t, { root, groups, stopMs, env = process.env }) {
  const sandbox = spawnGroup(process.execPath, ["-e", SANDBOX, root, JSON.stringify(groups), String(stopMs)], { stdio: ["ignore", "pipe", "ignore"], env });
  t.after(() => cleanUpFixture({ root, groups: [{ name: "sandbox", pid: sandbox.pid }] }));
  return { sandbox, ...JSON.parse(await lineWithin(sandbox, 20000)) };
}

function groupGone(pid) {
  try {
    process.kill(-pid, 0);
    return false;
  } catch (error) {
    return error.code === "ESRCH";
  }
}

test("a ps slower than the bound makes the cleanup fail at the bound, is killed, and the directory stays", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const bin = join(root, "bin");
  mkdirSync(bin);
  // Answers like a ps that finds nothing, but only after 5 s.
  writeFileSync(join(bin, "ps"), "#!/bin/sh\nexec /bin/sleep 5\n");
  chmodSync(join(bin, "ps"), 0o755);
  const result = await cleanUpInSandbox(t, { root, groups: [], stopMs: 1000, env: { ...process.env, PATH: `${bin}:${process.env.PATH}` } });
  assert.equal(result.ok, false, "a cleanup whose check outlasted the bound must not succeed");
  assert.match(result.message, new RegExp(`^fixture directory ${root} kept: could not check within 1000 ms that the processes exited: ps did not answer within \\d+ ms$`));
  assert.ok(result.ms < 4000, `the bound holds while ps is slow (${result.ms} ms)`);
  assert.ok(existsSync(root));
  // The slow ps is in the group of the sandbox, which exits right after printing its result.
  await until(() => groupGone(result.sandbox.pid), "the slow ps must be killed at the bound", 2000);
});

test("a process that names a neighbouring directory is not waited for", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const neighbour = `${root}-other`;
  mkdirSync(neighbour);
  const other = spawnGroup(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)", join(neighbour, "chrome", "profile"), `/elsewhere${root}/chrome`], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(() => cleanUpFixture({ root: neighbour, groups: [{ name: "neighbour", pid: other.pid }] }));
  assert.equal(await lineWithin(other), "up");
  const started = Date.now();
  await cleanUpFixture({ root, groups: [], stopMs: 3000 });
  assert.ok(Date.now() - started < 2000, "the neighbour must not hold the cleanup until the bound");
  assert.ok(!existsSync(root));
  assert.ok(!gone(other.pid) && existsSync(neighbour), "the neighbour and its directory are left alone");
});

test("a process that names the directory through its real path is waited for", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const stuck = spawnGroup(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)", `profile=${realpathSync(root)}/chrome/profile`], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(() => cleanUpFixture({ root, groups: [{ name: "stuck", pid: stuck.pid }] }));
  assert.equal(await lineWithin(stuck), "up");
  await assert.rejects(cleanUpFixture({ root, groups: [], stopMs: 2000 }), new RegExp(`still running 2000 ms after SIGKILL: process ${stuck.pid} `));
  assert.ok(existsSync(root));
});

// A parent that never reaps (like PID 1 in some containers) keeps an exited child as a zombie:
// kill(-group, 0) still finds the group (EPERM on macOS), though nothing in it can write any more.
const NEVER_REAPS = '$| = 1; my $pid = fork(); if ($pid == 0) { setpgrp(0, 0); exit 0 } print "$pid\\n"; sleep 60';

test("a group that only holds a zombie counts as gone", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const parent = spawnGroup("perl", ["-e", NEVER_REAPS], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(() => cleanUpFixture({ root, groups: [{ name: "parent", pid: parent.pid }] }));
  const zombie = Number(await lineWithin(parent));
  const row = () => execFileSync("ps", ["-o", "pgid=,stat=", "-p", String(zombie)], { encoding: "utf8", timeout: 5000 }).trim().split(/\s+/);
  await until(() => row()[1]?.startsWith("Z"), "the child never became a zombie");
  assert.equal(Number(row()[0]), zombie, "the unreaped child still owns its group");
  try {
    assert.equal(process.kill(-zombie, 0), true, "signal 0 still finds the zombie group");
  } catch (error) {
    assert.equal(error.code, "EPERM", "only permission denial also establishes that the group exists");
  }
  await cleanUpFixture({ root, groups: [{ name: "zombie", pid: zombie }], stopMs: 3000 });
  assert.ok(!existsSync(root));
});

test("only a group the fixture could have started is ever signalled", () => {
  for (const pid of [0, 1, -1, -42, 1.5, NaN, Infinity, undefined, null, "123", process.pid]) {
    assert.throws(() => checkGroupPid(pid), /refusing to signal process group/, String(pid));
  }
  const child = spawn(process.execPath, ["-e", ""], { stdio: "ignore" });
  assert.equal(checkGroupPid(child.pid), child.pid);
});

test("a group that is refused does not stop the cleanup of the next ones, and the cleanup fails clearly", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const next = spawnGroup(process.execPath, ["-e", "console.log('up'); setInterval(() => {}, 1000)"], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(() => cleanUpFixture({ root, groups: [{ name: "next", pid: next.pid }] }));
  assert.equal(await lineWithin(next), "up");
  const result = await cleanUpInSandbox(t, { root, groups: [{ name: "bad", pid: 0 }, { name: "next", pid: next.pid }], stopMs: 5000 });
  assert.equal(result.ok, false);
  assert.match(result.message, /kept: not signalled: bad: refusing to signal process group 0/);
  await until(() => gone(next.pid), "the group after the refused one must be stopped");
  assert.ok(existsSync(root));
});

test("a directory that is not a fixture directory inside the temporary directory is never removed", async (t) => {
  const parent = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  t.after(() => cleanUpFixture({ root: parent, groups: [] }));
  const nested = join(parent, "nested");
  mkdirSync(nested);
  await assert.rejects(cleanUpFixture({ root: nested, groups: [] }), /refusing .*nested: not a fixture directory inside/);
  assert.ok(existsSync(nested));
});

// The runner dies after starting a child and before the guard knows it.
const EARLY_DEATH = `
const { writeSync } = require("node:fs");
const { spawnGroup, startGuard } = require(${JSON.stringify(FIXTURE)});
const [root, marker] = process.argv.slice(1);
startGuard(root);
spawnGroup(process.execPath, ["-e", "require('node:fs').writeFileSync(process.argv[1], 'ran')", marker], { stdio: "ignore" }, (pid) => {
  writeSync(1, pid + "\\n");
  process.kill(process.pid, "SIGKILL");
});
setInterval(() => {}, 1000);
`;

test("a runner that dies between starting a child and registering it leaves nothing running", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const elsewhere = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const marker = join(elsewhere, "ran");
  const runner = spawn(process.execPath, ["-e", EARLY_DEATH, root, marker], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(() => { runner.kill("SIGKILL"); });
  const child = Number(await lineWithin(runner));
  t.after(() => cleanUpFixture({ root: elsewhere, groups: [{ name: "held", pid: child }] }));
  t.after(() => cleanUpFixture({ root, groups: [] }));
  await until(() => runner.exitCode !== null || runner.signalCode !== null, "the runner did not die");
  await until(() => gone(child), "the unregistered child must not survive its runner");
  await pause(150);
  assert.ok(!existsSync(marker), "the command of an unregistered child never runs");
  await until(() => !existsSync(root), "the guard still removes the directory");
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


test("a rejected guard registration never releases a useful child", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const child = spawnGroup(process.execPath, ["-e", "console.log('unexpected launch')"], {
    stdio: ["ignore", "pipe", "ignore"],
  }, () => Promise.reject(Object.assign(new Error("registration failed"), { code: "EPIPE" })));
  let output = "";
  child.stdout.on("data", (chunk) => { output += chunk; });
  const closed = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code) => resolve(code));
  });
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    await beforeDeadline(closed, Date.now() + 3000, "owned launcher did not exit during teardown");
    await cleanUpFixture({ root, groups: [] });
  });
  assert.equal(await beforeDeadline(closed, Date.now() + 3000, "unregistered launcher did not exit"), 1);
  assert.equal(output, "", "the held command never executes after rejected registration");
});


test("registering on a closed guard rejects its failed write", async (t) => {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  t.after(() => cleanUpFixture({ root, groups: [] }));
  const guard = startGuard(root);
  guard.release();
  await assert.rejects(guard.register("closed", process.pid), { code: "ERR_STREAM_WRITE_AFTER_END" });
});
