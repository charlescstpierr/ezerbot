import { test } from "node:test";
import assert from "node:assert/strict";
import { screenActionState, screenWarnings } from "./screen-view.js";
test("screen journal does not duplicate a pending decision and distinguishes its outcomes", () => {
  assert.equal(screenActionState({ approval_status: "pending" }).hidden, true);
  assert.equal(screenActionState({ approval_status: "resolved", run_status: "running" }).label, "En cours…");
  assert.equal(screenActionState({ outcome: { status: "done" } }).label, "Terminée");
  assert.equal(screenActionState({ approval_status: "denied" }).label, "Refusée");
  assert.equal(screenActionState({ approval_status: "cancelled" }).reason, "Session arrêtée — action non exécutée.");
  assert.equal(screenActionState({ approval_status: "expired" }).label, "Délai écoulé");
  assert.equal(screenActionState({ outcome: { status: "failed", reason: "Permission Accessibilité manquante." } }).label, "Permission manquante");
  assert.equal(screenActionState({ outcome: { status: "failed", reason: "Contrôle arrêté, expiré ou repris par vous." } }).label, "Arrêtée");
});
test("passive screen actions do not claim to buy, send or delete", () => {
  for (const name of ["computer_screenshot", "computer_move", "computer_scroll"]) assert.doesNotMatch(screenWarnings[name], /achat|irréversible|supprimer/);
  assert.match(screenWarnings.computer_click, /irréversible/);
  assert.match(screenWarnings.computer_type, /mot de passe/);
});
