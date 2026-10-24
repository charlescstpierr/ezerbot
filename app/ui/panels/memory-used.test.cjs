// Run with: node --test app/ui/panels/memory-used.test.cjs
// « N souvenirs utilisés » sous une réponse : l'écran Mémoire s'ouvre sur ces souvenirs (point d'entrée `window.EzerbotMemorySpace.openWith`),
// chargé avec le panneau de détail réel (API publique `window.EzerbotMemoryDetail`), comme dans le bundle des panneaux.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const attr = (el, name) => el.dataset[name.slice(5).replace(/-(\w)/g, (_, c) => c.toUpperCase())] ?? el.attributes[name];
class Element {
  constructor(tag) { Object.assign(this, { tagName: tag, children: [], classes: [], dataset: {}, attributes: {}, listeners: {}, hidden: false, disabled: false, value: "" }); this.classList = { toggle: (name, on) => { this.classes = this.classes.filter((c) => c !== name); if (on) this.classes.push(name); }, remove: (name) => this.classList.toggle(name, false), add: (name) => this.classes.push(name) }; }
  append(...kids) { for (const kid of kids) { if (kid instanceof Element) kid.parentElement = this; this.children.push(kid); } }
  replaceChildren(...kids) { this.children = []; this.append(...kids); }
  insertBefore(kid, before) { kid.parentElement = this; const i = this.children.indexOf(before); this.children.splice(i < 0 ? this.children.length : i, 0, kid); }
  setAttribute(key, value) { this.attributes[key] = String(value); if (key === "class") this.className = value; }
  removeAttribute(key) { delete this.attributes[key]; }
  addEventListener(name, action) { (this.listeners[name] ||= []).push(action); }
  fire(name, extra = {}) { const event = { target: this, preventDefault() { this.defaultPrevented = true; }, ...extra }; for (const action of this.listeners[name] || []) action(event); return event; }
  click() { this.fire("click"); }
  focus() { Element.active = this; }
  contains(el) { return el === this || this.children.some((kid) => kid instanceof Element && kid.contains(el)); }
  matches(selector) { return selector.split(", ").some((part) => { const [, tag, classes, attrs] = part.match(/^(\w*)((?:\.[\w-]+)*)((?:\[[^\]]+\])*)$/); return (!tag || tag === this.tagName) && classes.split(".").filter(Boolean).every((c) => this.classes.includes(c)) && [...attrs.matchAll(/\[([\w-]+)="([^"]*)"\]/g)].every(([, k, v]) => attr(this, k) === v); }); }
  querySelectorAll(selector) { const all = (el) => el.children.filter((kid) => kid instanceof Element).flatMap((kid) => [kid, ...all(kid)]); return all(this).filter((kid) => kid.matches(selector)); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  set className(value) { this.classes = String(value).split(/\s+/).filter(Boolean); }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map((kid) => (kid instanceof Element ? kid.textContent : String(kid))).join(""); }
}
const fact = (id, bot, scope, text) => ({ fact_id: id, bot_id: bot, scope, text, provenance: { source: "user_stated" }, learned_at: Date.parse("2026-10-20T10:00:00Z") });
const FACTS = [fact("f1", "grokbot", "private", "Aime le café <b>noir</b>"), fact("f2", "chef", "team", "L’équipe déjeune à midi"), fact("f3", "chef", "private", "Le garde-meuble ferme à 18 h")];
const ORIGIN = { conversationId: "c1", messageId: "reply-9" };

// `opened` : les fils que l'application sait ouvrir ; `busy` : une action est en cours dans l'application.
async function harness({ opened = ["c1"], mounted = true } = {}) {
  const sidebar = new Element("aside"), main = new Element("main"), dock = new Element("div"), bots = new Element("div"), registered = [];
  bots.className = "sidebar-bots"; dock.className = "composer-dock"; sidebar.append(bots); main.append(dock);
  const nodes = { sidebar, "conversation-title": new Element("h1"), "workspace-label": new Element("button") };
  const document = { createElement: (tag) => new Element(tag), createElementNS: (_, tag) => new Element(tag), getElementById: (id) => nodes[id] || null, querySelector: (s) => (s === ".main-panel" ? main : s === "dialog[open]" ? null : dock), addEventListener() {}, get activeElement() { return Element.active; }, body: new Element("body") };
  const net = { facts: FACTS.map((item) => ({ ...item })), down: false };
  const calls = { left: 0, opened: [] };
  const panels = {
    busy: false,
    register: (panel) => { registered.push(panel); return mounted; },
    leaveConversation() { calls.left += 1; registered[0].onNavigate?.(); }, // la navigation de l'application ferme l'écran Mémoire
    openConversation: (id, messageId) => { calls.opened.push([id, messageId]); return opened.includes(id); },
    async api() { await new Promise((resolve) => setImmediate(resolve)); if (net.down) throw new Error("hors ligne"); return { facts: net.facts.map((item) => ({ ...item })) }; },
  };
  const window = { GrokbotPanels: panels };
  const context = vm.createContext({ window, document, setInterval: () => 0, JSON, Number, String, Date, Promise, Object, Array, Boolean, Math, Set, Map });
  for (const file of ["memory-detail.js", "memory-filter.js", "memory-graph.js", "memory-space.js"]) vm.runInContext(fs.readFileSync(`${__dirname}/${file}`, "utf8"), context);
  const settle = async () => { for (let i = 0; i < 4; i += 1) await new Promise((resolve) => setImmediate(resolve)); };
  Element.active = null;
  registered[0].onState({ bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }], memory_auto: { revision: 1 } }, {});
  await settle();
  const space = window.EzerbotMemorySpace;
  const h = {
    reload: async () => { registered[0].onState({ bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }], memory_auto: { revision: Math.random() } }, {}); await settle(); },
    space, panels, net, calls, settle, main, nodes, navigate: () => registered[0].onNavigate(), entry: () => sidebar.querySelector(".memory-entry"),
    screen: () => main.querySelector(".memory-space"),
    async openWith(ids, origin = ORIGIN) { const result = space.openWith(ids, origin); await settle(); return result; },
    items: () => main.querySelectorAll(".memory-space-item").map((item) => item.dataset.factId),
    panel: () => main.querySelector(".memory-detail"),
    shown: () => !h.panel().hidden,
    detailText: () => h.panel().querySelector(".memory-detail-text").textContent,
    selected: () => main.querySelectorAll(".is-selected").map((el) => el.dataset.factId),
    bar: () => main.querySelector(".memory-space-used"),
    buttons: () => h.bar().querySelectorAll("button").map((button) => button.textContent),
    press: (label) => h.bar().querySelectorAll("button").find((button) => button.textContent === label).click(),
  };
  return h;
}

test("un souvenir : l'écran Mémoire s'ouvre, la liste reste entière, le détail est celui du bon souvenir", async () => {
  const h = await harness();
  assert.equal((await h.openWith(["f2"])).ok, true);
  assert.equal(h.calls.left, 1, "le fil est quitté");
  assert.equal(h.screen().hidden, false);
  assert.equal(h.nodes["conversation-title"].textContent, "Mémoire · 3 souvenirs");
  assert.deepEqual(h.items(), ["f1", "f2", "f3"], "un seul souvenir ne restreint pas la Liste");
  assert.equal(h.shown(), true);
  assert.equal(h.detailText(), "L’équipe déjeune à midi");
  assert.deepEqual(h.selected(), ["f2"]);
  assert.equal(h.bar().hidden, false);
  assert.equal(h.bar().querySelector("p").textContent, "1 souvenir utilisé dans cette réponse.");
  assert.equal(Element.active, h.panel().querySelector("h2"), "le focus passe au détail");
});

test("plusieurs souvenirs : la Liste ne montre qu'eux, le premier est ouvert, « Tout afficher » rend toute la mémoire", async () => {
  const h = await harness();
  await h.openWith(["f3", "f1"]);
  assert.deepEqual(h.items(), ["f1", "f3"]);
  assert.equal(h.detailText(), "Le garde-meuble ferme à 18 h", "le premier de la réponse, pas celui de la liste");
  assert.equal(h.bar().querySelector("p").textContent, "2 souvenirs utilisés dans cette réponse.");
  assert.deepEqual(h.buttons(), ["Tout afficher", "Retour à la réponse"]);
  h.press("Tout afficher");
  assert.deepEqual(h.items(), ["f1", "f2", "f3"]);
  assert.equal(h.shown(), true, "le détail ouvert reste ouvert");
  assert.deepEqual(h.buttons(), ["Retour à la réponse"]);
  assert.equal(Element.active, h.screen().querySelector(".is-list"), "le focus n'est pas perdu avec le bouton");
});

test("le graphe n'est pas laissé affiché : l'écran s'ouvre en Liste", async () => {
  const h = await harness();
  await h.openWith(["f1"]);
  h.screen().querySelector(".is-graph").click();
  await h.openWith(["f2", "f3"]);
  assert.equal(h.screen().querySelector(".is-list").attributes["aria-pressed"], "true");
  assert.deepEqual(h.items(), ["f2", "f3"]);
});

test("un souvenir oublié depuis la réponse : message lisible, aucun panneau vide", async () => {
  const h = await harness();
  await h.settle();
  h.net.facts = h.net.facts.filter((item) => item.fact_id !== "f2"); // oublié entre l'affichage de la réponse et le clic
  await h.openWith(["f2"]);
  assert.equal(h.shown(), false);
  assert.equal(h.bar().querySelector("p").textContent, "Ce souvenir n’existe plus : il a été oublié ou remplacé depuis cette réponse.");
  assert.equal(h.bar().querySelector("p").attributes.role, "status");
  assert.deepEqual(h.items(), ["f1", "f3"], "la mémoire reste consultable");
  assert.deepEqual(h.selected(), []);
  await h.openWith(["gone", "gone-too"]);
  assert.match(h.bar().querySelector("p").textContent, /^Ces souvenirs n’existent plus : ils ont été oubliés ou remplacés/);
  assert.equal(h.shown(), false);
});

test("un souvenir disparu parmi d'autres : les survivants s'ouvrent, le manque est dit", async () => {
  const h = await harness();
  await h.openWith(["gone", "f2", "f1"]);
  assert.deepEqual(h.items(), ["f1", "f2"]);
  assert.equal(h.detailText(), "L’équipe déjeune à midi", "le premier qui existe encore");
  assert.equal(h.bar().querySelector("p").textContent, "3 souvenirs utilisés dans cette réponse. 1 n’existe plus (oublié ou remplacé depuis).");
  await h.openWith(["gone", "f3"]);
  assert.deepEqual(h.items(), ["f3"]);
  assert.equal(h.bar().querySelector("p").textContent, "2 souvenirs utilisés dans cette réponse. 1 n’existe plus (oublié ou remplacé depuis).");
});

test("un souvenir ouvert avant est remplacé par celui de la réponse, et le texte reste du texte", async () => {
  const h = await harness();
  await h.openWith(["f1"]);
  assert.equal(h.detailText(), "Aime le café <b>noir</b>");
  await h.openWith(["f3"]);
  assert.equal(h.detailText(), "Le garde-meuble ferme à 18 h");
  assert.deepEqual(h.selected(), ["f3"]);
  await h.openWith(["gone"]);
  assert.equal(h.shown(), false, "le détail d'avant ne reste pas sous un message qui dit autre chose");
});

test("un souvenir oublié pendant que l'écran est ouvert : le bandeau suit, la Liste n'est jamais vide", async () => {
  const h = await harness();
  await h.openWith(["f1", "f2"]);
  assert.deepEqual(h.items(), ["f1", "f2"]);
  h.net.facts = h.net.facts.filter((item) => item.fact_id !== "f1");
  await h.reload();
  assert.deepEqual(h.items(), ["f2"]);
  assert.equal(h.bar().querySelector("p").textContent, "2 souvenirs utilisés dans cette réponse. 1 n’existe plus (oublié ou remplacé depuis).");
  h.net.facts = h.net.facts.filter((item) => item.fact_id !== "f2");
  await h.reload();
  assert.deepEqual(h.items(), ["f3"], "plus aucun souvenir de la réponse : la mémoire entière, pas une liste vide");
  assert.equal(h.shown(), false, "pas de panneau sur un souvenir disparu");
  assert.match(h.bar().querySelector("p").textContent, /^Ces souvenirs n’existent plus/);
  assert.deepEqual(h.buttons(), ["Retour à la réponse"]);
});

test("« Retour à la réponse » rouvre la conversation sur la réponse ; sinon l'écran le dit", async () => {
  const h = await harness();
  await h.openWith(["f1"]);
  h.press("Retour à la réponse");
  assert.deepEqual(h.calls.opened, [["c1", "reply-9"]]);
  const lost = await harness({ opened: [] });
  await lost.openWith(["f1"]);
  lost.press("Retour à la réponse");
  assert.equal(lost.bar().querySelector(".memory-space-used-problem").textContent, "Cette conversation n’existe plus, ou une autre action est en cours.");
  const bare = await harness();
  await bare.openWith(["f1"], null);
  assert.deepEqual(bare.buttons(), [], "sans fil d'origine, pas de retour proposé");
});

test("la mémoire ouverte autrement (entrée de la barre latérale) ne garde ni bandeau ni restriction", async () => {
  const h = await harness();
  await h.openWith(["f1", "f2"]);
  h.entry().click();
  await h.settle();
  assert.equal(h.bar().hidden, true);
  assert.deepEqual(h.items(), ["f1", "f2", "f3"]);
  await h.openWith(["f1", "f2"]);
  h.navigate(); // l'écran se ferme à tout changement de fil
  assert.equal(h.screen().hidden, true);
  h.entry().click();
  await h.settle();
  assert.deepEqual([h.bar().hidden, h.items(), h.shown()], [true, ["f1", "f2", "f3"], false]);
});

test("une liste qui ne peut pas être lue ne devine rien : ni détail, ni bandeau", async () => {
  const h = await harness();
  h.net.down = true;
  await h.openWith(["f1"]);
  assert.equal(h.shown(), false);
  assert.equal(h.bar().hidden, true);
  h.net.down = false;
  h.entry().click();
  await h.settle();
  assert.equal(h.shown(), false, "la demande périmée ne s'ouvre pas plus tard");
});

test("pendant une action de l'application, rien ne s'ouvre et la raison est donnée ; sans identifiant non plus", async () => {
  const h = await harness();
  h.panels.busy = true;
  const refused = h.space.openWith(["f1"], ORIGIN);
  assert.equal(refused.ok, false);
  assert.match(refused.message, /action est en cours/);
  assert.equal(h.calls.left, 0);
  h.panels.busy = false;
  assert.equal(h.space.openWith([], ORIGIN).ok, false);
  assert.equal(h.calls.left, 0);
});

test("là où l'écran Mémoire ne se monte pas (iPhone distant), aucun point d'entrée n'est exposé", async () => {
  const h = await harness({ mounted: false });
  assert.equal(h.space, undefined);
});
