// Run with: node --test app/ui/panels/memory-used.test.cjs
// « N souvenirs utilisés » sous une réponse : l'écran Mémoire s'ouvre sur ces souvenirs (point d'entrée `window.EzerbotMemorySpace.openWith`),
// chargé avec les panneaux réels de l'écran (ajout, détail, filtres, liens, graphe, « À revoir »), dans l'ordre du bundle des panneaux.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const attr = (el, name) => el.dataset[name.slice(5).replace(/-(\w)/g, (_, c) => c.toUpperCase())] ?? el.attributes[name];
class Element {
  constructor(tag) { Object.assign(this, { tagName: tag, children: [], classes: [], dataset: {}, attributes: {}, listeners: {}, hidden: false, disabled: false, value: "" }); this.classList = { toggle: (name, on) => { this.classes = this.classes.filter((c) => c !== name); if (on) this.classes.push(name); }, remove: (name) => this.classList.toggle(name, false), add: (name) => this.classes.push(name) }; }
  append(...kids) { for (const kid of kids) { if (kid instanceof Element) kid.parentElement = this; this.children.push(kid); } }
  replaceChildren(...kids) { this.children = []; this.append(...kids); }
  remove() { if (this.parentElement) this.parentElement.children = this.parentElement.children.filter((kid) => kid !== this); }
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

// `opened` : les fils que l'application sait ouvrir ; `facts` : la mémoire du moteur ; `loaded` : la liste a été lue avant le clic.
async function harness({ opened = ["c1"], mounted = true, facts = FACTS, loaded = true } = {}) {
  const sidebar = new Element("aside"), main = new Element("main"), dock = new Element("div"), bots = new Element("div"), registered = [];
  bots.className = "sidebar-bots"; dock.className = "composer-dock"; sidebar.append(bots); main.append(dock);
  const nodes = { sidebar, "conversation-title": new Element("h1"), "workspace-label": new Element("button") };
  const document = { createElement: (tag) => new Element(tag), createElementNS: (_, tag) => new Element(tag), getElementById: (id) => nodes[id] || null, querySelector: (s) => (s === ".main-panel" ? main : s === "dialog[open]" ? null : dock), addEventListener() {}, get activeElement() { return Element.active; }, body: new Element("body") };
  // `slow` : nombre de tours que prend chaque lecture de la liste, dans l'ordre des lectures (1 par défaut) ; `hold` : une action d'écriture attend cette promesse.
  const net = { facts: facts.map((item) => ({ ...item })), down: false, slow: [], hold: null };
  const calls = { left: 0, opened: [], api: [] };
  const tick = () => new Promise((resolve) => setImmediate(resolve));
  const panels = {
    busy: false,
    register: (panel) => { registered.push(panel); return mounted; },
    leaveConversation() { calls.left += 1; for (const panel of registered) panel.onNavigate?.(); }, // la navigation de l'application ferme l'écran Mémoire
    openConversation: (id, messageId) => { calls.opened.push([id, messageId]); return opened.includes(id); },
    refresh: async () => {},
    notify() {},
    async api(path, body) {
      calls.api.push(path);
      if (path === "/api/memory/all") {
        const snapshot = net.facts.map((item) => ({ ...item })), down = net.down; // ce que le moteur avait au moment de la demande
        for (let i = net.slow.shift() ?? 1; i > 0; i -= 1) await tick();
        if (down) throw new Error("hors ligne");
        return { facts: snapshot };
      }
      await net.hold;
      if (path === "/api/memory/forget") net.facts = net.facts.filter((item) => item.fact_id !== body.fact_id);
      if (path === "/api/memory/reconfirm") net.facts = net.facts.map((item) => (item.fact_id === body.fact_id ? { ...item, needs_review: false } : item));
      if (path === "/api/memory/add") net.facts = [...net.facts, fact(`n${net.facts.length}`, body.bot_id, body.scope, body.text)];
      return {};
    },
  };
  const window = { GrokbotPanels: panels };
  const context = vm.createContext({ window, document, setInterval: () => 0, JSON, Number, String, Date, Promise, Object, Array, Boolean, Math, Set, Map });
  for (const file of ["memory-add.js", "memory-detail.js", "memory-filter.js", "memory-links.js", "memory-graph.js", "memory-review.js", "memory-space.js"]) vm.runInContext(fs.readFileSync(`${__dirname}/${file}`, "utf8"), context);
  const settle = async (turns = 8) => { for (let i = 0; i < turns; i += 1) await tick(); };
  Element.active = null;
  const state = () => ({ bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }], memory_auto: { revision: Math.random() } });
  const notify = () => { for (const panel of registered) panel.onState?.(state(), {}); }; // l'état de l'application change : la liste se relit
  const space = window.EzerbotMemorySpace; // exposé dès le chargement du bundle, avant tout état et toute ouverture
  notify(); // l’application passe son état au panneau dès son inscription : la première lecture part
  if (loaded) await settle();
  const h = {
    notify,
    reload: async () => { notify(); await settle(); },
    space, panels, net, calls, settle, main, nodes, navigate: () => { for (const panel of registered) panel.onNavigate?.(); }, entry: () => sidebar.querySelector(".memory-entry"),
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
    field: (label) => h.screen().querySelector(`[aria-label="${label}"]`),
    choose: (label, value) => { const el = h.field(label); el.value = value; el.fire(el.tagName === "input" ? "input" : "change"); },
    body: () => h.main.querySelector(".memory-space-body").textContent,
    review: () => h.main.querySelector(".memory-review"),
    reviewIds: () => h.main.querySelectorAll(".memory-review-item").map((item) => item.dataset.reviewId),
    detailButton: (label) => h.panel().querySelectorAll("button").find((button) => button.textContent === label),
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

test("rien à ouvrir depuis le Graphe : la Liste s'affiche et le focus est sur son onglet", async () => {
  const h = await harness();
  await h.openWith(["f1"]);
  h.screen().querySelector(".is-graph").click();
  h.screen().querySelector(".is-graph").focus();
  await h.openWith(["gone"]);
  assert.equal(h.screen().querySelector(".is-list").attributes["aria-pressed"], "true");
  assert.equal(Element.active, h.screen().querySelector(".is-list"));
  assert.equal(h.shown(), false);
});

test("les filtres d'une visite précédente ne cachent pas les souvenirs de la réponse", async () => {
  const h = await harness();
  h.entry().click();
  await h.settle();
  const field = (label) => h.screen().querySelector(`[aria-label="${label}"]`);
  field("Rechercher un souvenir").value = "garde-meuble";
  field("Rechercher un souvenir").fire("input");
  field("Filtrer par bot").value = "chef";
  field("Filtrer par bot").fire("change");
  assert.deepEqual(h.items(), ["f3"], "le filtre d'avant est bien actif");
  h.navigate();
  await h.openWith(["f1", "f2"]);
  assert.deepEqual(h.items(), ["f1", "f2"]);
  assert.equal(field("Rechercher un souvenir").value, "");
  assert.equal(field("Filtrer par bot").value, "");
  assert.equal(h.main.querySelector(".memory-filter-count").textContent, "");
  h.press("Tout afficher");
  assert.deepEqual(h.items(), ["f1", "f2", "f3"]);
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

test("le bandeau reconstruit rend le focus au bouton qui l'avait", async () => {
  const h = await harness();
  await h.openWith(["f1", "f2"]);
  const focused = (label) => h.bar().querySelectorAll("button").find((button) => button.textContent === label);
  focused("Retour à la réponse").focus();
  h.net.facts = h.net.facts.filter((item) => item.fact_id !== "f2");
  await h.reload();
  assert.match(h.bar().querySelector("p").textContent, /1 n’existe plus/);
  assert.equal(Element.active, focused("Retour à la réponse"), "le bouton du nouveau bandeau, pas un bouton disparu");
  assert.equal(h.bar().contains(Element.active), true);
  focused("Tout afficher").focus();
  h.net.facts = h.net.facts.filter((item) => item.fact_id !== "f1");
  await h.reload(); // plus de souvenir de la réponse : « Tout afficher » disparaît, le focus reste dans le bandeau
  assert.deepEqual(h.buttons(), ["Retour à la réponse"]);
  assert.equal(h.bar().contains(Element.active), true);
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

// Recensement de la classe (règle 33) : tout état laissé par une visite précédente, ou qui change pendant l'ouverture, et qui contredirait la réponse.
const C2 = { conversationId: "c2", messageId: "reply-4" };
const review = (item) => ({ ...item, needs_review: true });
const linked = (item, conversation) => ({ ...item, provenance: { ...item.provenance, conversation_id: conversation } });

test("écran Mémoire jamais ouvert et liste pas encore lue au clic : le point d'entrée existe, la réponse s'ouvre une fois la liste lue", async () => {
  const h = await harness({ loaded: false });
  assert.equal(typeof h.space?.openWith, "function", "exposé dès le chargement du bundle des panneaux, sans ouverture préalable");
  assert.equal(h.space.openWith(["f2"], ORIGIN).ok, true);
  assert.equal(h.screen().hidden, false);
  assert.equal(h.body(), "Chargement…");
  assert.equal(h.bar().hidden, true, "pas de bandeau tant que la liste ne dit pas ce qui existe");
  await h.settle();
  assert.deepEqual(h.items(), ["f1", "f2", "f3"]);
  assert.equal(h.detailText(), "L’équipe déjeune à midi");
  assert.equal(h.bar().querySelector("p").textContent, "1 souvenir utilisé dans cette réponse.");
});

test("entre le clic et la liste relue, la liste en cache ne s'affiche pas : un souvenir oublié depuis n'apparaît jamais", async () => {
  const h = await harness();
  h.net.facts = h.net.facts.filter((item) => item.fact_id !== "f2"); // oublié après la dernière lecture, le cache l'a encore
  h.space.openWith(["f2", "f1"], ORIGIN);
  assert.equal(h.body(), "Chargement…");
  assert.deepEqual([h.items(), h.selected(), h.shown()], [[], [], false]);
  assert.equal(h.main.querySelector(".memory-filter").hidden, true);
  await h.settle();
  assert.deepEqual(h.items(), ["f1"]);
  assert.equal(h.detailText(), "Aime le café <b>noir</b>");
  assert.equal(h.bar().querySelector("p").textContent, "2 souvenirs utilisés dans cette réponse. 1 n’existe plus (oublié ou remplacé depuis).");
});

test("une relecture (30 s, état de l'application) pendant l'ouverture : la plus récente décide, une lecture plus ancienne et lente est ignorée", async () => {
  const h = await harness();
  h.net.slow = [6, 1]; // la lecture du clic traîne, celle de l'état arrive avant elle
  h.space.openWith(["f1", "f3"], ORIGIN);
  h.net.facts = h.net.facts.filter((item) => item.fact_id !== "f3");
  h.notify();
  await h.settle(12);
  assert.deepEqual(h.items(), ["f1"], "la lecture du clic, partie quand f3 existait, n'écrase pas la plus récente");
  assert.equal(h.detailText(), "Aime le café <b>noir</b>");
  assert.match(h.bar().querySelector("p").textContent, /^2 souvenirs utilisés dans cette réponse\. 1 n’existe plus/);
  assert.equal(h.calls.api.filter((path) => path === "/api/memory/all").length, 3);
});

test("une liste qui ne peut pas être relue après le clic : le dernier état connu s'affiche, pas un chargement sans fin", async () => {
  const h = await harness();
  h.net.down = true;
  await h.openWith(["f1", "f2"]);
  assert.deepEqual(h.items(), ["f1", "f2", "f3"]);
  assert.match(h.body(), /La mémoire n’a pas pu être actualisée/);
  h.net.down = false;
  await h.reload(); // la relecture suivante (30 s) réussit : la demande abandonnée ne s'ouvre pas en surprise
  assert.deepEqual([h.bar().hidden, h.shown()], [true, false]);
});

test("deux pastilles cliquées l'une après l'autre : seule la seconde compte, retour compris", async () => {
  const h = await harness({ opened: ["c1", "c2"] });
  h.space.openWith(["f1"], ORIGIN);
  h.space.openWith(["f2", "f3"], C2);
  await h.settle();
  assert.deepEqual(h.items(), ["f2", "f3"]);
  assert.equal(h.detailText(), "L’équipe déjeune à midi");
  assert.equal(h.bar().querySelector("p").textContent, "2 souvenirs utilisés dans cette réponse.");
  h.press("Retour à la réponse");
  assert.deepEqual(h.calls.opened, [["c2", "reply-4"]]);
});

test("la pastille d'une autre conversation : le bandeau et son retour sont ceux de la nouvelle réponse, sans l'alerte d'avant", async () => {
  const h = await harness({ opened: ["c2"] });
  await h.openWith(["f1"], ORIGIN);
  h.press("Retour à la réponse"); // c1 a disparu : l'alerte s'affiche
  h.press("Retour à la réponse");
  assert.equal(h.bar().querySelectorAll(".memory-space-used-problem").length, 1, "un second clic ne répète pas l'alerte");
  h.navigate();
  await h.openWith(["f2"], C2); // même forme de bandeau (1 souvenir, un fil d'origine), autre réponse
  assert.equal(h.bar().querySelector(".memory-space-used-problem"), null, "l'alerte de la réponse précédente ne reste pas");
  h.press("Retour à la réponse");
  assert.deepEqual(h.calls.opened, [["c1", "reply-9"], ["c1", "reply-9"], ["c2", "reply-4"]], "le retour va à la réponse cliquée, pas à la précédente");
});

test("« Tout afficher » puis « Retour à la réponse » : retour à la même réponse, du même fil", async () => {
  const h = await harness();
  await h.openWith(["f1", "f2"]);
  h.press("Tout afficher");
  h.press("Retour à la réponse");
  assert.deepEqual(h.calls.opened, [["c1", "reply-9"]]);
});

test("la portée filtrée à la visite précédente ne cache pas les souvenirs de la réponse", async () => {
  const h = await harness();
  h.entry().click();
  await h.settle();
  h.choose("Filtrer par portée", "team");
  assert.deepEqual(h.items(), ["f2"]);
  h.navigate();
  await h.openWith(["f1", "f3"]);
  assert.deepEqual(h.items(), ["f1", "f3"]);
  assert.equal(h.field("Filtrer par portée").value, "");
  h.press("Tout afficher");
  assert.deepEqual(h.items(), ["f1", "f2", "f3"], "aucun filtre ne revient après « Tout afficher »");
});

test("« À revoir » : la note d'une visite précédente disparaît, et le panneau ne montre que les souvenirs de la réponse", async () => {
  const h = await harness({ facts: [FACTS[0], review(FACTS[1]), review(FACTS[2])] });
  h.entry().click();
  await h.settle();
  assert.deepEqual(h.reviewIds().sort(), ["f2", "f3"]);
  h.review().querySelectorAll("button").find((button) => button.attributes["aria-label"] === "Oui, ce souvenir est encore vrai" && button.parentElement.parentElement.dataset.reviewId === "f2").click();
  await h.settle();
  assert.match(h.review().textContent, /Souvenir confirmé\./);
  h.navigate();
  await h.openWith(["f1", "f2"]);
  assert.deepEqual(h.reviewIds(), [], "f3 est à revoir mais n'a pas servi à la réponse");
  assert.equal(h.review().hidden, true, "ni la note « Souvenir confirmé. » d'avant");
  h.press("Tout afficher");
  assert.deepEqual(h.reviewIds(), ["f3"]);
});

test("une action de l'écran Mémoire encore en vol au clic (Oublier, Oui, Ajouter) : rien ne s'ouvre, la raison est dite ; ensuite la pastille marche", async () => {
  for (const start of [
    async (h) => { await h.openWith(["f1"]); h.detailButton("Oublier…").click(); h.detailButton("Oublier ce souvenir").click(); },
    async (h) => { h.entry().click(); await h.settle(); h.review().querySelectorAll("button").find((button) => button.attributes["aria-label"] === "Oui, ce souvenir est encore vrai").click(); },
    async (h) => {
      h.entry().click();
      await h.settle();
      h.screen().querySelector(".memory-add-open").click();
      const form = h.screen().querySelector(".memory-add");
      Object.assign(form.querySelector("textarea"), { value: "Range ses clés dans l’entrée" }).fire("input");
      Object.assign(form.querySelectorAll("select")[0], { value: "grokbot" }).fire("change");
      form.fire("submit");
    },
  ]) {
    const h = await harness({ facts: [FACTS[0], review(FACTS[1]), FACTS[2]] });
    let release;
    h.net.hold = new Promise((resolve) => { release = resolve; });
    await start(h);
    await h.settle();
    h.navigate(); // l'utilisateur est reparti dans un fil avant la réponse du moteur
    const left = h.calls.left, refused = h.space.openWith(["f3"], ORIGIN);
    assert.deepEqual([refused.ok, h.calls.left], [false, left]);
    assert.equal(refused.message, "Une action est en cours : réessayez dans un instant.");
    release();
    await h.settle();
    assert.equal((await h.openWith(["f3"])).ok, true);
    assert.equal(h.detailText(), "Le garde-meuble ferme à 18 h");
  }
});

test("le détail laissé en confirmation d'oubli ne revient pas ; un brouillon laissé en modification revient sur son souvenir (#342)", async () => {
  const h = await harness();
  await h.openWith(["f1"]);
  h.detailButton("Oublier…").click();
  h.navigate();
  await h.openWith(["f1"]);
  assert.equal(h.detailButton("Oublier ce souvenir"), undefined, "pas de confirmation d'oubli héritée");
  assert.ok(h.detailButton("Modifier"));
  h.detailButton("Modifier").click();
  Object.assign(h.panel().querySelector("textarea"), { value: "Aime le café serré" }).fire("input");
  h.navigate();
  await h.openWith(["f2"]);
  assert.equal(h.detailText(), "L’équipe déjeune à midi", "l'autre souvenir s'ouvre en lecture");
  h.navigate();
  await h.openWith(["f1"]);
  assert.equal(h.panel().querySelector("textarea").value, "Aime le café serré");
  assert.match(h.panel().textContent, /Brouillon retrouvé/);
});

test("un souvenir lié (#265) ouvert depuis le détail, hors de la réponse : le bandeau reste, « Tout afficher » le montre sélectionné", async () => {
  const h = await harness({ facts: [linked(FACTS[0], "conv-a"), linked(FACTS[1], "conv-a"), FACTS[2]] });
  await h.openWith(["f1", "f3"]);
  h.panel().querySelector(".memory-detail-link-item").click();
  assert.equal(h.detailText(), "L’équipe déjeune à midi");
  assert.equal(h.bar().querySelector("p").textContent, "2 souvenirs utilisés dans cette réponse.");
  assert.deepEqual(h.items(), ["f1", "f3"]);
  h.press("Tout afficher");
  assert.deepEqual([h.items(), h.selected()], [["f1", "f2", "f3"], ["f2"]]);
});

test("le Graphe choisi après l'ouverture ne montre que les souvenirs de la réponse, jusqu'à « Tout afficher »", async () => {
  const h = await harness();
  await h.openWith(["f1", "f3"]);
  h.screen().querySelector(".is-graph").click();
  const points = () => h.main.querySelectorAll(".memory-graph-point").map((el) => el.attributes["data-id"]);
  assert.deepEqual(points(), ["f1", "f3"]);
  h.press("Tout afficher");
  assert.deepEqual(points().sort(), ["f1", "f2", "f3"]);
});

test("le formulaire « Ajouter un souvenir » laissé ouvert garde son texte et ne cache rien de la réponse", async () => {
  const h = await harness();
  h.entry().click();
  await h.settle();
  h.screen().querySelector(".memory-add-open").click();
  Object.assign(h.screen().querySelector(".memory-add").querySelector("textarea"), { value: "À noter" }).fire("input");
  h.navigate();
  await h.openWith(["f1", "f2"]);
  assert.deepEqual(h.items(), ["f1", "f2"]);
  assert.equal(h.detailText(), "Aime le café <b>noir</b>");
  assert.equal(h.screen().querySelector(".memory-add").querySelector("textarea").value, "À noter", "le texte saisi n'est pas perdu");
});
