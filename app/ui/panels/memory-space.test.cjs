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
  setAttribute(key, value) { this.attributes[key] = String(value); if (key === "class") this.className = value; }
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
  const document = { title: "Ezerbot", createElement: (tag) => new Element(tag), createElementNS: (_, tag) => new Element(tag), getElementById: (id) => nodes[id] || null, querySelector: (selector) => (selector === ".main-panel" ? main : dock) };
  const calls = { left: 0, api: [] };
  let panel = null;
  const panels = {
    busy: false,
    register(value) { panel = value; return registered; },
    async api(path) { calls.api.push(path); if (fail || panels.down) throw new Error("hors ligne"); return { facts }; },
    leaveConversation() { calls.left += 1; },
  };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document, setInterval: () => 0, JSON, Number, String, Date, Promise, Object, Array, Math, Set, Map });
  for (const file of ["memory-graph.js", "memory-space.js"]) vm.runInContext(fs.readFileSync(`${__dirname}/${file}`, "utf8"), context);
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
test("un clic ouvre « Espace personnel / Mémoire · 3 souvenirs » avec Graphe et Liste", async () => {
  const h = harness();
  await h.open({ id: "c1" });
  assert.equal(h.calls.left, 1, "le fil affiché est quitté");
  assert.equal(h.screen().hidden, false);
  assert.equal(h.main.dataset.memorySpace, "list");
  assert.equal(h.nodes["workspace-label"].textContent, "Espace personnel");
  assert.equal(h.nodes["conversation-title"].textContent, "Mémoire · 3 souvenirs");
  assert.equal(h.entry().getAttribute("aria-current"), "page");
  const [graph, list] = named(h.screen(), "memory-space-tab");
  assert.equal(graph.textContent, "Graphe");
  assert.equal(list.getAttribute("aria-pressed"), "true");
  graph.click();
  assert.equal(graph.getAttribute("aria-pressed"), "true");
  assert.equal(named(h.screen(), "memory-graph-point").length, 3);
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
  // Échec de chargement : message honnête, pas de compteur inventé.
  const down = harness({ fail: true });
  await down.open();
  assert.equal(down.entry().querySelector(".memory-entry-count").textContent, "");
  assert.equal(down.nodes["conversation-title"].textContent, "Mémoire");
  assert.match(named(down.screen(), "memory-space-empty")[0].textContent, /n’a pas pu être chargée/);
  // Un rechargement manqué garde la dernière liste, même vide, signalée comme telle.
  for (const known of [FACTS, []]) {
    const stale = harness({ facts: known });
    await stale.open();
    stale.panels.down = true;
    stale.panel().onNavigate();
    stale.entry().click();
    await stale.settle();
    assert.equal(named(stale.screen(), "memory-space-item").length, known.length);
    assert.match(stale.screen().textContent, /dernier état connu/);
  }
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
const view = (h) => named(h.screen(), "memory-graph-canvas")[0].getAttribute("viewBox").split(" ").map(Number);
const hasClass = (el, name) => el.classes.includes(name);
test("Graphe : un point par souvenir, un centre, une pastille par groupe, la couleur suit la portée", async () => {
  const h = harness();
  await h.open();
  named(h.screen(), "memory-space-tab")[0].click();
  const points = named(h.screen(), "memory-graph-point");
  assert.equal(points.length, 3);
  assert.equal(named(h.screen(), "memory-graph-center").length, 1);
  assert.equal(named(h.screen(), "memory-graph-group").length, 3, "Personnel, Travail et Déménagement");
  assert.deepEqual(points.map((point) => point.classes.find((name) => name.startsWith("is-"))), ["is-private", "is-team", "is-topic"]);
  assert.deepEqual(named(h.screen(), "memory-graph-group").map((group) => group.getAttribute("aria-label")), ["Personnel, 1 souvenir", "Travail, 1 souvenir", "Déménagement, 1 souvenir"]);
  assert.match(points[0].getAttribute("aria-label"), /^Ezerbot, Privé : Aime le café <b>noir<\/b>$/, "le texte reste du texte, jamais du HTML");
  assert.deepEqual(named(h.screen(), "memory-graph-key-name").map((key) => key.textContent), ["Privé", "Équipe", "Sujet"]);
  assert.equal(named(h.screen(), "memory-graph-label").map((label) => label.textContent).join("|"), "Vous|Personnel|Travail|Déménagement");
});
test("Graphe : zoom +, −, recadrer ; survol et focus mettent un nœud en avant et estompent les autres", async () => {
  const h = harness();
  await h.open();
  named(h.screen(), "memory-space-tab")[0].click();
  const [plus, minus, reset] = named(h.screen(), "memory-graph-tool");
  assert.deepEqual([plus, minus, reset].map((button) => button.getAttribute("aria-label")), ["Zoom avant", "Zoom arrière", "Recadrer"]);
  const initial = view(h);
  plus.click();
  assert.ok(view(h)[2] < initial[2] && view(h)[3] < initial[3], "zoom avant : la vue se resserre");
  reset.click();
  assert.deepEqual(view(h), initial);
  minus.click();
  assert.ok(view(h)[2] > initial[2], "zoom arrière : la vue s’élargit");
  reset.click();
  const [first, second] = named(h.screen(), "memory-graph-point");
  for (const [on, off] of [["mouseenter", "mouseleave"], ["focus", "blur"]]) {
    first.listeners[on]();
    assert.ok(hasClass(first, "is-active") && !hasClass(first, "is-dim"));
    assert.ok(hasClass(second, "is-dim") && !hasClass(second, "is-active"), "un autre groupe s’estompe");
    assert.ok(named(h.screen(), "memory-graph-group").some((group) => !hasClass(group, "is-dim")), "son groupe reste lisible");
    first.listeners[off]();
    assert.ok(![first, second].some((point) => hasClass(point, "is-active") || hasClass(point, "is-dim")));
  }
});
test("Graphe : sans souvenir, la Liste affiche son état vide au lieu d’un graphe vide", async () => {
  const h = harness({ facts: [] });
  await h.open();
  named(h.screen(), "memory-space-tab")[0].click();
  assert.equal(named(h.screen(), "memory-graph-point").length, 0);
  assert.match(named(h.screen(), "memory-space-empty")[0].textContent, /Aucun souvenir pour l’instant/);
});
