// The cleanup of wrapper-chrome-fixture (#490): the fixture directory is removed only after the
// wrapper and Chrome, helper processes included, have really exited. Chrome's helpers outlive a
// SIGKILL of the browser process and keep writing into its profile, so the stand-in here is a
// leader with a helper (same process group, as Chrome's) that writes into the directory without
// pause and recreates it if it disappears. Nothing here starts a browser.
const test = require("node:test");
const assert = require("node:assert/strict");
const { existsSync, mkdtempSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const { join } = require("node:path");
const { cleanUpFixture, spawnGroup, waitForGroupExit } = require("./wrapper-chrome-fixture.cjs");

const HELPER = `
const { mkdirSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");
const root = process.argv[1];
let n = 0;
setInterval(() => {
  for (let i = 0; i < 50; i++) {
    mkdirSync(join(root, "profile"), { recursive: true });
    writeFileSync(join(root, "profile", "file-" + n++), "x");
  }
}, 1);
`;

const LEADER = `
const { spawn } = require("node:child_process");
const helper = spawn(process.execPath, ["-e", ${JSON.stringify(HELPER)}, process.argv[1]], { stdio: "ignore" });
console.log(helper.pid);
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

async function untilWriting(root, helperPid) {
  const deadline = Date.now() + 10000;
  while (!existsSync(join(root, "profile", "file-100"))) {
    assert.ok(Date.now() < deadline, "the helper never started writing");
    assert.ok(!gone(helperPid), "the helper ended on its own");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

// Starts the leader and its writing helper in a group of their own; the group is killed at the end of the
// test whatever happens, and so is the directory.
async function startLeaderWithHelper(t) {
  const root = mkdtempSync(join(tmpdir(), "fixture-cleanup-test-"));
  const leader = spawnGroup(process.execPath, ["-e", LEADER, root], { stdio: ["ignore", "pipe", "ignore"] });
  t.after(() => {
    try { process.kill(-leader.pid, "SIGKILL"); } catch { /* already gone */ }
    rmSync(root, { recursive: true, force: true });
  });
  const helperPid = await new Promise((resolve, reject) => {
    leader.stdout.once("data", (chunk) => resolve(Number(String(chunk).trim())));
    leader.once("error", reject);
    leader.once("exit", () => reject(new Error("the leader ended before printing the helper pid")));
  });
  await untilWriting(root, helperPid);
  return { root, leader, helperPid };
}

test("the fixture directory is removed only after the helper process of a killed leader has exited", async (t) => {
  const { root, leader, helperPid } = await startLeaderWithHelper(t);
  await cleanUpFixture({ root, groups: [{ name: "chrome", child: leader }] });
  assert.ok(gone(helperPid), "the helper must be gone when the cleanup returns");
  assert.ok(!existsSync(root), "the directory must be removed");
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.ok(!existsSync(root), "nothing writes into the directory after the cleanup");
});

test("the cleanup also stops a helper whose leader already exited", async (t) => {
  const { root, leader, helperPid } = await startLeaderWithHelper(t);
  leader.kill("SIGKILL");
  await new Promise((resolve) => leader.once("exit", resolve));
  assert.ok(!gone(helperPid), "the helper outlives its leader");
  await cleanUpFixture({ root, groups: [{ name: "chrome", child: leader }] });
  assert.ok(gone(helperPid));
  assert.ok(!existsSync(root));
});

test("waiting for a group that does not exit fails clearly after the bound", async (t) => {
  const { leader } = await startLeaderWithHelper(t);
  await assert.rejects(
    waitForGroupExit("chrome", leader.pid, 100),
    /chrome \(process group \d+\) is still running 100 ms after SIGKILL/,
  );
});

test("a group that is already gone is not an error", async () => {
  const leader = spawnGroup(process.execPath, ["-e", ""], { stdio: "ignore" });
  await new Promise((resolve) => leader.once("exit", resolve));
  await waitForGroupExit("wrapper", leader.pid, 1000);
});
