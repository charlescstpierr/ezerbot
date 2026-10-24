// Run with: node --test app/ui/panels/memory-space.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.classes = []; this.dataset = {}; this.attributes = {}; this.listeners = {}; this.hidden = false; this.classList = { toggle: (name, on) => { this.classes = this.classes.filter((c) => c !== name); if (on) this.classes.push(name); } }; }
  append(...kids) { for (const kid of kids) { if (kid instanceof Element) kid.parentElement = this; this.children.push(kid); } }
  insertBefore(kid, before) { kid.parentElement = this; const i = this.children.indexOf(before); if (i < 0) this.children.push(kid); else this.children.splice(i, 0, kid); }
  replaceChildren(...kids) { this.children = []; this.append(...kids); }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  getAttribute(key) { return this.attributes[key]; }
  removeAttribute(key) { delete this.attributes[key]; }
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { this.listeners.click?.({ target: this }); }
  focus() {}
  querySelectorAll(selector) { return this.children.filter((kid) => kid instanceof Element).flatMap((kid) => [...(kid.classes.includes(selector.slice(1)) ? [kid] : []), ...kid.querySelectorAll(selector)]); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  set className(value) { this.classes = String(value).split(/\s+/).filter(Boolean); }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map((kid) => (kid instanceof Element ? kid.textContent : String(kid))).join(""); }
}
const named = (root, className) => root.querySelectorAll(`.${className}`);
const FACTS = [
  { fact_id: "f1", bot_id: "grokbot", bot_deleted: false, scope: "private", text: "Aime le café <b>noir</b>", learned_at: Date.parse("2026-10-20T10:00:00Z") },
  { fact_id: "f2", bot_id: "chef", bot_deleted: false, scope: "team", text: "L’équipe déjeune à midi", learned_at: Date.parse("2026-10-19T10:00:00Z") },
  { fact_id: "f3", bot_id: "vieux", bot_deleted: true, scope: "topic", text: "Le garde-meuble ferme à 18 h", learned_at: Date.parse("2026-10-18T10:00:00Z"), topic_name: "Déménagement" },
];
function harness({ facts = FACTS, fail = false, registered = true } = {}) {
  const sidebar = new Element("aside");
  const bots = new Element("div"); bots.className = "sidebar-bots";
  sidebar.append(bots);
  const main = new Element("main");
  const dock = new Element("div"); dock.className = "composer-dock";
  main.append(dock);
  const nodes = { sidebar, "conversation-title": new Element("h1"), "workspace-label": new Element("button") };
  const document = { title: "Ezerbot", createElement: (tag) => new Element(tag), getElementById: (id) => nodes[id] || null, querySelector: (selector) => (selector === ".main-panel" ? main : dock) };
  const calls = { left: 0, api: [] };
  let panel = null;
  const panels = {
    busy: false,
    register(value) { panel = value; return registered; },
    async api(path) { calls.api.push(path); if (fail || panels.down) throw new Error("hors ligne"); return { facts }; },
    leaveConversation() { calls.left += 1; },
  };
  vm.runInContext(fs.readFileSync(`${__dirname}/memory-space.js`, "utf8"), vm.createContext({ window: { GrokbotPanels: panels }, document, setInterval: () => 0, JSON, Number, String, Date, Promise, Object, Array }));
  const state = { bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }, { id: "vieux", name: "Ancien" }], memory_auto: { revision: 1 } };
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  return {
    panels, sidebar, main, nodes, calls, settle, panel: () => panel,
    entry: () => sidebar.querySelector(".memory-entry"), screen: () => main.querySelector(".memory-space"),
    async open(conversation = null) { panel.onState(state, { conversation }); await settle(); this.entry().click(); await settle(); },
  };
}
test("l’entrée « Mémoire » se place sous Rechercher, avant les bots, avec le bon compteur", async () => {
  const h = harness();
  h.panel().onState({ bots: [] }, {});
  await h.settle();
  assert.equal(h.sidebar.children[0].classes[0], "sidebar-memory");
  assert.equal(h.entry().querySelector(".memory-entry-count").textContent, "3");
  assert.equal(h.entry().getAttribute("aria-label"), "Mémoire, 3 souvenirs");
  assert.deepEqual(h.calls.api, ["/api/memory/all"]);
});
test("un clic ouvre « Espace personnel / Mémoire · 3 souvenirs » avec Graphe (bientôt) et Liste", async () => {
  const h = harness();
  await h.open({ id: "c1" });
  assert.equal(h.calls.left, 1, "le fil affiché est quitté");
  assert.equal(h.screen().hidden, false);
  assert.equal(h.main.dataset.memorySpace, "list");
  assert.equal(h.nodes["workspace-label"].textContent, "Espace personnel");
  assert.equal(h.nodes["conversation-title"].textContent, "Mémoire · 3 souvenirs");
  assert.equal(h.entry().getAttribute("aria-current"), "page");
  const [graph, list] = named(h.screen(), "memory-space-tab");
  assert.match(graph.textContent, /Graphe\s*bientôt/);
  assert.equal(list.getAttribute("aria-pressed"), "true");
  graph.click();
  assert.match(h.screen().textContent, /arrive bientôt/);
  assert.equal(named(h.screen(), "memory-space-item").length, 0);
});
test("la Liste montre tous les souvenirs : texte, bot, portée, date, sans HTML interprété", async () => {
  const h = harness();
  await h.open();
  assert.equal(h.calls.left, 1, "sans fil ouvert, on quitte aussi la page d’un sujet éventuelle");
  const items = named(h.screen(), "memory-space-item");
  const cells = (className) => items.map((item) => named(item, className)[0].textContent);
  assert.deepEqual(items.map((item) => item.dataset.factId), ["f1", "f2", "f3"]);
  assert.deepEqual(cells("memory-space-text"), ["Aime le café <b>noir</b>", "L’équipe déjeune à midi", "Le garde-meuble ferme à 18 h"]);
  assert.deepEqual(cells("memory-space-bot"), ["Ezerbot", "Chef", "Ancien (supprimé)"]);
  assert.deepEqual(cells("memory-space-scope"), ["Privé", "Équipe", "Sujet · Déménagement"]);
  for (const date of cells("memory-space-date")) assert.match(date, /2026/);
  assert.equal(named(items[0], "memory-space-text")[0].children.length, 1, "le texte reste un simple noeud texte");
});
test("zéro souvenir : état vide lisible ; un seul : singulier ; échec : pas de compteur inventé", async () => {
  const none = harness({ facts: [] });
  await none.open();
  assert.equal(none.entry().querySelector(".memory-entry-count").textContent, "0");
  assert.equal(none.nodes["conversation-title"].textContent, "Mémoire · 0 souvenir");
  assert.match(named(none.screen(), "memory-space-empty")[0].textContent, /Aucun souvenir pour l’instant/);
  const one = harness({ facts: [FACTS[0]] });
  await one.open();
  assert.equal(one.nodes["conversation-title"].textContent, "Mémoire · 1 souvenir");
  // Échec de chargement : message honnête, pas de compteur inventé.
  const down = harness({ fail: true });
  await down.open();
  assert.equal(down.entry().querySelector(".memory-entry-count").textContent, "");
  assert.equal(down.nodes["conversation-title"].textContent, "Mémoire");
  assert.match(named(down.screen(), "memory-space-empty")[0].textContent, /n’a pas pu être chargée/);
  // Un rechargement manqué garde la dernière liste, signalée comme telle.
  const stale = harness();
  await stale.open();
  stale.panels.down = true;
  stale.panel().onNavigate();
  stale.entry().click();
  await stale.settle();
  assert.equal(named(stale.screen(), "memory-space-item").length, 3);
  assert.match(stale.screen().textContent, /dernier état connu/);
});
test("changer d’écran ferme la Mémoire ; occupé, le clic ne fait rien ; non enregistré, rien n’est monté", async () => {
  const h = harness();
  await h.open();
  h.panel().onNavigate();
  assert.equal(h.screen().hidden, true);
  assert.equal(h.main.dataset.memorySpace, undefined);
  assert.equal(h.entry().getAttribute("aria-current"), undefined);
  h.panels.busy = true;
  h.entry().click();
  assert.equal(h.screen().hidden, true);
  assert.equal(harness({ registered: false }).sidebar.children.length, 1);
});
