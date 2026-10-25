// Run with: node --test app/ui/draft-transfer.test.cjs
// Boots the real app.js (editor and network stubbed) and reproduces the welcome
// draft that used to survive a transfer into the created thread.
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test: nodeTest } = require("node:test");
const test = (name, run) => nodeTest(name, { timeout: 10000 }, run);
const vm = require("node:vm");

const appSource = readFileSync(`${__dirname}/app.js`, "utf8");
const pagesSource = readFileSync(`${__dirname}/panels/pages.js`, "utf8");
const pageSource = readFileSync(`${__dirname}/index.html`, "utf8");
const SUITE = "SUITE À GARDER DANS LE PREMIER FIL";

function stored(app, key) { return app.storage.has(key) ? app.storage.get(key) : null; }
function textOf(doc) {
  if (!doc?.content) return "";
  return doc.content.map((block) => (block.content || []).map((node) => node.text || "").join("")).join("\n");
}
function docOf(text) {
  const lines = text ? String(text).split("\n") : [""];
  return { type: "doc", content: lines.map((line) => ({ type: "paragraph", content: line ? [{ type: "text", text: line }] : [] })) };
}

class Element {
  constructor(tag, document) {
    this.ownerDocument = document;
    this.localName = String(tag || "div").toLowerCase();
    this.tagName = this.localName.toUpperCase();
    this.attributes = {};
    this.dataset = {};
    this.listeners = {};
    this.childNodes = [];
    this.className = "";
    this.id = "";
    this.hidden = false;
    this.disabled = false;
    this.open = false;
    this.value = "";
    this.checked = false;
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this.clientHeight = 0;
    this.clientWidth = 0;
    this.offsetTop = 0;
    this.offsetHeight = 0;
    this.isConnected = true;
    this.parentElement = null;
    this.style = { setProperty() {} };
    this.classList = {
      add: (...names) => names.forEach((name) => this.classList.toggle(name, true)),
      remove: (...names) => names.forEach((name) => this.classList.toggle(name, false)),
      toggle: (name, force) => {
        const names = new Set(this.className.split(/\s+/).filter(Boolean));
        const on = force === undefined ? !names.has(name) : Boolean(force);
        if (on) names.add(name); else names.delete(name);
        this.className = [...names].join(" ");
        return on;
      },
      contains: (name) => this.className.split(/\s+/).includes(name),
    };
  }
  get children() { return this.childNodes.filter((node) => node instanceof Element); }
  get firstElementChild() { return this.children[0] || null; }
  get textContent() { return this.childNodes.map((node) => node.textContent ?? "").join(""); }
  set textContent(value) { this.childNodes = value === "" ? [] : [this.ownerDocument.createTextNode(String(value))]; }
  append(...nodes) {
    for (const node of nodes) {
      if (node == null) continue;
      if (node.parentElement) node.remove();
      node.parentElement = this;
      this.childNodes.push(node);
    }
  }
  prepend(...nodes) {
    for (const node of [...nodes].reverse()) this.insertBefore(node, this.childNodes[0]);
  }
  replaceChildren(...nodes) {
    if (this.contains(this.ownerDocument.activeElement)) this.ownerDocument.activeElement = this.ownerDocument.body;
    this.childNodes = [];
    this.append(...nodes);
  }
  remove() {
    const parent = this.parentElement;
    if (!parent) return;
    parent.childNodes = parent.childNodes.filter((node) => node !== this);
    this.parentElement = null;
  }
  contains(node) { return node === this || this.childNodes.some((child) => child.contains?.(node)); }
  insertBefore(node, before) {
    if (node.parentElement) node.remove();
    node.parentElement = this;
    const index = this.childNodes.indexOf(before);
    if (index < 0) this.childNodes.push(node); else this.childNodes.splice(index, 0, node);
  }
  setAttribute(name, value) {
    const key = String(name).toLowerCase();
    this.attributes[key] = String(value);
    if (key === "class") this.className = String(value);
    else if (key === "id") { this.id = String(value); this.ownerDocument.ids.set(this.id, this); }
    else if (key === "hidden") this.hidden = true;
    else if (key.startsWith("data-")) this.dataset[key.slice(5).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = String(value);
  }
  getAttribute(name) { return this.attributes[String(name).toLowerCase()] ?? null; }
  removeAttribute(name) {
    const key = String(name).toLowerCase();
    delete this.attributes[key];
    if (key === "hidden") this.hidden = false;
    if (key.startsWith("data-")) delete this.dataset[key.slice(5).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())];
  }
  addEventListener(type, handler) { (this.listeners[type] ||= []).push(handler); }
  focus() { this.ownerDocument.activeElement = this; }
  scrollIntoView() {}
  getBoundingClientRect() { return { top: 80, left: 20, right: 420, bottom: 140, width: 400, height: 60 }; }
  showModal() { this.open = true; this.setAttribute("open", ""); }
  close() { this.open = false; this.removeAttribute("open"); }
  requestSubmit() {
    const event = { preventDefault() {}, target: this, defaultPrevented: false };
    return this.listeners.submit?.at(-1)?.(event);
  }
  click() { this.dispatchEvent({ type: "click" }); }
  dispatchEvent(event) {
    event.target ||= this;
    const chain = [];
    let node = this;
    while (node) {
      chain.push(node);
      node = node.parentElement;
    }
    chain.push(this.ownerDocument);
    for (const current of chain) {
      for (const handler of current.listeners?.[event.type] || []) handler(event);
      if (event.cancelBubble) break;
    }
  }
  closest(selector) {
    let node = this;
    while (node) {
      if (node instanceof Element && matches(node, selector)) return node;
      node = node.parentElement;
    }
    return null;
  }
  querySelectorAll(selector) { return queryAll(this, selector); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  cloneNode(deep) {
    const copy = this.ownerDocument.createElement(this.localName);
    for (const [key, value] of Object.entries(this.attributes)) copy.setAttribute(key, value);
    if (deep) for (const child of this.childNodes) copy.append(child instanceof Element ? child.cloneNode(true) : this.ownerDocument.createTextNode(child.textContent));
    return copy;
  }
}

function matches(element, selector) {
  let rest = selector.trim();
  if (!rest) return false;
  const tag = /^[a-zA-Z][\w-]*/.exec(rest);
  if (tag) {
    if (element.localName !== tag[0].toLowerCase()) return false;
    rest = rest.slice(tag[0].length);
  }
  while (rest) {
    if (rest.startsWith(".")) {
      const name = /^[.\w-]+/.exec(rest)[0].slice(1);
      if (!element.className.split(/\s+/).includes(name)) return false;
      rest = rest.slice(name.length + 1);
    } else if (rest.startsWith("#")) {
      const name = /^#[\w-]+/.exec(rest)[0].slice(1);
      if (element.id !== name) return false;
      rest = rest.slice(name.length + 1);
    } else if (rest.startsWith("[")) {
      const end = rest.indexOf("]");
      const body = rest.slice(1, end);
      const [, key, , quoted, bare] = /^([\w-]+)(?:(=)"([^"]*)"|=([^\]]+))?$/.exec(body) || [];
      if (!key) return false;
      const value = element.attributes[key.toLowerCase()];
      const expected = quoted ?? bare;
      if (expected === undefined ? value === undefined && element[key] !== true : value !== expected) return false;
      rest = rest.slice(end + 1);
    } else if (rest.startsWith(":not(")) {
      const end = rest.indexOf(")");
      if (matches(element, rest.slice(5, end))) return false;
      rest = rest.slice(end + 1);
    } else return false;
  }
  return true;
}

function queryAll(root, selector) {
  const selectors = [];
  let depth = 0; let current = "";
  for (const char of selector) {
    if (char === "(") depth += 1;
    if (char === ")") depth -= 1;
    if (char === "," && depth === 0) { selectors.push(current); current = ""; }
    else current += char;
  }
  selectors.push(current);
  const found = [];
  const visit = (node) => {
    for (const child of node.children || []) {
      if (selectors.some((part) => matchesDescendant(child, part.trim()))) found.push(child);
      visit(child);
    }
  };
  visit(root);
  return found;
}
function matchesDescendant(element, selector) {
  const parts = selector.split(/\s+/).filter(Boolean);
  if (!matches(element, parts.at(-1))) return false;
  let ancestor = element.parentElement;
  for (let index = parts.length - 2; index >= 0; index -= 1) {
    while (ancestor && !matches(ancestor, parts[index])) ancestor = ancestor.parentElement;
    if (!ancestor) return false;
    ancestor = ancestor.parentElement;
  }
  return true;
}

function parsePage(document) {
  const voidTags = new Set("area base br col embed hr img input link meta param source track wbr".split(" "));
  const root = document.documentElement;
  const stack = [root];
  const tokens = pageSource.match(/<!--[\s\S]*?-->|<!doctype[^>]*>|<\/[^>]+>|<(?:[^>"']|"[^"]*"|'[^']*')*>|[^<]+/gi) || [];
  for (const token of tokens) {
    if (token.startsWith("<!--") || token.startsWith("<!")) continue;
    if (token.startsWith("</")) { if (stack.length > 1) stack.pop(); continue; }
    if (!token.startsWith("<")) {
      if (token.trim()) stack.at(-1).append(document.createTextNode(token));
      continue;
    }
    const body = token.slice(1, token.endsWith("/>") ? -2 : -1).trim();
    const name = /^[\w-]+/.exec(body)[0].toLowerCase();
    const parent = stack.at(-1);
    const node = name === "html" ? root : name === "body" ? document.body : document.createElement(name);
    const attrs = body.slice(name.length);
    for (const match of attrs.matchAll(/([\w:-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|(\S+)))?/g)) {
      const key = match[1].toLowerCase();
      const value = match[2] ?? match[3] ?? match[4] ?? "";
      if (key === "class") node.className = value;
      node.setAttribute(key, value);
      if (key === "disabled") node.disabled = true;
      if (key === "open") node.open = true;
    }
    if (node !== root && node !== parent) parent.append(node);
    if (name === "html") { /* document element already roots the stack */ }
    else if (name === "body") stack.push(document.body);
    else if (!token.endsWith("/>") && !voidTags.has(name)) stack.push(node);
  }
}

function boot(options = {}) {
  const document = {
    ids: new Map(),
    listeners: {},
    title: "",
    activeElement: null,
    createElement: (tag) => new Element(tag, document),
    createElementNS: (_ns, tag) => new Element(tag, document),
    createTextNode: (text) => ({ textContent: String(text), parentElement: null, contains: () => false }),
    getElementById(id) { return document.ids.get(id) || null; },
    getSelection: () => selection,
    addEventListener(type, handler) { (document.listeners[type] ||= []).push(handler); },
    querySelectorAll: (selector) => queryAll(document, selector),
    querySelector: (selector) => queryAll(document, selector)[0] || null,
  };
  document.documentElement = new Element("html", document);
  document.body = new Element("body", document);
  document.documentElement.append(document.body);
  document.activeElement = document.body;
  Object.defineProperty(document, "children", { get: () => [document.documentElement] });
  parsePage(document);

  const selection = {
    rangeCount: 0, isCollapsed: true, text: "", range: null,
    getRangeAt() { return this.range; },
    toString() { return this.text; },
    removeAllRanges() { this.rangeCount = 0; this.isCollapsed = true; this.text = ""; this.range = null; this.fire(); },
    fire() { for (const handler of document.listeners.selectionchange || []) handler(); },
  };
  const storage = options.storage || new Map();
  const localStorage = {
    getItem: (key) => storage.has(key) ? storage.get(key) : null,
    setItem: (key, value) => { if (options.storageFails) throw new Error("Stockage indisponible"); storage.set(key, String(value)); },
    removeItem: (key) => storage.delete(key),
  };
  const bots = [
    { id: "grokbot", name: "Ezerbot", color: "citron", shape: "galet", is_default: true, description: "Le bot par défaut.", instructions: "" },
    { id: "qa-redacteur", name: "Rédacteur", color: "violet", shape: "etoile", description: "Relit vos textes.", instructions: "" },
    { id: "qa-long", name: "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMN", color: "cyan", shape: "galet", description: "", instructions: "" },
  ];
  const state = options.state || {
    account: { name: "Espace personnel" }, mode: "simulated", conversations: [], bots,
    preferences: { theme: "dark", accent: "citron" },
    provider: { connected: false, signin: "idle", backend: "sharing", models: [], connections: {} },
    state_revision: 1,
  };
  const sent = [];
  const failures = { send: null, state: false, page: false };
  const requests = [];
  const intercept = new Map();
  let pageDetail = null;
  let conversationSerial = state.conversations.length;
  let threadOptions = null;
  let holdCreate = null;
  let failCreate = false;
  let createWaiting = null;
  let holdSend = null;
  let sendWaiting = null;
  const realSetTimeout = setTimeout;
  const realClearTimeout = clearTimeout;
  function relaxedTimeout(handler, delay, ...args) {
    const timer = realSetTimeout(handler, delay, ...args);
    timer.unref?.();
    return timer;
  }
  function json(data, status = 200) {
    return { ok: status >= 200 && status < 300, status, async json() { return structuredClone(data); } };
  }
  async function fetch(path, options = {}) {
    const url = String(path);
    const body = options.body ? JSON.parse(options.body) : undefined;
    requests.push({ url, body });
    if (intercept.has(url)) return intercept.get(url)(body, json);
    if (url === "/api/state") { if (failures.state) throw new Error("État injoignable"); return json(state); }
    if (url === `/api/pages/${pageDetail?.id}`) { if (failures.page) return json({ error: "Page injoignable" }, 503); return json({ page: pageDetail }); }
    if (url === "/api/ping") return json({ revision: state.state_revision });
    if (url === "/api/preferences") return json({ preferences: { ...state.preferences, ...body } });
    if (url === "/api/conversations") {
      createWaiting?.();
      if (holdCreate) await holdCreate;
      if (failCreate) return json({ error: "Création refusée pour le test" }, 503);
      const conversation = {
        id: `conv-${++conversationSerial}`, bot_id: body.bot_id, title: "Nouvelle conversation",
        messages: [], kind: "direct", updated_at: Date.now(), favorite: false, member_ids: [body.bot_id],
      };
      state.conversations.unshift(conversation);
      state.state_revision += 1;
      return json({ conversation });
    }
    if (url === "/api/send") {
      sent.push(body);
      sendWaiting?.();
      if (holdSend) await holdSend;
      if (failures.send) {
        if (failures.send === "network") throw new Error("Envoi interrompu");
        return json({ error: "Envoi refusé" }, failures.send);
      }
      if (body.page_anchor?.passage === "REFUSÉ") {
        // Un vrai moteur ne garde pas le message refusé ; ce faux partage l'état avec le client.
        for (const thread of state.conversations) thread.messages = thread.messages.filter((message) => message.state !== "queued");
        return json({ error: "Ce passage n’apparaît pas dans le texte affiché de la Page." }, 400);
      }
      const conversation = state.conversations.find((item) => item.id === body.conversation_id);
      conversation.messages.push({ id: body.message_id, role: "user", body: body.body, state: "accepted", attachments: body.attachments || [], payload: body.page_anchor ? { page_anchor: body.page_anchor } : {}, created_at: new Date().toISOString() });
      state.state_revision += 1;
      return json({ conversation: structuredClone(conversation) });
    }
    if (url === "/api/messages/resend") return json({ error: "Message introuvable." }, 404);
    return json({ error: `unexpected ${url}` }, 500);
  }

  let composer = null;
  const window = {
    GrokbotMascot: {
      shapes: { galet: { label: "Galet" }, etoile: { label: "Étoile" } },
      create: () => document.createElement("span"),
      setShape() {}, setContext() {}, celebrate() {},
    },
    GrokbotChat: {
      createDeepLinkQueue: () => ({ receive() {}, setReady() {} }),
      createApprovalFollow: () => ({ cancel() {}, arm() {}, bind() {} }),
      isPendingReply: () => false,
      connectionMessages: (messages) => messages,
      composerConnectionState: ({ offline = false, live = false, busy = false }) => ({
        offline, live: live && !offline, busy: busy && !offline,
        mode: live && !offline ? "stop" : "send",
        icon: live && !offline ? "#i-stop" : "#i-send",
        title: offline ? "Moteur local indisponible" : "Envoyer le message",
        label: "Envoyer le message",
      }),
      revealPendingApproval() {},
      teachPrompt: "",
      searchableMessageText: (message) => message.body || "",
      renderThread(_host, options) { threadOptions = options; },
      exportMessage: (message) => message,
      mountInput(host, options) {
        const model = { doc: docOf("") };
        const editor = document.createElement("div");
        editor.className = "gb-editor";
        editor.editor = {
          getText: () => textOf(model.doc),
          getJSON: () => model.doc,
          commands: { clearContent() { model.doc = docOf(""); } },
        };
        host.append(editor);
        const api = {
          mentions: [],
          get value() { return textOf(model.doc); },
          set value(text) { model.doc = docOf(text); },
          get document() { return model.doc; },
          set document(doc) { model.doc = doc || docOf(""); },
          appendText(text) { api.value = `${api.value}${api.value ? "\n" : ""}${text}`; },
          setReadOnly() {}, setPlaceholder() {}, setInvalid() {}, focus() { editor.focus(); },
        };
        composer = { api, onInput: options.onInput, editor: editor.editor, element: editor };
        return api;
      },
    },
    matchMedia: () => ({ matches: true, addEventListener() {} }),
    addEventListener() {},
  };
  const context = {
    window, document, localStorage, fetch, console, structuredClone, TextEncoder, URL, AbortController,
    setTimeout: relaxedTimeout, clearTimeout: realClearTimeout, queueMicrotask, setImmediate,
    requestAnimationFrame: (callback) => relaxedTimeout(callback, 0),
    cancelAnimationFrame: realClearTimeout,
    crypto, Intl, Date, Map, Set, WeakMap, WeakSet, Symbol, Promise, JSON, Math, Number, String, Boolean,
    Array, Object, RegExp, Error, TypeError, SyntaxError, parseInt, parseFloat, isNaN, encodeURIComponent, decodeURIComponent,
    innerHeight: 780, innerWidth: 1140, navigator: { clipboard: { writeText: async () => {} } },
    Node: Element, ResizeObserver: class { observe() {} disconnect() {} },
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(appSource, context);
  return {
    window, document, storage, state, sent, context, failures, requests, options, intercept,
    servePage(detail) { pageDetail = detail; },
    thread: () => threadOptions,
    composer: () => composer,
    type(text) { composer.api.value = text; composer.onInput(); },
    cmdN() {
      document.body.dispatchEvent({ type: "keydown", key: "n", metaKey: true, ctrlKey: false, altKey: false, preventDefault() {} });
    },
    gateCreation() {
      let release;
      holdCreate = new Promise((resolve) => { release = resolve; });
      let opened;
      const waiting = new Promise((resolve) => { opened = resolve; });
      createWaiting = opened;
      return {
        waiting,
        release() { holdCreate = null; failCreate = false; release(); },
        fail() { failCreate = true; holdCreate = null; release(); },
      };
    },
    // Retient la réponse du prochain /api/send : le serveur a reçu le message, l'accusé n'est pas encore revenu.
    gateSend() {
      let release;
      holdSend = new Promise((resolve) => { release = resolve; });
      let opened;
      const waiting = new Promise((resolve) => { opened = resolve; });
      sendWaiting = () => { sendWaiting = null; opened(); };
      return { waiting, release() { holdSend = null; release(); } };
    },
  };
}

async function ready(app) {
  await app.window.GrokbotPanels.refresh();
  await new Promise((resolve) => setImmediate(resolve));
}

test("a draft typed during creation stays on its thread and does not return on Nouvelle conversation", async () => {
  const app = boot();
  await ready(app);
  const editor = () => app.composer().editor;
  app.type("Premier message QA");
  const gate = app.gateCreation();
  const pending = app.document.getElementById("composer-form").requestSubmit();
  await gate.waiting;
  assert.equal(editor().getText(), "", "the submitted composer is cleared before creation resolves");
  app.type(SUITE);
  assert.equal(stored(app, "grokbot.draft.new"), SUITE);
  assert.equal(stored(app, "grokbot.draft.new.document"), JSON.stringify(app.composer().api.document));
  gate.release();
  await pending;

  const threadId = stored(app, "grokbot.selected");
  assert.equal(editor().getText(), SUITE);
  assert.equal(stored(app, `grokbot.draft.${threadId}`), SUITE);
  assert.equal(stored(app, "grokbot.draft.new"), null);
  assert.equal(stored(app, "grokbot.draft.new.document"), null, "the welcome rich document must be deleted with its text key");

  app.cmdN();
  assert.equal(stored(app, "grokbot.selected"), "");
  assert.equal(editor().getText(), "", "⌘N opens an empty composer");
  assert.equal(app.composer().api.document.content.length, 1);
  assert.equal(app.composer().api.document.content[0].content.length, 0);
  assert.equal(stored(app, "grokbot.draft.new"), null);
  assert.equal(stored(app, "grokbot.draft.new.document"), null);
  assert.equal(stored(app, `grokbot.draft.${threadId}`), SUITE, "the first thread keeps the unsent continuation");
});

test("an orphan welcome document left by an older transfer is purged instead of restored", async () => {
  const app = boot();
  await ready(app);
  app.type("Premier message QA");
  const pending = app.document.getElementById("composer-form").requestSubmit();
  await pending;
  const threadId = stored(app, "grokbot.selected");
  const orphan = docOf(SUITE);
  app.storage.set("grokbot.draft.new.document", JSON.stringify(orphan));
  app.storage.delete("grokbot.draft.new");

  app.cmdN();
  assert.equal(app.composer().editor.getText(), "");
  assert.equal(stored(app, "grokbot.draft.new.document"), null);
  assert.equal(stored(app, `grokbot.draft.${threadId}`), null);
});

test("a refused creation keeps both the submitted text and the text typed while waiting", async () => {
  const app = boot();
  await ready(app);
  app.type("Message à restaurer");
  const gate = app.gateCreation();
  const pending = app.document.getElementById("composer-form").requestSubmit();
  await gate.waiting;
  app.type("Suite à conserver");
  gate.fail();
  await pending;
  assert.equal(app.composer().editor.getText(), "Message à restaurer\nSuite à conserver");
  assert.equal(stored(app, "grokbot.draft.new"), "Message à restaurer\nSuite à conserver");
  assert.equal(app.state.conversations.length, 0);
});

test("a maximal bot name and its edit action stay inside an ellipsis rule", async () => {
  const css = readFileSync(`${__dirname}/styles.css`, "utf8");
  const rule = css.match(/\.bot-menu \.bot-menu-name,\s*\.bot-menu \.bot-menu-copy small,\s*\.bot-menu \.bot-menu-action > span \{([^}]*)\}/);
  assert.ok(rule, "the name and the Modifier action share one ellipsis rule");
  for (const declaration of ["min-width: 0", "overflow: hidden", "text-overflow: ellipsis", "white-space: nowrap"]) {
    assert.ok(rule[1].includes(declaration), declaration);
  }
  assert.match(css, /\.bot-menu button > \.icon \{[^}]*flex:\s*0 0 auto/);
  assert.match(css, /\.bot-menu button \{[^}]*min-width:\s*0/);
  assert.match(css, /\.bot-menu-list \{[^}]*overflow-x:\s*hidden/);

  const app = boot();
  await ready(app);
  const longName = "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMN";
  [...app.document.querySelectorAll(".sidebar-bot")].find((node) => node.textContent.includes(longName)).click();
  app.document.getElementById("composer-bot").click();
  const option = [...app.document.querySelectorAll("#bot-menu button")].find((button) => button.getAttribute("role") === "menuitemradio" && button.textContent.includes(longName));
  assert.equal(option?.querySelector(".bot-menu-name")?.textContent, longName);
  const action = [...app.document.querySelectorAll("#bot-menu .bot-menu-action span")].find((node) => node.textContent.includes(longName));
  assert.equal(action?.textContent, `Modifier ${longName}`);
});

test("the bot menu keeps focus across refresh, moves with the keyboard, and returns it on Escape", async () => {
  const app = boot();
  await ready(app);
  const menu = app.document.getElementById("bot-menu");
  const trigger = app.document.getElementById("composer-bot");
  trigger.click();
  assert.equal(menu.hidden, false);
  assert.equal(app.document.activeElement?.getAttribute("aria-checked"), "true");
  const focusedId = app.document.activeElement.dataset.menuKey;
  menu.querySelector(".bot-menu-list").scrollTop = 48;

  await app.window.GrokbotPanels.refresh();
  assert.equal(menu.hidden, false);
  assert.equal(app.document.activeElement?.dataset.menuKey, focusedId);
  assert.equal(app.document.activeElement?.getAttribute("aria-checked"), "true");
  assert.equal(menu.querySelector(".bot-menu-list").scrollTop, 48);

  const items = () => [...menu.querySelectorAll("button")];
  const before = items().indexOf(app.document.activeElement);
  app.document.activeElement.dispatchEvent({ type: "keydown", key: "ArrowDown", preventDefault() {} });
  assert.equal(items().indexOf(app.document.activeElement), (before + 1) % items().length);
  app.document.activeElement.dispatchEvent({ type: "keydown", key: "End", preventDefault() {} });
  assert.equal(app.document.activeElement, items().at(-1));
  app.document.activeElement.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  assert.equal(menu.hidden, true);
  assert.equal(app.document.activeElement, trigger);
});

test("a disabled composer bot focuses the conversation title when the editor is not editable", async () => {
  const app = boot();
  await ready(app);
  const menu = app.document.getElementById("bot-menu");
  const trigger = app.document.getElementById("composer-bot");
  const title = app.document.getElementById("conversation-title");
  app.composer().editor.isEditable = false;
  trigger.click();
  assert.equal(app.document.activeElement?.getAttribute("aria-checked"), "true");
  trigger.disabled = true;
  app.document.activeElement.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  assert.equal(menu.hidden, true);
  assert.equal(app.document.activeElement === title, true);
  assert.equal(title.tabIndex, -1);
});

test("a disabled composer bot returns menu focus to the editor", async () => {
  const app = boot();
  await ready(app);
  const menu = app.document.getElementById("bot-menu");
  const trigger = app.document.getElementById("composer-bot");
  const editor = app.composer().element;
  trigger.click();
  assert.equal(app.document.activeElement?.getAttribute("aria-checked"), "true");
  trigger.disabled = true;
  app.document.activeElement.dispatchEvent({ type: "keydown", key: "Escape", preventDefault() {} });
  assert.equal(menu.hidden, true);
  assert.equal(app.document.activeElement === editor, true);
});

test("short composer names stay whole and a maximal name ellipsizes only when the bar is full", async () => {
  const css = readFileSync(`${__dirname}/styles.css`, "utf8");
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)];
  const composerBot = rules.find(([, selector]) => selector.trim() === ".composer .composer-bot");
  assert.ok(composerBot, "the composer bot keeps a flex rule");
  assert.match(composerBot[2], /flex:\s*0\s+1\s+auto/, "the composer bot shrinks only when the bar is full");
  assert.match(composerBot[2], /min-width:\s*0/);
  const composerName = rules.find(([, selector]) => selector.trim() === ".composer-bot-name");
  assert.ok(composerName, "the composer name has its own rule");
  assert.match(composerName[2], /min-width:\s*0/, "the name shrinks inside the bot button");
  assert.match(composerName[2], /text-align:\s*left/);
  assert.match(composerName[2], /overflow:\s*hidden/);
  assert.match(composerName[2], /text-overflow:\s*ellipsis/);
  assert.match(composerName[2], /white-space:\s*nowrap/);

  const menu = css.match(/\.bot-menu \.bot-menu-name,\s*\.bot-menu \.bot-menu-copy small,\s*\.bot-menu \.bot-menu-action > span \{([^}]*)\}/);
  assert.ok(menu, "a maximal menu name still has an ellipsis rule");
  assert.match(menu[1], /text-overflow:\s*ellipsis/);

  const app = boot();
  await ready(app);
  const label = () => app.document.getElementById("composer-bot-name");
  const choose = (name) => [...app.document.querySelectorAll(".sidebar-bot")].find((node) => node.textContent.includes(name)).click();
  assert.equal(label().textContent, "Ezerbot");
  choose("Rédacteur");
  assert.equal(label().textContent, "Rédacteur");
  const longName = "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMN";
  choose(longName);
  assert.equal(label().textContent, longName);
  assert.equal(label().className, "composer-bot-name");
});

// Ouvre la Page « Trois forfaits » dans le premier fil, avec app.js et le panneau Pages réels.
async function quotedPage(orphan = false, options = {}) {
  const app = boot(options);
  await ready(app);
  app.type("Bonjour");
  await app.document.getElementById("composer-form").requestSubmit();
  const conversation = app.state.conversations[0];
  if (orphan) { app.state.conversations.length = 0; await ready(app); }
  // Ce faux serveur partage son état avec le client : on retire l'envoi optimiste resté « queued ».
  conversation.messages = conversation.messages.filter((message) => message.state !== "queued");
  const detail = {
    id: "page_forfaits", bot_id: "grokbot", title: "Trois forfaits", status: "ready", diagnostic: null, source_version: "c".repeat(64), preview: "ConfortREFUSÉ",
    document: { states: [], view: { type: "element", tag: "table", children: [
      { type: "element", tag: "tr", children: [{ type: "element", tag: "td", children: [{ type: "text", text: "Confort" }] }] },
    ] } },
  };
  app.servePage(detail);
  app.state.pages = { by_bot: { grokbot: [{ id: detail.id, bot_id: "grokbot", conversation_id: conversation.id, title: detail.title, status: "ready" }] } };
  const register = app.window.GrokbotPanels.register;
  let pagePanel;
  app.window.GrokbotPanels.register = (panel) => { pagePanel = panel; return register(panel); };
  vm.runInContext(pagesSource, app.context);
  await ready(app);
  const doc = app.document;
  if (orphan) { doc.querySelector(".page-link").click(); await new Promise((resolve) => setImmediate(resolve)); }
  const cell = doc.querySelector(".page-canvas td");
  const comment = doc.querySelector(".page-comment");
  const chip = doc.querySelector(".gb-quote-chip");
  const select = (text) => {
    const selection = doc.getSelection();
    selection.rangeCount = 1; selection.isCollapsed = false; selection.text = text;
    selection.range = { startContainer: cell, endContainer: cell, getBoundingClientRect: () => ({ left: 30, top: 90, right: 90, bottom: 110, width: 60, height: 20 }) };
    selection.fire();
  };
  const sendWith = async (text) => {
    app.type(text);
    await doc.getElementById("composer-form").requestSubmit();
    for (const thread of app.state.conversations) thread.messages = thread.messages.filter((message) => message.state !== "queued");
  };
  return { app, doc, comment, chip, select, sendWith, detail, pagePanel };
}

test("l’envoi d’un passage cité transmet page_anchor ; après la croix, le message part sans ancre ; un refus garde la puce", async () => {
  const { app, comment, chip, select, sendWith } = await quotedPage();
  select("Confort");
  assert.equal(comment.hidden, false);
  comment.click();
  assert.equal(chip.hidden, false);
  await sendWith("Passe-le à 21 €");
  assert.deepEqual(JSON.parse(JSON.stringify(app.sent.at(-1).page_anchor)), { page_id: "page_forfaits", source_version: "c".repeat(64), passage: "Confort" });
  assert.equal(chip.hidden, true, "la puce part avec le message");

  select("Confort");
  comment.click();
  chip.querySelector(".gb-quote-remove").click();
  await sendWith("Sans citation");
  assert.equal(app.sent.at(-1).body, "Sans citation");
  assert.equal("page_anchor" in app.sent.at(-1), false);

  select("REFUSÉ");
  comment.click();
  await sendWith("Mal citée");
  assert.equal(app.sent.at(-1).page_anchor.passage, "REFUSÉ");
  assert.equal(chip.hidden, false, "un refus du moteur garde la puce pour corriger");

  // Un suivi de sous-tâche part par /api/delegation/send, sans ancre : pas de citation proposée ni gardée.
  app.state.conversations[0].kind = "task";
  await ready(app);
  assert.equal(chip.hidden, true);
  select("Confort");
  assert.equal(comment.hidden, true);
});

// Course : l'accusé d'un envoi lent ne doit retirer que la citation partie avec lui, même quand une nouvelle citation
// du même passage a été posée entre-temps (la croix, puis « Commenter » à nouveau).
test("l’accusé d’un envoi lent garde la nouvelle citation du même passage, qui part avec sa propre ancre", async () => {
  const { app, doc, comment, chip, select, sendWith } = await quotedPage();
  const anchor = { page_id: "page_forfaits", source_version: "c".repeat(64), passage: "Confort" };
  select("Confort");
  comment.click();
  const gate = app.gateSend();
  app.type("Premier");
  const first = doc.getElementById("composer-form").requestSubmit();
  await gate.waiting;
  assert.deepEqual(structuredClone(app.sent.at(-1).page_anchor), anchor, "le premier message part avec la première citation");
  chip.querySelector(".gb-quote-remove").click();
  assert.equal(chip.hidden, true);
  select("Confort");
  comment.click();
  assert.equal(chip.hidden, false, "la nouvelle citation est posée pendant l’envoi");
  app.type("Deuxième");
  gate.release();
  await first;
  for (const thread of app.state.conversations) thread.messages = thread.messages.filter((message) => message.state !== "queued");
  assert.equal(app.composer().api.value, "Deuxième", "le brouillon tapé pendant l’envoi reste");
  assert.equal(chip.hidden, false, "l’accusé du premier envoi ne retire pas la nouvelle citation");
  await sendWith("Deuxième");
  assert.equal(app.sent.at(-1).body, "Deuxième");
  assert.deepEqual(structuredClone(app.sent.at(-1).page_anchor), anchor, "le deuxième message part avec la nouvelle citation");
  assert.equal(chip.hidden, true, "la nouvelle citation part avec le deuxième message");
});

// Après un redémarrage, la carte pendingSends est vide : seule l'ancre enregistrée dans le fil peut la rendre au renvoi.
async function savedAnchoredMessage(state) {
  const saved = { page_id: "page_forfaits", source_version: "d".repeat(64), passage: "Confort" };
  const conversation = state.conversations[0];
  conversation.messages = [{
    id: "ancre-enregistree", role: "user", body: "Passe-le à 21 €", state: "queued", created_at: new Date().toISOString(),
    payload: { page_anchor: { ...saved, title: "Trois forfaits" } },
  }];
  state.state_revision += 1;
  return saved;
}

test("« Reprendre l’envoi » après un rechargement renvoie l’ancre enregistrée du message en file", async () => {
  const app = boot();
  await ready(app);
  app.type("Bonjour");
  await app.document.getElementById("composer-form").requestSubmit();
  const saved = await savedAnchoredMessage(app.state);
  await ready(app);
  const message = app.thread().messages.find((item) => item.id === "ancre-enregistree");
  assert.ok(message, "le message en file est dans le fil");
  app.sent.length = 0;
  await app.thread().onRetry(message);
  const body = app.sent.at(-1);
  assert.equal(body.message_id, "ancre-enregistree");
  assert.deepEqual(structuredClone(body.page_anchor), saved);
});

test("le renvoi de repli (/api/messages/resend introuvable) rend lui aussi l’ancre enregistrée", async () => {
  const app = boot();
  await ready(app);
  app.type("Bonjour");
  await app.document.getElementById("composer-form").requestSubmit();
  const saved = await savedAnchoredMessage(app.state);
  app.state.conversations[0].messages[0].state = "refused";
  await ready(app);
  const message = app.thread().messages.find((item) => item.id === "ancre-enregistree");
  app.sent.length = 0;
  await app.thread().onResend(message);
  assert.deepEqual(structuredClone(app.sent.at(-1).page_anchor), saved);
});

for (const status of [400, 409]) test(`r10 T01/T02 citation orpheline récupérée après refus ${status}`, async () => {
  const { app, comment, chip, select, sendWith } = await quotedPage(true);
  select("Confort"); comment.click(); app.failures.send = status;
  await sendWith("Corrige ce prix");
  assert.equal(app.composer().api.value, "Corrige ce prix");
  assert.equal(chip.hidden, false, "la citation accompagne le brouillon récupéré");
  app.failures.send = null;
  await sendWith("Nouvel essai");
  assert.equal(app.sent.at(-1).page_anchor?.passage, "Confort");
});

test("r10 T24 le même brouillon partage une création de fil et transfère sa citation", async () => {
  const { app, comment, chip, select } = await quotedPage(true);
  select("Confort"); comment.click();
  const gate = app.gateCreation();
  const a = app.window.GrokbotPanels.ensureDraftConversation();
  await gate.waiting;
  const b = app.window.GrokbotPanels.ensureDraftConversation();
  gate.release();
  const [first, second] = await Promise.all([a, b]);
  assert.equal(first.id, second.id);
  assert.equal(chip.hidden, false);
  app.failures.send = 400;
  app.type("Avec fichier"); await app.document.getElementById("composer-form").requestSubmit();
  assert.equal(app.sent.at(-1).page_anchor.passage, "Confort");
  assert.equal(chip.hidden, false);
});

for (const refreshFails of [false, true]) test(`r10 T05/T22 tentative interrompue récupérable après rechargement, refresh échoué ${refreshFails}`, async () => {
  const { app, comment, select, doc } = await quotedPage();
  select("Confort"); comment.click();
  app.failures.send = "network"; app.failures.state = refreshFails;
  app.type("Tentative interrompue"); await doc.getElementById("composer-form").requestSubmit();
  const original = structuredClone(app.sent.at(-1));
  const restarted = boot({ storage: new Map(app.storage), state: structuredClone(app.state) });
  await ready(restarted);
  const message = restarted.thread().messages.find((m) => m.id === original.message_id);
  assert.ok(message, "la tentative absente du journal reste récupérable après redémarrage du client");
  await restarted.thread().onRetry(message);
  assert.deepEqual(structuredClone(restarted.sent.at(-1)), original);
});

for (const savedState of ["queued", "dispatching", "unknown", "accepted", "echoed"]) test(`r10 T04 le serveur détient ${savedState}, aucun brouillon dupliqué`, async () => {
  const { app, comment, select, doc, chip } = await quotedPage();
  select("Confort"); comment.click();
  const gate = app.gateSend(); app.failures.send = "network";
  app.type("Déjà enregistré"); const pending = doc.getElementById("composer-form").requestSubmit();
  await gate.waiting;
  const sent = app.sent.at(-1);
  app.state.conversations[0].messages.push({ id: sent.message_id, body: sent.body, role: "user", state: savedState, payload: { page_anchor: sent.page_anchor } });
  gate.release(); await pending;
  assert.equal(app.composer().api.value, "");
  assert.equal(chip.hidden, true);
});

for (const change of ["rename", "missing", "passage", "load", "reorder", "error", "version"]) test(`r10 T10/T11 citation au refus après ${change}`, async () => {
  const { app, comment, select, doc, chip, detail } = await quotedPage();
  select("Confort"); comment.click();
  const gate = app.gateSend(); app.failures.send = 400;
  app.type("À retoucher"); const pending = doc.getElementById("composer-form").requestSubmit();
  await gate.waiting;
  if (change === "rename") { detail.title = "Forfaits actuels"; app.state.pages.by_bot.grokbot[0].title = detail.title; }
  if (change === "missing") app.state.pages.by_bot.grokbot = [];
  if (change === "passage") detail.preview = "Texte remplacé";
  if (change === "error") detail.status = "error";
  if (change === "version") detail.source_version = "d".repeat(64);
  if (change === "load") app.failures.page = true;
  if (change === "reorder") app.state.pages.by_bot.grokbot.unshift({ ...app.state.pages.by_bot.grokbot[0], id: "autre-page" });
  gate.release(); await pending;
  if (["rename", "reorder", "version"].includes(change)) {
    assert.equal(chip.hidden, false);
    assert.ok(chip.textContent.includes(change === "rename" ? "Forfaits actuels" : "Confort"));
  } else assert.match(doc.getElementById("toast").textContent, /citation|Citation/, "l'explication de récupération reste visible après l'erreur d'envoi");
});

for (const edit of ["remove", "same", "other"]) test(`r10 T12 refus ancien après ${edit} de citation`, async () => {
  const { app, comment, select, doc, chip } = await quotedPage();
  select("Confort"); comment.click();
  const gate = app.gateSend(); app.failures.send = 400;
  app.type("Première demande"); const pending = doc.getElementById("composer-form").requestSubmit();
  await gate.waiting;
  chip.querySelector(".gb-quote-remove").click();
  if (edit !== "remove") { select(edit === "same" ? "Confort" : "REFUSÉ"); comment.click(); }
  gate.release(); await pending;
  assert.equal(chip.hidden, edit === "remove");
  if (edit !== "remove") assert.ok(chip.textContent.includes(edit === "same" ? "Confort" : "REFUSÉ"));
});

test("r10 T09/N03 un snapshot omet le fil soumis sans écraser le brouillon new", async () => {
  const { app, comment, select, doc } = await quotedPage();
  app.storage.set("grokbot.draft.new", "Autre brouillon");
  select("Confort"); comment.click();
  const gate = app.gateSend(); app.failures.send = 400;
  app.type("Brouillon du fil"); const pending = doc.getElementById("composer-form").requestSubmit();
  await gate.waiting; app.state.conversations.length = 0;
  gate.release(); await pending;
  assert.equal(app.storage.get("grokbot.draft.new"), "Autre brouillon");
  assert.match(doc.getElementById("toast").textContent, /conversation|fil/i);
});

test("r10 T21 stockage indisponible refuse une tentative avant de perdre le brouillon", async () => {
  const { app, comment, select, chip, doc } = await quotedPage();
  select("Confort"); comment.click(); app.type("À conserver"); app.options.storageFails = true;
  const before = app.sent.length;
  await doc.getElementById("composer-form").requestSubmit();
  assert.equal(app.sent.length, before);
  assert.equal(app.composer().api.value, "À conserver");
  assert.equal(chip.hidden, false);
  assert.match(doc.getElementById("toast").textContent, /stockage|Stockage/);
});

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

for (const saved of [false, true]) test(`r10 T07 ouverture tardive du sous-fil pendant un envoi, enregistré ${saved}`, async () => {
  const { app, doc, comment, select, chip } = await quotedPage();
  const owner = app.state.conversations[0];
  const thread = { ...structuredClone(owner), id: "sous-fil", messages: [] };
  app.state.conversations.push(thread);
  app.storage.set("grokbot.draft.sous-fil", "Brouillon de B");
  const threadGate = deferred(), threadEntered = deferred();
  app.intercept.set("/api/messages/thread", async (_body, json) => { threadEntered.resolve(); await threadGate.promise; return json({ conversation: thread, thread: {} }); });
  app.thread().onThread(owner.messages[0]); await threadEntered.promise;
  select("Confort"); comment.click();
  const sendGate = app.gateSend(); app.failures.send = 400;
  app.type("Demande de A"); const sending = doc.getElementById("composer-form").requestSubmit();
  await sendGate.waiting;
  if (saved) {
    const request = app.sent.at(-1);
    owner.messages.push({ id: request.message_id, role: "user", body: request.body, state: "accepted", payload: { page_anchor: request.page_anchor } });
  }
  threadGate.resolve(); await new Promise((done) => setImmediate(done));
  sendGate.release(); await sending;
  assert.equal(app.storage.get("grokbot.selected"), owner.id);
  assert.equal(app.storage.get("grokbot.draft.sous-fil"), "Brouillon de B");
  assert.equal(app.composer().api.value, saved ? "" : "Demande de A");
  assert.equal(chip.hidden, saved);
});

test("r10 T08 la reprise d'un brouillon de lien attend la création en cours", async () => {
  const storage = new Map([["ezerbot.link-previous-draft", JSON.stringify([{ text: "Autre demande", bot: "qa-redacteur" }])]]);
  const { app, doc, comment, select, chip } = await quotedPage(true, { storage });
  select("Confort"); comment.click();
  const gate = app.gateCreation();
  app.type("À envoyer"); const sending = doc.getElementById("composer-form").requestSubmit();
  await gate.waiting;
  doc.getElementById("restore-link-draft").click();
  assert.equal(app.window.GrokbotPanels.state.bots[0].id, "grokbot");
  assert.equal(app.storage.get("grokbot.bot") || "grokbot", "grokbot");
  app.failures.send = 400; gate.release(); await sending;
  assert.equal(app.composer().api.value, "À envoyer");
  assert.equal(chip.hidden, false);
});

test("r10 T16 la fin de sous-tâche conserve texte et fichiers visibles au refus", async () => {
  const app = boot(); await ready(app); app.type("Bonjour");
  await app.document.getElementById("composer-form").requestSubmit();
  const conversation = app.state.conversations[0]; conversation.kind = "task"; conversation.parent_id = "parent";
  let follow = true, discarded = 0;
  app.window.GrokbotDelegation = { canFollowUp: () => follow, taskId: () => "task" };
  app.window.GrokbotPanels.register({ id: "files", sendExtras: () => ({ attachments: ["document-pret"] }), composer: { discard() { discarded++; } } });
  const gate = deferred(), entered = deferred();
  app.intercept.set("/api/delegation/send", async (_body, json) => { entered.resolve(); await gate.promise; return json({ error: "Sous-tâche terminée" }, 409); });
  await ready(app); app.type("Suivi à conserver");
  const sending = app.document.getElementById("composer-form").requestSubmit(); await entered.promise;
  follow = false; gate.resolve(); await sending;
  assert.equal(discarded, 0);
  assert.equal(app.composer().api.value, "Suivi à conserver");
  assert.match(app.document.getElementById("toast").textContent, /brouillon.*fichiers/);
});

test("r10 T03/T21 création refusée conserve mentions et saisie dans leurs documents", async () => {
  const { app, doc, comment, select, chip } = await quotedPage(true);
  select("Confort"); comment.click();
  const original = { type: "doc", content: [{ type: "paragraph", content: [{ type: "mention", attrs: { id: "grokbot", label: "Ezerbot" } }, { type: "text", text: " demande" }] }] };
  app.composer().api.document = original; app.composer().onInput();
  const gate = app.gateCreation();
  const sending = doc.getElementById("composer-form").requestSubmit(); await gate.waiting;
  app.type("Texte suivant"); const next = structuredClone(app.composer().api.document);
  gate.fail(); await sending;
  assert.deepEqual(structuredClone(app.composer().api.document.content), [...original.content, ...next.content]);
  assert.equal(chip.hidden, false);
});

for (const change of ["missing", "navigate"]) test(`r10 T11/T12 restauration tardive après ${change} ne ressuscite pas la citation`, async () => {
  const { app, doc, comment, select, chip, detail } = await quotedPage(true);
  select("Confort"); comment.click();
  const gate = deferred(), entered = deferred(); let reads = 0;
  app.intercept.set(`/api/pages/${detail.id}`, async (_body, json) => {
    reads++; entered.resolve(); await gate.promise; return json({ page: detail });
  });
  app.failures.send = 400; app.type("Retouche");
  const sending = doc.getElementById("composer-form").requestSubmit(); await entered.promise;
  if (change === "missing") { app.state.pages.by_bot.grokbot = []; await ready(app); }
  else {
    const other = { ...app.state.pages.by_bot.grokbot[0], id: "autre", conversation_id: null };
    app.state.pages.by_bot.grokbot.push(other); await ready(app);
    doc.querySelectorAll(".page-link").at(-1).click();
  }
  gate.resolve(); await sending;
  assert.equal(chip.hidden, true);
  if (change === "missing") assert.match(doc.getElementById("toast").textContent, /citation/i);
});

test("r10 T11 panne réseau de Page ne relance pas sa lecture en boucle", async () => {
  const { app, doc, detail, comment, select } = await quotedPage();
  select("Confort"); comment.click(); let attempts = 0;
  app.intercept.set(`/api/pages/${detail.id}`, async (_body, json) => {
    attempts++;
    if (attempts < 5) throw new Error("Réseau coupé");
    return json({ error: "Indisponible" }, 503);
  });
  app.failures.send = 400; app.type("Retouche");
  await doc.getElementById("composer-form").requestSubmit();
  await new Promise((done) => setImmediate(done));
  assert.ok(attempts < 5, `la panne a déclenché ${attempts} lectures`);
  assert.match(doc.getElementById("toast").textContent, /citation/i);
});

for (const change of ["missing", "navigate"]) test(`r10 T11/T12 résultat de revalidation tardif après ${change}`, async () => {
  const { app, doc, comment, select, chip, detail, pagePanel } = await quotedPage(true);
  select("Confort"); comment.click();
  const captured = pagePanel.composer.capture();
  const gate = deferred();
  app.intercept.set(`/api/pages/${detail.id}`, async (_body, json) => { await gate.promise; return json({ page: detail }); });
  const restoring = pagePanel.composer.restore({ captured }, { conversation: null, bot: { id: "grokbot" } });
  app.state.pages.by_bot.grokbot = [];
  await ready(app);
  if (change === "navigate") {
    app.state.pages.by_bot.grokbot.push({ id: "autre", bot_id: "grokbot", title: "Autre Page", conversation_id: null, status: "ready" });
    await ready(app); doc.querySelectorAll(".page-link").at(-1).click();
  }
  gate.resolve(); const notice = await restoring;
  assert.equal(chip.hidden, true);
  if (change === "missing") assert.match(notice, /citation/i);
});

test("r10 T08 une création de bot tardive laisse le brouillon soumis à son bot", async () => {
  const { app, doc, comment, select, chip } = await quotedPage(true);
  const botGate = deferred(), botEntered = deferred();
  app.intercept.set("/api/bots", async (body, json) => {
    botEntered.resolve(); await botGate.promise;
    const bot = { ...app.state.bots[0], ...body, id: "bot-recent", is_default: false };
    app.state.bots.push(bot); return json({ bot });
  });
  doc.querySelector("[data-new-bot]").click(); doc.getElementById("bot-name").value = "Bot récent";
  const creatingBot = doc.getElementById("bot-form").requestSubmit(); await botEntered.promise;
  select("Confort"); comment.click();
  const gate = app.gateCreation(); app.type("Brouillon original");
  const sending = doc.getElementById("composer-form").requestSubmit(); await gate.waiting;
  botGate.resolve(); await creatingBot;
  assert.equal(app.storage.get("grokbot.bot") || "grokbot", "grokbot");
  app.failures.send = 400; gate.release(); await sending;
  assert.equal(chip.hidden, false);
  assert.equal(app.composer().api.value, "Brouillon original");
});

test("r10 T21 le document restauré survit à un aller-retour sans stockage", async () => {
  const { app, doc } = await quotedPage();
  const first = app.state.conversations[0].id;
  const document = { type: "doc", content: [{ type: "paragraph", content: [{ type: "mention", attrs: { id: "grokbot", label: "Ezerbot" } }, { type: "text", text: " reviens" }] }] };
  app.composer().api.document = document; app.composer().onInput();
  app.options.storageFails = true; app.failures.send = 400;
  await doc.getElementById("composer-form").requestSubmit();
  doc.getElementById("new-chat").click();
  assert.equal(doc.getElementById("conversation-title").textContent, "Nouvelle conversation");
  assert.equal(app.window.GrokbotPanels.openConversation(first), true);
  assert.deepEqual(structuredClone(app.composer().api.document), document);
});

for (const disabledOn of ["send", "refresh"]) test(`r10 T06 fichiers iPhone préservés, accès désactivé sur ${disabledOn}`, async () => {
  const app = boot(); await ready(app); app.type("Bonjour");
  await app.document.getElementById("composer-form").requestSubmit();
  app.window.EzerbotIPhone = { remote: true };
  let consumed = false;
  app.window.GrokbotPanels.register({ id: "files", sendExtras: () => ({ attachments: ["document-pret"] }), composer: { onSent() { consumed = true; } } });
  app.intercept.set("/api/send", async (_body, json) => {
    app.intercept.set("/api/state", async (_body, reply) => reply({ error: "L’accès iPhone est désactivé sur le Mac." }, 403));
    if (disabledOn === "send") return json({ error: "L’accès iPhone est désactivé sur le Mac." }, 403);
    throw new Error("Envoi interrompu");
  });
  app.type("Avec fichier"); await app.document.getElementById("composer-form").requestSubmit();
  assert.equal(consumed, false);
  assert.equal(app.composer().api.value, "Avec fichier");
  assert.equal(app.thread().messages.some((message) => message.body === "Avec fichier"), disabledOn !== "send");
});

for (const order of ["before", "during"]) test(`r10 N06 notification ${order} envoi refusé récupère A avant ouverture de B`, async () => {
  const { app, doc, select, comment } = await quotedPage();
  const a = app.state.conversations[0];
  const b = { ...structuredClone(a), id: "notification-b", messages: [] };
  app.state.conversations.push(b); app.storage.set("grokbot.draft.notification-b", "B distinct");
  app.storage.set("grokbot.draft.new", "Nouveau distinct");
  const gate = deferred(), entered = deferred(); let poll;
  app.context.setInterval = (fn) => { poll = fn; };
  doc.hasFocus = () => true; doc.visibilityState = "visible";
  app.intercept.set("/api/notifications/poll", async (_body, json) => json({ items: [], deliveries: [], settings: {}, unread: 0 }));
  app.intercept.set("/api/notifications/open", async (_body, json) => { entered.resolve(); await gate.promise; return json({ id: "reply:preuve", conversation_id: b.id }); });
  vm.runInContext(readFileSync(`${__dirname}/panels/notifications.js`, "utf8"), app.context);
  let opening;
  if (order === "before") { opening = app.window.EzerbotNotifications.activate("reply:preuve"); await entered.promise; }
  select("Confort"); comment.click(); app.type("A à récupérer"); app.failures.send = 400;
  const sendGate = app.gateSend(); const sending = doc.getElementById("composer-form").requestSubmit(); await sendGate.waiting;
  if (order === "during") { opening = app.window.EzerbotNotifications.activate("reply:preuve"); await entered.promise; }
  app.state.conversations = [b]; gate.resolve(); await opening;
  assert.equal(app.storage.get("grokbot.selected"), a.id);
  sendGate.release(); await sending;
  assert.equal(app.composer().api.value, "A à récupérer");
  assert.equal(app.storage.get("grokbot.draft.new"), "Nouveau distinct");
  await poll();
  assert.equal(app.storage.get("grokbot.selected"), b.id);
  assert.equal(app.composer().api.value, "B distinct");
  assert.equal(app.storage.get(`grokbot.draft.${a.id}`), "A à récupérer");
});

test("r10 R21/T21 le brouillon refusé du fil vide garde ses mentions en changeant de bot", async () => {
  const app = boot(); await ready(app); app.failures.send = 400;
  const document = { type: "doc", content: [{ type: "paragraph", content: [{ type: "mention", attrs: { id: "grokbot", label: "Ezerbot" } }, { type: "text", text: " demande" }] }] };
  app.composer().api.document = document; app.composer().onInput();
  await app.document.getElementById("composer-form").requestSubmit();
  app.intercept.set("/api/bots", async (body, json) => { const bot = { ...app.state.bots[0], ...body, id: "autre-bot", is_default: false }; app.state.bots.push(bot); return json({ bot }); });
  app.document.querySelector("[data-new-bot]").click(); app.document.getElementById("bot-name").value = "Autre bot";
  await app.document.getElementById("bot-form").requestSubmit();
  assert.equal(app.storage.get("grokbot.selected"), "");
  assert.deepEqual(structuredClone(app.composer().api.document), document);
  assert.deepEqual(JSON.parse(app.storage.get("grokbot.draft.new.document")), document);
});
