// Run with: node --test app/ui/select-bot.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class Element {
  constructor(tag, document) {
    this.ownerDocument = document;
    this.tagName = String(tag).toUpperCase();
    this.localName = String(tag).toLowerCase();
    this.children = [];
    this.attributes = {};
    this.listeners = {};
    this.className = "";
    this.hidden = false;
    this.open = false;
    this.disabled = false;
    this.isConnected = true;
    this.id = "";
    this.style = { setProperty() {} };
    this.classList = {
      add: (...names) => { this.className = [...new Set(`${this.className} ${names.join(" ")}`.trim().split(/\s+/))].join(" "); },
      remove: (...names) => { this.className = this.className.split(/\s+/).filter((name) => !names.includes(name)).join(" "); },
      toggle: (name) => { const has = this.classList.contains(name); if (has) this.classList.remove(name); else this.classList.add(name); return !has; },
      contains: (name) => this.className.split(/\s+/).includes(name),
    };
    const attributes = this.attributes;
    this.dataset = new Proxy({}, {
      set(target, key, value) {
        target[key] = String(value);
        attributes[`data-${String(key).replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}`] = String(value);
        return true;
      },
      get: (target, key) => target[key],
      has: (target, key) => key in target,
      deleteProperty(target, key) {
        delete target[key];
        delete attributes[`data-${String(key).replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}`];
        return true;
      },
    });
  }
  append(...children) {
    for (const child of children) if (child instanceof Element) child.parentElement = this;
    this.children.push(...children);
  }
  prepend(...children) { this.children.unshift(...children); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter((child) => child !== this); }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  getAttribute(key) { return this.attributes[key]; }
  removeAttribute(key) { delete this.attributes[key]; }
  addEventListener(name, action) { (this.listeners[name] ||= []).push(action); }
  contains(node) { return this === node || this.children.some((child) => child instanceof Element && child.contains(node)); }
  focus() {}
  select() {}
  scrollIntoView() {}
  showModal() { this.open = true; }
  close() { this.open = false; }
  getBoundingClientRect() { return { left: 0, top: 0, right: 200, bottom: 40, width: 200, height: 40 }; }
  matches(selector) { return splitSelectors(selector).some((part) => matchesCompound(this, part)); }
  querySelectorAll(selector) { return collect(this.children, selector); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  set textContent(text) { this._text = String(text); this.children = [String(text)]; }
  get textContent() { return this.children.map((child) => child?.textContent ?? String(child)).join(""); }
  set value(value) { this._value = String(value); }
  get value() { return this._value || ""; }
}

function splitSelectors(selector) {
  const parts = [];
  let current = "";
  let depth = 0;
  for (const char of selector) {
    if (char === "[") depth += 1;
    else if (char === "]") depth -= 1;
    if (char === "," && depth === 0) { parts.push(current.trim()); current = ""; }
    else current += char;
  }
  if (current.trim()) parts.push(current.trim());
  return parts;
}

function matchesCompound(element, selector) {
  if (!(element instanceof Element)) return false;
  const tag = /^[a-z]+/i.exec(selector)?.[0];
  if (tag && element.tagName !== tag.toUpperCase()) return false;
  const classes = [...selector.matchAll(/\.([a-zA-Z0-9_-]+)/g)].map((match) => match[1]);
  if (classes.some((name) => !element.className.split(/\s+/).includes(name))) return false;
  const attrs = [...selector.matchAll(/\[([a-zA-Z-]+)(?:="?([^"\]]+)"?)?\]/g)];
  for (const [, name, value] of attrs) {
    const actual = name === "open" && element.open ? "" : element.attributes[name];
    if (value === undefined) { if (actual === undefined && !(name === "open" && element.open)) return false; }
    else if (String(actual) !== value) return false;
  }
  return Boolean(tag || classes.length || attrs.length);
}

function collect(nodes, selector) {
  const found = [];
  for (const alternative of splitSelectors(selector)) {
    const parts = alternative.split(/\s+/).filter(Boolean);
    walk(nodes, parts, found);
  }
  return found;
}

function walk(nodes, parts, found) {
  for (const node of nodes) {
    if (!(node instanceof Element)) continue;
    if (matchesCompound(node, parts[0])) {
      if (parts.length === 1) found.push(node);
      else walk(node.children, parts.slice(1), found);
    }
    walk(node.children, parts, found);
  }
}

function element(document, tag, className) {
  const node = new Element(tag, document);
  if (className) node.className = className;
  return node;
}

function child(document, parent, tag, className) {
  const node = element(document, tag, className);
  parent.append(node);
  return node;
}

function boot({ attachments = false, draft = "Brouillon du fil" } = {}) {
  const storage = new Map([
    ["grokbot.selected", "conv-1"],
    ["grokbot.bot", "bot-a"],
    ["grokbot.draft.conv-1", draft],
  ]);
  const state = {
    state_revision: 1,
    account: { name: "Espace personnel" },
    mode: "simulated",
    preferences: { theme: "dark", accent: "citron" },
    provider: { connected: false, signin: "idle", models: [], backend: "sharing" },
    conversations: [{
      id: "conv-1", title: "Fil vide", bot_id: "bot-a", kind: "chat", favorite: false,
      messages: [], updated_at: "2026-10-21T00:00:00Z",
    }],
    bots: [
      { id: "bot-a", name: "Alpha", color: "citron", shape: "galet", deleted: false },
      { id: "bot-b", name: "Bêta", color: "orange", shape: "carre", deleted: false },
    ],
  };
  const byId = {};
  const document = {
    title: "",
    activeElement: null,
    documentElement: new Element("html"),
    createElement: (tag) => new Element(tag, document),
    createElementNS: (_ns, tag) => new Element(tag, document),
    createTextNode: (text) => String(text),
    getElementById: (id) => byId[id] || null,
    querySelector(selector) { return this.querySelectorAll(selector)[0] || null; },
    querySelectorAll(selector) { return collect([this.body, this.documentElement], selector); },
    addEventListener() {},
  };
  document.body = new Element("body", document);
  const ids = ["conversation-list", "search", "send-button", "welcome", "messages", "chat-scroll", "conversation-title", "conversation-menu", "conversation-menu-button", "composer-hint", "connection-banner", "connection-message", "toast", "welcome-description", "bot-extensions", "settings-extensions", "bot-dialog", "composer-attachments", "link-error-message", "link-error-dialog", "settings-dialog", "provider-section", "settings-appearance", "settings-bots", "settings-shortcuts", "new-chat", "restore-link-draft", "learn-skill-button", "bot-chips", "bot-list", "bot-dialog-title", "bot-save", "bot-name", "bot-description", "bot-instructions", "bot-delete-button", "bot-copy-link", "bot-colors", "bot-shapes", "account-name", "account-avatar", "workspace-label", "edit-bot-button", "favorite-button", "picker", "mode-badge-text", "plan-banner", "composer-tools", "mode-note", "plan-dialog", "plan-title", "composer-input", "composer-form", "plan-acknowledge", "sidebar-toggle", "sidebar-close", "sidebar-scrim", "settings-button", "simulate-loss", "rename-button", "rename-input", "rename-dialog", "rename-form", "copy-link-button", "export-button", "bot-form", "bot-delete-dialog", "confirm-bot-delete", "delete-button", "delete-dialog", "confirm-delete", "reconnect-button", "home-link", "favorites-filter", "bot-preview-mascot", "sidebar-bot-list", "composer-bot", "bot-menu", "composer-bot-name"];
  for (const id of ids) {
    const node = new Element("div", document);
    node.id = id;
    byId[id] = node;
    document.body.append(node);
  }
  child(document, byId["favorite-button"], "span");
  child(document, byId["plan-banner"], "span");
  child(document, byId["send-button"], "use");
  const editor = child(document, byId["composer-input"], "div", "gb-editor");
  const input = {
    value: "", document: null, mentions: [],
    setReadOnly() {}, setPlaceholder() {}, setInvalid() {}, focus() {},
  };
  editor.editor = { commands: { clearContent() { input.value = ""; } } };
  child(document, byId["plan-dialog"], "a");
  child(document, byId["plan-dialog"], "p", "delete-description");
  const stage = child(document, document.body, "div");
  const hero = child(document, stage, "div");
  hero.dataset.mascot = "hero";
  child(document, document.body, "div", "main-panel");
  child(document, document.body, "div", "bot-picker");
  child(document, document.body, "div", "composer-dock");
  byId["chat-scroll"].scrollHeight = 0;
  byId["chat-scroll"].scrollTop = 0;
  byId["chat-scroll"].clientHeight = 0;
  byId["welcome-description"].textContent = "Bienvenue";
  if (attachments) child(document, byId["composer-attachments"], "span", "attachment-chip");

  const window = {
    GrokbotChat: {
      createDeepLinkQueue: () => ({ receive() {}, setReady() {} }),
      createApprovalFollow: () => ({ arm() {}, cancel() {} }),
      mountInput: () => input,
      isPendingReply: () => false,
      renderThread() {},
      searchableMessageText: () => "",
      revealPendingApproval() {},
      teachPrompt: "",
    },
    GrokbotMascot: { shapes: {}, setShape() {}, setContext() {}, celebrate() {}, create: () => new Element("span", document) },
    matchMedia: () => ({ matches: false, addEventListener() {} }),
    addEventListener() {},
  };
  const context = vm.createContext({
    window, document, console, URL, Blob, TextEncoder, innerHeight: 800,
    crypto: { randomUUID: () => "id-test" },
    localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value), removeItem: (key) => storage.delete(key) },
    navigator: { clipboard: { writeText: async () => {} } },
    fetch: async (path) => ({ ok: true, json: async () => path === "/api/ping" ? { revision: state.state_revision } : structuredClone(state) }),
    setTimeout: () => 0, clearTimeout() {}, requestAnimationFrame: (fn) => fn(),
    AbortController,
    ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
    Node: Element,
  });
  vm.runInContext(fs.readFileSync(`${__dirname}/app.js`, "utf8"), context);
  return { panels: window.GrokbotPanels, input, storage, toast: byId.toast, tick: () => new Promise((resolve) => process.nextTick(resolve)) };
}

test("selectBot refuses staged attachments on an empty thread", async () => {
  const { panels, input, storage, toast, tick } = boot({ attachments: true });
  await tick();
  input.value = "Brouillon du fil";
  const switched = panels.selectBot("bot-b");
  assert.equal(switched, false);
  assert.equal(storage.get("grokbot.bot"), "bot-a");
  assert.equal(input.value, "Brouillon du fil");
  assert.match(toast.textContent, /pièces jointes/);
});

test("selectBot leaves an empty thread before opening the new bot", async () => {
  const { panels, input, storage, tick } = boot();
  await tick();
  assert.equal(input.value, "Brouillon du fil");
  const switched = panels.selectBot("bot-b", "Relis ce diff.");
  assert.equal(switched, true);
  assert.equal(storage.get("grokbot.bot"), "bot-b");
  assert.equal(storage.get("grokbot.selected"), "");
  assert.equal(input.value, "Relis ce diff.");
  const kept = boot();
  await kept.tick();
  assert.equal(kept.panels.selectBot("bot-b"), true);
  assert.equal(kept.input.value, "Brouillon du fil");
});
