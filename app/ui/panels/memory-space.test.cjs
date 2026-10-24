// Run with: node --test app/ui/panels/memory-space.test.cjs
// Rend vraiment l’entrée « Mémoire » et l’écran Liste dans un DOM minimal.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class Element {
  constructor(tag) { this.tagName = String(tag).toLowerCase(); this.children = []; this.classes = []; this.dataset = {}; this.attributes = {}; this.listeners = {}; this.hidden = false; this.id = ""; this.classList = { toggle: (name, on) => { this.classes = this.classes.filter((c) => c !== name); if (on) this.classes.push(name); } }; }
  append(...kids) { for (const kid of kids) { if (kid instanceof Element) kid.parentElement = this; this.children.push(kid); } }
  insertBefore(kid, before) { kid.parentElement = this; const i = this.children.indexOf(before); if (i < 0) this.children.push(kid); else this.children.splice(i, 0, kid); }
  replaceChildren(...kids) { this.children = []; this.append(...kids); }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  getAttribute(key) { return this.attributes[key]; }
  removeAttribute(key) { delete this.attributes[key]; }
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { this.listeners.click?.({ target: this }); }
  focus() {}
  querySelectorAll(selector) {
    const wanted = selector.slice(1);
    return this.children.filter((kid) => kid instanceof Element).flatMap((kid) => [...(kid.classes.includes(wanted) ? [kid] : []), ...kid.querySelectorAll(selector)]);
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  set className(value) { this.classes = String(value).split(/\s+/).filter(Boolean); }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map((kid) => (kid instanceof Element ? kid.textContent : String(kid))).join(""); }
}
const named = (root, className) => root.querySelectorAll(`.${className}`);

const FACTS = [
  { fact_id: "f1", bot_id: "grokbot", bot_deleted: false, scope: "private", text: "Aime le café <b>noir</b>", learned_at: Date.parse("2026-10-20T10:00:00Z"), topic_id: null, topic_name: null },
  { fact_id: "f2", bot_id: "chef", bot_deleted: false, scope: "team", text: "L’équipe déjeune à midi", learned_at: Date.parse("2026-10-19T10:00:00Z"), topic_id: null, topic_name: null },
  { fact_id: "f3", bot_id: "vieux", bot_deleted: true, scope: "topic", text: "Le garde-meuble ferme à 18 h", learned_at: Date.parse("2026-10-18T10:00:00Z"), topic_id: "t1", topic_name: "Déménagement" },
];

function harness({ facts = FACTS, fail = false, registered = true } = {}) {
  const sidebar = new Element("aside"); sidebar.id = "sidebar";
  const bots = new Element("div"); bots.className = "sidebar-bots";
  sidebar.append(bots);
  const main = new Element("main"); main.className = "main-panel";
  const dock = new Element("div"); dock.className = "composer-dock";
  main.append(dock);
  const nodes = { sidebar, "conversation-title": new Element("h1"), "workspace-label": new Element("button") };
  const document = {
    title: "Ezerbot",
    body: new Element("body"),
    createElement: (tag) => new Element(tag),
    getElementById: (id) => nodes[id] || null,
    querySelector: (selector) => (selector === ".main-panel" ? main : selector === ".composer-dock" ? dock : null),
  };
  const calls = { left: 0, api: [] };
  let panel = null;
  const panels = {
    busy: false,
    register(value) { panel = value; return registered; },
    async api(path) { calls.api.push(path); if (fail) throw new Error("hors ligne"); return { facts }; },
    leaveConversation() { calls.left += 1; return true; },
  };
  const context = { window: { GrokbotPanels: panels }, document, setInterval: () => 0, console, JSON, Number, String, Array, Object, Date, Promise };
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(`${__dirname}/memory-space.js`, "utf8"), context);
  const state = { bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }, { id: "vieux", name: "Ancien" }], memory_auto: { revision: 1 } };
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  const push = async (conversation = null) => { panel.onState(state, { conversation }); await settle(); };
  return { panels, sidebar, main, nodes, dock, calls, panel: () => panel, push, settle, entry: () => sidebar.querySelector(".memory-entry"), screen: () => main.querySelector(".memory-space") };
}

test("l’entrée « Mémoire » se place avant les bots et affiche le bon compteur", async () => {
  const h = harness();
  await h.push();
  assert.equal(h.sidebar.children[0].classes[0], "sidebar-memory", "sous Rechercher, avant BOTS");
  assert.equal(h.entry().querySelector(".conversation-name").textContent, "Mémoire");
  assert.equal(named(h.entry(), "memory-entry-count")[0].textContent, "3");
  assert.equal(h.entry().getAttribute("aria-label"), "Mémoire, 3 souvenirs");
  assert.deepEqual(h.calls.api, ["/api/memory/all"]);
});

test("un clic ouvre « Espace personnel / Mémoire · 3 souvenirs » avec Graphe et Liste", async () => {
  const h = harness();
  await h.push({ id: "c1" });
  h.entry().click();
  await h.settle();
  assert.equal(h.calls.left, 1, "le fil affiché est quitté");
  assert.equal(h.screen().hidden, false);
  assert.equal(h.main.dataset.memorySpace, "list");
  assert.equal(h.nodes["workspace-label"].textContent, "Espace personnel");
  assert.equal(h.nodes["conversation-title"].textContent, "Mémoire · 3 souvenirs");
  assert.equal(h.entry().getAttribute("aria-current"), "page");
  const [graph, list] = named(h.screen(), "memory-space-tab");
  assert.match(graph.textContent, /Graphe\s*bientôt/);
  assert.equal(list.textContent, "Liste");
  assert.equal(list.getAttribute("aria-pressed"), "true");
  graph.click();
  assert.match(h.screen().textContent, /arrive bientôt/);
  assert.equal(named(h.screen(), "memory-space-item").length, 0);
});

test("la Liste montre tous les souvenirs : texte, bot, portée, date, sans HTML interprété", async () => {
  const h = harness();
  await h.push();
  h.entry().click();
  await h.settle();
  const items = named(h.screen(), "memory-space-item");
  assert.deepEqual(items.map((item) => item.dataset.factId), ["f1", "f2", "f3"]);
  assert.deepEqual(items.map((item) => named(item, "memory-space-text")[0].textContent), ["Aime le café <b>noir</b>", "L’équipe déjeune à midi", "Le garde-meuble ferme à 18 h"]);
  assert.deepEqual(items.map((item) => named(item, "memory-space-bot")[0].textContent), ["Ezerbot", "Chef", "Ancien (supprimé)"]);
  assert.deepEqual(items.map((item) => named(item, "memory-space-scope")[0].textContent), ["Privé", "Équipe", "Sujet · Déménagement"]);
  for (const item of items) assert.match(named(item, "memory-space-date")[0].textContent, /2026/);
  assert.equal(items[0].querySelectorAll(".memory-space-text")[0].children.length, 1, "le texte reste un simple noeud texte");
});

test("zéro souvenir : compteur 0 et état vide lisible", async () => {
  const h = harness({ facts: [] });
  await h.push();
  h.entry().click();
  await h.settle();
  assert.equal(named(h.entry(), "memory-entry-count")[0].textContent, "0");
  assert.equal(h.nodes["conversation-title"].textContent, "Mémoire · 0 souvenir");
  assert.match(named(h.screen(), "memory-space-empty")[0].textContent, /Aucun souvenir pour l’instant/);
});

test("un souvenir : singulier ; échec de chargement : message, pas de compteur faux", async () => {
  const one = harness({ facts: [FACTS[0]] });
  await one.push();
  one.entry().click();
  assert.equal(one.nodes["conversation-title"].textContent, "Mémoire · 1 souvenir");
  const down = harness({ fail: true });
  await down.push();
  down.entry().click();
  await down.settle();
  assert.equal(named(down.entry(), "memory-entry-count")[0].textContent, "");
  assert.equal(down.nodes["conversation-title"].textContent, "Mémoire");
  assert.match(named(down.screen(), "memory-space-empty")[0].textContent, /n’a pas pu être chargée/);
});

test("changer d’écran ferme la Mémoire ; occupé, le clic ne fait rien ; hors iPhone distant, rien n’est monté", async () => {
  const h = harness();
  await h.push();
  h.entry().click();
  h.panel().onNavigate();
  assert.equal(h.screen().hidden, true);
  assert.equal(h.main.dataset.memorySpace, undefined);
  assert.equal(h.entry().getAttribute("aria-current"), undefined);
  const busy = harness();
  await busy.push();
  busy.panels.busy = true;
  busy.entry().click();
  assert.equal(busy.screen().hidden, true);
  const remote = harness({ registered: false });
  assert.equal(remote.sidebar.children.length, 1, "aucune entrée quand le panneau n’est pas enregistré");
});
