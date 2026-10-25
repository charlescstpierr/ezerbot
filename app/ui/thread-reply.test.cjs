// Run with: node --test app/ui/thread-reply.test.cjs
// T19 : « Répondre en fil » ouvre un sous-fil lié au message, annonce le parent, et le fil principal garde ce message.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const PARENT = "La capitale précise est Ottawa.";

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
    this.checked = false;
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
  setAttribute(key, value) { this.attributes[key] = String(value); if (key === "id") this.id = String(value); }
  getAttribute(key) { return this.attributes[key]; }
  removeAttribute(key) { delete this.attributes[key]; }
  hasAttribute(key) { return this.attributes[key] !== undefined; }
  addEventListener(name, action) { (this.listeners[name] ||= []).push(action); }
  contains(node) { return this === node || this.children.some((child) => child instanceof Element && child.contains(node)); }
  focus() { this.ownerDocument.activeElement = this; }
  select() { this.ownerDocument.selected = this; }
  click() { this.dispatch("click"); }
  showModal() { this.open = true; }
  close() { this.open = false; for (const handler of this.listeners.close || []) handler({}); }
  getBoundingClientRect() { return { left: 0, top: 0, right: 200, bottom: 40, width: 200, height: 40 }; }
  matches(selector) { return splitSelectors(selector).some((part) => matchesCompound(this, part)); }
  closest(selector) {
    let node = this;
    while (node instanceof Element) {
      if (node.matches(selector)) return node;
      node = node.parentElement;
    }
    return null;
  }
  querySelectorAll(selector) { return collect(this.children, selector); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  dispatch(type, extra = {}) {
    const event = { type, target: this, currentTarget: this, preventDefault() { event.defaultPrevented = true; }, stopPropagation() {}, ...extra };
    for (const handler of this.listeners[type] || []) handler(event);
    return event;
  }
  set textContent(text) { this._text = String(text); this.children = [String(text)]; }
  get textContent() { return this.children.map((child) => child?.textContent ?? String(child)).join(""); }
  set value(value) { this._value = String(value); }
  get value() { return this._value ?? this.attributes.value ?? ""; }
  reset() {
    for (const field of this.querySelectorAll("input, textarea")) {
      if (field.getAttribute("type") === "radio") field.checked = false;
      else field.value = "";
    }
  }
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
  for (const alternative of splitSelectors(selector)) walk(nodes, alternative.split(/\s+/).filter(Boolean), found);
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

function register(byId, node, id) {
  node.id = id;
  byId[id] = node;
  return node;
}


async function boot() {
  const storage = new Map([["grokbot.selected", "main"], ["grokbot.bot", "bot-a"]]);
  const parent = { id: "reply-q", role: "assistant", state: "echoed", created_at: "2026-10-21T12:00:01.000Z", body: PARENT, kind: "message" };
  const question = { id: "q", role: "user", state: "echoed", created_at: "2026-10-21T12:00:00.000Z", body: "Quelle est la capitale du Canada ?", kind: "message" };
  const state = {
    state_revision: 1,
    account: { name: "Espace personnel" },
    mode: "simulated",
    preferences: { theme: "dark", accent: "citron" },
    provider: { connected: false, signin: "idle", models: [], backend: "sharing", simulation: true },
    conversations: [{ id: "main", title: "Capitales", bot_id: "bot-a", kind: "chat", favorite: false, updated_at: "2026-10-21T12:00:01.000Z", messages: [question, parent] }],
    bots: [{ id: "bot-a", name: "Alpha", color: "citron", shape: "galet", deleted: false }],
    threads: {},
  };
  const calls = [];
  const byId = {};
  const document = {
    title: "",
    activeElement: null,
    listeners: {},
    documentElement: null,
    createElement: (tag) => new Element(tag, document),
    createElementNS: (_ns, tag) => new Element(tag, document),
    createTextNode: (text) => String(text),
    getElementById: (id) => byId[id] || null,
    querySelector(selector) { return this.querySelectorAll(selector)[0] || null; },
    querySelectorAll(selector) { return collect([this.body, this.documentElement], selector); },
    addEventListener(type, handler) { (this.listeners[type] ||= []).push(handler); },
  };
  document.documentElement = new Element("html", document);
  document.body = new Element("body", document);
  const ids = ["conversation-list", "search", "send-button", "welcome", "messages", "chat-scroll", "conversation-title", "conversation-menu", "conversation-menu-button", "composer-hint", "connection-banner", "connection-message", "toast", "welcome-description", "bot-extensions", "settings-extensions", "bot-dialog", "composer-attachments", "link-error-message", "link-error-dialog", "settings-dialog", "provider-section", "settings-appearance", "settings-bots", "settings-shortcuts", "new-chat", "restore-link-draft", "learn-skill-button", "bot-chips", "bot-list", "bot-dialog-title", "bot-save", "bot-name", "bot-description", "bot-instructions", "bot-delete-button", "bot-copy-link", "bot-colors", "bot-shapes", "account-name", "account-avatar", "workspace-label", "edit-bot-button", "favorite-button", "picker", "plan-banner", "composer-tools", "mode-note", "plan-dialog", "plan-title", "composer-input", "composer-form", "plan-acknowledge", "sidebar-toggle", "sidebar-close", "sidebar-scrim", "settings-button", "simulate-loss", "rename-button", "rename-input", "rename-dialog", "rename-form", "copy-link-button", "export-button", "export-ratings-button", "bot-form", "bot-delete-dialog", "confirm-bot-delete", "delete-button", "delete-dialog", "confirm-delete", "reconnect-button", "home-link", "favorites-filter", "bot-preview-mascot", "sidebar-bot-list", "composer-bot", "bot-menu", "composer-bot-name", "thread-banner", "thread-banner-title", "thread-banner-quote", "thread-banner-open"];
  for (const id of ids) {
    const node = new Element("div", document);
    node.id = id;
    byId[id] = node;
    document.body.append(node);
  }
  byId["thread-banner"].hidden = true;
  child(document, byId["favorite-button"], "span");
  child(document, byId["plan-banner"], "span");
  child(document, byId["send-button"], "use");
  const editor = child(document, byId["composer-input"], "div", "gb-editor");
  const input = {
    value: "", document: null, mentions: [],
    setReadOnly() {}, setPlaceholder() {}, setInvalid() {}, focus() { document.activeElement = byId["composer-input"]; },
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
  byId["chat-scroll"].scrollHeight = 400;
  byId["chat-scroll"].scrollTop = 0;
  byId["chat-scroll"].clientHeight = 400;

  // Le fil rendu : ce que l’utilisateur voit dans la zone des messages, par conversation affichée.
  let threadProps = null;
  const window = {
    GrokbotChat: {
      createDeepLinkQueue: () => ({ receive() {}, setReady() {} }),
      createApprovalFollow: () => ({ arm() {}, cancel() {} }),
      mountInput: () => input,
      isPendingReply: () => false,
      renderThread(_node, props) { threadProps = props; },
      connectionMessages: (messages) => messages,
      composerConnectionState: ({ offline = false, live = false, busy = false }) => ({ offline, live: live && !offline, busy: busy && !offline, mode: "send", icon: "#i-send", title: "Envoyer le message", label: "Envoyer le message" }),
      searchableMessageText: (message) => message.body || "",
      revealPendingApproval() {},
      findInRendered: () => [],
      cycleFind: (index) => index,
      findCountLabel: () => "",
      createRatingTable: () => ({ list: () => [], recordGood() {}, recordBad() {}, exportDocument: () => ({ notations: [] }) }),
    },
    GrokbotMascot: { shapes: {}, setShape() {}, setContext() {}, celebrate() {}, create: () => new Element("span", document) },
    matchMedia: () => ({ matches: false, addEventListener() {} }),
    addEventListener() {},
  };
  const localStorage = {
    getItem: (key) => storage.has(key) ? storage.get(key) : null,
    setItem: (key, value) => storage.set(key, String(value)),
    removeItem: (key) => storage.delete(key),
  };
  const context = vm.createContext({
    window, document, console, Blob, TextEncoder, innerHeight: 800,
    URL: { createObjectURL() { return "blob:x"; }, revokeObjectURL() {} },
    crypto: { randomUUID: () => `id-${storage.size}` },
    localStorage,
    navigator: { clipboard: { writeText: async () => {} } },
    fetch: async (path, options = {}) => {
      calls.push({ path: String(path), body: options.body || "" });
      if (String(path) === "/api/ping") return { ok: true, json: async () => ({ revision: state.state_revision }) };
      if (String(path) === "/api/messages/thread") {
        const sent = JSON.parse(options.body);
        if (sent.message_id !== "reply-q") return { ok: false, status: 404, json: async () => ({ error: "Message introuvable." }) };
        const sub = { id: "sub", title: "Fil : La capitale précise est Ottawa.", bot_id: "bot-a", kind: "chat", favorite: false, updated_at: "2026-10-21T12:05:00.000Z", messages: [] };
        const thread = { parent_conversation_id: "main", parent_message_id: "reply-q", parent_role: "assistant", parent_excerpt: PARENT };
        state.conversations.unshift(sub);
        state.threads.sub = thread;
        return { ok: true, json: async () => ({ conversation: structuredClone(sub), thread }) };
      }
      return { ok: true, json: async () => structuredClone(state) };
    },
    setTimeout: () => 0, clearTimeout() {}, requestAnimationFrame: (fn) => fn(),
    AbortController,
    ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
    Node: Element,
  });
  vm.runInContext(fs.readFileSync(`${__dirname}/app.js`, "utf8"), context);
  for (let attempt = 0; attempt < 20 && document.getElementById("conversation-title").textContent !== "Capitales"; attempt += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  return { document, calls, parent, window, get threadProps() { return threadProps; } };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test("Répondre en fil ouvre un sous-fil qui cite le parent, et le fil principal garde ce message", async () => {
  const app = await boot();
  const title = () => app.document.getElementById("conversation-title").textContent;
  const banner = app.document.getElementById("thread-banner");
  assert.equal(title(), "Capitales");
  assert.equal(banner.hidden, true, "le fil principal n’annonce aucun parent");
  assert.equal(typeof app.threadProps.onThread, "function");

  app.threadProps.onThread(app.parent);
  await settle();

  const sent = app.calls.find((call) => call.path === "/api/messages/thread");
  assert.deepEqual(JSON.parse(sent.body), { conversation_id: "main", message_id: "reply-q" });
  assert.equal(title(), "Fil : La capitale précise est Ottawa.");
  assert.equal(banner.hidden, false);
  assert.match(app.document.getElementById("thread-banner-quote").textContent, /Alpha : « La capitale précise est Ottawa\. »/);
  assert.equal(app.document.getElementById("thread-banner-open").disabled, false);
  assert.deepEqual(app.threadProps.messages, [], "le sous-fil démarre vide");

  app.document.getElementById("thread-banner-open").click();
  await settle();
  assert.equal(title(), "Capitales");
  assert.equal(banner.hidden, true);
  assert.deepEqual(app.threadProps.messages.map((message) => message.id), ["q", "reply-q"]);
  assert.equal(app.threadProps.messages[1].body, PARENT);
});

test("un message refusé par le moteur n’ouvre pas de fil et l’annonce", async () => {
  const app = await boot();
  app.threadProps.onThread({ ...app.parent, id: "absent" });
  await settle();
  assert.equal(app.document.getElementById("conversation-title").textContent, "Capitales");
  assert.equal(app.document.getElementById("thread-banner").hidden, true);
  assert.equal(app.document.getElementById("toast").textContent, "Message introuvable.");
});

test("« N souvenirs utilisés » : l'écran Mémoire reçoit la conversation affichée et la réponse cliquée, sous-fil compris", async () => {
  const app = await boot();
  const asked = [];
  const plain = (value) => JSON.parse(JSON.stringify(value)); // objets nés dans le contexte de app.js
  assert.deepEqual(plain(app.threadProps.onOpenMemory(["f1"], app.parent)), { ok: false, message: "L’écran Mémoire n’est pas disponible ici." }, "sans écran Mémoire (iPhone distant), la raison est dite");
  app.window.EzerbotMemorySpace = { openWith: (ids, origin) => { asked.push([ids, origin]); return { ok: true }; } };
  assert.deepEqual(app.threadProps.onOpenMemory(["f1", "f2"], app.parent), { ok: true });
  app.threadProps.onThread(app.parent);
  await settle();
  assert.equal(app.document.getElementById("conversation-title").textContent, "Fil : La capitale précise est Ottawa.");
  app.threadProps.onOpenMemory(["f3"], { id: "reply-sub" });
  assert.deepEqual(plain(asked), [[["f1", "f2"], { conversationId: "main", messageId: "reply-q" }], [["f3"], { conversationId: "sub", messageId: "reply-sub" }]], "le retour vise le fil où la pastille a été cliquée");
});
