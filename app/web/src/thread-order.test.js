import assert from "node:assert/strict";
import { test } from "node:test";
import { orderThread } from "./thread-order.js";

const user = { id: "question", role: "user", body: "Question" };
const tool = { id: "run:3", kind: "tool", payload: { run_id: "run" } };
const reply = (state, parts, extra = {}) => ({
  id: "reply-question", role: "assistant", kind: "message", state,
  body: parts.map((part) => part.body).join(""),
  payload: { run_id: "run", parts }, ...extra,
});
const preCall = { body: "Avant.", before: tool.id };

for (const state of ["streaming", "failed", "stopped"]) {
  test(`${state} after pre-call text belongs below the call`, () => {
    const result = orderThread([user, reply(state, [preCall], { error: "Erreur" }), tool]);
    assert.deepEqual(result.map((m) => m.body ?? m.id), ["Question", "Avant.", tool.id, ""]);
    assert.equal(result[1].state, "echoed");
    assert.equal(result[1].error, undefined);
    assert.equal(result.at(-1).state, state);
    assert.equal(result.at(-1).error, "Erreur");
    assert.equal(result.at(-1).id, "reply-question"); // Stop/Retry transport identity
  });
}

test("notice after a call is a tail fragment too", () => {
  const result = orderThread([user, reply("echoed", [preCall], { notice: "Attention" }), tool]);
  assert.equal(result[1].notice, undefined);
  assert.equal(result.at(-1).notice, "Attention");
  assert.equal(result.at(-1).body, "");
});

test("post-call text already carries the status without a redundant tail", () => {
  const result = orderThread([user, reply("failed", [preCall, { body: "Après." }]), tool]);
  assert.equal(result.length, 4);
  assert.equal(result.at(-1).body, "Après.");
  assert.equal(result.at(-1).state, "failed");
});

test("copy exists only on the final fragment and contains the full response", () => {
  for (const state of ["echoed", "failed", "stopped"]) {
    const message = reply(state, [preCall, { body: "Après." }]);
    const fragments = orderThread([user, message, tool]).filter((m) => m.key);
    assert.deepEqual(fragments.map((m) => m.copyText), [null, "Avant.Après."]);
  }
  const message = reply("failed", [preCall]);
  assert.equal(orderThread([user, message, tool]).at(-1).copyText, "Avant.");
});

test("only the first fragment has an assistant heading/avatar", () => {
  const fragments = orderThread([user, reply("streaming", [preCall]), tool]).filter((m) => m.key);
  assert.deepEqual(fragments.map((m) => m.continuation), [false, true]);
});

test("existing text keeps its React key when its call and status arrive", () => {
  const before = orderThread([user, reply("streaming", [{ body: "Avant." }])]);
  const after = orderThread([user, reply("streaming", [preCall]), tool]);
  assert.equal(before[1].key, after[1].key);
  assert.equal(after[1].key, "reply-question#0");
  assert.notEqual(after[1].id, after.at(-1).id);
});

test("tool-only completion stays empty; empty live/error/stop statuses survive", () => {
  assert.deepEqual(orderThread([user, reply("echoed", []), tool]), [user, tool]);
  for (const state of ["streaming", "failed", "stopped"]) {
    assert.equal(orderThread([user, reply(state, []), tool]).at(-1).state, state);
  }
});

test("task/delegation/form cards stay before the tail without moving another turn", () => {
  const cards = [tool,
    { id: "delegation", kind: "delegation", payload: { parent_run_id: "run" } },
    { id: "form", kind: "form", payload: { run_id: "run" } },
    { id: "task", kind: "task", payload: { run_id: "run" } },
  ];
  const other = { id: "other", role: "user", body: "Second turn" };
  const unrelated = { id: "secret", kind: "secret", payload: { run_id: "child" } };
  const result = orderThread([unrelated, user, reply("failed", [preCall]), ...cards, other]);
  assert.equal(result[0], unrelated);
  assert.deepEqual(result.slice(3, 7), cards);
  assert.equal(result.at(-2).state, "failed");
  assert.equal(result.at(-1), other);
});
