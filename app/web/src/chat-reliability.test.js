import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { pendingReply } from "./thread-order.js";
import { connectionMessages, composerConnectionState, signInCardState, replyProblemState, preparingTool, visibleNotice } from "./connection-view.js";

test("an authoritative terminal snapshot releases loading without a final stream event", () => {
  const live = [{ role: "assistant", state: "streaming", body: "Début" }];
  assert.equal(pendingReply(connectionMessages(live, true)[0]), true);
  for (const state of ["failed", "echoed", "stopped"]) {
    const snapshot = [{ ...live[0], state }];
    assert.equal(pendingReply(connectionMessages(snapshot, true)[0]), false);
  }
});

test("a dropped local connection replaces direct and group spinners without mutating or replaying the saved turn", () => {
  const saved = [{ role: "assistant", state: "streaming", body: "Début", payload: { run_id: "r", turn_status: "running" } }];
  const offline = connectionMessages(saved, false);
  assert.equal(offline[0].state, "failed");
  assert.equal(offline[0].body, "Début");
  assert.equal(offline[0].error_kind, "local_connection");
  assert.equal(pendingReply(offline[0], true), false);
  assert.equal(saved[0].state, "streaming");
  assert.equal(connectionMessages(saved, true), saved);
  const finished = [{ ...saved[0], state: "echoed", payload: { run_id: "r", turn_status: "finished" } }];
  assert.equal(connectionMessages(finished, true), finished);
  assert.equal(pendingReply(connectionMessages(finished, true)[0], true), false);
});

test("offline recovery preserves saved questions, sign-in cards, human waits and settled replies", () => {
  const messages = [
    { role: "user", state: "unknown", body: "Question conservée" },
    { role: "assistant", state: "failed", error_kind: "signin", error: "Connectez votre compte ChatGPT" },
    { role: "assistant", kind: "form", state: "echoed", payload: { status: "pending" } },
    ...["failed", "stopped", "echoed"].map((state) => ({ role: "assistant", state, body: "Texte conservé" })),
  ];
  const offline = connectionMessages(messages, false);
  for (const [index, message] of messages.entries()) assert.equal(offline[index], message);
  assert.equal(offline[0].body, "Question conservée");
  assert.equal(offline[1].error_kind, "signin");
  assert.equal(offline[2].payload.status, "pending");
});

test("an offline composer is an idle send arrow and recovers its real control on reconnect", () => {
  const live = { live: true, busy: true, group: false };
  const offline = composerConnectionState({ ...live, offline: true });
  assert.equal(offline.mode, "send");
  assert.equal(offline.icon, "#i-send");
  assert.equal(offline.live, false);
  assert.equal(offline.busy, false);
  assert.equal(offline.title, "Moteur local indisponible");
  assert.equal(composerConnectionState(live).mode, "stop");
  assert.equal(composerConnectionState({ live: true, group: true }).title, "Arrêter les tours");
  assert.equal(composerConnectionState({ busy: true }).label, "Envoi en cours");
});

test("a group sign-in card never promises a retry, even after account changes", () => {
  const message = { error_kind: "signin", payload: { turn_id: "t" } };
  for (const providerConnected of [false, true]) {
    const card = signInCardState({ message, providerConnected, isGroup: true });
    assert.equal(card.showRetry, false);
    assert.equal(card.copy, providerConnected ? "Envoyez un nouveau message pour obtenir une réponse." : "Connectez votre compte ChatGPT via Codex, puis envoyez un nouveau message.");
    assert.equal(card.showSignIn, !providerConnected);
  }
});

test("a connected account card is a status, not a red alert", () => {
  const connected = signInCardState({ message: { error_kind: "signin" }, providerConnected: true });
  assert.equal(connected.title, "Compte ChatGPT connecté");
  assert.equal(connected.alert, false);
  const offline = signInCardState({ message: { error_kind: "signin" }, providerConnected: true, engineConnected: false });
  assert.equal(offline.alert, false);
  assert.equal(signInCardState({ message: { error_kind: "signin" } }).alert, true);
  assert.equal(signInCardState({ message: { error_kind: "signin_expired" } }).alert, true);
  const source = readFileSync(new URL("./Thread.jsx", import.meta.url), "utf8");
  assert.match(source, /className=\{`gb-pending\$\{view\.alert \? " gb-problem" : ""\}`\}/);
  assert.match(source, /\{view\.alert \? <TriangleAlert \/> : <Check \/>\}/);
});

test("direct retries are enabled only after both account and local engine are available", () => {
  const message = { error_kind: "signin" };
  const missing = signInCardState({ message });
  assert.equal(missing.title, "Connectez votre compte ChatGPT");
  assert.equal(missing.showRetry, true);
  assert.equal(missing.retryDisabled, true);
  const ready = signInCardState({ message, providerConnected: true });
  assert.equal(ready.retryDisabled, false);
  assert.equal(ready.title, "Compte ChatGPT connecté");
  assert.equal(ready.showSignIn, false);
  assert.match(ready.copy, /Réessayez pour obtenir la réponse/);
  const offline = signInCardState({ message, providerConnected: true, engineConnected: false });
  assert.equal(offline.retryDisabled, true);
  assert.equal(offline.signInDisabled, true);
  assert.equal(offline.retryTitle, "Moteur local indisponible");
});

test("an expired session has an explicit persistent reconnect presentation", () => {
  const card = signInCardState({ message: { error_kind: "signin_expired" } });
  assert.equal(card.title, "Votre session ChatGPT a expiré");
  assert.equal(card.signInLabel, "Se reconnecter via Codex");
  assert.equal(replyProblemState({ error_kind: "signin_expired" }).signin, true);
});

test("an expired session body states the expiry instead of the generic sign-in copy", () => {
  const expired = signInCardState({ message: { error_kind: "signin_expired" } });
  const generic = signInCardState({ message: { error_kind: "signin" } });
  assert.match(expired.copy, /^Votre session ChatGPT a expiré\. /);
  assert.notEqual(expired.copy, generic.copy);
  assert.match(expired.copy, /Reconnectez-vous via Codex, puis réessayez\./);
  const group = signInCardState({ message: { error_kind: "signin_expired" }, isGroup: true });
  assert.match(group.copy, /^Votre session ChatGPT a expiré\. /);
  assert.match(group.copy, /envoyez un nouveau message/);
  assert.equal(group.showRetry, false);
  const task = signInCardState({ message: { error_kind: "signin_expired" }, isTask: true });
  assert.match(task.copy, /^Votre session ChatGPT a expiré\. /);
  assert.match(task.copy, /conversation principale/);
  const reconnected = signInCardState({ message: { error_kind: "signin_expired" }, providerConnected: true });
  assert.equal(reconnected.title, "Compte ChatGPT connecté");
  assert.match(reconnected.copy, /Réessayez pour obtenir la réponse/);
});

test("a group failure does not tell the reader to retry when no retry button is offered", () => {
  const message = { error_kind: "other", error: "La réponse a dépassé le délai maximal. Réessayez.", payload: { turn_id: "t" } };
  const view = replyProblemState(message, { isGroup: true });
  assert.equal(view.retry, false);
  assert.equal(view.error, "La réponse a dépassé le délai maximal. Envoyez un nouveau message.");
  assert.equal(view.advice, null);
  const direct = replyProblemState({ error_kind: "other", error: message.error });
  assert.equal(direct.retry, true);
  assert.equal(direct.error, message.error);
  const task = replyProblemState({ error_kind: "other", error: message.error }, { isTask: true });
  assert.equal(task.retry, false);
  assert.match(task.error, /Poursuivez dans la conversation principale\.$/);
  assert.equal(task.advice, null);
  const local = replyProblemState({ error_kind: "local_connection", error: "La connexion au moteur local est interrompue. La réponse peut avoir continué : réessayez pour retrouver son état, sans renvoyer votre question." }, { isGroup: true });
  assert.equal(local.retry, true);
  assert.match(local.error, /réessayez pour retrouver son état/);
  const source = readFileSync(new URL("./Thread.jsx", import.meta.url), "utf8");
  assert.match(source, /<p>\{error\}<\/p>/);
  assert.ok(!source.includes("<p>{message.error}</p>"));
});

test("deduplicating group advice does not accidentally enable forbidden regeneration", () => {
  const group = replyProblemState({ error_kind: "other", error: "Reconnectez-vous et envoyez un nouveau message." }, { isGroup: true });
  assert.equal(group.advice, null);
  assert.equal(group.retry, false);
  assert.equal(replyProblemState({ error_kind: "other" }, { isGroup: true }).advice, "Envoyez un nouveau message pour demander une nouvelle réponse.");
  const offline = replyProblemState({ error_kind: "local_connection" }, { isGroup: true, engineConnected: false });
  assert.equal(offline.retry, true);
  assert.equal(offline.retryDisabled, true);
});

test("a pending tool has a single visible progress state, never after failure or completion", () => {
  const notice = "Préparation d’un outil par OpenAI… Aucune action n’a encore été exécutée.";
  assert.equal(preparingTool({ state: "streaming", notice }), true);
  for (const state of ["failed", "echoed", "stopped"]) {
    assert.equal(preparingTool({ state, notice }), false);
    assert.equal(visibleNotice({ state, notice }), null);
  }
  assert.equal(visibleNotice({ state: "streaming", notice }), notice);
  assert.equal(visibleNotice({ state: "stopped", notice: "Autre information" }), "Autre information");
  assert.equal(preparingTool({ state: "streaming" }), false);
});

test("the thread stays disconnected until the shell reports both connections", () => {
  const source = readFileSync(new URL("./Thread.jsx", import.meta.url), "utf8");
  const signature = source.match(/export function Thread\((\{[^}]*\})/)[1];
  assert.match(signature, /providerConnected = false/);
  assert.match(signature, /engineConnected = false/);
});

test("a disabled secondary button is dimmed and does not look actionable", () => {
  const css = readFileSync(new URL("../../ui/styles.css", import.meta.url), "utf8");
  const rule = css.match(/\.secondary-button:disabled\s*\{[^}]*\}/);
  assert.ok(rule, "règle .secondary-button:disabled absente");
  assert.match(rule[0], /opacity:\s*\.5/);
  assert.match(rule[0], /cursor:\s*default/);
});
