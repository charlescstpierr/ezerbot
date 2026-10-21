import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement as h } from "react";
import { renderToStaticMarkup as render } from "react-dom/server";
import { documentToolView, documentApprovalEffect } from "./generated-presentation.js";
import { LocalApprovalWarning } from "./local-approval.js";

const call = { name: "create_document", arguments: { content: "" } };
test("document failures are explicit, diagnostic syntax stays separate", () => {
  const failure = documentToolView({ call, outcome: { status: "failed", reason: 'Pour CSV ou Excel, content doit être un tableau JSON.' } });
  assert.equal(failure.title, "Échec de la création du document");
  assert.equal(failure.tone, "danger");
  assert.equal(failure.state, "failed");
  assert.equal(failure.summary, "Le tableau fourni par le modèle est mal formé.");
  assert.match(failure.failure.message, /JSON/);
  assert.equal(documentToolView({ call, outcome: { status: "failed", reason: "Diagramme Mermaid non pris en charge" } }).summary, "Le diagramme fourni par le modèle n’a pas pu être rendu.");
  assert.match(documentToolView({ call, outcome: { status: "failed", reason: "Ce PDF contient « → » (U+2192)." } }).summary, /U\+2192/);
});
test("document action status distinguishes running, refusal, expiry and success", () => {
  const action = { name: "document_action", arguments: { action: "save" } };
  assert.equal(documentToolView({ call: action }, { status: "running" }).label, "En cours…");
  assert.equal(documentToolView({ call: action }, { status: "running" }).state, "running");
  assert.equal(documentToolView({ call: action }, { status: "awaiting_approval" }).label, "Accord requis");
  for (const approval_status of ["denied", "expired", "cancelled"]) {
    const view = documentToolView({ call: action, approval_status, outcome: { status: "failed", reason: "L’action a été refusée." } });
    assert.equal(view.tone, "warning");
    assert.notEqual(view.label, "Erreur");
  }
  assert.equal(documentToolView({ call: action, outcome: { status: "done", output: {} } }).tone, "success");
  assert.equal(documentToolView({ call: action, run_status: "running" }, { status: "ended" }).label, "En cours…");
  assert.equal(documentToolView({ call: action, run_status: "interrupted" }).label, "Arrêtée");
  assert.notEqual(documentToolView({ call: action, run_status: "interrupted" }).state, "running");
});
test("document approvals describe Finder without opening and warn about broad authority", () => {
  assert.match(documentApprovalEffect("reveal"), /sans l’ouvrir/);
  assert.match(documentApprovalEffect("open"), /application par défaut/);
  assert.match(documentApprovalEffect("save"), /Aucun remplacement/);
  assert.match(render(h(LocalApprovalWarning, { tool: "document_action", scope: "global" })), /n’importe où sur votre Mac sans vous demander/);
  assert.equal(render(h(LocalApprovalWarning, { tool: "document_action", scope: "subject" })), "");
});
