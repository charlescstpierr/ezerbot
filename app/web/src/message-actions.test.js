import test from "node:test";
import assert from "node:assert/strict";
import { messageActions, messageMenuOpenId, toggleReaction } from "./message-actions.js";

const delivered = { role: "user", kind: "message", state: "echoed", body: "Déjà livré", payload: {} };
const pending = { role: "user", kind: "message", state: "unknown", body: "Coupure", payload: {} };
const refused = { role: "user", kind: "message", state: "refused", body: "Raté", payload: {} };
const queued = { role: "user", kind: "message", state: "queued", body: "En attente", payload: {} };

test("une réaction se pose et se retire en la rechoisissant", () => {
  assert.equal(toggleReaction(null, "👍"), "👍");
  assert.equal(toggleReaction("👍", "👍"), null);
  assert.equal(toggleReaction("👍", "❤️"), "❤️");
});

test("un message livré n’a ni Renvoyer, ni Supprimer, ni édition, et Copier reste", () => {
  for (const state of ["accepted", "echoed"]) {
    const menu = messageActions({ ...delivered, state });
    assert.equal(menu.copy, true);
    assert.equal(menu.resend, false);
    assert.equal(menu.delete, false);
    assert.equal(menu.edit, false);
    assert.equal(menu.status, null);
  }
  const assistant = messageActions({ role: "assistant", state: "echoed", body: "Réponse", payload: { reaction: "👍" } });
  assert.equal(assistant.copy, true);
  assert.equal(assistant.resend, false);
  assert.equal(assistant.delete, false);
  assert.equal(assistant.edit, false);
  assert.equal(assistant.reaction, "👍");
});

test("un envoi qui n’est pas parti affiche Échec de l’envoi ou Pas encore livré, avec Renvoyer et Supprimer", () => {
  const waiting = messageActions(pending);
  assert.equal(waiting.status, "Pas encore livré");
  assert.equal(waiting.resend, true);
  assert.equal(waiting.delete, true);
  assert.equal(waiting.edit, false);
  assert.equal(waiting.copy, true);
  assert.equal(messageActions(queued).status, "Pas encore livré");
  assert.equal(messageActions({ ...queued, state: "dispatching" }).status, "Pas encore livré");
  const failed = messageActions(refused);
  assert.equal(failed.status, "Échec de l’envoi");
  assert.equal(failed.resend, true);
  assert.equal(failed.delete, true);
  assert.equal(failed.confirm, false);
});

test("l’envoi en cours ne propose pas encore Renvoyer", () => {
  const menu = messageActions(queued, { busy: true });
  assert.equal(menu.resend, false);
  assert.equal(menu.delete, false);
  assert.equal(menu.status, null);
});

test("ouvrir un menu de message ferme celui qui était déjà ouvert", () => {
  assert.equal(messageMenuOpenId(null, "un", true), "un");
  assert.equal(messageMenuOpenId("un", "deux", true), "deux");
  assert.equal(messageMenuOpenId("deux", "deux", false), null);
  assert.equal(messageMenuOpenId("deux", "un", false), "deux");
});
