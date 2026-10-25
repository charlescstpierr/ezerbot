// The cleanup of wrapper-chrome-fixture (#490): the fixture directory is removed only after the
// wrapper and Chrome, helper processes included, have really exited, and whatever happens to the
// test runner. Chrome's helpers outlive a SIGKILL of the browser process and keep writing into its
// profile, and Crashpad leaves the process group (setsid) but still names the profile on its command
// line. The stand-ins here do the same: a leader with a helper in its group that writes into the
// directory without pause (recreating it if it disappears), and a "Crashpad" in a session of its own
// that does the same until shortly after the leader is gone. Nothing here starts a browser.
const test = require("node:test");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const { existsSync, mkdtempSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { cleanUpFixture, spawnGroup, startGuard } = require("./wrapper-chrome-fixture.cjs");

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
  const stuck = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)", join(root, "chrome", "profile")], { stdio: "ignore", detached: true });
  t.after(async () => {
    process.kill(stuck.pid, "SIGKILL");
    await cleanUpFixture({ root, groups: [] });
  });
  const started = Date.now();
  await assert.rejects(
    cleanUpFixture({ root, groups: [], stopMs: 300 }),
    new RegExp(`fixture directory ${root} kept: still running 300 ms after SIGKILL: process ${stuck.pid} `),
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
const leader = spawnGroup(process.execPath, ["-e", ${JSON.stringify(LEADER)}, root], { stdio: ["ignore", "pipe", "ignore"] });
guard.register("chrome", leader.pid);
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
