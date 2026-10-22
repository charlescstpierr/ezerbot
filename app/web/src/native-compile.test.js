import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

test("macOS CI compiles the Cocoa shell", () => {
  const workflow = readFileSync(new URL("../../../.github/workflows/macos-shell.yml", import.meta.url), "utf8");
  assert.match(workflow, /runs-on:\s*macos-/);
  assert.match(workflow, /tools\/compile-native\.sh/);
  const script = readFileSync(new URL("../../../tools/compile-native.sh", import.meta.url), "utf8");
  assert.match(script, /clang/);
  assert.match(script, /-Werror/);
  assert.match(script, /-c /);
  assert.match(script, /app\/native\/main\.m/);
  assert.match(script, /Darwin/);
});

test("the Cocoa shell compiles on macOS", { skip: process.platform !== "darwin" }, () => {
  const result = spawnSync("bash", [fileURLToPath(new URL("../../../tools/compile-native.sh", import.meta.url))], { encoding: "utf8" });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  assert.match(result.stdout, /Coque Cocoa compilée/);
});
