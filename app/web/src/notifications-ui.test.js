import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";

// Run the shipping panel, not a second implementation of its presentation logic.
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.attributes = {}; this.dataset = {}; this.className = ""; }
  set textContent(text) { this.text = String(text); this.children = []; }
  get textContent() { return (this.text || "") + this.children.map(child => child.textContent).join(""); }
  append(...children) { this.children.push(...children); }
  prepend(...children) { this.children.unshift(...children); }
  replaceChildren(...children) { this.text = ""; this.children = children; }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  addEventListener() {}
  querySelector() { return null; }
  getClientRects() { return this.rects || [{}]; }
  getBoundingClientRect() { return { height: this.height ?? 50 }; }
  focus() { this.focused = true; }
  showModal() { this.open = true; }
  close() { this.open = false; }
  remove() { this.removed = true; }
  scrollIntoView(options) { this.scrolled = options; }
  get classList() {
    const element = this;
    const toggle = (name, on) => { const classes = new Set(element.className.split(" ").filter(Boolean)); if (on) classes.add(name); else classes.delete(name); element.className = [...classes].join(" "); };
    return { toggle, add(name) { toggle(name, true); }, remove(name) { toggle(name, false); } };
  }
}
const descendants = root => [root, ...root.children.flatMap(descendants)];
const find = (root, predicate) => descendants(root).find(predicate);
const byClass = (root, cls) => find(root, el => el.className.split(" ").includes(cls));
const flush = () => new Promise(resolve => setImmediate(resolve));
async function fixture({ enabled = true, dnd = false, items = [], status = "recording", search = "", bots = [] } = {}) {
  const top = new Element("div"), panels = new Element("div"), settings = new Element("div");
  const remote = { settings: { enabled, dnd, reply: true, approval: true, form: true, secret: true, routine: true, delegation: true }, items, unread: items.length, deliveries: [] };
  let interval, registration;
  const nodes = [], frames = [], calls = [], sendOutcomes = [], gate = {};
  const target = { id: "routine:occ", conversation_id: "chat", card_id: "reply-run" };
  const host = {
    register(value) { registration = value; value.settingsSection(settings); },
    calls,
    state: { conversations: [{ id: "chat", bot_id: "atlas" }], bots: [{ id: "atlas", name: "Atlas" }] },
    async api(path, body) {
      calls.push([path, body]);
      if (path === "/api/send") { if (gate.hold) await gate.hold; if (sendOutcomes.length) { const outcome = sendOutcomes.shift(); if (outcome) throw outcome; } return { ok: true }; }
      if (path.endsWith("/open")) return target; if (path.endsWith("/settings")) remote.settings = body; return JSON.parse(JSON.stringify(remote));
    },
    openConversation(id) { registration.onState({}, { conversation: { id } }); return true; },
    async refresh() {}, notify() {},
  };
  const document = { createElement: tag => new Element(tag), createElementNS: (ns, tag) => Object.assign(new Element(tag), { namespaceURI: ns }),
    getElementById: id => id === "chat-scroll" ? { clientHeight: 200 } : panels, querySelector: selector => selector === ".topbar-actions" ? top : null,
    querySelectorAll: selector => selector.includes("data-notification-card") ? nodes : [], hasFocus: () => true, visibilityState: "visible", addEventListener() {},
  };
  const window = { GrokbotPanels: host, location: { search, pathname: "/" }, history: { replaceState(_, __, url) { this.url = url; } }, webkit: { messageHandlers: { ezerbotNotifications: { async postMessage() { return { status }; } } } }, addEventListener() {} };
  runInNewContext(readFileSync(new URL("../../ui/panels/notifications.js", import.meta.url), "utf8"), {
    window, document, URLSearchParams, crypto: { randomUUID: (() => { let n = 0; return () => `message-${++n}`; })() }, setInterval(fn) { interval = fn; }, setTimeout() {}, requestAnimationFrame(fn) { frames.push(fn); },
  });
  await flush();
  return { top, panels, settings, remote, registration, window, nodes, frames, calls, sendOutcomes, gate, async poll() { await interval(); await flush(); } };
}
const item = (kind, title) => ({ id: `${kind}:1`, kind, title, card_id: "card", conversation_id: "chat", conversation_title: "Conversation", created_at: Date.now() });

test("unread count is a pill badge and action requests have a distinct label and local time", async () => {
  const f = await fixture({ items: [item("reply", "Réponse terminée"), item("form", "Informations demandées")] });
  assert.ok(byClass(f.top, "tool-pill"));
  assert.equal(byClass(f.top, "notification-count").textContent, "2");
  assert.equal(byClass(f.top, "notification-count").hidden, false);
  assert.equal(descendants(f.panels).filter(el => el.className === "notification-action-required").length, 1);
  assert.equal(byClass(f.panels, "notification-action-required").textContent, "Action requise");
  const time = find(f.panels, el => el.tagName === "time");
  assert.ok(time.dateTime);
  assert.ok(!time.textContent.includes(String(new Date().getFullYear())));
  f.remote.items = []; f.remote.unread = 0; await f.poll();
  assert.equal(byClass(f.top, "notification-count").hidden, true);
  assert.equal(byClass(f.panels, "dialog-footer").hidden, true);
});

test("DND explains suspension and mutes all six type switches without losing their saved values", async () => {
  const f = await fixture({ dnd: true });
  assert.match(f.settings.textContent, /Ne pas déranger est actif : les alertes sont suspendues/);
  const labels = descendants(f.settings).filter(el => el.className.includes("is-muted"));
  assert.equal(labels.length, 6);
  for (const row of labels) assert.equal(find(row, el => el.tagName === "input").checked, true);
  assert.match(byClass(f.top, "notifications-button").attributes["aria-label"], /Ne pas déranger actif/);
  f.remote.settings.dnd = false; f.remote.settings.enabled = false; await f.poll();
  assert.equal(descendants(f.settings).filter(el => el.className.includes("is-muted")).length, 6);
});

test("OS denial is an explicit alert with a warning icon and a disabled master preference", async () => {
  const f = await fixture({ enabled: false, status: "denied" });
  const input = find(f.settings, el => el.dataset.notificationSetting === "enabled");
  input.checked = true; await input.onchange(); await flush();
  const warning = byClass(f.settings, "notification-warning");
  assert.equal(warning.role, "alert");
  assert.ok(find(warning, el => el.tagName === "svg"));
  assert.match(warning.textContent, /Notifications refusées par macOS/);
  assert.equal(f.remote.settings.enabled, false);
});

test("tool-only routine completion focuses the existing visible task, never its empty terminal snapshot", async () => {
  const f = await fixture();
  const visible = new Element("div"); visible.dataset.runId = "run"; visible.height = 350;
  const empty = new Element("div"); empty.dataset.runId = "run"; empty.rects = [];
  f.nodes.push(visible, empty);
  await f.window.EzerbotNotifications.activate("routine:occ");
  f.frames.forEach(fn => fn());
  assert.equal(visible.focused, true);
  assert.equal(empty.focused, undefined);
  assert.equal(visible.scrolled.block, "start");
  assert.ok(visible.className.includes("notification-target"));
});

test("dialog scroll stays in the list and target focus uses theme ink without the system outline", () => {
  const css = readFileSync(new URL("../../ui/panels/notifications.css", import.meta.url), "utf8");
  assert.match(css, /\.notifications-dialog\[open\] \{ display: flex;/);
  assert.match(css, /\.notification-list \{[^}]*overflow-y: auto;[^}]*min-height: 0;/);
  assert.match(css, /\[data-notification-card\]:focus, \[data-message-id\]:focus \{ outline: none;/);
  assert.match(css, /\.notification-target \{[^}]*inset 0 0 0 2px var\(--accent-ink\)/);
  assert.match(css, /\.notification-target::after \{[^}]*z-index: 2;[^}]*border: 2px solid var\(--accent-ink\);[^}]*pointer-events: none;/);
});

const sheet = f => descendants(f.panels).find(el => el.className.includes("reply-sheet"));
const submit = async (f, text) => {
  const form = find(sheet(f), el => el.tagName === "form");
  find(sheet(f), el => el.tagName === "textarea").value = text;
  const first = form.onsubmit({ preventDefault() {} }), second = form.onsubmit({ preventDefault() {} });
  await Promise.all([first, second]); await flush();
};
const sent = f => JSON.parse(JSON.stringify(f.calls.filter(([path]) => path === "/api/send")));

test("Répondre opens a sheet on that conversation, named for its bot, field ready", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  const dialog = sheet(f), field = find(dialog, el => el.tagName === "textarea");
  assert.equal(dialog.open, true);
  assert.equal(byClass(dialog, "dialog-header").textContent.startsWith("Répondre à Atlas"), true);
  assert.equal(field.focused, true);
  assert.equal(find(dialog, el => el.tagName === "button" && el.textContent === "Envoyer").disabled, undefined);
  assert.deepEqual(sent(f), []);
});

test("a plain tap opens the conversation without any sheet", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ");
  assert.equal(sheet(f), undefined);
  assert.ok(f.calls.some(([path]) => path === "/api/notifications/open"));
});

test("the reply is sent once to that conversation, even when Envoyer is pressed twice", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  await submit(f, "  Vu depuis la notification ");
  assert.deepEqual(sent(f), [["/api/send", { conversation_id: "chat", message_id: "message-1", body: "Vu depuis la notification" }]]);
  assert.equal(sheet(f).open, false);
});

test("an empty reply sends nothing", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  await submit(f, "   ");
  assert.deepEqual(sent(f), []);
  assert.equal(sheet(f).open, true);
});

test("with the Mac out of reach the sheet says so, keeps the text and a retry reuses the same message", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  f.sendOutcomes.push(new Error("La connexion au moteur local est interrompue."));
  await submit(f, "Vu depuis la notification");
  const status = byClass(sheet(f), "notification-warning");
  assert.match(status.textContent, /Le Mac est injoignable\. Ce message n’est pas marqué envoyé et n’est pas mis en file/);
  assert.equal(status.role, "alert");
  assert.equal(sheet(f).open, true);
  assert.equal(find(sheet(f), el => el.tagName === "textarea").value, "Vu depuis la notification");
  await submit(f, "Vu depuis la notification");
  assert.deepEqual(sent(f).map(([, body]) => body.message_id), ["message-1", "message-1"]);
  assert.equal(sheet(f).open, false);
});

test("a refusal from the Mac is shown as the Mac wrote it", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  f.sendOutcomes.push(Object.assign(new Error("L’accès iPhone est désactivé sur le Mac."), { status: 403 }));
  await submit(f, "Bonjour");
  assert.equal(byClass(sheet(f), "notification-warning").textContent, "L’accès iPhone est désactivé sur le Mac.");
});

test("opening the app from a push shows the reply sheet, or only the conversation for a plain tap", async () => {
  const reply = await fixture({ search: "?notification=routine%3Aocc&reply=1" });
  await reply.poll(); await flush(); await flush();
  assert.equal(sheet(reply).open, true);
  assert.equal(reply.window.history.url, "/");
  const plain = await fixture({ search: "?notification=routine%3Aocc" });
  await plain.poll(); await flush(); await flush();
  assert.equal(sheet(plain), undefined);
  assert.ok(plain.calls.some(([path]) => path === "/api/notifications/open"));
});

test("the field cannot be edited while the reply is on its way, and is editable again if it fails", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  const field = find(sheet(f), el => el.tagName === "textarea"), form = find(sheet(f), el => el.tagName === "form");
  let release; f.gate.hold = new Promise(resolve => { release = resolve; });
  field.value = "Vu depuis la notification";
  const pending = form.onsubmit({ preventDefault() {} }); await flush();
  assert.equal(field.readOnly, true);
  f.sendOutcomes.push(new Error("La connexion au moteur local est interrompue."));
  release(); await pending; await flush();
  assert.equal(field.readOnly, false);
  assert.equal(sheet(f).open, true);
});

const offer = f => descendants(f.panels).find(el => el.className === "reply-offer" && !el.removed);

test("a plain tap on a push offers Répondre on the conversation, and the offer opens the sheet", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("reply:run-1", { push: true });
  assert.equal(sheet(f), undefined);
  const button = find(offer(f), el => el.tagName === "button");
  assert.equal(button.textContent, "Répondre à Atlas");
  button.onclick();
  assert.equal(sheet(f).open, true);
  assert.equal(offer(f), undefined);
});

test("opening a reply from the notification list, or a non-reply push, offers nothing", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("reply:run-1");
  await f.window.EzerbotNotifications.activate("routine:occ", { push: true });
  assert.equal(offer(f), undefined);
});

test("leaving the conversation withdraws the offer", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("reply:run-1", { push: true });
  assert.ok(offer(f));
  f.registration.onState({}, { conversation: { id: "autre" } });
  assert.equal(offer(f), undefined);
});

test("a cold start from a plain push tap offers Répondre", async () => {
  const f = await fixture({ search: "?notification=reply%3Arun-1" });
  await f.poll(); await flush(); await flush();
  assert.ok(offer(f));
  assert.equal(sheet(f), undefined);
});

test("a send that got no answer stays unconfirmed: a reopened sheet reuses its message id", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  f.sendOutcomes.push(new Error("La connexion au moteur local est interrompue."));
  await submit(f, "Vu depuis la notification");
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  await submit(f, "Vu depuis la notification");
  assert.deepEqual(sent(f).map(([, body]) => body.message_id), ["message-1", "message-1"]);
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  await submit(f, "Un autre texte");
  assert.equal(sent(f)[2][1].message_id, "message-2");
});

test("a late failure of an earlier reply never shows on the sheet opened since", async () => {
  const f = await fixture();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  const sheets = () => descendants(f.panels).filter(el => el.className.includes("reply-sheet"));
  const [first] = sheets();
  let release; f.gate.hold = new Promise(resolve => { release = resolve; });
  find(first, el => el.tagName === "textarea").value = "Premier";
  const pending = find(first, el => el.tagName === "form").onsubmit({ preventDefault() {} }); await flush();
  await f.window.EzerbotNotifications.activate("routine:occ", { reply: true });
  const second = sheets().at(-1);
  f.sendOutcomes.push(Object.assign(new Error("Refus du Mac."), { status: 403 }));
  release(); await pending; await flush();
  assert.match(byClass(first, "notification-warning").textContent, /Refus du Mac/);
  assert.equal(byClass(second, "notification-warning"), undefined);
  assert.equal(find(second, el => el.tagName === "p").textContent, "");
});
