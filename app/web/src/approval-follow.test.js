import assert from "node:assert/strict";
import { test } from "node:test";
import { createApprovalFollow } from "./approval-follow.js";
import { TEACH_PROMPT, isTeachMessage, searchableMessageText, exportMessage } from "./teach.js";

const card = (id, status = "pending") => ({ id, kind: "approval", payload: { status: { status } } });

test("Teach uses one exact model-facing prompt and a compact non-searchable marker", () => {
  const message = { role: "user", kind: "submission", body: TEACH_PROMPT };
  assert.equal(isTeachMessage(message), true);
  assert.equal(searchableMessageText(message), "");
  assert.equal(exportMessage(message).body, "Apprendre ceci comme skill");
  assert.equal(exportMessage(message).display, "teach");
  assert.equal(message.body, TEACH_PROMPT); // storage/model input is not rewritten
  assert.ok(TEACH_PROMPT.includes("propose_skill")); // instruction stays model-facing
  for (const ordinary of [{ ...message, body: `Citation : ${TEACH_PROMPT}` }, { ...message, role: "assistant" }, { ...message, payload: { source: "follow_up" } }]) {
    assert.equal(isTeachMessage(ordinary), false);
    assert.equal(searchableMessageText(ordinary), ordinary.body);
    assert.equal(exportMessage(ordinary), ordinary);
  }
});

test("pending approval arriving over several polls is followed after React commits", () => {
  const follow = createApprovalFollow();
  follow.arm("chat", [], true);
  assert.equal(follow.observe("chat", []), null);
  assert.equal(follow.observe("chat", [{ id: "reply", kind: "message" }]), null);
  assert.equal(follow.observe("chat", [card("new")]), "new");
  assert.equal(follow.observe("chat", [card("new")]), "new"); // asynchronous commit retry
  assert.equal(follow.observe("chat", [card("new", "resolved")]), null);
});

test("reading gestures cancel automatic approval scrolling until the next pinned send", () => {
  const follow = createApprovalFollow();
  follow.arm("chat", [], true);
  follow.cancel();
  assert.equal(follow.observe("chat", [card("later")]), null);
  follow.arm("chat", [], false);
  assert.equal(follow.observe("chat", [card("still-later")]), null);
  follow.arm("chat", [], true);
  assert.equal(follow.observe("chat", [card("next-send")]), "next-send");
});

test("existing requests and other conversations never steal focus", () => {
  const follow = createApprovalFollow();
  follow.arm("chat", [card("old")], true);
  assert.equal(follow.observe("other", [card("foreign")]), null);
  assert.equal(follow.observe("chat", [card("old")]), null);
  assert.equal(follow.observe("chat", [card("old"), card("new")]), "new");
});

test("first-message conversation binding preserves a user cancellation", () => {
  const follow = createApprovalFollow();
  follow.arm(null, [], true);
  follow.bind("created");
  assert.equal(follow.observe("created", [card("new")]), "new");
  follow.arm(null, [], true);
  follow.cancel();
  follow.bind("next");
  assert.equal(follow.observe("next", [card("not-followed")]), null);
});
