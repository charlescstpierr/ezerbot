import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { once } from "node:events";
import { test } from "node:test";

const native = new URL("../../native/", import.meta.url);
const shell = readFileSync(new URL("main.m", native), "utf8");
const method = (signature) => {
  const start = shell.indexOf(signature);
  assert.notEqual(start, -1, signature);
  const end = shell.indexOf("\n}\n", start);
  assert.notEqual(end, -1, signature);
  return shell.slice(start, end);
};

test("SIGTERM and SIGINT quit through the same stop as Cmd+Q", () => {
  const mainStart = shell.indexOf("int main(");
  const run = shell.indexOf("[application run]", mainStart);
  const arm = shell.indexOf("shell_stop_arm", mainStart);
  assert.ok(arm > mainStart && arm < run, "main arms quit signals before the run loop");
  assert.match(shell.slice(mainStart, run), /shell_stop_take_quit/);
  assert.match(shell.slice(mainStart, run), /\[NSApp terminate:nil\]/);
  assert.doesNotMatch(shell, /SIG_IGN/);

  const stop = method("- (void)stopBackend");
  assert.match(stop, /\[self\.voice cancel\]/);
  assert.match(stop, /\[self\.voiceSession cancel\]/);
  assert.match(stop, /\[self\.floating invalidate\]/);
  assert.match(stop, /shell_stop_child/);
  assert.doesNotMatch(stop, /\[task terminate\]/);
  assert.doesNotMatch(stop, /\bkill\s*\(/);

  const quit = method("- (void)applicationWillTerminate:");
  const terminating = quit.indexOf("self.terminating = YES");
  const generation = quit.indexOf("++self.startupGeneration");
  const stopped = quit.indexOf("[self stopBackend]");
  assert.ok(terminating !== -1 && terminating < generation && generation < stopped);

  assert.match(shell, /action:@selector\(terminate:\) keyEquivalent:@"q"/);
  assert.match(method("- (BOOL)windowShouldClose:"), /\[sender orderOut:nil\]; return NO/);
  assert.match(method("- (BOOL)applicationShouldTerminateAfterLastWindowClosed:"), /return NO/);
});

const compile = () => {
  const binary = join(mkdtempSync(join(tmpdir(), "ezerbot-shell-stop-")), "shell-stop");
  execFileSync("cc", [
    "-Wall", "-Wextra", "-Werror",
    "-I", new URL(".", native).pathname,
    new URL("shell_stop_driver.c", native).pathname,
    "-o", binary,
  ]);
  return binary;
};

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    assert.equal(error.code, "ESRCH");
    return false;
  }
};

const quit = async (signal, mode = []) => {
  const parent = spawn(compile(), mode, { stdio: ["ignore", "pipe", "inherit"] });
  let out = "";
  parent.stdout.on("data", (chunk) => { out += chunk; });
  const started = Date.now();
  while (!/ready \d+\n/.test(out)) await once(parent.stdout, "data");
  const child = Number(out.match(/ready (\d+)/)[1]);
  parent.kill(signal);
  const [code, died] = await once(parent, "exit");
  const orphan = alive(child);
  if (orphan) process.kill(child, "SIGKILL");
  return { code, died, orphan, stopped: out.match(/stopped (\w+)/)?.[1], ms: Date.now() - started };
};

test("SIGTERM stops the engine child and the shell exits 0", { timeout: 5000 }, async () => {
  const result = await quit("SIGTERM");
  assert.equal(result.died, null);
  assert.equal(result.code, 0);
  assert.equal(result.orphan, false);
  assert.equal(result.stopped, "term");
  assert.ok(result.ms < 800, `polite stop took ${result.ms} ms`);
});

test("SIGINT stops the engine child and the shell exits 0", { timeout: 5000 }, async () => {
  const result = await quit("SIGINT");
  assert.equal(result.died, null);
  assert.equal(result.code, 0);
  assert.equal(result.orphan, false);
  assert.equal(result.stopped, "term");
  assert.ok(result.ms < 800, `polite stop took ${result.ms} ms`);
});

test("a second SIGTERM is still one polite stop", { timeout: 5000 }, async () => {
  const parent = spawn(compile(), [], { stdio: ["ignore", "pipe", "inherit"] });
  let out = "";
  parent.stdout.on("data", (chunk) => { out += chunk; });
  while (!/ready \d+\n/.test(out)) await once(parent.stdout, "data");
  const child = Number(out.match(/ready (\d+)/)[1]);
  parent.kill("SIGTERM");
  parent.kill("SIGTERM");
  const [code, died] = await once(parent, "exit");
  const orphan = alive(child);
  if (orphan) process.kill(child, "SIGKILL");
  assert.equal(died, null);
  assert.equal(code, 0);
  assert.equal(orphan, false);
  assert.equal(out.match(/stopped (\w+)/)?.[1], "term");
});

test("a child that ignores SIGTERM is reaped with SIGKILL inside the one second bound", { timeout: 8000 }, async () => {
  const result = await quit("SIGTERM", ["ignore"]);
  assert.equal(result.died, null);
  assert.equal(result.code, 0);
  assert.equal(result.orphan, false);
  assert.equal(result.stopped, "kill");
  assert.ok(result.ms >= 900, `fallback returned in ${result.ms} ms`);
  assert.ok(result.ms < 4000, `fallback took ${result.ms} ms`);
});
