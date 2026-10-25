import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { composerConnectionState } from "./connection-view.js";
import { pendingReply } from "./thread-order.js";

// Boot the real, whole app.js in node:vm (as app/ui/draft-transfer.test.cjs does) and observe updateComposer
// through the public GrokbotPanels.composerChanged() hook: no text is cut out of app.js.
const shell = readFileSync(new URL("../../ui/app.js", import.meta.url), "utf8");
// Permissive stand-in for a DOM element: a few real fields, anything else is a callable no-op fake.
function fake() {
  const own = Object.assign(() => {}, { dataset: {}, children: [], classList: { add() {}, remove() {}, toggle() {}, contains: () => false },
    textContent: "", value: "", querySelector: fake, querySelectorAll: () => [], closest: () => null, contains: () => false, getAttribute: () => null });
  return new Proxy(own, { get: (target, key) => key in target || typeof key === "symbol" ? target[key] : (target[key] = fake()) });
}
async function composer() {
  const ctx = { active: true }, removed = [], discarded = [], mascots = [], storage = new Map([["grokbot.selected", "thread"]]);
  const input = { value: "", mentions: [], setReadOnly(value) { this.readOnly = value; }, setPlaceholder(value) { this.placeholder = value; }, setInvalid(value) { this.invalid = value; }, focus() {} };
  const conversation = (id, kind) => ({ id, kind, member_ids: ["bot"], messages: [{ role: "assistant", state: "streaming" }] });
  const state = { account: {}, bots: [{ id: "bot", name: "Bot" }], conversations: [conversation("thread", "task"), conversation("group", "group")] };
  const nodes = new Map(), document = { getElementById: (id) => nodes.get(id) || nodes.set(id, fake()).get(id), createElement: fake, createElementNS: fake, createTextNode: String,
    body: fake(), documentElement: fake(), querySelector: fake, querySelectorAll: () => [], addEventListener() {} };
  document.getElementById("composer-input").editor = { commands: { clearContent: () => { input.value = ""; } } };
  document.getElementById("composer-input").querySelector = () => document.getElementById("composer-input");
  const window = { GrokbotDelegation: { canFollowUp: () => ctx.active }, matchMedia: () => ({ matches: false, addEventListener() {} }), addEventListener() {},
    GrokbotChat: { composerConnectionState, isPendingReply: pendingReply, mountInput: () => input, connectionMessages: (messages) => messages,
      createDeepLinkQueue: () => ({ setReady() {} }), createApprovalFollow: () => ({ cancel() {}, arm() {} }), renderThread() {}, searchableMessageText: () => "" },
    GrokbotMascot: { shapes: {}, create: fake, setContext: (value) => mascots.push(value) } };
  runInContext(shell, createContext({ window, document, console, TextEncoder, AbortController, crypto, structuredClone, Node: class {}, ResizeObserver: class { observe() {} }, innerHeight: 800,
    localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, String(value)), removeItem: (key) => removed.push(key) },
    fetch: async () => ({ ok: true, json: async () => structuredClone(state) }), setTimeout: () => 0, clearTimeout() {}, requestAnimationFrame: () => 0 }));
  const panels = window.GrokbotPanels;
  await new Promise((resolve) => setImmediate(resolve));
  panels.register({ id: "attachments", composer: { discard: ({ conversation }) => discarded.push(conversation.id) } });
  removed.length = 0;
  return { ctx, removed, discarded, mascots, panels, update: () => panels.composerChanged(), ui: { input, send: document.getElementById("send-button"), hint: document.getElementById("composer-hint") },
    get conversation() { return panels.state.conversations.find((item) => item.id === storage.get("grokbot.selected")); } };
}

test("a running task composer only sends follow-ups; cancellation stays in the banner", async () => {
  const c = await composer(); c.update();
  assert.equal(c.ui.send.dataset.mode, "send");
  assert.equal(c.ui.send.disabled, true);
  assert.equal(c.ui.input.readOnly, false);
  assert.equal(c.ui.input.placeholder, "Écrire un suivi pour cette sous-tâche…");
  c.ui.input.value = "Mon suivi"; c.update();
  assert.equal(c.ui.send.disabled, false);
  assert.equal(c.ui.send.dataset.mode, "send");
});

test("a finished task clears drafts and attachments, is read-only and is no longer attentive", async () => {
  const c = await composer(); c.ctx.active = false;
  c.ui.input.value = "Brouillon qui ne peut plus partir"; c.update();
  assert.equal(c.ui.input.value, "");
  assert.equal(c.ui.input.readOnly, true);
  assert.equal(c.ui.send.disabled, true);
  assert.deepEqual(c.removed, ["grokbot.draft.thread", "grokbot.draft.thread.document"]);
  assert.deepEqual(c.discarded, ["thread"]);
  assert.equal(c.mascots.at(-1).drafting, false);
  assert.equal(c.mascots.at(-1).activity, null);
});

test("task hints prioritize blocked-send alerts over the generic follow-up hint", async () => {
  const c = await composer(); c.ui.input.value = "a".repeat(17000); c.update();
  assert.match(c.ui.hint.textContent, /Ce message est trop long/);
  assert.equal(c.ui.send.disabled, true);
  c.ui.input.value = "Encore";
  c.conversation.messages = [{ role: "user", state: "unknown" }]; c.update();
  assert.match(c.ui.hint.textContent, /Vérifiez le dernier envoi/);
  c.conversation.messages = []; c.update();
  assert.match(c.ui.hint.textContent, /Suivi réservé à cette sous-tâche/);
  assert.equal(c.ui.send.title, "Envoyer le message");
});

test("leaving a finished task restores the ordinary group and chat composer", async () => {
  const c = await composer(); c.ctx.active = false; c.update();
  assert.equal(c.panels.openConversation("group"), true);
  c.ui.input.value = "Bonjour au groupe"; c.update();
  assert.equal(c.ui.input.readOnly, false);
  assert.equal(c.ui.input.placeholder, "Qu’avez-vous en tête ?");
  assert.equal(c.ui.send.dataset.mode, "send");
  assert.equal(c.ui.send.disabled, false);
  c.conversation.kind = "chat"; c.update();
  assert.equal(c.ui.send.dataset.mode, "stop");
});
