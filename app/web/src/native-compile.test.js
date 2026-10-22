import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

test("the Cocoa shell compiles on macOS", { skip: process.platform !== "darwin" }, () => {
  const result = spawnSync("bash", [fileURLToPath(new URL("../../../tools/compile-native.sh", import.meta.url))], { encoding: "utf8" });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  assert.match(result.stdout, /Coque Cocoa compilée/);
});
