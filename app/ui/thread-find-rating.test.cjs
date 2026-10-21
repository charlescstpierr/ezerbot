// Run with: node --test app/ui/thread-find-rating.test.cjs
// T18 : ⌘F trouve un mot dans le fil, Fermer n’enregistre pas, l’export contient l’envoi, zéro requête de notation.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const RARE = "xyloglotte";
const MOTIF = "Pas ce que j’ai demandé";
const DETAILS = "Il invente une date.";

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
  const findMod = await import("../web/src/thread-find.js").catch(() => null);
  const ratingMod = await import("../web/src/thread-rating.js").catch(() => null);
  const teach = await import("../web/src/teach.js");
  const storage = new Map([
    ["grokbot.selected", "conv-1"],
    ["grokbot.bot", "bot-a"],
  ]);
  const assistant = {
    id: "m-bot", role: "assistant", state: "echoed", created_at: "2026-10-21T12:00:01.000Z",
    body: `Oui, ${RARE} revient, et encore ${RARE}.`,
  };
  const state = {
    state_revision: 1,
    account: { name: "Espace personnel" },
    mode: "simulated",
    preferences: { theme: "dark", accent: "citron" },
    provider: { connected: false, signin: "idle", models: [], backend: "sharing", simulation: true },
    conversations: [{
      id: "conv-1", title: "Mot rare", bot_id: "bot-a", kind: "chat", favorite: false,
      updated_at: "2026-10-21T12:00:01.000Z",
      messages: [
        { id: "m-user", role: "user", state: "accepted", created_at: "2026-10-21T12:00:00.000Z", body: `As-tu vu ${RARE} ?` },
        assistant,
      ],
    }],
    bots: [
      { id: "bot-a", name: "Alpha", color: "citron", shape: "galet", deleted: false },
      { id: "bot-b", name: "Bêta", color: "orange", shape: "carre", deleted: false },
    ],
  };
  const calls = [];
  const blobs = [];
  const byId = {};
  const document = {
    title: "",
    activeElement: null,
    selected: null,
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
  const ids = ["conversation-list", "search", "send-button", "welcome", "messages", "chat-scroll", "conversation-title", "conversation-menu", "conversation-menu-button", "composer-hint", "connection-banner", "connection-message", "toast", "welcome-description", "bot-extensions", "settings-extensions", "bot-dialog", "composer-attachments", "link-error-message", "link-error-dialog", "settings-dialog", "provider-section", "settings-appearance", "settings-bots", "settings-shortcuts", "new-chat", "restore-link-draft", "learn-skill-button", "bot-chips", "bot-list", "bot-dialog-title", "bot-save", "bot-name", "bot-description", "bot-instructions", "bot-delete-button", "bot-copy-link", "bot-colors", "bot-shapes", "account-name", "account-avatar", "workspace-label", "edit-bot-button", "favorite-button", "picker", "mode-badge-text", "plan-banner", "composer-tools", "mode-note", "plan-dialog", "plan-title", "composer-input", "composer-form", "plan-acknowledge", "sidebar-toggle", "sidebar-close", "sidebar-scrim", "settings-button", "simulate-loss", "rename-button", "rename-input", "rename-dialog", "rename-form", "copy-link-button", "export-button", "export-ratings-button", "bot-form", "bot-delete-dialog", "confirm-bot-delete", "delete-button", "delete-dialog", "confirm-delete", "reconnect-button", "home-link", "favorites-filter", "bot-preview-mascot", "sidebar-bot-list", "composer-bot", "bot-menu", "composer-bot-name"];
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
  byId["welcome-description"].textContent = "Bienvenue";

  const bar = register(byId, element(document, "div"), "find-bar");
  bar.hidden = true;
  const title = register(byId, child(document, bar, "p"), "find-title");
  title.textContent = "Chercher dans le fil";
  const findInput = register(byId, child(document, bar, "input"), "find-input");
  const count = register(byId, child(document, bar, "p"), "find-count");
  const previous = register(byId, child(document, bar, "button"), "find-previous");
  previous.textContent = "Précédent";
  const next = register(byId, child(document, bar, "button"), "find-next");
  next.textContent = "Suivant";
  const findClose = register(byId, child(document, bar, "button"), "find-close");
  findClose.textContent = "Fermer";
  document.body.append(bar);

  const dialog = register(byId, element(document, "dialog"), "rating-dialog");
  const form = register(byId, child(document, dialog, "form"), "rating-form");
  const heading = register(byId, child(document, form, "h2"), "rating-title");
  heading.textContent = "Comment s’améliorer ?";
  for (const [value, label] of [["pas-demande", MOTIF], ["inexact", "Inexact"], ["qualite", "Qualité faible"], ["lent", "Trop lent"]]) {
    const choice = child(document, form, "label");
    const radio = child(document, choice, "input");
    radio.setAttribute("type", "radio");
    radio.setAttribute("name", "motif");
    radio.setAttribute("value", value);
    radio.value = value;
    choice.append(` ${label}`);
  }
  const details = register(byId, child(document, form, "textarea"), "rating-details");
  const ratingClose = register(byId, child(document, form, "button"), "rating-close");
  ratingClose.textContent = "Fermer";
  ratingClose.setAttribute("data-close-dialog", "");
  const ratingSend = register(byId, child(document, form, "button"), "rating-send");
  ratingSend.textContent = "Envoyer";
  document.body.append(dialog);

  let threadProps = null;
  const window = {
    GrokbotChat: {
      createDeepLinkQueue: () => ({ receive() {}, setReady() {} }),
      createApprovalFollow: () => ({ arm() {}, cancel() {} }),
      mountInput: () => input,
      isPendingReply: () => false,
      renderThread(_node, props) { threadProps = props; },
      connectionMessages: (messages) => messages,
      composerConnectionState: ({ offline = false, live = false, busy = false }) => ({ offline, live: live && !offline, busy: busy && !offline, mode: live && !offline ? "stop" : "send", icon: live && !offline ? "#i-stop" : "#i-send", title: offline ? "Moteur local indisponible" : "Envoyer le message", label: "Envoyer le message" }),
      searchableMessageText: teach.searchableMessageText,
      revealPendingApproval() {},
      teachPrompt: teach.TEACH_PROMPT,
      exportMessage: teach.exportMessage,
      findInThread: findMod?.findInThread || (() => []),
      cycleFind: findMod?.cycleFind || ((index) => index),
      findCountLabel: findMod?.findCountLabel || (() => ""),
      createRatingTable: ratingMod?.createRatingTable || (() => ({ list: () => [], recordGood() {}, recordBad() {}, exportDocument: () => ({ notations: [] }) })),
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
    URL: { createObjectURL(blob) { blobs.push(blob); return "blob:notation"; }, revokeObjectURL() {} },
    crypto: { randomUUID: () => `id-${storage.size}` },
    localStorage,
    navigator: { clipboard: { writeText: async () => {} } },
    fetch: async (path, options = {}) => {
      calls.push({ path: String(path), body: options.body || "" });
      if (String(path) === "/api/ping") return { ok: true, json: async () => ({ revision: state.state_revision }) };
      return { ok: true, json: async () => structuredClone(state) };
    },
    setTimeout: () => 0, clearTimeout() {}, requestAnimationFrame: (fn) => fn(),
    AbortController,
    ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
    Node: Element,
  });
  vm.runInContext(fs.readFileSync(`${__dirname}/app.js`, "utf8"), context);
  for (let attempt = 0; attempt < 20 && document.getElementById("conversation-title").textContent !== "Mot rare"; attempt += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  function key(extra) {
    const event = { key: "", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, preventDefault() { event.defaultPrevented = true; }, target: document.body, ...extra };
    for (const handler of document.listeners.keydown || []) handler(event);
    return event;
  }
  function notations() {
    const raw = localStorage.getItem("ezerbot.notations");
    return raw ? JSON.parse(raw) : [];
  }
  return { document, key, notations, calls, blobs, assistant, get threadProps() { return threadProps; } };
}

test("⌘F trouve le mot rare, Fermer n’enregistre pas, l’export contient l’envoi et aucune requête de notation ne part", async () => {
  const app = await boot();
  const page = fs.readFileSync(`${__dirname}/index.html`, "utf8");
  const bar = app.document.getElementById("find-bar");
  const opened = app.key({ key: "f", metaKey: true });
  assert.equal(opened.defaultPrevented, true);
  assert.equal(bar.hidden, false);
  assert.equal(app.document.getElementById("find-title").textContent, "Chercher dans le fil");
  assert.match(page, /Chercher dans le fil/);
  assert.match(page, /⌘ F/);
  assert.match(page, /⌘ K/);

  const field = app.document.getElementById("find-input");
  field.value = "XYLOGLOTTE";
  field.dispatch("input");
  assert.equal(app.document.getElementById("find-count").textContent, "1 sur 3 résultats");
  assert.equal(app.threadProps.find.messageId, "m-user");
  assert.equal(app.threadProps.find.indexInMessage, 0);

  app.document.getElementById("find-next").click();
  assert.equal(app.document.getElementById("find-count").textContent, "2 sur 3 résultats");
  assert.equal(app.threadProps.find.messageId, "m-bot");
  assert.equal(app.threadProps.find.indexInMessage, 0);
  app.document.getElementById("find-next").click();
  assert.equal(app.document.getElementById("find-count").textContent, "3 sur 3 résultats");
  assert.equal(app.threadProps.find.indexInMessage, 1);
  app.document.getElementById("find-next").click();
  assert.equal(app.document.getElementById("find-count").textContent, "1 sur 3 résultats");
  app.document.getElementById("find-previous").click();
  assert.equal(app.document.getElementById("find-count").textContent, "3 sur 3 résultats");

  const globalSearch = app.key({ key: "k", metaKey: true });
  assert.equal(globalSearch.defaultPrevented, true);
  assert.equal(app.document.activeElement.id, "search");
  assert.notEqual(app.document.activeElement.id, "find-input");

  app.threadProps.onRateBad(app.assistant);
  assert.equal(app.document.getElementById("rating-dialog").open, true);
  assert.equal(app.document.getElementById("rating-title").textContent, "Comment s’améliorer ?");
  for (const label of [MOTIF, "Inexact", "Qualité faible", "Trop lent", "Détails", "Envoyer", "Fermer"]) {
    assert.match(page, new RegExp(label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
  app.document.getElementById("rating-close").click();
  assert.equal(app.document.getElementById("rating-dialog").open, false);
  assert.deepEqual(app.notations(), []);

  app.threadProps.onRateGood(app.assistant);
  assert.equal(app.notations().length, 1);
  assert.equal(app.notations()[0].vote, "bon");
  assert.equal(app.notations()[0].message_id, "m-bot");

  app.threadProps.onRateBad(app.assistant);
  app.document.getElementById("rating-details").value = DETAILS;
  app.document.getElementById("rating-close").click();
  assert.equal(app.notations().length, 1);
  assert.equal(app.notations().some((row) => row.vote === "mauvais"), false);

  app.threadProps.onRateBad(app.assistant);
  const choice = app.document.getElementById("rating-form").querySelectorAll('input[name="motif"]')[0];
  choice.checked = true;
  app.document.getElementById("rating-details").value = DETAILS;
  app.document.getElementById("rating-form").dispatch("submit");
  assert.equal(app.document.getElementById("rating-dialog").open, false);
  const rows = app.notations();
  assert.equal(rows.length, 2);
  assert.equal(rows.filter((row) => row.vote === "mauvais").length, 1);
  assert.equal(rows[1].motif, MOTIF);
  assert.equal(rows[1].details, DETAILS);

  app.document.getElementById("export-ratings-button").click();
  const exported = JSON.parse(await app.blobs.at(-1).text());
  assert.equal(exported.notations.length, 2);
  assert.equal(exported.notations[1].motif, MOTIF);
  assert.equal(exported.notations[1].details, DETAILS);
  assert.equal(exported.notations[1].message_id, "m-bot");
  assert.equal(JSON.stringify(exported).includes("voteGrokBotFeedback"), false);

  app.document.getElementById("export-button").click();
  const conversation = JSON.parse(await app.blobs.at(-1).text());
  assert.equal(conversation.notations[1].motif, MOTIF);
  assert.equal(conversation.messages.some((message) => JSON.stringify(message).includes(MOTIF)), false);

  const ratingCalls = app.calls.filter((call) => /notation|feedback|vote|rating/i.test(`${call.path}\n${call.body}`) || String(call.body).includes(MOTIF) || String(call.body).includes(DETAILS));
  assert.deepEqual(ratingCalls, []);
});
