import assert from "node:assert/strict";
import test, { after } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createTestServer } from "./vite-test-server.js";
import react from "@vitejs/plugin-react";

const vite = await createTestServer({ plugins: [react()] });
after(() => vite.close());
const { Card, kind } = await vite.ssrLoadModule("/src/cards/topic-memory.jsx");

const TEXT = "Le camion est réservé pour le 1er juillet";
function render(payload) {
  const message = { id: "topic-offer-1", kind, payload: { offer_id: "topic-offer-1", topic_id: "t1", topic_name: "Déménagement", text: TEXT, forgotten: false, ...payload } };
  return renderToStaticMarkup(React.createElement(Card, { message, onAction: async () => true }));
}
const buttons = (html) => [...html.matchAll(/<button[^>]*>([^<]*)<\/button>/g)].map((match) => match[1]);

test("la carte s’enregistre sous le type que le serveur envoie", () => {
  assert.equal(kind, "topic_memory_offer");
});

test("en attente : « Retenir pour ce sujet ? » avec Retenir et Pas maintenant, et le texte proposé", () => {
  const html = render({ status: "pending" });
  assert.ok(html.includes("<h3>Retenir pour ce sujet ?</h3>"));
  assert.ok(html.includes(TEXT));
  assert.ok(html.includes("Tous les fils de « Déménagement » pourront le lire"));
  assert.ok(html.includes("Rien n’est retenu sans votre accord"));
  assert.deepEqual(buttons(html), ["Retenir", "Pas maintenant"]);
});

test("retenu : « Retenu pour ce sujet » avec le lien vers la mémoire du sujet, sans les deux boutons", () => {
  const html = render({ status: "accepted" });
  assert.ok(html.includes("<h3>Retenu pour ce sujet</h3>"));
  assert.ok(html.includes(TEXT));
  assert.deepEqual(buttons(html), ["Mémoire du sujet"]);
  assert.ok(!html.includes("Pas maintenant"));
});

test("écarté ou oublié : un simple reçu, sans texte ni action", () => {
  const dismissed = render({ status: "dismissed", text: "" });
  assert.ok(dismissed.includes("Pas retenu pour ce sujet."));
  assert.deepEqual(buttons(dismissed), []);
  const forgotten = render({ status: "accepted", forgotten: true, text: "" });
  assert.ok(forgotten.includes("Ce souvenir a été oublié."));
  assert.ok(!forgotten.includes(TEXT));
  assert.deepEqual(buttons(forgotten), []);
});
