import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { isDemonstrationMessage, exportMessage, searchableMessageText } from "./teach.js";

test("only host-bound demonstration messages get the compact marker", () => {
  const bound = { role: "user", body: "instruction interne et synthèse", payload: { demonstration_id: "demo" } };
  assert.equal(isDemonstrationMessage(bound), true);
  assert.equal(exportMessage(bound).body, "Démonstration terminée — apprendre la tâche");
  assert.equal(searchableMessageText(bound), "");
  assert.equal(isDemonstrationMessage({ role: "user", body: "La démonstration est terminée." }), false);
  assert.equal(isDemonstrationMessage({ ...bound, role: "assistant" }), false);
});

test("demonstration entries, explicit consent, stop/discard and existing approval stay connected", () => {
  const panel = readFileSync(new URL("../../ui/panels/demonstrations.js", import.meta.url), "utf8");
  const skills = readFileSync(new URL("../../ui/panels/skills.js", import.meta.url), "utf8");
  const card = readFileSync(new URL("./cards/skill-proposal.jsx", import.meta.url), "utf8");
  assert.match(panel, /show-bot-button/);
  assert.match(skills, /Montrer au bot/);
  assert.match(panel, /demonstration-consent/);
  for (const route of ["start", "stop", "draft", "discard"]) assert.ok(panel.includes(`/api/demonstrations/${route}`));
  assert.match(panel, /Enregistrement de la démonstration/);
  assert.match(panel, /Aucune saisie de texte/);
  assert.match(card, /forms\.decide/);
  assert.match(card, /display\.demonstration/);
  const css = readFileSync(new URL("../../ui/panels/demonstrations.css", import.meta.url), "utf8");
  assert.match(css, /button:disabled.*opacity: \.45/);
  assert.match(css, /demonstration-overlay \{ flex: 0 0 auto/);
  assert.doesNotMatch(css, /position: fixed|left: 50%/);
  assert.doesNotMatch(panel, /addEventListener\("cancel"/);
  assert.doesNotMatch(panel, /for \(const d of document\.querySelectorAll\("dialog\[open\]"\)\)/);
  assert.match(panel, /#i-monitor/);
  assert.match(panel, /pending_notice/);
  assert.match(panel, /capture_error/);
  assert.match(skills, /\[add, demonstrate, chooseFiles/);
});
