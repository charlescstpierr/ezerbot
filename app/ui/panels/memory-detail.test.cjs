// Run with: node --test app/ui/panels/memory-detail.test.cjs
// Le panneau de détail d'un souvenir, chargé avec le graphe et l'écran Mémoire comme dans le bundle des panneaux.
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
const fact = (id, bot, scope, text, source, conversation, extra = {}) => ({ fact_id: id, bot_id: bot, scope, text, provenance: { source, conversation_id: conversation }, learned_at: Date.parse("2026-10-20T10:00:00Z"), ...extra });
const FACTS = [fact("f1", "grokbot", "private", "Aime le café <b>noir</b>", "user_stated", "c1"), fact("f2", "chef", "team", "L’équipe déjeune à midi", "model_inferred", null), fact("f3", "chef", "topic", "Le garde-meuble ferme à 18 h", "user_stated", null, { topic_id: "t1", topic_name: "Déménagement" })];
// `refuse` : le moteur répond à toute action par cette erreur ; `opened` : les fils que l'application sait ouvrir.
async function harness({ refuse = null, opened = ["c1"] } = {}) {
  const sidebar = new Element("aside"), main = new Element("main"), dock = new Element("div"), bots = new Element("div"), keys = [], registered = [];
  bots.className = "sidebar-bots"; dock.className = "composer-dock"; sidebar.append(bots); main.append(dock);
  const nodes = { sidebar, "conversation-title": new Element("h1"), "workspace-label": new Element("button") };
  const document = { createElement: (tag) => new Element(tag), createElementNS: (_, tag) => new Element(tag), getElementById: (id) => nodes[id] || null, querySelector: (s) => (s === ".main-panel" ? main : s === "dialog[open]" ? null : dock), addEventListener: (_, action) => keys.push(action), get activeElement() { return Element.active; }, body: new Element("body") };
  let facts = FACTS.map((item) => ({ ...item })), n = 0;
  const calls = { api: [], opened: [] };
  const panels = {
    register: (panel) => registered.push(panel), leaveConversation() {},
    openConversation: (id) => { calls.opened.push(id); return opened.includes(id); },
    async api(path, body) {
      calls.api.push([path, JSON.parse(JSON.stringify(body || {}))]);
      if (path === "/api/memory/all") return { facts: facts.map((item) => ({ ...item })) };
      if (refuse) throw new Error(refuse);
      if (path === "/api/memory/correct") { const old = facts.find((item) => item.fact_id === body.fact_id), next = { ...old, fact_id: `n${++n}`, text: body.text }; facts = [next, ...facts.filter((item) => item !== old)]; return { fact: next }; }
      facts = facts.filter((item) => item.fact_id !== body.fact_id);
      return { forgotten: [body.fact_id] };
    },
  };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document, setInterval: () => 0, JSON, Number, String, Date, Promise, Object, Array, Boolean, Math, Set, Map });
  for (const file of ["memory-detail.js", "memory-graph.js", "memory-space.js"]) vm.runInContext(fs.readFileSync(`${__dirname}/${file}`, "utf8"), context);
  const settle = async () => { for (let i = 0; i < 4; i += 1) await new Promise((resolve) => setImmediate(resolve)); };
  Element.active = null;
  registered[0].onState({ bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }], memory_auto: { revision: 1 } }, {});
  await settle();
  sidebar.querySelector(".memory-entry").click();
  await settle();
  const h = {
    calls, settle, main, svg: () => main.querySelector(".memory-graph-canvas"),
    tab: (view) => main.querySelector(`.is-${view}`).click(),
    point: (id) => main.querySelector(`.memory-graph-point[data-id="${id}"]`),
    item: (id) => main.querySelector(`.memory-space-item[data-fact-id="${id}"]`),
    panel: () => main.querySelector(".memory-detail"),
    open: () => !h.panel().hidden,
    text: (cls) => h.panel().querySelector(`.memory-detail-${cls}`).textContent,
    cells: () => h.panel().querySelectorAll("dd").map((dd) => dd.textContent),
    button: (label) => h.panel().querySelectorAll("button").find((b) => b.textContent === label),
    points: () => main.querySelectorAll(".memory-graph-point").length,
    items: () => main.querySelectorAll(".memory-space-item").length,
    escape: () => keys.forEach((key) => key({ key: "Escape", preventDefault() {} })),
    mutations: () => calls.api.filter(([path]) => path !== "/api/memory/all"),
    async type(label, text) { const field = h.panel().querySelector(".memory-detail-field"); field.value = text; field.fire("input"); h.button(label).click(); await settle(); },
  };
  return h;
}

test("un clic sur un point du graphe ou sur une ligne de la Liste ouvre le panneau avec les bons champs", async () => {
  const h = await harness();
  h.tab("graph");
  assert.equal(h.open(), false, "fermé tant qu'aucun souvenir n'est choisi");
  h.point("f1").click();
  assert.equal(h.text("text"), "Aime le café <b>noir</b>", "le texte reste du texte");
  assert.deepEqual(h.panel().querySelectorAll("dt").map((dt) => dt.textContent), ["Bot", "Origine", "Appris le", "Provenance", "Portée"]);
  const [bot, origin, date, source, scope] = h.cells();
  assert.deepEqual([bot, origin, source, scope], ["Ezerbot", "Ouvrir la conversation", "Déclaré", "Privé"]);
  assert.match(date, /2026/);
  assert.deepEqual([h.main.querySelector(".memory-space").dataset.detail, h.point("f1").classes.includes("is-selected")], ["open", true]);
  h.button("Ouvrir la conversation").click();
  assert.deepEqual(h.calls.opened, ["c1"]);
  const gone = await harness({ opened: [] });
  gone.item("f1").click();
  gone.button("Ouvrir la conversation").click();
  assert.match(gone.text("error"), /n’existe plus/);
  assert.equal(Element.active, gone.button("Ouvrir la conversation"), "le focus reste sur le bouton après l'erreur");
  gone.button("Fermer").click();
  assert.deepEqual([gone.open(), gone.item("f1").contains(Element.active)], [false, true], "le focus revient à la ligne");
  h.tab("list");
  h.item("f2").click();
  assert.equal(h.text("text"), "L’équipe déjeune à midi");
  assert.deepEqual([h.cells()[1], h.cells()[3], h.item("f2").classes.includes("is-selected")], ["Hors conversation", "Déduit", true]);
  assert.equal(h.item("f2").querySelector(".memory-space-text").tagName, "button", "focalisable et activable au clavier");
  h.tab("graph");
  assert.equal(h.point("f2").classes.includes("is-selected"), true, "la sélection suit le changement de vue");
  h.tab("list");
  h.escape();
  assert.equal(Element.active, h.item("f2").querySelector(".memory-space-text"), "le focus revient au bouton de la ligne");
});

test("Entrée ouvre un point ; Échap ferme le panneau et rend le focus au point ; un glisser n'ouvre rien", async () => {
  const h = await harness();
  h.tab("graph");
  assert.equal(h.point("f2").fire("keydown", { key: "Enter" }).defaultPrevented, true);
  assert.equal(h.text("text"), "L’équipe déjeune à midi");
  h.escape();
  assert.deepEqual([h.open(), h.main.querySelector(".memory-space").dataset.detail, Element.active === h.point("f2")], [false, undefined, true]);
  const drag = (dx) => { const before = h.svg().attributes.viewBox; h.svg().fire("pointerdown", { clientX: 100, clientY: 100, buttons: 1 }); h.svg().fire("pointermove", { clientX: 100 + dx, clientY: 101, buttons: 1 }); h.svg().fire("pointerup", {}); return h.svg().attributes.viewBox !== before; };
  assert.equal(drag(2), false, "2 px : encore un clic");
  h.point("f1").click();
  assert.equal(h.open(), true);
  h.escape();
  assert.equal(drag(60), true, "60 px : la vue a bougé");
  h.point("f1").click();
  assert.equal(h.open(), false, "le clic qui termine un glisser ne choisit pas de souvenir");
  h.svg().fire("pointerdown", { clientX: 100, clientY: 100, buttons: 1 });
  h.svg().fire("pointermove", { clientX: 102, clientY: 100, buttons: 1 });
  const parked = h.svg().attributes.viewBox; // le bouton est relâché hors du graphe : aucun pointerup n'arrive
  h.svg().fire("pointermove", { clientX: 300, clientY: 300, buttons: 0 });
  assert.equal(h.svg().attributes.viewBox, parked, "pas de glisser fantôme");
});

test("Modifier envoie /api/memory/correct puis relit : le texte affiché change ; l'erreur du moteur garde la saisie", async () => {
  const bad = await harness({ refuse: "Texte refusé par le moteur." });
  bad.item("f1").click();
  bad.button("Modifier").click();
  assert.equal(bad.panel().querySelector(".memory-detail-field").value, "Aime le café <b>noir</b>");
  await bad.type("Enregistrer", "Aime le thé");
  assert.equal(bad.text("error"), "Texte refusé par le moteur.");
  assert.equal(bad.panel().querySelector(".memory-detail-field").value, "Aime le thé", "la saisie est gardée");
  assert.equal(bad.item("f1").querySelector(".memory-space-text").textContent, "Aime le café <b>noir</b>", "rien n'est changé d'avance");
  const ok = await harness();
  ok.item("f1").click();
  ok.button("Modifier").click();
  await ok.type("Enregistrer", "Aime le thé");
  assert.deepEqual(ok.mutations(), [["/api/memory/correct", { bot_id: "grokbot", fact_id: "f1", text: "Aime le thé" }]]);
  assert.equal(ok.calls.api.at(-1)[0], "/api/memory/all", "la liste est relue");
  assert.deepEqual([ok.text("text"), ok.main.querySelectorAll(".memory-space-text").map((t) => t.textContent)[0]], ["Aime le thé", "Aime le thé"]);
});

test("Oublier demande confirmation ; annuler ne change rien ; confirmer retire le souvenir du graphe, de la Liste et du panneau", async () => {
  const h = await harness();
  h.tab("graph");
  h.point("f1").click();
  h.button("Oublier…").click();
  assert.match(h.text("warning"), /effacé pour de bon.*trace sans contenu/);
  h.button("Annuler").click();
  assert.deepEqual([h.mutations(), h.points(), h.open(), h.text("text")], [[], 3, true, "Aime le café <b>noir</b>"]);
  h.button("Oublier…").click();
  h.button("Oublier ce souvenir").click();
  await h.settle();
  assert.deepEqual(h.mutations(), [["/api/memory/forget", { bot_id: "grokbot", fact_id: "f1" }]]);
  assert.deepEqual([h.points(), h.open(), h.point("f1")], [2, false, null]);
  h.tab("list");
  assert.deepEqual([h.items(), h.item("f1")], [2, null]);
});

test("un souvenir de sujet s'oublie par /api/topics/memory/forget, sans Modifier ; l'erreur d'oubli garde le souvenir", async () => {
  const bad = await harness({ refuse: "Fait de sujet introuvable." });
  bad.item("f3").click();
  assert.deepEqual([bad.button("Modifier"), bad.cells()[4]], [undefined, "Sujet · Déménagement"]);
  bad.button("Oublier…").click();
  assert.match(bad.text("warning"), /tous les bots du sujet/);
  bad.button("Oublier ce souvenir").click();
  await bad.settle();
  assert.deepEqual([bad.text("error"), bad.open(), bad.items()], ["Fait de sujet introuvable.", true, 3]);
  const ok = await harness();
  ok.item("f3").click();
  ok.button("Oublier…").click();
  ok.button("Oublier ce souvenir").click();
  ok.escape();
  assert.equal(ok.open(), false, "Échap ferme aussi pendant l'envoi");
  await ok.settle();
  assert.deepEqual([ok.mutations(), ok.open(), ok.items()], [[["/api/topics/memory/forget", { fact_id: "f3" }]], false, 2]);
});

test("styles : jetons seulement, aucun texte sous 11,5 px", () => {
  const css = fs.readFileSync(`${__dirname}/memory-detail.css`, "utf8");
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b|rgb\(|hsl\(/);
  for (const [, size] of css.matchAll(/font-size:\s*(\d+(?:\.\d+)?)px/g)) assert.ok(Number(size) >= 11.5, `${size}px`);
});
