import test from "node:test";
import assert from "node:assert/strict";
import { pendingReply } from "./thread-order.js";
import { connectionMessages, composerConnectionState } from "./connection-view.js";

test("an authoritative terminal snapshot releases loading without a final stream event", () => {
  const live = [{ role: "assistant", state: "streaming", body: "Début" }];
  assert.equal(pendingReply(connectionMessages(live, true)[0]), true);
  for (const state of ["failed", "echoed", "stopped"]) {
    const snapshot = [{ ...live[0], state }];
    assert.equal(pendingReply(connectionMessages(snapshot, true)[0]), false);
  }
});

test("a dropped local connection replaces direct and group spinners without mutating or replaying the saved turn", () => {
  const saved = [{ role: "assistant", state: "streaming", body: "Début", payload: { run_id: "r", turn_status: "running" } }];
  const offline = connectionMessages(saved, false);
  assert.equal(offline[0].state, "failed");
  assert.equal(offline[0].body, "Début");
  assert.equal(offline[0].error_kind, "local_connection");
  assert.equal(pendingReply(offline[0], true), false);
  assert.equal(saved[0].state, "streaming");
  assert.equal(connectionMessages(saved, true), saved);
  const finished = [{ ...saved[0], state: "echoed", payload: { run_id: "r", turn_status: "finished" } }];
  assert.equal(connectionMessages(finished, true), finished);
  assert.equal(pendingReply(connectionMessages(finished, true)[0], true), false);
});

test("offline recovery preserves saved questions, sign-in cards, human waits and settled replies", () => {
  const messages = [
    { role: "user", state: "unknown", body: "Question conservée" },
    { role: "assistant", state: "failed", error_kind: "signin", error: "Connecte ton compte ChatGPT" },
    { role: "assistant", kind: "form", state: "echoed", payload: { status: "pending" } },
    ...["failed", "stopped", "echoed"].map((state) => ({ role: "assistant", state, body: "Texte conservé" })),
  ];
  const offline = connectionMessages(messages, false);
  for (const [index, message] of messages.entries()) assert.equal(offline[index], message);
  assert.equal(offline[0].body, "Question conservée");
  assert.equal(offline[1].error_kind, "signin");
  assert.equal(offline[2].payload.status, "pending");
});

test("an offline composer is an idle send arrow and recovers its real control on reconnect", () => {
  const live = { live: true, busy: true, group: false };
  const offline = composerConnectionState({ ...live, offline: true });
  assert.equal(offline.mode, "send");
  assert.equal(offline.icon, "#i-send");
  assert.equal(offline.live, false);
  assert.equal(offline.busy, false);
  assert.equal(offline.title, "Moteur local indisponible");
  assert.equal(composerConnectionState(live).mode, "stop");
  assert.equal(composerConnectionState({ live: true, group: true }).title, "Arrêter les tours");
  assert.equal(composerConnectionState({ busy: true }).label, "Envoi en cours");
});
