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

test("usage appears once on the last fragment, including tool-only replies", () => {
  const usage = { input_tokens: 12340, output_tokens: 100, cached_input_tokens: 11008 };
  for (const parts of [[], [preCall], [preCall, { body: "Après." }]]) {
    const message = reply("echoed", parts);
    message.payload.usage = usage;
    const fragments = orderThread([user, message, tool]).filter((m) => m.key);
    assert.equal(fragments.filter((m) => m.payload.usage).length, 1);
    assert.equal(fragments.at(-1).payload.usage, usage);
    assert.equal(fragments.at(-1).copyText, message.body);
    if (parts.length) assert.ok(fragments.at(-1).body);
  }
});

for (const routine of [false, true]) {
  test(`${routine ? "routine" : "delegated"} task status after text does not detach usage or copy`, () => {
    const task = { id: "task-end", kind: "task", payload: { run_id: "run", routine } };
    const message = reply("echoed", [{ body: "Résultat de la tâche.", before: task.id }]);
    const usage = { input_tokens: 12340, output_tokens: 100, cached_input_tokens: 11008 };
    message.payload.usage = usage;
    const result = orderThread([message, task]);
    assert.equal(result.length, 2);
    assert.equal(result[0].body, message.body);
    assert.equal(result[0].copyText, message.body);
    assert.equal(result[0].payload.usage, usage);
    assert.equal(result[1], task);
    assert.equal(result.filter((m) => m.key && !m.body).length, 0);
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

test("card-first turn opens with one heading, then tools, then response text", () => {
  const message = reply("echoed", [{ body: "Après." }], { created_at: 1234 });
  const child = { id: "child", kind: "delegation", payload: { parent_run_id: "run" } };
  const result = orderThread([user, message, tool, child]);
  assert.deepEqual(result.map((m) => m.kind || m.id), [user.id, "turn-heading", "tool", "delegation", "message"]);
  assert.equal(result[1].created_at, message.created_at);
  assert.equal(result[1].id, `${message.id}:heading`);
  assert.equal(result.at(-1).continuation, true);
  assert.equal(result.at(-1).body, "Après.");
  assert.equal(result.at(-1).copyText, message.body);
});

test("heading opens card-first live, error and stopped turns without duplicating status", () => {
  for (const state of ["streaming", "failed", "stopped"]) {
    const result = orderThread([user, reply(state, [], { error: "Erreur" }), tool]);
    assert.equal(result[1].kind, "turn-heading");
    assert.equal(result[2], tool);
    assert.equal(result[3].state, state);
    assert.equal(result[3].continuation, true);
  }
});

test("pre-call text keeps the heading and tool-only completions never get an empty heading", () => {
  assert.equal(orderThread([user, reply("echoed", [preCall]), tool]).some((m) => m.kind === "turn-heading"), false);
  const internal = { id: "complete", kind: "tool", payload: { run_id: "run", call: { name: "complete_task" } } };
  const task = { id: "task", kind: "task", payload: { run_id: "run" } };
  assert.deepEqual(orderThread([user, reply("echoed", []), internal, task]), [user, task]);
});

test("each turn's heading stays with its own cards, not an unrelated routed request", () => {
  const unrelated = { id: "routed", kind: "secret", payload: { run_id: "another-run" } };
  const nextUser = { ...user, id: "next" };
  const nextReply = { ...reply("echoed", [{ body: "Second." }]), id: "second", payload: { run_id: "second-run", parts: [{ body: "Second." }] } };
  const nextTool = { ...tool, id: "second-tool", payload: { run_id: "second-run" } };
  const result = orderThread([unrelated, user, reply("echoed", [{ body: "First." }]), tool, nextUser, nextReply, nextTool]);
  assert.deepEqual(result.map((m) => m.id), ["routed", "question", "reply-question:heading", tool.id, "reply-question", "next", "second:heading", "second-tool", "second"]);
});

for (const kind of ["form", "secret"]) {
  const request = { id: "input", kind, payload: { id: "input", run_id: "run", status: { status: "pending" } } };
  const call = { ...tool, payload: { run_id: "run", call: { name: `request_${kind}` }, outcome: { status: "suspend", cause: { kind: "human_input", id: "input" } } } };
  test(`${kind} request replaces only its matching tool, including after resolution`, () => {
    for (const status of ["pending", "resolved", "cancelled"]) {
      const card = { ...request, payload: { ...request.payload, status: { status } } };
      assert.deepEqual(orderThread([user, reply("echoed", []), call, card]), [user, card]);
      const result = orderThread([user, reply("echoed", [{ body: "Avant.", before: call.id }, { body: "Après." }]), call, card]);
      assert.deepEqual(result.map((m) => m.body ?? m.id), ["Question", "Avant.", card.id, "Après."]);
    }
  });
  test(`${kind} missing/routed request, unrelated run and failed calls keep their diagnostic card`, () => {
    assert.ok(orderThread([call]).includes(call));
    assert.ok(orderThread([call, { ...request, payload: { ...request.payload, run_id: "child" } }]).includes(call));
    assert.ok(orderThread([call, { ...request, payload: { ...request.payload, id: "other-input" } }]).includes(call));
    const failed = { ...call, payload: { ...call.payload, outcome: { status: "failed", reason: "Invalid fields" } } };
    assert.ok(orderThread([failed, request]).includes(failed));
  });
}

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
