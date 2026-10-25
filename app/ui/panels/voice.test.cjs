const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const path = require("node:path");

class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.dataset = {}; this.attributes = {}; this.style = { setProperty() {} }; }
  append(...children) { for (const child of children) { child.parent = this; this.children.push(child); } }
  before(child) { this.parent.append(child); }
  remove() { this.parent.children = this.parent.children.filter((child) => child !== this); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener() {}
}
const descendants = (el) => [el, ...el.children.flatMap(descendants)];

function harness({ conversation = null, omitCreated = false } = {}) {
  const root = new Element("form"), send = new Element("button");
  root.append(send);
  const notifications = [], requests = [], native = [];
  let serial = 0, draft = "", token, created, pendingCreation;
  let ctx = { conversation, bot: { id: "bot" }, state: { conversations: conversation ? [conversation] : [], voice: { drafts: [] } } };
  const publish = (next = ctx) => { ctx = next; panels.panel.onState(ctx.state, ctx); };
  const transfer = (nextConversation) => {
    const next = { ...ctx, conversation: nextConversation };
    panels.panel.composer.transfer?.({ from: ctx.conversation?.id || `new:${ctx.bot.id}`, to: nextConversation.id }, next);
    publish(next);
  };
  const panels = {
    register(panel) { this.panel = panel; },
    notify(message) { notifications.push(message); },
    notifyDraft(message) { notifications.push(message); },
    composerChanged() {}, draft: () => draft, draftDocument: () => null,
    appendDraft(text) { draft += text; },
    async api(url, body) {
      requests.push({ url, body });
      if (this.reject?.(url)) throw new Error(`Refus ${url}`);
      if (url === "/api/conversations") { created = { id: "created", bot_id: "bot" }; return { conversation: created }; }
      if (url === "/api/voice/memos") return { id: `memo-${++serial}`, duration_ms: 1000, ...body };
      if (url === "/api/voice/remove") return {};
      throw new Error(`Unexpected ${url}`);
    },
    async refresh() {
      if (this.refreshFails) throw new Error("Refresh interrompu");
      if (created && !omitCreated) ctx.state.conversations = [created];
      publish();
    },
    openConversation(id) {
      const found = ctx.state.conversations.find((item) => item.id === id);
      if (!found) return false;
      publish({ ...ctx, conversation: found });
      return true;
    },
    async ensureDraftConversation() {
      if (ctx.conversation) return ctx.conversation;
      if (!pendingCreation) pendingCreation = this.api("/api/conversations", { bot_id: ctx.bot.id }).then(({ conversation: value }) => {
        ctx.state.conversations = [value];
        transfer(value);
        return value;
      }).finally(() => { pendingCreation = null; });
      return pendingCreation;
    },
  };
  const window = {
    GrokbotPanels: panels, addEventListener() {}, dispatchEvent() {},
    webkit: { messageHandlers: { ezerbotVoice: { async postMessage(body) {
      native.push(body);
      if (body.command === "start") { token = body.token; return { token, status: "recording" }; }
      if (body.command === "stop") return { token, status: "ready", data: "synthetic-audio", transcript: `transcript-${serial + 1}`, language: "fr" };
      return {};
    } } } },
  };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "voice.js"), "utf8"), {
    window, document: { documentElement: { lang: "fr" }, createElement: (tag) => new Element(tag), createTextNode: (text) => Object.assign(new Element("text"), { textContent: text }), getElementById: (id) => id === "send-button" ? send : null, addEventListener() {} },
    crypto: { randomUUID: () => `capture-${native.length}` }, Event: class {}, setTimeout: () => 1, clearTimeout() {},
    localStorage: { setItem() {}, removeItem() {} },
  });
  panels.panel.composer.mount(root, ctx);
  const by = (predicate) => descendants(root).find(predicate);
  const mic = by((el) => el.id === "voice-microphone");
  by((el) => el.type === "checkbox").checked = true;
  return {
    panels, notifications, requests, native, publish, transfer,
    start: () => mic.onclick({ detail: 0 }), stop: () => mic.onclick({ detail: 0 }),
    retry: () => by((el) => el.textContent === "Réessayer"),
    chips: () => descendants(root).filter((el) => el.className === "voice-memo-chip").map((el) => el.dataset.id),
    extras: () => JSON.parse(JSON.stringify(panels.panel.sendExtras())),
    context: () => ctx, draft: () => draft,
    navigate(conversation, conversations = conversation ? [conversation] : []) { publish({ ...ctx, conversation, state: { ...ctx.state, conversations } }); },
  };
}

test("T17 R23 memo survives a created thread omitted from refresh and another onState before retry", async () => {
  const h = harness({ omitCreated: true });
  h.panels.reject = (url) => url === "/api/voice/memos";
  await h.start(); await h.stop();
  h.publish();
  assert.equal(h.retry().hidden, false, "failed memo remains retryable after onState");
  h.panels.reject = null;
  await h.retry().onclick();
  assert.deepEqual(h.chips(), ["memo-1"]);
  assert.equal(h.draft(), "transcript-1");
  assert.equal(h.requests.filter((request) => request.url === "/api/conversations").length, 1);
});

test("T17 creation refusal preserves the captured result for retry", async () => {
  const h = harness();
  h.panels.reject = (url) => url === "/api/conversations";
  await h.start(); await h.stop(); h.publish();
  assert.equal(h.retry().hidden, false);
  h.panels.reject = null;
  await h.retry().onclick();
  assert.deepEqual(h.chips(), ["memo-1"]);
  assert.equal(h.native.filter((request) => request.command === "start").length, 1);
});

test("T17 created memo does not depend on a successful follow-up refresh", async () => {
  const h = harness(); h.panels.refreshFails = true;
  await h.start(); await h.stop(); h.publish();
  assert.deepEqual(h.chips(), ["memo-1"]);
  assert.equal(h.draft(), "transcript-1");
});

test("N05 T24 recent capture follows the automatic transfer of the same draft", async () => {
  const h = harness();
  await h.start();
  h.transfer({ id: "submitted", bot_id: "bot" });
  await h.stop();
  assert.deepEqual(h.chips(), ["memo-1"]);
  assert.equal(h.requests.find((request) => request.url === "/api/voice/memos").body.conversation_id, "submitted");
  assert.equal(h.native.filter((request) => request.command === "start").length, 1);
});

test("R19 T14 prepared memos survive stale snapshots and old receipts consume only the submitted memo", async () => {
  const owner = { id: "owner" }, h = harness({ conversation: owner });
  await h.start(); await h.stop();
  const submitted = h.extras();
  await h.start(); await h.stop(); h.publish();
  assert.deepEqual(h.chips(), ["memo-1", "memo-2"]);
  h.navigate({ id: "other" }, [owner, { id: "other" }]);
  h.panels.panel.composer.onSent({ extras: submitted });
  h.navigate(owner);
  assert.deepEqual(h.chips(), ["memo-2"]);
  assert.deepEqual(h.extras(), { attachments: ["memo-2"] });
});

test("R19 T17 a context change announces abandonment of a pending memo", async () => {
  const h = harness({ conversation: { id: "owner" } });
  h.panels.reject = (url) => url === "/api/voice/memos";
  await h.start(); await h.stop();
  h.navigate({ id: "other" });
  assert.equal(h.retry().hidden, true);
  assert.match(h.notifications.at(-1), /mémo.*(?:abandonné|perdu|pas été conservé)/i);
});

test("R19 T16 discard announces a pending memo before removing its retry", async () => {
  const h = harness({ conversation: { id: "owner" } });
  h.panels.reject = (url) => url === "/api/voice/memos";
  await h.start(); await h.stop();
  h.panels.panel.composer.discard();
  assert.equal(h.retry().hidden, true);
  assert.match(h.notifications.at(-1), /mémo.*(?:abandonné|perdu|pas été conservé)/i);
});

test("R19 T09 prepared memo hidden by an absent owner is announced and remains recoverable", async () => {
  const owner = { id: "owner" }, h = harness({ conversation: owner });
  await h.start(); await h.stop();
  h.navigate(null);
  assert.match(h.notifications.at(-1), /mémo.*(?:introuvable|inaccessible|indisponible)/i);
  h.navigate(owner);
  assert.deepEqual(h.chips(), ["memo-1"]);
});
