// Run with: node --test app/ui/panels/group-options.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const COST = "Les groupes consomment plus de ton usage, car plusieurs bots traitent chaque message.";
const HINT = "Pour la plupart des tâches, parle plutôt à un seul bot.";
const HUMAN = /personne|humain|e-mail|courriel|inviter|utilisateur/i;

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
    this.type = "";
    this.scrollTop = 0;
    this.selectionStart = 0;
    this.selectionEnd = 0;
    this.isConnected = true;
    this.id = "";
    const styleProps = {};
    this.style = {
      setProperty(name, value) { styleProps[name] = String(value); },
      getPropertyValue(name) { return styleProps[name] || ""; },
    };
    this.classList = {
      add: (...names) => { this.className = [...new Set(`${this.className} ${names.join(" ")}`.trim().split(/\s+/))].join(" "); },
      remove: (...names) => { this.className = this.className.split(/\s+/).filter((name) => !names.includes(name)).join(" "); },
      toggle: (name, force) => { const next = force === undefined ? !this.classList.contains(name) : Boolean(force); if (next) this.classList.add(name); else this.classList.remove(name); return next; },
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
  prepend(...children) {
    for (const child of children) if (child instanceof Element) child.parentElement = this;
    this.children.unshift(...children);
  }
  after(node) {
    if (node instanceof Element) node.parentElement = this.parentElement;
    const parent = this.parentElement;
    if (!parent) return;
    const index = parent.children.indexOf(this);
    parent.children.splice(index + 1, 0, node);
  }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter((child) => child !== this); }
  setAttribute(key, value) { this.attributes[key] = String(value); if (key === "class") this.className = String(value); }
  getAttribute(key) { return this.attributes[key]; }
  removeAttribute(key) { delete this.attributes[key]; }
  addEventListener(name, action) { (this.listeners[name] ||= []).push(action); }
  contains(node) { return Boolean(node) && (this === node || this.children.some((child) => child instanceof Element && child.contains(node))); }
  closest() { return null; }
  focus() { if (this.ownerDocument) this.ownerDocument.activeElement = this; }
  setSelectionRange(start, end = start) { this.selectionStart = start; this.selectionEnd = end; }
  select() {}
  scrollIntoView() {}
  showModal() { this.open = true; }
  close() { this.open = false; }
  getBoundingClientRect() { return { left: 0, top: 0, right: 200, bottom: 40, width: 200, height: 40 }; }
  matches(selector) { return splitSelectors(selector).some((part) => matchesCompound(this, part)); }
  querySelectorAll(selector) { return collect(this.children, selector); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  dispatch(type, event = {}) {
    const evt = { preventDefault() {}, stopPropagation() {}, target: this, currentTarget: this, ...event };
    let result;
    for (const fn of this.listeners[type] || []) result = fn(evt);
    const prop = this[`_on${type}`];
    if (prop) result = prop(evt);
    return result;
  }
  click() { if (this.disabled) return; return this.dispatch("click"); }
  set onclick(fn) { this._onclick = fn; }
  get onclick() { return this._onclick; }
  set onchange(fn) { this._onchange = fn; }
  get onchange() { return this._onchange; }
  set oninput(fn) { this._oninput = fn; }
  get oninput() { return this._oninput; }
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

function fill(input, value) {
  input.value = value;
  input.oninput?.();
}

function bot(id, name) {
  return { id, name, color: "citron", shape: "galet", description: `Bot ${name}`, deleted: false };
}

function harness({ bots = [bot("alpha", "Alpha"), bot("beta", "Bêta")], session = new Map(), calls = [] } = {}) {
  const topbar = new Element("header");
  topbar.className = "topbar";
  const actions = new Element("div");
  actions.className = "topbar-actions";
  const panels = new Element("div");
  panels.id = "panels";
  const body = new Element("body");
  topbar.append(actions);
  body.append(topbar, panels);
  const document = {
    activeElement: null,
    body,
    createElement: (tag) => new Element(tag, document),
    getElementById: (id) => (id === "panels" ? panels : null),
    querySelector(selector) { return body.querySelector(selector); },
    querySelectorAll(selector) { return body.querySelectorAll(selector); },
  };
  const host = {
    calls,
    register(panel) { this.panel = panel; },
    async api(path, body) {
      calls.push(JSON.parse(JSON.stringify({ path, body })));
      return { conversation: { id: "nouveau" } };
    },
    async refresh() {},
    openConversation() {},
    notify(message) { this.message = message; },
    createBotMascot: () => () => new Element("span", document),
    botSummary: (item) => item.description || "Bot",
  };
  const sessionStorage = {
    getItem: (key) => session.get(key) ?? null,
    setItem: (key, value) => session.set(key, String(value)),
  };
  vm.runInContext(fs.readFileSync(`${__dirname}/multibot.js`, "utf8"), vm.createContext({
    window: { GrokbotPanels: host },
    document,
    sessionStorage,
    crypto: { randomUUID: () => "commande-test" },
  }));
  const current = (conversation) => ({ conversation, state: { bots }, bot: bots[0] });
  return {
    host,
    calls,
    session,
    bots,
    document,
    button: actions.querySelector("[data-topbar-intent=members]"),
    dialog: panels.querySelector("dialog"),
    bar: body.querySelector(".group-bar"),
    banner: body.querySelector(".group-cost-banner"),
    show(conversation) { host.panel.onState({}, current(conversation)); },
    openCreate() { host.panel.onState({}, current(null)); actions.querySelector("[data-topbar-intent=create]").click(); },
  };
}

function group(id, title, description, avatar) {
  return {
    id, title, description, avatar, kind: "group", bot_id: "alpha",
    member_ids: ["alpha", "beta"], messages: [], favorite: false,
  };
}

test("un groupe affiche son nom, le bandeau, une fermeture locale et aucun champ humain", async () => {
  const session = new Map();
  const h = harness({ session });
  h.openCreate();
  assert.equal(h.dialog.open, true);
  assert.doesNotMatch(h.dialog.textContent, HUMAN);
  assert.match(h.dialog.textContent, /Deux à six bots/);
  const name = h.dialog.querySelector("[data-group-field=name]");
  const description = h.dialog.querySelector("[data-group-field=description]");
  const mark = h.dialog.querySelector("[data-group-field=mark]");
  assert.ok(name && description && mark, "nom, description et avatar");
  fill(name, "Atelier");
  fill(description, "Croiser les lectures");
  fill(mark, "A");
  h.dialog.querySelector("[data-avatar-color=cyan]").click();
  const create = () => h.dialog.querySelector("[data-group-action=create]");
  assert.equal(create().disabled, true);
  for (const id of ["alpha", "beta"]) {
    const choice = h.dialog.querySelector(`[data-bot-choice=${id}]`);
    choice.checked = true;
    choice.onchange();
  }
  assert.equal(create().disabled, false);
  await create().click();
  assert.deepEqual(h.calls[0], {
    path: "/api/conversations",
    body: { bot_ids: ["alpha", "beta"], title: "Atelier", description: "Croiser les lectures", avatar: { color: "cyan", mark: "A" } },
  });

  const first = group("groupe-a", "Atelier", "Croiser les lectures", { color: "cyan", mark: "A" });
  const second = group("groupe-b", "Revue", "Un second groupe", { color: "violet", mark: "R" });
  h.show(first);
  assert.equal(h.bar.querySelector(".group-bar-title").textContent, "Atelier");
  assert.equal(h.banner.hidden, false);
  assert.match(h.banner.textContent, new RegExp(COST.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.match(h.banner.textContent, new RegExp(HINT.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  h.banner.querySelector("[data-dismiss-banner]").click();
  assert.equal(h.banner.hidden, true);
  h.show(second);
  assert.equal(h.banner.hidden, false);
  assert.match(h.banner.textContent, /Pour la plupart des tâches/);
  assert.equal(h.bar.querySelector(".group-bar-title").textContent, "Revue");
  h.show(first);
  assert.equal(h.banner.hidden, true);

  const again = harness({ session, calls: [] });
  again.show(first);
  assert.equal(again.banner.hidden, true);
  again.show(second);
  assert.equal(again.banner.hidden, false);

  h.button.click();
  assert.doesNotMatch(h.dialog.textContent, HUMAN);
  assert.equal(h.dialog.querySelector("[data-group-field=name]").value, "Atelier");
  fill(h.dialog.querySelector("[data-group-field=name]"), "Atelier renommé");
  fill(h.dialog.querySelector("[data-group-field=description]"), "Description revue");
  h.dialog.querySelector("[data-avatar-color=violet]").click();
  fill(h.dialog.querySelector("[data-group-field=mark]"), "R");
  await h.dialog.querySelector("[data-group-action=save]").click();
  assert.deepEqual(h.calls.at(-1), {
    path: "/api/conversations/update",
    body: { id: "groupe-a", title: "Atelier renommé", description: "Description revue", avatar: { color: "violet", mark: "R" } },
  });
});

test("la borne reste de deux à six bots", () => {
  const ids = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta"];
  const h = harness({ bots: ids.map((id, index) => bot(id, `Bot ${index + 1}`)) });
  h.openCreate();
  const boxes = () => h.dialog.querySelectorAll("[data-bot-choice]");
  assert.equal(boxes().length, 7);
  while (boxes().filter((box) => box.checked).length < 6) {
    const next = boxes().find((box) => !box.checked && !box.disabled);
    assert.ok(next);
    next.checked = true;
    next.onchange();
  }
  assert.equal(h.dialog.querySelector("[data-bot-choice=eta]").disabled, true);
  assert.match(h.dialog.textContent, /Deux à six bots|six au maximum/);
  assert.doesNotMatch(h.dialog.textContent, HUMAN);
});

function bootApp() {
  const storage = new Map([["grokbot.selected", "groupe-a"], ["grokbot.bot", "alpha"]]);
  const posts = [];
  const state = {
    state_revision: 1,
    account: { name: "Espace personnel" },
    mode: "simulated",
    preferences: { theme: "dark", accent: "citron" },
    provider: { connected: false, signin: "idle", models: [], backend: "sharing" },
    conversations: [
      group("groupe-a", "Atelier", "Croiser les lectures", { color: "cyan", mark: "A" }),
      group("groupe-b", "Revue", "Un second groupe", { color: "violet", mark: "R" }),
    ].map((item) => ({ ...item, updated_at: item.id === "groupe-a" ? "2026-10-21T12:00:00Z" : "2026-10-21T11:00:00Z" })),
    bots: [bot("alpha", "Alpha"), bot("beta", "Bêta")],
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
  const ids = ["conversation-list", "search", "send-button", "welcome", "messages", "chat-scroll", "conversation-title", "conversation-menu", "conversation-menu-button", "composer-hint", "connection-banner", "connection-message", "toast", "welcome-description", "bot-extensions", "settings-extensions", "bot-dialog", "composer-attachments", "link-error-message", "link-error-dialog", "settings-dialog", "provider-section", "settings-appearance", "settings-bots", "settings-shortcuts", "new-chat", "restore-link-draft", "learn-skill-button", "bot-chips", "bot-list", "bot-dialog-title", "bot-save", "bot-name", "bot-description", "bot-instructions", "bot-delete-button", "bot-copy-link", "bot-colors", "bot-shapes", "account-name", "account-avatar", "workspace-label", "edit-bot-button", "favorite-button", "picker", "mode-badge-text", "plan-banner", "composer-tools", "mode-note", "plan-dialog", "plan-title", "composer-input", "composer-form", "plan-acknowledge", "sidebar-toggle", "sidebar-close", "sidebar-scrim", "settings-button", "simulate-loss", "rename-button", "rename-input", "rename-dialog", "rename-form", "rename-description", "rename-mark", "rename-avatar", "rename-avatar-colors", "rename-label", "rename-title", "copy-link-button", "export-button", "bot-form", "bot-delete-dialog", "confirm-bot-delete", "delete-button", "delete-dialog", "confirm-delete", "reconnect-button", "home-link", "favorites-filter", "bot-preview-mascot", "sidebar-bot-list", "composer-bot", "bot-menu", "composer-bot-name"];
  for (const id of ids) {
    const node = new Element("div", document);
    node.id = id;
    byId[id] = node;
    document.body.append(node);
  }
  byId["rename-title"].textContent = "Renommer la conversation";
  byId["rename-label"].textContent = "Nom de la conversation";
  for (const id of ["rename-description", "rename-mark", "rename-avatar"]) {
    byId[id].hidden = true;
    byId[id].className = "group-rename";
  }
  byId["favorite-button"].append(new Element("span", document));
  byId["plan-banner"].append(new Element("span", document));
  byId["send-button"].append(new Element("use", document));
  const editor = new Element("div", document);
  editor.className = "gb-editor";
  editor.editor = { commands: { clearContent() {} } };
  byId["composer-input"].append(editor);
  byId["plan-dialog"].append(new Element("a", document), Object.assign(new Element("p", document), { className: "delete-description" }));
  const stage = new Element("div", document);
  const hero = new Element("div", document);
  hero.dataset.mascot = "hero";
  stage.append(hero);
  document.body.append(stage, Object.assign(new Element("div", document), { className: "main-panel" }), Object.assign(new Element("div", document), { className: "composer-dock" }));
  byId["chat-scroll"].scrollHeight = 0;
  byId["chat-scroll"].scrollTop = 0;
  byId["chat-scroll"].clientHeight = 0;
  byId["welcome-description"].textContent = "Bienvenue";
  const input = { value: "", document: null, mentions: [], setReadOnly() {}, setPlaceholder() {}, setInvalid() {}, focus() {} };
  const window = {
    GrokbotChat: {
      createDeepLinkQueue: () => ({ receive() {}, setReady() {} }),
      createApprovalFollow: () => ({ arm() {}, cancel() {} }),
      mountInput: () => input,
      isPendingReply: () => false,
      renderThread() {},
      connectionMessages: (messages) => messages,
      composerConnectionState: ({ offline = false, live = false, busy = false }) => ({ offline, live: live && !offline, busy: busy && !offline, mode: live && !offline ? "stop" : "send", icon: "#i-send", title: "Envoyer", label: "Envoyer" }),
      searchableMessageText: () => "",
      revealPendingApproval() {},
      teachPrompt: "",
    },
    GrokbotMascot: { shapes: {}, setShape() {}, setContext() {}, celebrate() {}, create: () => new Element("span", document) },
    matchMedia: () => ({ matches: false, addEventListener() {} }),
    addEventListener() {},
  };
  vm.runInContext(fs.readFileSync(`${__dirname}/../app.js`, "utf8"), vm.createContext({
    window, document, console, URL, Blob, TextEncoder, innerHeight: 800, Node: Element,
    crypto: { randomUUID: () => "id-test" },
    localStorage: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value), removeItem: (key) => storage.delete(key) },
    navigator: { clipboard: { writeText: async () => {} } },
    fetch: async (path, options = {}) => {
      if (options.method === "POST" && path === "/api/conversations/update") {
        const body = JSON.parse(options.body);
        posts.push(body);
        const conversation = state.conversations.find((item) => item.id === body.id);
        if (body.title !== undefined) conversation.title = body.title;
        if (body.description !== undefined) conversation.description = body.description;
        if (body.avatar !== undefined) conversation.avatar = body.avatar;
        return { ok: true, json: async () => ({ conversation: structuredClone(conversation) }) };
      }
      return { ok: true, json: async () => path === "/api/ping" ? { revision: state.state_revision } : structuredClone(state) };
    },
    setTimeout: () => 0, clearTimeout() {}, requestAnimationFrame: () => 0,
    AbortController, ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
  }));
  return { byId, posts, state };
}

const item = (root, id) => root.querySelectorAll(".conversation-item").find((node) => node.dataset.conversationId === id);

test("la liste montre le nom, la description et l’avatar, et le renommage les met à jour", async () => {
  const app = bootApp();
  for (let i = 0; i < 20 && !app.byId["conversation-list"].querySelector(".conversation-item"); i += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  const atelier = item(app.byId["conversation-list"], "groupe-a");
  const revue = item(app.byId["conversation-list"], "groupe-b");
  assert.ok(atelier && revue);
  assert.match(atelier.textContent, /Atelier/);
  assert.match(atelier.textContent, /Croiser les lectures/);
  assert.equal(atelier.querySelector(".group-mark").textContent, "A");
  assert.equal(atelier.querySelector(".group-mark").dataset.botColor, "cyan");
  assert.match(revue.textContent, /Un second groupe/);
  assert.equal(revue.querySelector(".group-mark").textContent, "R");

  app.byId["rename-button"].click();
  assert.equal(app.byId["rename-title"].textContent, "Renommer le groupe");
  assert.equal(app.byId["rename-description"].hidden, false);
  assert.equal(app.byId["rename-description"].value, "Croiser les lectures");
  assert.equal(app.byId["rename-mark"].value, "A");
  app.byId["rename-input"].value = "Atelier renommé";
  app.byId["rename-description"].value = "Description revue";
  app.byId["rename-mark"].value = "Z";
  await app.byId["rename-form"].dispatch("submit");
  assert.deepEqual(app.posts[0], {
    id: "groupe-a",
    title: "Atelier renommé",
    description: "Description revue",
    avatar: { color: "cyan", mark: "Z" },
  });
  const renamed = item(app.byId["conversation-list"], "groupe-a");
  assert.match(renamed.textContent, /Atelier renommé/);
  assert.match(renamed.textContent, /Description revue/);
  assert.equal(renamed.querySelector(".group-mark").textContent, "Z");
});

const AVATAR_COLORS = [
  ["citron", "Jaune citron", "#E6FF3D"],
  ["orange", "Orange vif", "#FF5C1F"],
  ["ambre", "Ambre", "#FFB224"],
  ["menthe", "Vert menthe", "#35E0A1"],
  ["cyan", "Cyan", "#22D3EE"],
  ["magenta", "Rose magenta", "#FF3D8B"],
  ["vermillon", "Rouge vermillon", "#FF4530"],
  ["violet", "Violet", "#9B7BFF"],
  ["blanc", "Sans couleur", "#F5F5F5"],
];

test("rouvrir Créer un groupe repart d’un formulaire vide", () => {
  const ids = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta"];
  const h = harness({ bots: ids.map((id, index) => bot(id, `Bot ${index + 1}`)) });
  h.openCreate();
  fill(h.dialog.querySelector("[data-group-field=name]"), "Atelier");
  fill(h.dialog.querySelector("[data-group-field=description]"), "Croiser les lectures");
  fill(h.dialog.querySelector("[data-group-field=mark]"), "ZZ");
  while (h.dialog.querySelectorAll("[data-bot-choice]").filter((box) => box.checked).length < 6) {
    const next = h.dialog.querySelectorAll("[data-bot-choice]").find((box) => !box.checked && !box.disabled);
    next.checked = true;
    next.onchange();
  }
  assert.equal(h.dialog.querySelectorAll("[data-bot-choice]").filter((box) => box.checked).length, 6);
  h.dialog.close();
  h.openCreate();
  assert.equal(h.dialog.open, true);
  assert.equal(h.dialog.querySelectorAll("[data-bot-choice]").filter((box) => box.checked).length, 0);
  assert.equal(h.dialog.querySelector("[data-group-field=name]").value, "");
  assert.equal(h.dialog.querySelector("[data-group-field=description]").value, "");
  assert.equal(h.dialog.querySelector("[data-group-field=mark]").value, "");
  assert.equal(h.dialog.querySelector("[data-bot-choice=eta]").disabled, false);
});

test("la couleur d’avatar est enregistrée même si le signe est vide", async () => {
  const h = harness();
  h.openCreate();
  fill(h.dialog.querySelector("[data-group-field=name]"), "Atelier");
  fill(h.dialog.querySelector("[data-group-field=description]"), "Croiser les lectures");
  h.dialog.querySelector("[data-avatar-color=cyan]").click();
  for (const id of ["alpha", "beta"]) {
    const choice = h.dialog.querySelector(`[data-bot-choice=${id}]`);
    choice.checked = true;
    choice.onchange();
  }
  await h.dialog.querySelector("[data-group-action=create]").click();
  assert.deepEqual(h.calls[0].body.avatar, { color: "cyan", mark: "A" });

  const app = bootApp();
  for (let i = 0; i < 20 && !app.byId["conversation-list"].querySelector(".conversation-item"); i += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  app.byId["rename-button"].click();
  app.byId["rename-input"].value = "Atelier";
  app.byId["rename-description"].value = "Croiser les lectures";
  app.byId["rename-mark"].value = "";
  app.byId["rename-avatar-colors"].querySelectorAll("button").find((node) => node.dataset.avatarColor === "violet").click();
  await app.byId["rename-form"].dispatch("submit");
  assert.deepEqual(app.posts[0].avatar, { color: "violet", mark: "A" });
});

test("le nom, la description et le signe gardent le focus pendant un rafraîchissement", () => {
  const h = harness();
  h.openCreate();
  const name = h.dialog.querySelector("[data-group-field=name]");
  const description = h.dialog.querySelector("[data-group-field=description]");
  const mark = h.dialog.querySelector("[data-group-field=mark]");
  assert.equal(name.dataset.focusKey, "profil:nom");
  assert.equal(description.dataset.focusKey, "profil:description");
  assert.equal(mark.dataset.focusKey, "profil:signe");
  fill(name, "Atelier");
  name.focus();
  name.setSelectionRange(2, 5);
  h.show(null);
  const again = h.dialog.querySelector("[data-group-field=name]");
  assert.equal(h.document.activeElement, again);
  assert.equal(again.value, "Atelier");
  assert.equal(again.selectionStart, 2);
  assert.equal(again.selectionEnd, 5);
});

test("l’accueil ne montre que Créer un groupe", () => {
  const h = harness();
  h.show(null);
  const labels = h.document.querySelectorAll(".topbar-actions button").filter((button) => !button.hidden).map((button) => button.textContent);
  assert.deepEqual(labels, ["Créer un groupe"]);
});

test("une tâche cache Créer un groupe et Membres", () => {
  const h = harness();
  h.show({ id: "tache", kind: "task", title: "Tâche", member_ids: [], messages: [] });
  const labels = h.document.querySelectorAll(".topbar-actions button").filter((button) => !button.hidden).map((button) => button.textContent);
  assert.deepEqual(labels, []);
});

test("dans un fil direct, la barre montre Créer un groupe puis Membres", () => {
  const h = harness();
  h.show({ id: "direct", kind: "chat", title: "Direct", bot_id: "alpha", member_ids: ["alpha"], messages: [] });
  const labels = h.document.querySelectorAll(".topbar-actions button").filter((button) => !button.hidden).map((button) => button.textContent);
  assert.deepEqual(labels, ["Créer un groupe", "Membres"]);
  h.document.querySelectorAll(".topbar-actions button").find((button) => button.textContent === "Membres").click();
  assert.match(h.dialog.querySelector(".dialog-header").textContent, /Membres du fil/);
  assert.equal(h.dialog.querySelector("[data-group-field=name]"), null);
});

test("dans un groupe, Créer un groupe reste dans la barre et ouvre un formulaire vide", async () => {
  const gone = bot("ancien", "Ancien");
  gone.deleted = true;
  const h = harness({ bots: [bot("alpha", "Alpha"), bot("beta", "Bêta"), gone] });
  const room = group("groupe-a", "Atelier", "Croiser les lectures", { color: "cyan", mark: "A" });
  room.member_ids = ["alpha", "ancien"];
  h.show(room);
  const labels = h.document.querySelectorAll(".topbar-actions button").filter((button) => !button.hidden).map((button) => button.textContent);
  assert.deepEqual(labels, ["Créer un groupe", "Membres"]);
  const create = h.document.querySelectorAll(".topbar-actions button").find((button) => button.textContent === "Créer un groupe");
  create.click();
  assert.match(h.dialog.querySelector(".dialog-header").textContent, /Créer un groupe/);
  assert.doesNotMatch(h.dialog.querySelector(".dialog-header").textContent, /Membres du fil/);
  assert.equal(h.dialog.querySelector("[data-group-field=name]").value, "");
  assert.equal(h.dialog.querySelector("[data-group-field=description]").value, "");
  assert.equal(h.dialog.querySelector("[data-group-field=mark]").value, "");
  assert.equal(h.dialog.querySelector("[data-bot-choice=ancien]"), null);
  assert.equal(h.dialog.querySelectorAll("[data-bot-choice]").filter((box) => box.checked).length, 0);
  fill(h.dialog.querySelector("[data-group-field=name]"), "Nouveau");
  fill(h.dialog.querySelector("[data-group-field=description]"), "Un autre fil");
  for (const id of ["alpha", "beta"]) {
    const choice = h.dialog.querySelector(`[data-bot-choice=${id}]`);
    choice.checked = true;
    choice.onchange();
  }
  await h.dialog.querySelector("[data-group-action=create]").click();
  assert.deepEqual(h.calls.at(-1), {
    path: "/api/conversations",
    body: { bot_ids: ["alpha", "beta"], title: "Nouveau", description: "Un autre fil", avatar: { color: "citron", mark: "N" } },
  });
});

test("le renommage d’un groupe montre la pastille et le nom de chaque couleur", async () => {
  const app = bootApp();
  for (let i = 0; i < 20 && !app.byId["conversation-list"].querySelector(".conversation-item"); i += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  app.byId["rename-button"].click();
  const buttons = app.byId["rename-avatar-colors"].querySelectorAll("button");
  assert.equal(buttons.length, AVATAR_COLORS.length);
  for (const [accent, label, swatch] of AVATAR_COLORS) {
    const button = buttons.find((node) => node.dataset.avatarColor === accent);
    assert.ok(button, accent);
    assert.equal(button.querySelector("i").style.getPropertyValue("--swatch"), swatch, accent);
    assert.equal(button.querySelector("span").textContent, label);
  }
});
