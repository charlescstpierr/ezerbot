import assert from "node:assert/strict";
import { test } from "node:test";
import { runView } from "./run-status.js";

test("a live status wins, except the ended sentinel", () => {
  assert.deepEqual(runView("running", "interrupted"), { status: "interrupted", stopped: true });
  assert.deepEqual(runView("running", "ended"), { status: "running", stopped: false });
  assert.deepEqual(runView(undefined, "ended"), { status: "ended", stopped: true });
  assert.deepEqual(runView("interrupted", undefined), { status: "interrupted", stopped: true });
});

test("only a finished run is stopped", () => {
  for (const status of ["interrupted", "cancelled", "failed", "finished", "ended"]) {
    assert.equal(runView(status).stopped, true);
  }
  for (const status of ["running", "awaiting_approval", undefined]) {
    assert.equal(runView(status).stopped, false);
  }
});
