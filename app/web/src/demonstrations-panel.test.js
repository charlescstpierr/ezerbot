import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

// Execute the actual vanilla panel with controlled refresh timing, without a
// browser/media backend. In particular, there is no incidental poll after busy
// becomes false to hide a stale disabled menu or lost focus.
function fixture(status = "review", connected = true) {
  let document;
  const walk = el => [el, ...el.children.flatMap(walk)];
  const matches = (el, selector) => selector.startsWith(".") ? el.className.split(/\s+/).includes(selector.slice(1))
    : selector.startsWith("[name=") ? el.name === selector.slice(6, -1)
      : selector === "dialog[open]" ? el.tagName === "DIALOG" && el.open
        : el.tagName === selector.toUpperCase();
  class Element {
    constructor(tag) { this.tagName = tag.toUpperCase(); this.children = []; this.attributes = {}; this.dataset = {}; this.className = ""; this.text = ""; this.open = false; this.disabled = false; this.classList = { add: value => { this.className += ` ${value}`; } }; }
    append(...children) { for (const child of children) { if (child.parent) child.parent.children = child.parent.children.filter(el => el !== child); child.parent = this; this.children.push(child); } }
    prepend(child) { this.append(child); this.children.unshift(this.children.pop()); }
    after(child) { this.parent.append(child); this.parent.children.splice(this.parent.children.indexOf(this) + 1, 0, this.parent.children.pop()); }
    replaceChildren(...children) { if (this.contains(document.activeElement)) document.activeElement = this; for (const child of this.children) child.parent = null; this.children = []; this.text = ""; this.append(...children); }
    set textContent(text) { this.text = text; this.children = []; }
    get textContent() { return this.text + this.children.map(child => child.textContent).join(""); }
    setAttribute(name, value) { this.attributes[name] = value; }
    contains(child) { return Boolean(child) && walk(this).includes(child); }
    querySelectorAll(selector) { const selectors = selector.split(",").map(s => s.trim()); return walk(this).slice(1).filter(el => selectors.some(s => matches(el, s))); }
    querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
    focus() { if (!this.disabled) document.activeElement = this; }
    setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; }
    showModal() { this.open = true; (this.querySelectorAll("button, input, textarea").find(el => !el.disabled) || this).focus(); }
    close() { this.open = false; if (this.contains(document.activeElement)) document.activeElement = null; }
  }
  const root = new Element("div"), topbar = new Element("header"), actions = new Element("div"), menu = new Element("div"), learn = new Element("button"), panels = new Element("div");
  topbar.className = "topbar"; actions.className = "topbar-actions"; menu.id = "conversation-menu"; learn.id = "learn-skill-button"; panels.id = "panels";
  const menuToggle = new Element("button"); menuToggle.id = "conversation-menu-button";
  root.append(topbar, actions, menu, panels, menuToggle); menu.append(learn);
  document = { activeElement: null, createElement: tag => new Element(tag), createElementNS: (_ns, tag) => new Element(tag), getElementById: id => walk(root).find(el => el.id === id), querySelector: selector => walk(root).find(el => matches(el, selector)), querySelectorAll: selector => root.querySelectorAll(selector) };
  let registration, nextStatus = status, refreshes = 0;
  const requests = [], notifications = [];
  const session = state => state ? { id: "demo", bot_id: "bot", conversation_id: "chat", status: state, notice: "" } : null;
  const host = {
    state: { bots: [{ id: "bot", name: "Bot" }], provider: { connected }, demonstrations: { session: session(status), fake: true } },
    register(value) { registration = value; value.onState(this.state, context()); },
    async api(path, body) { requests.push([path, body]); nextStatus = path.endsWith("/stop") ? "review" : path.endsWith("/discard") ? null : path.endsWith("/start") ? "recording" : nextStatus; return {}; },
    async refresh() { refreshes++; this.state.demonstrations = { session: session(nextStatus), fake: true }; registration.onState(this.state, context()); },
    notify(text) { notifications.push(text); }, openConversation() {},
  };
  const context = () => ({ state: host.state, connected: true, bot: host.state.bots[0], conversation: { id: "chat", bot_id: "bot", kind: "chat" } });
  const window = { GrokbotPanels: host };
  runInNewContext(readFileSync(new URL("../../ui/panels/demonstrations.js", import.meta.url), "utf8"), { window, document, setInterval() {} });
  const dialog = walk(root).find(el => el.attributes["aria-labelledby"] === "demonstration-title");
  return { root, document, host, requests, notifications, get refreshes() { return refreshes; }, get panel() { return registration; }, renderState: () => registration.onState(host.state, context()), dialog, open: () => window.EzerbotDemonstration.open("bot"), button: text => walk(dialog).find(el => el.tagName === "BUTTON" && el.textContent === text), menu: document.getElementById("show-bot-button"), control: name => dialog.querySelector(`[name=${name}]`) };
}

for (const status of ["review", "drafting", "recording"]) test(`discarding ${status} unlocks Show the bot without another onState`, async () => {
  const f = fixture(status); f.open();
  await f.button(status === "review" ? "Ignorer et supprimer" : "Annuler et supprimer").onclick();
  assert.equal(f.refreshes, 1);
  assert.equal(f.panel.wantsPolling(), false);
  assert.equal(f.menu.disabled, false);
  f.menu.onclick();
  assert.equal(f.dialog.open, true);
  assert.ok(f.button("Commencer la démonstration"));
});

test("Stop focuses the explanation and later renders preserve the focused field and selection", async () => {
  const f = fixture("recording"); f.open();
  await f.button("Arrêter").onclick();
  const input = f.control("demonstration-explanation");
  assert.equal(f.document.activeElement, input);
  input.value = "Une explication"; input.oninput(); input.setSelectionRange(2, 5);
  f.host.state.demonstrations.session.notice = "Notice actualisée";
  f.renderState();
  const updated = f.control("demonstration-explanation");
  assert.notEqual(updated, input);
  assert.equal(f.document.activeElement, updated);
  assert.equal(updated.value, "Une explication");
  assert.equal(updated.selectionStart, 2); assert.equal(updated.selectionEnd, 5);
});

test("disconnected consent cannot create an empty thread or start a recording", async () => {
  const f = fixture(null, false); f.open();
  f.control("demonstration-consent").checked = true;
  f.control("demonstration-consent").onchange();
  assert.equal(f.button("Commencer la démonstration").disabled, true);
  await f.button("Commencer la démonstration").onclick();
  assert.equal(f.requests.length, 0);
  assert.match(f.dialog.textContent, /Connectez un fournisseur d’IA avant de montrer une tâche/);
});
