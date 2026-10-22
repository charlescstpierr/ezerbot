import { test } from "node:test";
import assert from "node:assert/strict";
import { screenActionState, screenWarnings, screenDecisionLabel } from "./screen-view.js";
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
test("queued calls are never called authorised before their own human decision", () => {
  for (const run of ["awaiting_approval", "running", "queued", undefined]) {
    assert.equal(screenActionState({approval_status:null,run_status:run}).hidden,true);
    assert.equal(screenActionState({run_status:run}).hidden,true);
  }
  for (const run of ["cancelled","failed","finished","ended","interrupted"]) {
    assert.equal(screenActionState({approval_status:null,run_status:run}).label,"Arrêtée");
    assert.equal(screenActionState({approval_status:"resolved",run_status:run}).label,"Arrêtée");
  }
  assert.equal(screenDecisionLabel(null),null);
  assert.equal(screenDecisionLabel("pending"),null);
  assert.equal(screenDecisionLabel("resolved"),"Autorisé une fois");
  assert.equal(screenDecisionLabel("denied"),"Refusée par vous");
  assert.equal(screenDecisionLabel("cancelled"),"Approbation annulée");
});
test("passive screen actions do not claim to buy, send or delete", () => {
  for (const name of ["computer_screenshot", "computer_move", "computer_scroll"]) assert.doesNotMatch(screenWarnings[name], /achat|irréversible|supprimer/);
  assert.match(screenWarnings.computer_click, /irréversible/);
  assert.match(screenWarnings.computer_type, /mot de passe/);
});
