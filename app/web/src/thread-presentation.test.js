import assert from "node:assert/strict";
import { test } from "node:test";
import { presentThread } from "./thread-presentation.js";

const engineReasons = [
  "Résultat inconnu après interruption. Action non relancée ; vérifiez son effet avant de réessayer.",
  "Appel externe interrompu ; son effet peut déjà avoir eu lieu.",
  "L’appel externe a été interrompu après son autorisation. Son effet est incertain ; vérifiez avant de réessayer.",
];
const tool = (payload) => ({ id: "run:3", role: "assistant", kind: "tool", payload: { run_id: "run", call: { call_id: "call", name: "run_shell", arguments: { command: "printf trace" } }, ...payload } });
const task = (id, payload) => ({ id, role: "assistant", kind: "task", body: "", payload: { run_id: "run", status: "finished", routine: true, task_status: { status: "working" }, ...payload } });
const reply = { id: "reply", role: "assistant", kind: "message", body: "Je consulte le dossier.", state: "echoed", payload: { run_id: "run" } };
const notes = (messages) => presentThread(messages).messages.filter((message) => message.presentation).map((message) => message.presentation.text);

test("une action au résultat inconnu reste inconnue, sans trace d’outil", () => {
  for (const reason of engineReasons) {
    const [note, ...others] = notes([tool({ code: "uncertain", outcome: { status: "failed", reason } })]);
    assert.deepEqual(others, []);
    assert.match(note, /résultat est inconnu/);
    assert.match(note, /Vérifiez son effet avant de réessayer/);
    assert.doesNotMatch(note, /n’a pas abouti|run_shell|printf|Commande/);
  }
});

test("le résultat inconnu passe avant un autre libellé de l’outil ou de l’approbation", () => {
  const outcome = { status: "failed", reason: engineReasons[0] };
  const [image] = notes([tool({ code: "uncertain", call: { name: "generate_image" }, outcome })]);
  assert.doesNotMatch(image, /n’a pas pu être créée/);
  const [approved] = notes([tool({ code: "uncertain", approval_status: "cancelled", outcome })]);
  assert.doesNotMatch(approved, /Aucune action n’a été faite/);
});

test("un échec établi reste un échec et un résultat établi reste silencieux", () => {
  assert.deepEqual(notes([tool({ code: "failed", outcome: { status: "failed", reason: "Commande introuvable." } })]), ["La commande n’a pas abouti."]);
  assert.deepEqual(notes([tool({ outcome: { status: "failed", reason: "Fin confirmée après coup." } })]), ["La commande n’a pas abouti."]);
  assert.deepEqual(notes([tool({ code: "ok", outcome: { status: "done", output: { status: "terminé" } } })]), []);
});

test("une routine arrêtée par le budget garde son avertissement dans le fil", () => {
  const [note, ...others] = notes([task("run:9", { event_kind: "turn_ended", turn_end: "budget_exhausted" })]);
  assert.deepEqual(others, []);
  assert.match(note, /budget de ce tour est épuisé/);
  assert.match(note, /tâche n’est pas terminée/);
});

test("l’avertissement de budget arrive une fois, après les dernières phrases du bot", () => {
  const stopped = { turn_end: "budget_exhausted", task_status: { status: "needs_input" } };
  const { messages } = presentThread([task("run:2", { ...stopped, event_kind: "task_changed" }), reply, task("run:9", { ...stopped, event_kind: "turn_ended" })]);
  assert.deepEqual(messages.map((message) => message.id), ["reply", "run:9"]);
  assert.equal(messages[1].presentation.kind, "problem");
});

test("seul un tour épuisé sur une tâche ouverte avertit", () => {
  assert.deepEqual(notes([task("run:9", { event_kind: "turn_ended", turn_end: "finished" })]), []);
  const done = presentThread([task("run:9", { event_kind: "turn_ended", turn_end: "budget_exhausted", task_status: { status: "done", summary: "Rapport envoyé." } })]).messages;
  assert.deepEqual(done.map((message) => [message.presentation.kind, message.presentation.text]), [["completion", "Rapport envoyé."]]);
});
