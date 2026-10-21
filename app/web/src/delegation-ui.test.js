import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// Exercise the actual shell function with a small editor/DOM adapter, without
// adding a second copy of its task/group/composer policy to the tests.
const shell = readFileSync(new URL("../../ui/app.js", import.meta.url), "utf8");
const updateSource = shell.slice(shell.indexOf("  function updateComposer()"), shell.indexOf("  function selectConversation("));
function composer(kind = "task") {
  const ctx = { active: true, streaming: true, conversation: { id: "thread", kind, messages: [] } };
  const removed = [], discarded = [], mascots = [];
  const input = { value: "", setReadOnly(value) { this.readOnly = value; }, setPlaceholder(value) { this.placeholder = value; }, setInvalid(value) { this.invalid = value; } };
  const nodes = { "new-chat": {}, "composer-hint": {}, "learn-skill-button": {}, "restore-link-draft": {} };
  const use = { setAttribute() {} };
  const ui = { input, send: { dataset: {}, setAttribute() {}, querySelector: () => use }, hint: { dataset: {} }, menuButton: {} };
  const panel = { composer: { discard() { discarded.push(ctx.conversation.id); } } };
  const update = runInNewContext(`(${updateSource})`, {
    ui, window: { GrokbotDelegation: { canFollowUp: () => ctx.active } },
    document: { activeElement: null }, TextEncoder,
    currentConversation: () => ctx.conversation, streamingMessage: () => ctx.streaming,
    pendingStates: new Set(["queued", "dispatching", "unknown"]),
    panels: new Map([["attachments", panel]]), panelCall: (_panel, hook) => hook?.(),
    sendExtras: () => ({}), removePreference: (key) => removed.push(key), draftKey: () => `draft.${ctx.conversation.id}`,
    setComposerText: (text) => { input.value = text; },
    busy: false, connected: true, submissionActivity: null, connectionStatus: "online",
    deepLinks: { setReady() {} }, deepLinksBootReady: true, previousLinkDrafts: () => [], selectedId: "thread",
    $: (id) => nodes[id], mascot: { setContext: (value) => mascots.push(value) },
  });
  return { ctx, ui, removed, discarded, mascots, update };
}

test("a running task composer only sends follow-ups; cancellation stays in the banner", () => {
  const c = composer(); c.update();
  assert.equal(c.ui.send.dataset.mode, "send");
  assert.equal(c.ui.send.disabled, true);
  assert.equal(c.ui.input.readOnly, false);
  assert.equal(c.ui.input.placeholder, "Écrire un suivi pour cette sous-tâche…");
  c.ui.input.value = "Mon suivi"; c.update();
  assert.equal(c.ui.send.disabled, false);
  assert.equal(c.ui.send.dataset.mode, "send");
});

test("a finished task clears drafts and attachments, is read-only and is no longer attentive", () => {
  const c = composer(); c.ctx.active = false;
  c.ui.input.value = "Brouillon qui ne peut plus partir"; c.update();
  assert.equal(c.ui.input.value, "");
  assert.equal(c.ui.input.readOnly, true);
  assert.equal(c.ui.send.disabled, true);
  assert.deepEqual(c.removed, ["draft.thread", "draft.thread.document"]);
  assert.deepEqual(c.discarded, ["thread"]);
  assert.equal(c.mascots.at(-1).drafting, false);
  assert.equal(c.mascots.at(-1).activity, null);
});

test("task hints prioritize blocked-send alerts over the generic follow-up hint", () => {
  const c = composer(); c.ui.input.value = "a".repeat(17000); c.update();
  assert.match(c.ui.hint.textContent, /Ce message est trop long/);
  assert.equal(c.ui.send.disabled, true);
  c.ui.input.value = "Encore";
  c.ctx.conversation.messages = [{ role: "user", state: "unknown" }]; c.update();
  assert.match(c.ui.hint.textContent, /Vérifiez le dernier envoi/);
  c.ctx.conversation.messages = []; c.update();
  assert.match(c.ui.hint.textContent, /Suivi réservé à cette sous-tâche/);
  assert.equal(c.ui.send.title, "Envoyer le message");
});

test("leaving a finished task restores the ordinary group and chat composer", () => {
  const c = composer(); c.ctx.active = false; c.update();
  c.ctx.conversation = { id: "group", kind: "group", messages: [], member_ids: ["bot"] };
  c.ui.input.value = "Bonjour au groupe"; c.update();
  assert.equal(c.ui.input.readOnly, false);
  assert.equal(c.ui.input.placeholder, "Qu’avez-vous en tête ?");
  assert.equal(c.ui.send.dataset.mode, "send");
  assert.equal(c.ui.send.disabled, false);
  c.ctx.conversation.kind = "chat"; c.update();
  assert.equal(c.ui.send.dataset.mode, "stop");
});
