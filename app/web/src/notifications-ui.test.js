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
async function fixture({ enabled = true, dnd = false, items = [], status = "recording" } = {}) {
  const top = new Element("div"), panels = new Element("div"), settings = new Element("div");
  const remote = { settings: { enabled, dnd, reply: true, approval: true, form: true, secret: true, routine: true, delegation: true }, items, unread: items.length, deliveries: [] };
  let interval, registration;
  const nodes = [], frames = [];
  const target = { id: "routine:occ", conversation_id: "chat", card_id: "reply-run" };
  const host = {
    register(value) { registration = value; value.settingsSection(settings); },
    async api(path, body) { if (path.endsWith("/open")) return target; if (path.endsWith("/settings")) remote.settings = body; return JSON.parse(JSON.stringify(remote)); },
    openConversation(id) { registration.onState({}, { conversation: { id } }); return true; },
    async refresh() {}, notify() {},
  };
  const document = { createElement: tag => new Element(tag), createElementNS: (ns, tag) => Object.assign(new Element(tag), { namespaceURI: ns }),
    getElementById: id => id === "chat-scroll" ? { clientHeight: 200 } : panels, querySelector: selector => selector === ".topbar-actions" ? top : null,
    querySelectorAll: selector => selector.includes("data-notification-card") ? nodes : [], hasFocus: () => true, visibilityState: "visible", addEventListener() {},
  };
  const window = { GrokbotPanels: host, webkit: { messageHandlers: { ezerbotNotifications: { async postMessage() { return { status }; } } } }, addEventListener() {} };
  runInNewContext(readFileSync(new URL("../../ui/panels/notifications.js", import.meta.url), "utf8"), {
    window, document, setInterval(fn) { interval = fn; }, setTimeout() {}, requestAnimationFrame(fn) { frames.push(fn); },
  });
  await flush();
  return { top, panels, settings, remote, registration, window, nodes, frames, async poll() { await interval(); await flush(); } };
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
