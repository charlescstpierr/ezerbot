import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

function mount(api) {
  const document = { createElement: () => ({}), getElementById: () => null };
  const panels = { state: { forms: {} }, register(value) { this.panel = value; }, api, async refresh() {}, notify() {} };
  const window = { GrokbotPanels: panels, dispatchEvent() {}, setTimeout, clearTimeout, Event: class Event { constructor(type) { this.type = type; } } };
  runInNewContext(readFileSync(new URL("../../ui/panels/forms.js", import.meta.url), "utf8"), { document, window, setTimeout, clearTimeout, Event: window.Event });
  return panels;
}
const message = {
  id: "draft-1",
  kind: "approval",
  payload: {
    entry_id: "draft-1",
    route: { target_conversation_id: "conv", requesting_bot_id: "grokbot" },
    kind: { kind: "approval", tool: "github_add_issue_comment" },
    status: { status: "pending" },
  },
};
const context = (panels) => ({ conversation: { id: "conv" }, state: panels.state });

test("Envoyer transmet le texte corrigé de la carte brouillon, Annuler n’en transmet aucun", async () => {
  const calls = [];
  const panels = mount(async (path, body) => { calls.push({ path, body: JSON.parse(JSON.stringify(body)) }); return { accepted: true, receipt: { wake: { status: "pending" } } }; });
  assert.equal(await panels.panel.onCardAction(message, "forms.decide", { decision: "allow_once", text: "Bonjour validé" }, context(panels)), true);
  assert.equal(calls[0].path, "/api/approvals/resolve");
  assert.equal(calls[0].body.decision, "allow_once");
  assert.equal(calls[0].body.text, "Bonjour validé");

  const other = { ...message, id: "draft-2", payload: { ...message.payload, entry_id: "draft-2" } };
  await panels.panel.onCardAction(other, "forms.decide", { decision: "deny" }, context(panels));
  assert.equal(calls[1].body.decision, "deny");
  assert.equal(Object.hasOwn(calls[1].body, "text"), false);
});

test("la carte brouillon n’offre ni « Toujours » ni règle à mémoriser", () => {
  const card = readFileSync(new URL("./cards/draft.jsx", import.meta.url), "utf8");
  assert.doesNotMatch(card, /Toujours|always_allow|never:|Mémoriser/);
  assert.match(card, />Envoyer</);
  assert.match(card, />Annuler</);
  assert.match(card, /<textarea/);
  const approval = readFileSync(new URL("./cards/approval.jsx", import.meta.url), "utf8");
  assert.match(approval, /preview\?\.brouillon\) return <DraftCard/);
});

test("Envoyer transmet le titre et le corps corrigés d’une issue, sans « Toujours »", async () => {
  const calls = [];
  const panels = mount(async (path, body) => { calls.push({ path, body: JSON.parse(JSON.stringify(body)) }); return { accepted: true, receipt: { wake: { status: "pending" } } }; });
  const issue = { ...message, id: "draft-3", payload: { ...message.payload, entry_id: "draft-3", kind: { kind: "approval", tool: "github_create_issue" } } };
  assert.equal(await panels.panel.onCardAction(issue, "forms.decide", { decision: "allow_once", title: "Essai validé", text: "Corps validé" }, context(panels)), true);
  assert.equal(calls[0].body.title, "Essai validé");
  assert.equal(calls[0].body.text, "Corps validé");
  const other = { ...issue, id: "draft-4", payload: { ...issue.payload, entry_id: "draft-4" } };
  await panels.panel.onCardAction(other, "forms.decide", { decision: "deny" }, context(panels));
  assert.equal(Object.hasOwn(calls[1].body, "title"), false);
  const card = readFileSync(new URL("./cards/draft.jsx", import.meta.url), "utf8");
  assert.match(card, /draft\.titre/);
  assert.match(card, /Titre de l’issue/);
  assert.doesNotMatch(card, /Toujours|always_allow|never:|Mémoriser/);
});

test("Envoyer transmet l’objet et le corps corrigés d’un courriel, et la carte nomme les trois échecs", async () => {
  const calls = [];
  const panels = mount(async (path, body) => { calls.push({ path, body: JSON.parse(JSON.stringify(body)) }); return { accepted: true, receipt: { wake: { status: "pending" } } }; });
  const mail = { ...message, id: "draft-5", payload: { ...message.payload, entry_id: "draft-5", kind: { kind: "approval", tool: "gmail_send_email" } } };
  assert.equal(await panels.panel.onCardAction(mail, "forms.decide", { decision: "allow_once", title: "Objet validé", text: "Bonjour validé" }, context(panels)), true);
  assert.equal(calls[0].body.title, "Objet validé");
  assert.equal(calls[0].body.text, "Bonjour validé");
  const other = { ...mail, id: "draft-6", payload: { ...mail.payload, entry_id: "draft-6" } };
  await panels.panel.onCardAction(other, "forms.decide", { decision: "deny" }, context(panels));
  assert.equal(Object.hasOwn(calls[1].body, "title"), false);
  assert.equal(Object.hasOwn(calls[1].body, "text"), false);
  const card = readFileSync(new URL("./cards/draft.jsx", import.meta.url), "utf8");
  for (const word of ["draft.objet", "draft.de", "draft.cc", "Objet (modifiable)", "Corps du courriel", "Refusé", "Trop de messages", "Envoi non confirmé"]) assert.ok(card.includes(word), word);
  assert.doesNotMatch(card, /Toujours|always_allow|never:|Mémoriser/);
});
