import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test, { after } from "node:test";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

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

test("Envoyer transmet l’objet et le corps corrigés d’un courriel, Annuler n’en transmet aucun", async () => {
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
});

const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)), configFile: false,
  plugins: [react()], server: { middlewareMode: true }, logLevel: "error",
  optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());
const { DraftCard } = await vite.ssrLoadModule("/src/cards/draft.jsx");

function renderEmailDraft({ status, etat, publication_result }) {
  const request = {
    entry_id: "mail", status: { status }, route: { requesting_bot_id: "grokbot" },
    kind: { kind: "approval", tool: "gmail_send_email" },
    arguments: { brouillon: { service: "gmail", action: "Envoyer un courriel", de: "charles@example.test", a: ["ami@example.test"], cc: ["bruno@example.test"], objet: "Objet proposé", texte: "Corps proposé", empreinte: "abc", ...(etat ? { etat } : {}) } },
    ...(publication_result ? { publication_result } : {}),
  };
  const input = { request, status, pending: status === "pending", busy: false, error: "", act: async () => true };
  globalThis.window = { GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(DraftCard, { message: { id: "mail", kind: "approval", payload: request }, input, disabled: false, expired: false }));
  } finally { delete globalThis.window; }
}

test("la carte brouillon d’un courriel montre De, À, Cc, Objet et Corps, Envoyer et Annuler, sans « Toujours »", () => {
  const html = renderEmailDraft({ status: "pending" });
  for (const label of [">De<", ">À<", ">Cc<", "Objet (modifiable)", "Corps du courriel (modifiable)", ">Envoyer<", ">Annuler<"]) assert.ok(html.includes(label), label);
  for (const value of ["charles@example.test", "ami@example.test", "bruno@example.test", "value=\"Objet proposé\"", ">Corps proposé</textarea>"]) assert.ok(html.includes(value), value);
  assert.ok(!html.includes("Toujours"));
});

test("Refusé, Trop de messages et Envoi non confirmé s’affichent en alerte, jamais comme un courriel envoyé", () => {
  for (const [etat, reason] of [
    ["refused", "Refusé · Google a refusé d’envoyer ce courriel. Aucun courriel n’est parti."],
    ["rate_limited", "Trop de messages · Google limite le nombre de courriels envoyés. Aucun courriel n’est parti."],
    ["unconfirmed", "Envoi non confirmé · l’accusé de réception de Google manque : le courriel est peut-être parti."],
  ]) {
    const html = renderEmailDraft({ status: "resolved", etat, publication_result: { status: "failed", reason } });
    assert.ok(html.includes(`role="alert">${reason}</p>`), etat);
    assert.ok(!html.includes("Courriel envoyé"), etat);
    assert.ok(!html.includes(">Envoyer<"), etat);
    assert.ok(!html.includes("Toujours"), etat);
  }
});

test("Envoyer transmet le texte corrigé d’un message Slack, sans titre", async () => {
  const calls = [];
  const panels = mount(async (path, body) => { calls.push({ path, body: JSON.parse(JSON.stringify(body)) }); return { accepted: true, receipt: { wake: { status: "pending" } } }; });
  const slack = { ...message, id: "draft-5", payload: { ...message.payload, entry_id: "draft-5", kind: { kind: "approval", tool: "slack_post_message" } } };
  assert.equal(await panels.panel.onCardAction(slack, "forms.decide", { decision: "allow_once", text: "Message validé" }, context(panels)), true);
  assert.equal(calls[0].body.text, "Message validé");
  assert.equal(Object.hasOwn(calls[0].body, "title"), false);
});

function renderSlackDraft({ status, etat, publication_result }) {
  const request = {
    entry_id: "slack", status: { status }, route: { requesting_bot_id: "grokbot" },
    kind: { kind: "approval", tool: "slack_post_message" },
    arguments: { brouillon: { service: "Slack", action: "Publier un message Slack", espace: "Atelier Ezerbot", cible: "#general", fil: "1760000100.000200", texte: "Message proposé", empreinte: "abc", ...(etat ? { etat } : {}) } },
    ...(publication_result ? { publication_result } : {}),
  };
  const input = { request, status, pending: status === "pending", busy: false, error: "", act: async () => true };
  globalThis.window = { GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(DraftCard, { message: { id: "slack", kind: "approval", payload: request }, input, disabled: false, expired: false }));
  } finally { delete globalThis.window; }
}

test("la carte brouillon d’un message Slack montre l’espace, la cible, le fil et le texte, Envoyer et Annuler, sans « Toujours »", () => {
  const html = renderSlackDraft({ status: "pending" });
  for (const label of [">Espace<", ">Cible<", ">Fil<", "Texte exact du message (modifiable)", ">Envoyer<", ">Annuler<"]) assert.ok(html.includes(label), label);
  for (const value of ["Atelier Ezerbot", "#general", "1760000100.000200", ">Message proposé</textarea>"]) assert.ok(html.includes(value), value);
  assert.ok(!html.includes("Toujours"));
  assert.ok(renderSlackDraft({ status: "pending" }).includes("Rien n’est publié avant votre clic"));
});

test("un message Slack publié le dit, et Refusé, Trop de messages et Envoi non confirmé s’affichent en alerte sans publication", () => {
  const sent = renderSlackDraft({ status: "resolved", etat: "sent", publication_result: { status: "done", output: { compte: "Atelier Ezerbot" } } });
  assert.ok(sent.includes("Message publié dans #general de l’espace Atelier Ezerbot."));
  for (const [etat, label] of [["refused", "Refusé"], ["rate_limited", "Trop de messages"], ["unconfirmed", "Envoi non confirmé"]]) {
    const html = renderSlackDraft({ status: "resolved", etat, publication_result: { status: "failed", reason: "Raison du refus." } });
    assert.ok(html.includes(`<strong>${label} · </strong>Raison du refus.`), etat);
    assert.ok(!html.includes("Message publié"), etat);
    assert.ok(!html.includes(">Envoyer<"), etat);
    assert.ok(!html.includes("Toujours"), etat);
  }
});
