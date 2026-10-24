import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test, { after } from "node:test";
import { runInNewContext } from "node:vm";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createTestServer } from "./vite-test-server.js";
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

const vite = await createTestServer({ plugins: [react()] });
after(() => vite.close());
const { DraftCard } = await vite.ssrLoadModule("/src/cards/draft.jsx");

function renderEmailDraft({ status, etat, publication_result, service = "gmail", tool = "gmail_send_email" }) {
  const request = {
    entry_id: "mail", status: { status }, route: { requesting_bot_id: "grokbot" },
    kind: { kind: "approval", tool },
    arguments: { brouillon: { service, action: "Envoyer un courriel", de: "charles@example.test", a: ["ami@example.test"], cc: ["bruno@example.test"], objet: "Objet proposé", texte: "Corps proposé", empreinte: "abc", ...(etat ? { etat } : {}) } },
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

function renderMeetingDraft({ status, etat, publication_result, texte = "Bonjour de la carte" }) {
  const request = {
    entry_id: "reunion", status: { status }, route: { requesting_bot_id: "grokbot" },
    kind: { kind: "approval", tool: "recall_reply" },
    arguments: { brouillon: { service: "Réunions", action: "Répondre dans le chat de la réunion", reunion: "meet.google.com", bot: "Ezerbot", texte, empreinte: "abc", ...(etat ? { etat } : {}) } },
    ...(publication_result ? { publication_result } : {}),
  };
  const input = { request, status, pending: status === "pending", busy: false, error: "", act: async () => true };
  globalThis.window = { GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(DraftCard, { message: { id: "reunion", kind: "approval", payload: request }, input, disabled: false, expired: false }));
  } finally { delete globalThis.window; }
}

test("la carte brouillon d’une réponse dans la réunion montre la réunion, le bot et le texte exact, Envoyer et Annuler, sans « Toujours »", () => {
  const html = renderMeetingDraft({ status: "pending" });
  for (const label of [">Réunion<", ">Envoyé au nom de<", "Texte exact du message (modifiable)", ">Envoyer<", ">Annuler<", "Rien n’est envoyé avant votre clic"]) assert.ok(html.includes(label), label);
  for (const value of ["meet.google.com", "Ezerbot", ">Bonjour de la carte</textarea>"]) assert.ok(html.includes(value), value);
  assert.ok(!html.includes("Toujours"));
  assert.ok(!html.includes("Rien n’est publié"));
});

test("un texte trop long pour le chat de la réunion désactive Envoyer", () => {
  const html = renderMeetingDraft({ status: "pending", texte: "a".repeat(4097) });
  assert.ok(html.includes("Le texte dépasse 4096 caractères."));
  assert.match(html, /<button[^>]*disabled=""[^>]*>Envoyer<\/button>/);
});

test("un message envoyé dans la réunion le dit, et Refusé, Trop de messages et Envoi non confirmé ne prétendent pas que le chat l’a reçu", () => {
  const sent = renderMeetingDraft({ status: "resolved", etat: "sent", publication_result: { status: "done", output: { reunion: "meet.google.com" } } });
  assert.ok(sent.includes("Message envoyé dans le chat de la réunion meet.google.com, au nom de « Ezerbot »."));
  for (const [etat, label] of [["refused", "Refusé"], ["rate_limited", "Trop de messages"], ["unconfirmed", "Envoi non confirmé · accusé de réception manquant"]]) {
    const html = renderMeetingDraft({ status: "resolved", etat, publication_result: { status: "failed", reason: "Raison du refus." } });
    assert.ok(html.includes(`<strong>${label} · </strong>Raison du refus.`), etat);
    assert.ok(!html.includes("Message envoyé dans le chat"), etat);
    assert.ok(!html.includes(">Envoyer<"), etat);
    assert.ok(!html.includes("Toujours"), etat);
  }
});

test("Annuler une réponse dans la réunion dit que rien n’a été envoyé", () => {
  const html = renderMeetingDraft({ status: "denied" });
  assert.ok(html.includes("Annulé · rien n’a été envoyé"));
  assert.ok(!html.includes(">Envoyer<"));
});

function renderLinearDraft({ status, etat, publication_result, brouillon }) {
  const request = {
    entry_id: "linear", status: { status }, route: { requesting_bot_id: "grokbot" },
    kind: { kind: "approval", tool: brouillon.titre === undefined ? "linear_comment_issue" : "linear_create_issue" },
    arguments: { brouillon: { service: "Linear", compte: "charles@atelier.test · Atelier Ezerbot", texte: "Texte proposé", empreinte: "abc", ...brouillon, ...(etat ? { etat } : {}) } },
    ...(publication_result ? { publication_result } : {}),
  };
  const input = { request, status, pending: status === "pending", busy: false, error: "", act: async () => true };
  globalThis.window = { GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(DraftCard, { message: { id: "linear", kind: "approval", payload: request }, input, disabled: false, expired: false }));
  } finally { delete globalThis.window; }
}

test("la carte brouillon d’une issue Linear montre le compte, l’équipe, le titre et la description, Envoyer et Annuler, sans « Toujours »", () => {
  const html = renderLinearDraft({ status: "pending", brouillon: { action: "Créer une issue Linear", equipe: "ENG", titre: "Titre proposé" } });
  for (const label of [">Publié sous le compte<", ">Équipe<", "Titre de l’issue (modifiable)", "Corps de l’issue (modifiable)", ">Envoyer<", ">Annuler<", "crée cette issue"]) assert.ok(html.includes(label), label);
  for (const value of ["charles@atelier.test · Atelier Ezerbot", ">ENG<", "value=\"Titre proposé\"", ">Texte proposé</textarea>"]) assert.ok(html.includes(value), value);
  assert.ok(!html.includes("Toujours"));
  assert.ok(!html.includes("Dépôt"));
});

test("la carte brouillon d’un commentaire Linear montre le compte, l’issue et le texte exact, sans titre", () => {
  const html = renderLinearDraft({ status: "pending", brouillon: { action: "Commenter une issue Linear", issue: "ENG-12" } });
  for (const label of [">Publié sous le compte<", ">Issue<", "Texte exact du commentaire (modifiable)", ">Envoyer<", ">Annuler<"]) assert.ok(html.includes(label), label);
  assert.ok(html.includes(">ENG-12<"));
  assert.ok(!html.includes("Titre de l’issue"));
  assert.ok(!html.includes("Toujours"));
});

test("une issue Linear créée nomme son identifiant, et un envoi refusé ou non confirmé ne prétend rien avoir publié", () => {
  const sent = renderLinearDraft({ status: "resolved", etat: "sent", brouillon: { action: "Créer une issue Linear", equipe: "ENG", titre: "Titre" }, publication_result: { status: "done", output: { compte: "charles@atelier.test", identifiant: "ENG-31", url: "https://linear.app/atelier/issue/ENG-31" } } });
  assert.ok(sent.includes("Issue ENG-31 créée sous charles@atelier.test"));
  assert.ok(sent.includes("https://linear.app/atelier/issue/ENG-31"));
  const commented = renderLinearDraft({ status: "resolved", etat: "sent", brouillon: { action: "Commenter une issue Linear", issue: "ENG-12" }, publication_result: { status: "done", output: { compte: "charles@atelier.test", url: "https://linear.app/atelier/issue/ENG-12#comment-1" } } });
  assert.ok(commented.includes("Commentaire publié sous charles@atelier.test"));
  for (const etat of ["refused", "rate_limited", "unconfirmed"]) {
    const html = renderLinearDraft({ status: "resolved", etat, brouillon: { action: "Commenter une issue Linear", issue: "ENG-12" }, publication_result: { status: "failed", reason: "Raison du refus." } });
    assert.ok(html.includes("Raison du refus."), etat);
    assert.ok(!html.includes("Commentaire publié"), etat);
    assert.ok(!html.includes(">Envoyer<"), etat);
  }
});

function renderJiraDraft({ status, etat, publication_result }) {
  const request = {
    entry_id: "jira", status: { status }, route: { requesting_bot_id: "grokbot" },
    kind: { kind: "approval", tool: "jira_comment_issue" },
    arguments: { brouillon: { service: "Jira Cloud", action: "Commenter une issue Jira", compte: "charles@atelier.test · Atelier Ezerbot", issue: "ENG-12", texte: "Texte proposé", empreinte: "abc", ...(etat ? { etat } : {}) } },
    ...(publication_result ? { publication_result } : {}),
  };
  const input = { request, status, pending: status === "pending", busy: false, error: "", act: async () => true };
  globalThis.window = { GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(DraftCard, { message: { id: "jira", kind: "approval", payload: request }, input, disabled: false, expired: false }));
  } finally { delete globalThis.window; }
}

test("la carte brouillon d’un commentaire Jira montre le compte et le site, l’issue et le texte exact, sans titre ni « Toujours »", () => {
  const html = renderJiraDraft({ status: "pending" });
  for (const label of [">Publié sous le compte<", ">Issue<", "Texte exact du commentaire (modifiable)", ">Envoyer<", ">Annuler<"]) assert.ok(html.includes(label), label);
  for (const value of ["charles@atelier.test · Atelier Ezerbot", ">ENG-12<", ">Texte proposé</textarea>"]) assert.ok(html.includes(value), value);
  assert.ok(!html.includes("Titre de l’issue"));
  assert.ok(!html.includes("Toujours"));
});

test("un commentaire Jira publié donne son adresse, et un envoi refusé ou non confirmé ne prétend rien avoir publié", () => {
  const sent = renderJiraDraft({ status: "resolved", etat: "sent", publication_result: { status: "done", output: { compte: "charles@atelier.test · Atelier Ezerbot", url: "https://atelier.atlassian.net/browse/ENG-12?focusedCommentId=10042" } } });
  assert.ok(sent.includes("Commentaire publié sous charles@atelier.test · Atelier Ezerbot"));
  assert.ok(sent.includes("https://atelier.atlassian.net/browse/ENG-12?focusedCommentId=10042"));
  for (const etat of ["refused", "rate_limited", "unconfirmed"]) {
    const html = renderJiraDraft({ status: "resolved", etat, publication_result: { status: "failed", reason: "Raison du refus." } });
    assert.ok(html.includes("Raison du refus."), etat);
    assert.ok(!html.includes("Commentaire publié"), etat);
    assert.ok(!html.includes(">Envoyer<"), etat);
  }
});

test("la carte brouillon d’un courriel Outlook montre De, À, Cc, Objet et Corps, Envoyer et Annuler, sans « Toujours », et dit ce qui est parti", () => {
  const outlook = { service: "Microsoft 365", tool: "outlook_send_mail" };
  const html = renderEmailDraft({ ...outlook, status: "pending" });
  for (const label of [">De<", ">À<", ">Cc<", "Objet (modifiable)", "Corps du courriel (modifiable)", ">Envoyer<", ">Annuler<", "envoie ce courriel"]) assert.ok(html.includes(label), label);
  assert.ok(html.includes("data-draft-service=\"Microsoft 365\""));
  assert.ok(!html.includes("Toujours"));
  const sent = renderEmailDraft({ ...outlook, status: "resolved", etat: "sent", publication_result: { status: "done", output: { compte: "charles@example.test", a: ["ami@example.test"] } } });
  assert.ok(sent.includes("Courriel envoyé sous charles@example.test à ami@example.test."));
  for (const [etat, reason] of [
    ["refused", "Microsoft a refusé la requête concernant l’envoi du courriel (ErrorInvalidRecipients). Aucun courriel n’est parti."],
    ["rate_limited", "Microsoft limite le débit des requêtes. Aucun courriel n’est parti."],
    ["unconfirmed", "Envoi non confirmé · l’accusé de réception de Microsoft manque : le courriel est peut-être parti."],
  ]) {
    const failed = renderEmailDraft({ ...outlook, status: "resolved", etat, publication_result: { status: "failed", reason } });
    assert.ok(failed.includes(reason), etat);
    assert.ok(!failed.includes("Courriel envoyé"), etat);
    assert.ok(!failed.includes(">Envoyer<"), etat);
  }
  const cancelled = renderEmailDraft({ ...outlook, status: "denied" });
  assert.ok(cancelled.includes("Annulé · rien n’a été envoyé"));
  assert.ok(!cancelled.includes(">Envoyer<"));
});
