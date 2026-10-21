import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { cycleFind, findCountLabel, findInThread } from "./thread-find.js";
import { createRatingTable, ratingLabels } from "./thread-rating.js";

const rare = "xyloglotte";

test("la recherche annonce le nombre et circule sans tenir compte de la casse", () => {
  const messages = [
    { id: "m-user", body: `As-tu vu ${rare} ?` },
    { id: "m-bot", body: `Oui, ${rare} revient, et encore ${rare}.` },
  ];
  const matches = findInThread(messages, "XYLOGLOTTE", (message) => message.body);
  assert.equal(matches.length, 3);
  assert.deepEqual(matches.map((match) => [match.messageId, match.indexInMessage]), [
    ["m-user", 0], ["m-bot", 0], ["m-bot", 1],
  ]);
  assert.equal(findCountLabel(3, 0), "1 sur 3 résultats");
  assert.equal(findCountLabel(0, 0), "Aucun résultat");
  assert.equal(cycleFind(0, 3, -1), 2);
  assert.equal(cycleFind(2, 3, 1), 0);
  assert.deepEqual(findInThread(messages, "   "), []);
});

test("Fermer n’écrit rien, l’envoi est dans l’export, et le module ne parle pas au réseau", () => {
  const saved = new Map();
  const storage = {
    getItem: (key) => saved.get(key) ?? null,
    setItem: (key, value) => saved.set(key, value),
  };
  const table = createRatingTable(storage, { now: () => "2026-10-21T12:00:00.000Z", createId: () => "note-1" });
  assert.deepEqual(table.list(), []);
  const good = table.recordGood({ conversationId: "conv-1", messageId: "m-bot" });
  assert.equal(good.vote, "bon");
  const sent = table.recordBad({ conversationId: "conv-1", messageId: "m-bot", motif: "pas-demande", details: "Il invente une date." });
  assert.equal(sent.motif, ratingLabels.reasons[0].label);
  assert.equal(sent.details, "Il invente une date.");
  const exported = table.exportDocument();
  assert.equal(exported.notations.length, 2);
  assert.equal(exported.notations[1].motif, "Pas ce que j’ai demandé");
  assert.equal(JSON.stringify(exported).includes("voteGrokBotFeedback"), false);
  const source = readFileSync(new URL("./thread-rating.js", import.meta.url), "utf8");
  assert.doesNotMatch(source, /fetch\s*\(/);
  assert.equal(ratingLabels.title, "Comment s’améliorer ?");
  assert.deepEqual(ratingLabels.reasons.map((item) => item.label), ["Pas ce que j’ai demandé", "Inexact", "Qualité faible", "Trop lent"]);
});

test("une réponse terminée propose Bon résultat et Mauvais résultat, pas pendant l’écriture", () => {
  const source = readFileSync(new URL("./Thread.jsx", import.meta.url), "utf8");
  const footer = source.slice(source.indexOf("!streaming && (copyText || message.state === \"stopped\")"));
  assert.match(footer, /onRateGood && <button type="button" className="gb-text-button" onClick=\{\(\) => onRateGood\(message\)\}>Bon résultat<\/button>/);
  assert.match(footer, /onRateBad && <button type="button" className="gb-text-button" onClick=\{\(\) => onRateBad\(message\)\}>Mauvais résultat<\/button>/);
  const bundle = readFileSync(new URL("../../ui/chat.js", import.meta.url), "utf8");
  assert.match(bundle, /Bon résultat/);
  assert.match(bundle, /Mauvais résultat/);
  assert.match(bundle, /findInThread/);
  assert.match(bundle, /createRatingTable/);
});
