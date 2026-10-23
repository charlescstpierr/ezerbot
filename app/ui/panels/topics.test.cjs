// Run with: node --test app/ui/panels/topics.test.cjs
// Rend vraiment la section SUJETS et la page d’un sujet dans un DOM minimal.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class ClassList {
  constructor(el) { this.el = el; }
  contains(name) { return this.el.classes.includes(name); }
}
class Element {
  constructor(tag) {
    this.tagName = String(tag).toLowerCase();
    this.children = [];
    this.classes = [];
    this.classList = new ClassList(this);
    this.dataset = {};
    this.attributes = {};
    this.listeners = {};
    this.hidden = false;
    this.value = "";
    this.id = "";
    this.opened = false;
  }
  append(...children) { for (const child of children) { if (child instanceof Element) child.parentElement = this; this.children.push(child); } }
  prepend(child) { child.parentElement = this; this.children.unshift(child); }
  insertBefore(node, before) {
    node.parentElement = this;
    const index = this.children.indexOf(before);
    if (index < 0) this.children.push(node); else this.children.splice(index, 0, node);
  }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  setAttribute(key, value) { this.attributes[key] = String(value); if (key === "id") this.id = String(value); }
  getAttribute(key) { return this.attributes[key]; }
  removeAttribute(key) { delete this.attributes[key]; }
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { this.listeners.click?.(); }
  focus() {}
  select() {}
  showModal() { this.opened = true; }
  close() { this.opened = false; }
  contains(other) { return other === this || this.children.some((child) => child instanceof Element && child.contains(other)); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  querySelectorAll(selector) {
    const matches = (el) => {
      const key = /^\[data-focus-key="(.+)"\]$/.exec(selector);
      if (key) return el.dataset.focusKey === key[1];
      if (selector.startsWith("#")) return el.id === selector.slice(1);
      if (selector.startsWith(".")) return el.classes.includes(selector.slice(1));
      return el.tagName === selector;
    };
    return this.children.filter((child) => child instanceof Element)
      .flatMap((child) => [...(matches(child) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  set className(value) { this.classes = String(value).split(/\s+/).filter(Boolean); }
  get className() { return this.classes.join(" "); }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map((child) => (child instanceof Element ? child.textContent : String(child))).join(""); }
}
const all = (root) => [root, ...root.children.filter((child) => child instanceof Element).flatMap(all)];
const named = (root, className) => all(root).filter((el) => el.classes.includes(className));
const one = (root, className) => named(root, className)[0];
const text = (root, className) => named(root, className).map((el) => el.textContent);
// Les objets du contexte vm n’ont pas le prototype de ce fichier : on compare leur JSON.
const plain = (value) => JSON.parse(JSON.stringify(value));
const settle = () => new Promise((resolve) => setImmediate(resolve));

const NOW = Date.parse("2026-10-23T12:00:00Z");
const hoursAgo = (hours) => NOW - hours * 3600 * 1000;

function harness({ registered = true, topics, membership, conversations, collapsed = null, memory = {} } = {}) {
  const nodes = {};
  const make = (tag, id, className = "") => {
    const el = new Element(tag);
    el.id = id;
    el.className = className;
    nodes[id] = el;
    return el;
  };
  const sidebarTopics = make("div", "sidebar-topics");
  sidebarTopics.hidden = true;
  const list = make("div", "topic-list");
  const newTopic = make("button", "new-topic");
  sidebarTopics.append(newTopic, list);
  const search = make("input", "search");
  const favorites = make("button", "favorites-filter");
  favorites.setAttribute("aria-pressed", "false");
  const title = make("h1", "conversation-title");
  title.textContent = "Nouvelle conversation";
  const label = make("button", "workspace-label");
  const menuEntry = make("button", "topic-button");
  menuEntry.append(new Element("span"));
  menuEntry.children[0].textContent = "Classer dans un sujet…";
  const main = new Element("main");
  main.className = "main-panel";
  const dock = new Element("div");
  dock.className = "composer-dock";
  main.append(dock);
  const body = new Element("body");
  body.append(sidebarTopics, main);
  const store = new Map(collapsed ? [["ezerbot.topics.collapsed", JSON.stringify(collapsed)]] : []);
  const document = {
    title: "Ezerbot",
    body,
    activeElement: null,
    createElement: (tag) => new Element(tag),
    getElementById: (id) => nodes[id] || all(body).find((el) => el.id === id) || null,
    querySelector: (selector) => (selector === ".main-panel" ? main : selector === ".composer-dock" ? dock : null),
  };
  const calls = { api: [], opened: [], left: 0, notices: [] };
  const state = {
    conversations,
    bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }],
    topics: { items: topics, membership, memory },
  };
  let panel = null;
  const panels = {
    busy: false,
    register(value) { panel = value; return registered; },
    async api(path, input) {
      calls.api.push({ path, input });
      if (path === "/api/conversations") return { conversation: { id: "nouveau-fil" } };
      if (path === "/api/topics") return { topic: { id: "cree", name: input.name } };
      return { ok: true };
    },
    async refresh() {},
    notify(message) { calls.notices.push(message); },
    openConversation(id) { calls.opened.push(id); return true; },
    leaveConversation() { calls.left += 1; return true; },
  };
  const realDate = Date;
  const context = {
    window: { GrokbotPanels: panels, GrokbotChat: { searchableMessageText: (message) => message.body || "" } },
    document,
    localStorage: { getItem: (key) => store.get(key) ?? null, setItem: (key, value) => store.set(key, value) },
    setInterval: () => 0,
    Date: class extends realDate { constructor(...args) { super(...(args.length ? args : [NOW])); } static now() { return NOW; } },
    console, JSON, Set, Map, Number, String, Array, Math, Object, Promise,
  };
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(`${__dirname}/topics.js`, "utf8"), context);
  const push = (context_ = {}) => panel.onState(state, { conversation: null, bot: { id: "grokbot" }, ...context_ });
  return { context, document, nodes, list, sidebarTopics, main, title, label, menuEntry, panel: () => panel, calls, state, push, store, search, favorites };
}

const FIXTURE = {
  topics: [
    { id: "t1", name: "Déménagement", description: "Tout ce qui touche au 1er juillet", created_at: 1, updated_at: 1 },
    { id: "t2", name: "Impôts", description: "", created_at: 2, updated_at: 2 },
  ],
  membership: { a: "t1", b: "t1", g: "t2" },
  conversations: [
    { id: "a", title: "Louer un camion", kind: "chat", bot_id: "grokbot", updated_at: hoursAgo(2), favorite: false, messages: [] },
    { id: "b", title: "Changer d’adresse", kind: "group", bot_id: "grokbot", updated_at: hoursAgo(30), favorite: false, messages: [] },
    { id: "g", title: "Déclaration", kind: "chat", bot_id: "chef", updated_at: hoursAgo(80), favorite: true, messages: [] },
    { id: "libre", title: "Sans sujet", kind: "chat", bot_id: "grokbot", updated_at: hoursAgo(1), favorite: false, messages: [] },
  ],
};

test("les temps relatifs vont des minutes aux jours, puis à la date", () => {
  const h = harness(FIXTURE);
  const { relative } = h.context.window.GrokbotTopics;
  const minutes = (count) => NOW - count * 60000;
  assert.equal(relative(minutes(0), NOW), "à l’instant");
  assert.equal(relative(minutes(5), NOW), "il y a 5 min");
  assert.equal(relative(minutes(59), NOW), "il y a 59 min");
  assert.equal(relative(minutes(60), NOW), "il y a 1 h");
  assert.equal(relative(minutes(60 * 23), NOW), "il y a 23 h");
  assert.equal(relative(minutes(60 * 24 * 3), NOW), "il y a 3 j");
  assert.match(relative(minutes(60 * 24 * 12), NOW), /\d/);
  // Les secondes d’un ancien format restent comprises.
  assert.equal(relative(Math.floor(minutes(120) / 1000), NOW), "il y a 2 h");
});

test("la section SUJETS se montre et range les fils sous leur dossier", () => {
  const h = harness(FIXTURE);
  assert.equal(h.sidebarTopics.hidden, false);
  h.push();
  const folders = named(h.list, "topic-folder");
  assert.equal(folders.length, 2);
  assert.deepEqual(text(h.list, "topic-name"), ["Déménagement", "Impôts"]);
  assert.deepEqual(text(h.list, "topic-count"), ["2", "1"]);
  // Les fils du sujet : indentés sous le dossier, du plus récent au plus ancien, avec leur temps relatif.
  const first = folders[0];
  assert.deepEqual(text(first, "conversation-name"), ["Louer un camion", "Changer d’adresse"]);
  assert.deepEqual(text(first, "topic-thread-time"), ["il y a 2 h", "il y a 1 j"]);
  assert.deepEqual(text(folders[1], "topic-thread-time"), ["il y a 3 j"]);
  assert.ok(one(first, "topic-threads"), "les fils sont dans un groupe décalé");
  // Un fil sans sujet n’apparaît pas ici : il reste dans CONVERSATIONS.
  assert.ok(!all(h.list).some((el) => el.textContent === "Sans sujet"));
});

test("un dossier replié cache ses fils et garde son état d’une session à l’autre", () => {
  const h = harness({ ...FIXTURE, collapsed: ["t2"] });
  h.push();
  const [open, closed] = named(h.list, "topic-folder");
  assert.equal(one(open, "topic-toggle").getAttribute("aria-expanded"), "true");
  assert.equal(one(closed, "topic-toggle").getAttribute("aria-expanded"), "false");
  assert.equal(named(closed, "topic-threads").length, 0);
  // Replier le premier, déplier le second.
  one(open, "topic-toggle").click();
  one(named(h.list, "topic-folder")[1], "topic-toggle").click();
  const [now1, now2] = named(h.list, "topic-folder");
  assert.equal(named(now1, "topic-threads").length, 0);
  assert.equal(named(now2, "topic-threads").length, 1);
  assert.deepEqual(JSON.parse(h.store.get("ezerbot.topics.collapsed")), ["t1"]);
});

test("ouvrir un sujet affiche sa page : titre, description, fils, nouveau fil et mémoire", () => {
  const h = harness(FIXTURE);
  h.push({ conversation: FIXTURE.conversations[3] });
  one(named(h.list, "topic-folder")[0], "topic-name").click();
  assert.equal(h.calls.left, 1, "le fil affiché est quitté");
  h.push();
  const page = h.nodes["topic-page"] || h.document.getElementById("topic-page");
  assert.ok(page && !page.hidden);
  assert.equal(h.main.dataset.view, "topic");
  assert.equal(one(page, "topic-page-title").textContent, "Déménagement");
  assert.equal(one(page, "topic-page-description").textContent, "Tout ce qui touche au 1er juillet");
  assert.deepEqual(text(page, "topic-page-thread-title"), ["Louer un camion", "Changer d’adresse"]);
  assert.deepEqual(text(page, "topic-page-thread-meta"), ["Ezerbot · il y a 2 h", "Groupe · il y a 1 j"]);
  assert.equal(one(page, "topic-new-thread").textContent, "Nouveau fil");
  assert.equal(one(page, "topic-memory").getAttribute("aria-label"), "Mémoire du sujet");
  assert.equal(h.title.textContent, "Déménagement");
  assert.equal(h.label.textContent, "Sujet");
  assert.equal(h.document.title, "Déménagement · Ezerbot");
  const current = one(h.list, "topic-row");
  assert.ok(current.classes.includes("active"));
});

test("« Nouveau fil » crée un fil déjà rangé dans le sujet et l’ouvre", async () => {
  const h = harness(FIXTURE);
  h.push();
  one(named(h.list, "topic-folder")[0], "topic-name").click();
  h.push();
  one(h.document.getElementById("topic-page"), "topic-new-thread").click();
  await settle();
  assert.deepEqual(plain(h.calls.api), [{ path: "/api/conversations", input: { topic_id: "t1", bot_id: "grokbot" } }]);
  assert.deepEqual(h.calls.opened, ["nouveau-fil"]);
});

test("changer d’écran ferme la page du sujet", () => {
  const h = harness(FIXTURE);
  h.push();
  one(named(h.list, "topic-folder")[0], "topic-name").click();
  h.push();
  assert.equal(h.main.dataset.view, "topic");
  h.panel().onNavigate();
  assert.equal(h.main.dataset.view, undefined);
  assert.equal(h.document.getElementById("topic-page").hidden, true);
  assert.equal(h.panel() && one(h.list, "topic-row").classes.includes("active"), false);
});

test("un sujet vide propose « Nouveau fil » et un sujet supprimé ferme sa page", () => {
  const h = harness({ ...FIXTURE, membership: {} });
  h.push();
  one(named(h.list, "topic-folder")[0], "topic-name").click();
  h.push();
  const page = h.document.getElementById("topic-page");
  assert.match(one(page, "topic-page-empty").textContent, /Nouveau fil/);
  h.state.topics.items = [];
  h.push();
  assert.equal(page.hidden, true);
  assert.equal(h.main.dataset.view, undefined);
  assert.match(one(h.list, "topic-empty").textContent, /Regroupez plusieurs fils/);
});

test("supprimer un sujet dit que les fils restent, puis appelle la route", async () => {
  const h = harness(FIXTURE);
  h.push();
  one(named(h.list, "topic-folder")[0], "topic-name").click();
  h.push();
  one(h.document.getElementById("topic-page"), "topic-delete").click();
  const dialog = h.document.getElementById("topic-delete-dialog");
  assert.ok(dialog.opened);
  assert.match(dialog.querySelector("#topic-delete-description").textContent, /Les 2 fils de « Déménagement » ne sont pas supprimés/);
  dialog.querySelector("#topic-confirm-delete").click();
  await settle();
  assert.deepEqual(plain(h.calls.api), [{ path: "/api/topics/delete", input: { id: "t1" } }]);
  assert.equal(dialog.opened, false);
});

test("renommer un sujet envoie son nom et sa description", async () => {
  const h = harness(FIXTURE);
  h.push();
  one(named(h.list, "topic-folder")[0], "topic-name").click();
  h.push();
  one(h.document.getElementById("topic-page"), "topic-action").click();
  const dialog = h.document.getElementById("topic-form-dialog");
  assert.equal(dialog.querySelector("h2").textContent, "Renommer le sujet");
  assert.equal(dialog.querySelector("#topic-name-input").value, "Déménagement");
  dialog.querySelector("#topic-name-input").value = "  Emménagement ";
  dialog.querySelector("#topic-description-input").value = "";
  dialog.querySelector("form").listeners.submit({ preventDefault() {} });
  await settle();
  assert.deepEqual(plain(h.calls.api), [{ path: "/api/topics/update", input: { id: "t1", name: "Emménagement", description: "" } }]);
});

test("« + » crée un sujet", async () => {
  const h = harness({ ...FIXTURE, topics: [], membership: {}, conversations: [] });
  h.push();
  h.nodes["new-topic"].click();
  const dialog = h.document.getElementById("topic-form-dialog");
  assert.equal(dialog.querySelector("h2").textContent, "Nouveau sujet");
  dialog.querySelector("#topic-name-input").value = "Voyage";
  dialog.querySelector("#topic-description-input").value = "Été 2027";
  dialog.querySelector("form").listeners.submit({ preventDefault() {} });
  await settle();
  assert.deepEqual(plain(h.calls.api), [{ path: "/api/topics", input: { name: "Voyage", description: "Été 2027" } }]);
});

test("classer un fil : l’entrée du menu propose les sujets, « Aucun sujet » et le sujet courant", async () => {
  const h = harness(FIXTURE);
  const conversation = FIXTURE.conversations[3];
  h.push({ conversation });
  assert.equal(h.menuEntry.children[0].textContent, "Classer dans un sujet…");
  h.panel().assign(conversation);
  const dialog = h.document.getElementById("topic-assign-dialog");
  const choices = named(dialog, "topic-choice");
  assert.deepEqual(choices.map((el) => el.textContent), ["Aucun sujet", "Déménagement", "Impôts", "Nouveau sujet…"]);
  assert.equal(choices[0].getAttribute("aria-pressed"), "true");
  choices[2].click();
  await settle();
  assert.deepEqual(plain(h.calls.api), [{ path: "/api/topics/assign", input: { conversation_id: "libre", topic_id: "t2" } }]);
  assert.equal(dialog.opened, false);
  // Un fil déjà rangé change d’intitulé, et « Aucun sujet » le retire.
  h.push({ conversation: FIXTURE.conversations[0] });
  assert.equal(h.menuEntry.children[0].textContent, "Changer de sujet…");
});

test("la recherche et les favoris filtrent aussi les fils rangés", () => {
  const h = harness(FIXTURE);
  h.push();
  h.search.value = "camion";
  h.push();
  assert.deepEqual(text(h.list, "topic-name"), ["Déménagement"]);
  assert.deepEqual(text(h.list, "conversation-name"), ["Louer un camion"]);
  h.search.value = "";
  h.favorites.setAttribute("aria-pressed", "true");
  h.push();
  assert.deepEqual(text(h.list, "topic-name"), ["Impôts"]);
});

test("sur un iPhone appairé le panneau reste muet", () => {
  const h = harness({ ...FIXTURE, registered: false });
  assert.equal(h.sidebarTopics.hidden, true);
});

// --- Mémoire du sujet (#234) ---

const FACTS = [
  { id: "f3", topic_id: "t1", text: "Le garde-meuble ferme à 18 h", conversation_id: "b", conversation_title: "Changer d’adresse", created_at: hoursAgo(1) },
  { id: "f2", topic_id: "t1", text: "Le budget est de 800 euros", conversation_id: "a", conversation_title: "Louer un camion", created_at: hoursAgo(5) },
  { id: "f1", topic_id: "t1", text: "Le camion est réservé pour le 1er juillet", conversation_id: "a", conversation_title: "Louer un camion", created_at: hoursAgo(30) },
  { id: "f0", topic_id: "t1", text: "Les clés sont chez la voisine", conversation_id: "gone", conversation_title: null, created_at: hoursAgo(80) },
];

function openMemory(h, topicName = "Déménagement") {
  h.push();
  one(named(h.list, "topic-folder").find((folder) => text(folder, "topic-name")[0] === topicName), "topic-name").click();
  h.push();
  return h.document.getElementById("topic-page");
}

test("la carte Mémoire du sujet affiche trois faits et « Tout voir », et reste vide sans fait", () => {
  const h = harness({ ...FIXTURE, memory: { t1: FACTS } });
  const page = openMemory(h);
  const card = one(page, "topic-memory");
  assert.deepEqual(text(card, "topic-memory-fact"), ["Le garde-meuble ferme à 18 h", "Le budget est de 800 euros", "Le camion est réservé pour le 1er juillet"]);
  assert.equal(one(card, "topic-memory-all").textContent, "Tout voir");
  assert.equal(named(card, "topic-memory-empty").length, 0);

  // Un sujet sans fait garde la phrase vide et n’offre pas de « Tout voir ».
  const h2 = harness({ ...FIXTURE, memory: { t1: FACTS } });
  const other = openMemory(h2, "Impôts");
  assert.match(one(other, "topic-memory").textContent, /Aucun fait retenu pour l’instant/);
  assert.equal(named(other, "topic-memory-all").length, 0);
});

test("« Tout voir » ouvre l’écran : titre, aide, recherche, faits avec fil d’origine et date, corbeille, lien gris", () => {
  const h = harness({ ...FIXTURE, memory: { t1: FACTS } });
  const page = openMemory(h);
  one(page, "topic-memory-all").click();
  h.push();
  assert.equal(h.document.getElementById("topic-memory-title").textContent, "Mémoire du sujet");
  assert.match(one(page, "topic-page-description").textContent, /Ce que tous les fils de « Déménagement » savent en commun/);
  const search = h.document.getElementById("topic-memory-search");
  assert.equal(search.getAttribute("aria-label"), "Rechercher dans la mémoire du sujet");
  assert.deepEqual(text(page, "topic-memory-text"), FACTS.map((fact) => fact.text));
  // Chaque fait : la pastille de son fil d’origine (ou « Fil supprimé ») et sa date.
  assert.deepEqual(text(page, "topic-memory-chip"), ["Changer d’adresse", "Louer un camion", "Louer un camion", "Fil supprimé"]);
  assert.equal(text(page, "topic-memory-date").length, 4);
  assert.ok(text(page, "topic-memory-date").every((date) => /\d/.test(date)));
  const trash = named(page, "topic-memory-forget");
  assert.equal(trash.length, 4);
  assert.ok(trash.every((button) => button.getAttribute("aria-label") === "Oublier ce fait"));
  assert.equal(one(page, "topic-memory-delete").textContent, "Supprimer le sujet…");
  assert.equal(one(page, "topic-memory-back").getAttribute("aria-label"), "Retour au sujet Déménagement");
  // Retour : la liste des fils revient.
  one(page, "topic-memory-back").click();
  h.push();
  assert.equal(named(page, "topic-memory-screen").length, 0);
  assert.equal(text(page, "topic-page-thread-title").length, 2);
});

test("le champ de recherche filtre la liste des faits, sans accent ni majuscule", () => {
  const h = harness({ ...FIXTURE, memory: { t1: FACTS } });
  const page = openMemory(h);
  one(page, "topic-memory-all").click();
  h.push();
  const search = h.document.getElementById("topic-memory-search");
  search.value = "JUILLET";
  search.listeners.input();
  assert.deepEqual(text(page, "topic-memory-text"), ["Le camion est réservé pour le 1er juillet"]);
  search.value = "reserve";
  search.listeners.input();
  assert.deepEqual(text(page, "topic-memory-text"), ["Le camion est réservé pour le 1er juillet"]);
  // Le titre du fil d’origine compte aussi.
  search.value = "adresse";
  search.listeners.input();
  assert.deepEqual(text(page, "topic-memory-text"), ["Le garde-meuble ferme à 18 h"]);
  search.value = "introuvable";
  search.listeners.input();
  assert.deepEqual(text(page, "topic-memory-text"), []);
  assert.match(one(page, "topic-memory-none").textContent, /Aucun fait ne correspond/);
  search.value = "";
  search.listeners.input();
  assert.equal(text(page, "topic-memory-text").length, 4);
  // Un nouvel état (sondage) ne vide pas la recherche en cours.
  search.value = "budget";
  search.listeners.input();
  h.push();
  assert.equal(h.document.getElementById("topic-memory-search").value, "budget");
  assert.deepEqual(text(page, "topic-memory-text"), ["Le budget est de 800 euros"]);
});

test("la corbeille demande « Oublier ce fait ? » ; Annuler ne touche à rien, Oublier retire le fait", async () => {
  const h = harness({ ...FIXTURE, memory: { t1: FACTS } });
  const page = openMemory(h);
  one(page, "topic-memory-all").click();
  h.push();
  named(page, "topic-memory-forget")[1].click();
  const dialog = h.document.getElementById("topic-forget-dialog");
  assert.ok(dialog.opened);
  assert.equal(dialog.querySelector("h2").textContent, "Oublier ce fait ?");
  assert.equal(all(dialog).find((el) => el.classes.includes("delete-description")).textContent, "Il sera retiré de tous les fils du sujet.");
  assert.equal(dialog.querySelector("#topic-forget-quote").textContent, "Le budget est de 800 euros");
  const buttons = all(dialog).filter((el) => el.tagName === "button").map((el) => el.textContent);
  assert.deepEqual(buttons, ["Annuler", "Oublier"]);
  all(dialog).find((el) => el.tagName === "button" && el.textContent === "Annuler").click();
  assert.equal(dialog.opened, false);
  assert.deepEqual(plain(h.calls.api), []);

  named(page, "topic-memory-forget")[1].click();
  dialog.querySelector("#topic-confirm-forget").click();
  await settle();
  assert.deepEqual(plain(h.calls.api), [{ path: "/api/topics/memory/forget", input: { fact_id: "f2" } }]);
  assert.equal(dialog.opened, false);
});

test("« Supprimer le sujet… » dit que les faits partent et envoie la confirmation ; sans fait, rien de plus", async () => {
  const h = harness({ ...FIXTURE, memory: { t1: FACTS } });
  const page = openMemory(h);
  one(page, "topic-memory-all").click();
  h.push();
  one(page, "topic-memory-delete").click();
  const dialog = h.document.getElementById("topic-delete-dialog");
  assert.ok(dialog.opened);
  assert.match(dialog.querySelector("#topic-delete-description").textContent, /Ses 4 faits retenus sont effacés ; la mémoire privée des bots reste/);
  dialog.querySelector("#topic-confirm-delete").click();
  await settle();
  assert.deepEqual(plain(h.calls.api), [{ path: "/api/topics/delete", input: { id: "t1", forget_memory: true } }]);

  const empty = harness({ ...FIXTURE, memory: { t1: FACTS } });
  const impots = openMemory(empty, "Impôts");
  one(impots, "topic-delete").click();
  const confirm = empty.document.getElementById("topic-delete-dialog");
  assert.doesNotMatch(confirm.querySelector("#topic-delete-description").textContent, /faits retenus/);
  confirm.querySelector("#topic-confirm-delete").click();
  await settle();
  assert.deepEqual(plain(empty.calls.api), [{ path: "/api/topics/delete", input: { id: "t2" } }]);
});

test("un autre sujet ne montre pas les faits de celui-ci", () => {
  const h = harness({ ...FIXTURE, memory: { t1: FACTS } });
  const page = openMemory(h, "Impôts");
  const card = one(page, "topic-memory");
  assert.deepEqual(text(card, "topic-memory-fact"), []);
  assert.ok(!card.textContent.includes("camion"));
});

test("la carte « Retenir pour ce sujet ? » : Retenir et Pas maintenant appellent la route, le lien ouvre la mémoire", async () => {
  const h = harness({ ...FIXTURE, memory: { t1: FACTS } });
  h.push();
  const offer = (status) => ({ id: "topic-offer-1", kind: "topic_memory_offer", payload: { offer_id: "topic-offer-1", status, topic_id: "t1", topic_name: "Déménagement", text: "Le camion est réservé" } });
  assert.equal(await h.panel().onCardAction(offer("pending"), "accept"), true);
  assert.equal(await h.panel().onCardAction(offer("pending"), "dismiss"), true);
  assert.deepEqual(plain(h.calls.api), [
    { path: "/api/topics/memory/accept", input: { offer_id: "topic-offer-1" } },
    { path: "/api/topics/memory/dismiss", input: { offer_id: "topic-offer-1" } },
  ]);
  // Une autre carte n’est pas la sienne.
  assert.equal(await h.panel().onCardAction({ kind: "memory_share" }, "accept"), false);
  assert.equal(await h.panel().onCardAction(offer("pending"), "autre"), false);
  // Le lien « Mémoire du sujet » de la carte retenue ouvre l’écran de ce sujet.
  assert.equal(await h.panel().onCardAction(offer("accepted"), "open-memory"), true);
  h.push();
  const page = h.document.getElementById("topic-page");
  assert.equal(h.document.getElementById("topic-memory-title").textContent, "Mémoire du sujet");
  assert.equal(text(page, "topic-memory-text").length, 4);
  // Un sujet supprimé depuis ne s’ouvre plus : l’erreur reste sur la carte.
  const gone = offer("accepted");
  gone.payload.topic_id = "disparu";
  await assert.rejects(() => h.panel().onCardAction(gone, "open-memory"), /n’existe plus/);
});
