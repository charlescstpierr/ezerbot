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
// `lose` : le moteur applique l'action mais la réponse se perd (erreur réseau sans `status`).
async function harness({ refuse = null, opened = ["c1"], lose = null, refreshFails = false, start = FACTS } = {}) {
  const sidebar = new Element("aside"), main = new Element("main"), dock = new Element("div"), bots = new Element("div"), keys = [], registered = [];
  bots.className = "sidebar-bots"; dock.className = "composer-dock"; sidebar.append(bots); main.append(dock);
  const nodes = { sidebar, "conversation-title": new Element("h1"), "workspace-label": new Element("button") };
  const document = { createElement: (tag) => new Element(tag), createElementNS: (_, tag) => new Element(tag), getElementById: (id) => nodes[id] || null, querySelector: (s) => (s === ".main-panel" ? main : s === "dialog[open]" ? null : dock), addEventListener: (_, action) => keys.push(action), get activeElement() { return Element.active; }, body: new Element("body") };
  let facts = start.map((item) => ({ ...item })), n = 0;
  const cache = { seen: null, work: null }, calls = { api: [], opened: [], refresh: 0 }, net = { down: false, drop: (id) => { facts = facts.filter((item) => item.fact_id !== id); }, add: (item) => { facts = [item, ...facts]; }, touch: (id) => { facts = facts.map((item) => (item.fact_id === id ? { ...item, needs_review: true } : item)); } };
  const panels = {
    register: (panel) => registered.push(panel), leaveConversation() {},
    // L'état partagé de l'application (fils en cache, reçus) : comme app.js, une lecture déjà en vol est partagée, et elle rend l'état tel qu'il était à son départ.
    refresh() {
      calls.refresh += 1;
      if (refreshFails) return Promise.reject(new Error("état indisponible"));
      if (!cache.work) cache.work = (async () => { const seen = facts.map((item) => item.fact_id).join(); await new Promise((resolve) => setImmediate(resolve)); cache.seen = seen; })().finally(() => { cache.work = null; });
      return cache.work;
    },
    openConversation: (id) => { calls.opened.push(id); return opened.includes(id); },
    async api(path, body) {
      calls.api.push([path, JSON.parse(JSON.stringify(body || {}))]);
      if (path === "/api/memory/all") { const snapshot = facts.map((item) => ({ ...item })); await net.gate?.(); return net.down ? Promise.reject(new Error("hors ligne")) : { facts: snapshot }; }
      if (refuse) { await net.hold; throw new Error(refuse); }
      if (path === "/api/memory/correct") { const old = facts.find((item) => item.fact_id === body.fact_id), next = { ...old, fact_id: `n${++n}`, text: body.text }; facts = [...(net.sidecar ? [net.sidecar] : []), next, ...facts.filter((item) => item !== old)]; await net.hold; if (lose) throw new Error(lose); return { fact: next }; }
      facts = facts.filter((item) => item.fact_id !== body.fact_id);
      if (lose) throw new Error(lose);
      return { forgotten: [body.fact_id] };
    },
  };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document, setInterval: () => 0, JSON, Number, String, Date, Promise, Object, Array, Boolean, Math, Set, Map });
  for (const file of ["memory-detail.js", "memory-filter.js", "memory-links.js", "memory-graph.js", "memory-space.js"]) vm.runInContext(fs.readFileSync(`${__dirname}/${file}`, "utf8"), context);
  const settle = async () => { for (let i = 0; i < 4; i += 1) await new Promise((resolve) => setImmediate(resolve)); };
  Element.active = null;
  registered[0].onState({ bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }], memory_auto: { revision: 1 } }, {});
  await settle();
  sidebar.querySelector(".memory-entry").click();
  await settle();
  const h = {
    calls, settle, main, net, cache, panels, reload: (name = "Ezerbot") => registered[0].onState({ bots: [{ id: "grokbot", name }], memory_auto: { revision: Math.random() } }, {}), svg: () => main.querySelector(".memory-graph-canvas"),
    tab: (view) => main.querySelector(`.is-${view}`).click(),
    point: (id) => main.querySelector(`.memory-graph-point[data-id="${id}"]`),
    item: (id) => main.querySelector(`.memory-space-item[data-fact-id="${id}"]`),
    panel: () => main.querySelector(".memory-detail"),
    // La barre de filtres de #262 : on la pilote par ses champs, comme l'utilisateur.
    filter: {
      field: (label) => main.querySelector(`[aria-label="${label}"]`),
      search(text) { const field = h.filter.field("Rechercher un souvenir"); field.value = text; field.fire("input"); },
      bot(id) { const field = h.filter.field("Filtrer par bot"); field.value = id; field.fire("change"); },
      count: () => main.querySelector(".memory-filter-count").textContent,
    },
    lit: () => main.querySelectorAll(".memory-graph-point").filter((point) => point.classes.includes("is-match")).map((point) => point.attributes["data-id"]),
    title: () => nodes["conversation-title"].textContent,
    empty: () => main.querySelector(".memory-space-empty")?.textContent,
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

test("une relecture manquée ne masque pas une correction réussie ; une action périmée ne fait pas perdre la saisie suivante", async () => {
  const h = await harness();
  h.item("f1").click();
  h.button("Modifier").click();
  h.net.down = true;
  await h.type("Enregistrer", "Aime le thé");
  assert.match(h.panel().textContent, /Souvenir modifié, mais la liste n’a pas pu être relue/);
  assert.equal(h.open(), true);
  const slow = await harness(); // une relecture arrive entre l'écriture du moteur et sa réponse
  let release;
  slow.net.hold = new Promise((resolve) => { release = resolve; });
  slow.item("f1").click();
  slow.button("Modifier").click();
  await slow.type("Enregistrer", "Aime le thé");
  slow.reload();
  await slow.settle();
  assert.equal(slow.open(), true, "le panneau attend la réponse du moteur");
  release();
  await slow.settle();
  assert.deepEqual([slow.open(), slow.text("text")], [true, "Aime le thé"]);
  const lost = await harness({ refuse: "Souvenir introuvable." }); // refusé, alors qu'une relecture a déjà retiré le souvenir
  lost.net.hold = new Promise((resolve) => { release = resolve; });
  lost.item("f2").click();
  lost.button("Oublier…").click();
  lost.button("Oublier ce souvenir").click();
  lost.net.drop("f2");
  lost.reload();
  await lost.settle();
  release();
  await lost.settle();
  assert.equal(lost.open(), false, "pas de panneau figé");
  const race = await harness(); // pendant qu'une correction attend sa réponse, aucun autre souvenir ne s'ouvre
  race.net.hold = new Promise((resolve) => { release = resolve; });
  race.item("f1").click();
  race.button("Modifier").click();
  await race.type("Enregistrer", "A");
  race.item("f2").click();
  race.reload();
  await race.settle();
  assert.deepEqual([race.open(), race.cells()[0]], [true, "Ezerbot"], "le panneau reste sur la correction en cours");
  release();
  await race.settle();
  assert.deepEqual([race.text("text"), race.cells()[0]], ["A", "Ezerbot"]);
  race.item("f2").click();
  assert.equal(race.text("text"), "L’équipe déjeune à midi", "une fois la réponse reçue, un autre souvenir s'ouvre");
});

test("quand la liste change autour du panneau : un bot renommé s'y lit, la saisie en cours garde le focus", async () => {
  const h = await harness();
  h.item("f1").click();
  h.button("Modifier").click();
  const field = () => h.panel().querySelector(".memory-detail-field");
  field().value = "Brouillon";
  field().fire("input");
  h.reload("Zéphyr");
  await h.settle();
  assert.equal(h.cells()[0], "Zéphyr");
  h.net.touch("f1");
  h.reload("Zéphyr");
  await h.settle();
  assert.deepEqual([field().value, Element.active === field()], ["Brouillon", true]);
  h.button("Annuler").click();
  h.button("Oublier…").click();
  h.reload("Aurore");
  await h.settle();
  assert.equal(Element.active, h.button("Annuler"), "même chose dans la confirmation d'oubli");
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

test("filtre actif : le clic sur un point mis en avant (ou sur son texte) ouvre le détail ; un souvenir qui sort du filtre garde son panneau", async () => {
  const h = await harness();
  h.tab("graph");
  h.filter.search("midi");
  assert.deepEqual([h.lit(), h.filter.count()], [["f2"], "1 souvenir sur 3"]);
  // Le texte de l'étiquette est un enfant du point : un clic dessus remonte jusqu'au point qui ouvre le détail.
  const label = h.point("f2").querySelector(".memory-graph-label");
  assert.equal(label.textContent, "L’équipe déjeune à…", "le texte raccourci de #262");
  assert.equal(h.point("f2").children.includes(label), true);
  h.point("f2").click();
  assert.deepEqual([h.open(), h.text("text"), h.point("f2").classes.includes("is-selected")], [true, "L’équipe déjeune à midi", true]);
  // Le filtre change et le souvenir choisi n'y est plus : le panneau reste (on y lit ou on y écrit), le filtre ne ferme rien.
  h.filter.search("café");
  assert.deepEqual([h.lit(), h.open(), h.text("text")], [["f1"], true, "L’équipe déjeune à midi"]);
  assert.equal(h.point("f2").classes.includes("is-selected"), true, "le point choisi reste marqué, même estompé");
  h.tab("list");
  assert.deepEqual([h.items(), h.item("f2"), h.open()], [1, null, true], "la Liste ne garde que les résultats ; le panneau reste ouvert");
  h.filter.search("");
  assert.equal(h.item("f2").classes.includes("is-selected"), true, "sans filtre, la ligne retrouve sa marque");
  h.filter.search("zzz");
  assert.deepEqual([h.empty(), h.open()], ["Aucun souvenir ne correspond à ces filtres.", true], "zéro résultat : le message, et le panneau reste");
  h.button("Fermer").click();
  assert.equal(h.open(), false);
});

test("filtre actif : après Oublier ou Modifier, le compteur « N souvenirs sur M », le titre et la mise en avant se mettent à jour", async () => {
  const h = await harness();
  h.tab("graph");
  h.filter.bot("chef");
  assert.deepEqual([h.lit(), h.filter.count(), h.title()], [["f2", "f3"], "2 souvenirs sur 3", "Mémoire · 3 souvenirs"]);
  h.point("f2").click();
  h.button("Modifier").click();
  await h.type("Enregistrer", "Déjeuner à 13 h");
  assert.deepEqual(h.mutations(), [["/api/memory/correct", { bot_id: "chef", fact_id: "f2", text: "Déjeuner à 13 h" }]]);
  assert.deepEqual([h.lit().sort(), h.filter.count(), h.text("text")], [["f3", "n1"], "2 souvenirs sur 3", "Déjeuner à 13 h"], "le souvenir corrigé garde son filtre de bot et son panneau");
  h.filter.search("13 h");
  assert.deepEqual([h.lit(), h.filter.count()], [["n1"], "1 souvenir sur 3"]);
  h.button("Oublier…").click();
  h.button("Oublier ce souvenir").click();
  await h.settle();
  assert.deepEqual(h.mutations().at(-1), ["/api/memory/forget", { bot_id: "chef", fact_id: "n1" }]);
  assert.deepEqual([h.open(), h.filter.count(), h.title(), h.points(), h.empty()], [false, "0 souvenir sur 2", "Mémoire · 2 souvenirs", 0, "Aucun souvenir ne correspond à ces filtres."]);
  h.filter.search("");
  assert.deepEqual([h.lit(), h.points(), h.filter.count()], [["f3"], 2, "1 souvenir sur 2"], "bot « chef » seul : il ne reste que le souvenir de sujet");
});

test("filtre actif : un souvenir de sujet trouvé par le filtre s'oublie par la route des sujets", async () => {
  const h = await harness();
  h.filter.search("garde");
  assert.deepEqual([h.items(), h.filter.count()], [1, "1 souvenir sur 3"]);
  h.item("f3").click();
  h.button("Oublier…").click();
  h.button("Oublier ce souvenir").click();
  await h.settle();
  assert.deepEqual(h.mutations(), [["/api/topics/memory/forget", { fact_id: "f3" }]]);
  assert.deepEqual([h.items(), h.filter.count(), h.open()], [0, "0 souvenir sur 2", false]);
});

const rereads = (h) => h.calls.api.filter(([path]) => path === "/api/memory/all").length;
test("réponse perdue après Modifier : le moteur a corrigé, le panneau relit la liste avant de rendre la main et affiche le texte relu", async () => {
  const edited = await harness({ lose: "Réseau coupé" });
  edited.tab("graph");
  edited.point("f1").click();
  edited.button("Modifier").click();
  const before = rereads(edited);
  await edited.type("Enregistrer", "Café au lait");
  assert.equal(rereads(edited) > before, true, "la liste est relue après l'erreur");
  assert.deepEqual([edited.open(), edited.text("text"), edited.point("n1") !== null, edited.point("f1")], [true, "Café au lait", true, null], "le texte relu s'affiche, le souvenir remplacé n'est plus proposé");
  assert.equal(edited.button("Modifier").disabled, false, "les contrôles sont rendus, sur le souvenir à jour");
  assert.equal(edited.panel().querySelector(".memory-detail-error").textContent, "", "aucune erreur : l'action a abouti");
  edited.button("Modifier").click();
  await edited.type("Enregistrer", "Café serré");
  assert.deepEqual(edited.mutations().map(([, body]) => body.fact_id), ["f1", "n1"], "la nouvelle correction vise le souvenir à jour, pas l'ancien");
});

test("réponse perdue après Modifier, avec une relecture périodique en même temps : le panneau attend la liste la plus récente, il ne se ferme pas", async () => {
  const h = await harness({ lose: "Réseau coupé" });
  h.tab("graph");
  h.point("f1").click();
  h.button("Modifier").click();
  const reads = []; // chaque lecture attend qu'on la libère : la plus ancienne arrive avant la plus récente
  h.net.gate = () => new Promise((resolve) => reads.push(resolve));
  const field = h.panel().querySelector(".memory-detail-field");
  field.value = "Café au lait";
  field.fire("input");
  h.button("Enregistrer").click();
  await h.settle();
  h.reload(); // l'actualisation périodique dépasse la relecture du panneau
  await h.settle();
  assert.equal(reads.length, 2);
  reads[0]();
  await h.settle();
  assert.equal(h.button("Enregistrer").disabled, true, "la lecture dépassée ne rend pas la main : la liste la plus récente n'est pas arrivée");
  reads[1]();
  await h.settle();
  assert.deepEqual([h.open(), h.text("text"), h.button("Modifier").disabled], [true, "Café au lait", false]);
});

test("réponse perdue après Modifier : un autre souvenir de même bot et de même texte n'est pas pris pour le remplaçant", async () => {
  const h = await harness({ lose: "Réseau coupé" });
  h.net.sidecar = fact("autre", "grokbot", "topic", "Café au lait", "user_stated", null, { topic_id: "t1", topic_name: "Déménagement" });
  h.tab("graph");
  h.point("f1").click();
  h.button("Modifier").click();
  await h.type("Enregistrer", "Café au lait");
  assert.deepEqual([h.open(), h.cells()[4]], [true, "Privé"], "le panneau suit le souvenir privé corrigé");
  h.button("Oublier…").click();
  h.button("Oublier ce souvenir").click();
  await h.settle();
  assert.deepEqual(h.mutations().at(-1), ["/api/memory/forget", { bot_id: "grokbot", fact_id: "n1" }], "Oublier vise le souvenir corrigé, pas le souvenir de sujet");
});

test("la fin d'une action ne vole pas le focus à quelqu'un qui est passé ailleurs ; un nouveau rendu garde le titre focalisé", async () => {
  const h = await harness();
  h.tab("graph");
  h.point("f1").click();
  assert.equal(Element.active, h.panel().querySelector(".memory-detail-title"), "à l'ouverture, le focus est sur le titre");
  h.reload("Café Bot"); // un bot renommé relit le panneau
  assert.equal(Element.active, h.panel().querySelector(".memory-detail-title"), "le titre rendu de nouveau garde le focus");
  h.button("Modifier").click();
  let release;
  h.net.hold = new Promise((resolve) => { release = resolve; });
  const field = h.panel().querySelector(".memory-detail-field");
  field.value = "Café au lait";
  field.fire("input");
  h.button("Enregistrer").click();
  await h.settle();
  const search = h.filter.field("Rechercher un souvenir");
  search.focus(); // pendant l'attente, la recherche reçoit le focus
  release();
  await h.settle();
  assert.deepEqual([h.text("text"), Element.active === search], ["Café au lait", true], "la correction aboutit, le focus reste dans la recherche");
});

test("après Modifier ou Oublier, l'état partagé de l'application est relu aussi (les fils en cache ne gardent pas un reçu périmé) ; son échec n'est pas celui de l'action", async () => {
  const h = await harness();
  h.tab("graph");
  h.point("f1").click();
  h.button("Modifier").click();
  await h.type("Enregistrer", "Café au lait");
  assert.equal(h.calls.refresh >= 1, true, "Modifier relit l'état partagé");
  h.button("Oublier…").click();
  h.button("Oublier ce souvenir").click();
  await h.settle();
  assert.deepEqual([h.calls.refresh >= 2, h.open()], [true, false], "Oublier aussi");
  const broken = await harness({ refreshFails: true });
  broken.tab("graph");
  broken.point("f1").click();
  broken.button("Modifier").click();
  await broken.type("Enregistrer", "Café au lait");
  assert.deepEqual([broken.text("text"), broken.text("error"), broken.button("Modifier").disabled], ["Café au lait", "", false], "l'état n'a pas pu être relu : la correction reste confirmée, sans erreur d'action");
  const lost = await harness({ lose: "Réseau coupé" });
  lost.tab("graph");
  lost.point("f2").click();
  lost.button("Oublier…").click();
  lost.button("Oublier ce souvenir").click();
  await lost.settle();
  assert.equal(lost.calls.refresh >= 1, true, "même une réponse perdue relit l'état partagé");
  const racing = await harness();
  racing.tab("graph");
  racing.point("f1").click();
  racing.panels.refresh(); // une lecture d'état part avant l'action et reste en vol
  racing.button("Oublier…").click();
  racing.button("Oublier ce souvenir").click();
  await racing.settle();
  assert.equal(racing.cache.seen.includes("f1"), false, "l'état partagé relu après l'oubli ne contient plus le souvenir, même si une lecture partie avant était en vol");
});

test("réponse perdue après Oublier : le moteur a oublié, le panneau relit la liste, se ferme et le graphe et la Liste sont à jour", async () => {
  const forgot = await harness({ lose: "Réseau coupé" });
  forgot.tab("graph");
  forgot.point("f2").click();
  forgot.button("Oublier…").click();
  forgot.button("Oublier ce souvenir").click();
  await forgot.settle();
  assert.deepEqual([forgot.open(), forgot.points(), forgot.point("f2")], [false, 2, null], "oublié : le panneau se ferme, le graphe est à jour");
  forgot.tab("list");
  assert.deepEqual([forgot.items(), forgot.item("f2")], [2, null]);
});

test("réponse perdue sans effet : la liste est relue, l'erreur reste affichée et la saisie est gardée", async () => {
  const h = await harness({ refuse: "Réseau coupé" });
  h.tab("list");
  h.item("f1").click();
  h.button("Modifier").click();
  const before = h.calls.api.filter(([path]) => path === "/api/memory/all").length;
  await h.type("Enregistrer", "Texte voulu");
  assert.equal(h.calls.api.filter(([path]) => path === "/api/memory/all").length > before, true);
  assert.deepEqual([h.text("error"), h.panel().querySelector(".memory-detail-field").value, h.button("Enregistrer").disabled], ["Réseau coupé", "Texte voulu", false]);
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

// Les liens (#265) : même conversation = « Lié à » dans le détail.
const CHAIN = [fact("l1", "grokbot", "private", "Aime le café noir", "automatic", "c1"), fact("l2", "chef", "team", "Déjeune à midi", "automatic", "c1"), fact("l3", "chef", "topic", "Garde-meuble fermé le soir", "user_stated", "c1", { topic_id: "t1", topic_name: "Déménagement" }), fact("l4", "grokbot", "private", "Préfère le thé le soir", "automatic", "c2"), fact("l5", "grokbot", "private", "Seul, sans conversation", "automatic", null)];
const linkedTexts = (h) => h.panel().querySelectorAll(".memory-detail-link-item").map((item) => item.textContent);
test("le détail liste « Lié à » : les autres souvenirs de la même conversation, pas lui-même ni ceux d'ailleurs", async () => {
  const h = await harness({ start: CHAIN });
  h.tab("graph");
  h.point("l1").click();
  assert.equal(h.panel().querySelector(".memory-detail-links-title").textContent, "Lié à");
  assert.deepEqual(linkedTexts(h), ["Déjeune à midi", "Garde-meuble fermé le soir"], "un souvenir d'équipe et un souvenir de sujet de la même conversation");
  for (const id of ["l4", "l5"]) { h.point(id).click(); assert.equal(h.panel().querySelector(".memory-detail-links"), null, `${id} est isolé : pas de section « Lié à »`); }
});
test("le graphe dessine un trait fin par lien, et un souvenir plus lié a un point un peu plus gros", async () => {
  const h = await harness({ start: CHAIN });
  h.tab("graph");
  const lines = h.main.querySelectorAll(".memory-graph-edge");
  assert.equal(lines.filter((line) => line.classes.includes("is-link")).length, 2, "trois souvenirs d'une même conversation : une chaîne de deux traits qui les relie tous");
  assert.equal(lines.length, 2 + (3 + 5), "deux traits de lien, plus les traits vous → groupe (3) et groupe → souvenir (5)");
  const r = (id) => Number(h.point(id).querySelector("circle").attributes.r);
  assert.ok(r("l1") > r("l4") && r("l4") === r("l5"), "lié deux fois > isolés");
});
test("au survol d'un point, tous les souvenirs de sa conversation s'allument, les autres s'estompent", async () => {
  const h = await harness({ start: CHAIN });
  h.tab("graph");
  h.point("l1").fire("mouseenter");
  const dim = (id) => h.point(id).classes.includes("is-dim");
  assert.deepEqual(["l2", "l3"].map(dim), [false, false], "l3 n'est pas voisin de l1 dans la chaîne tracée, mais il est lié");
  assert.deepEqual(["l4", "l5"].map(dim), [true, true]);
  const chain = () => h.main.querySelectorAll(".memory-graph-edge").filter((line) => line.classes.includes("is-link")).map((line) => line.classes.includes("is-dim"));
  assert.deepEqual(chain(), [false, false], "toute la chaîne de la conversation reste visible, y compris le trait qui ne touche pas l1");
  h.point("l1").fire("mouseleave");
  assert.deepEqual(["l2", "l3", "l4", "l5"].map(dim), [false, false, false, false]);
  h.point("l4").fire("mouseenter");
  assert.deepEqual(chain(), [true, true], "un souvenir d'une autre conversation estompe les traits de celle-ci");
});
test("un souvenir lié qui arrive pendant la confirmation d'oubli ne déplace pas le focus sur « Oublier ce souvenir »", async () => {
  const h = await harness({ start: CHAIN });
  h.item("l4").click();
  h.panel().querySelectorAll("button").find((b) => b.textContent === "Oublier…").click();
  assert.equal(Element.active.textContent, "Annuler", "après « Oublier… », le focus est sur Annuler");
  h.panel().querySelectorAll("button").find((b) => b.textContent === "Annuler").click();
  h.panel().querySelectorAll("button").find((b) => b.textContent === "Oublier…").click();
  h.net.add(fact("l6", "grokbot", "private", "Autre souvenir de la conversation c2", "automatic", "c2"));
  h.reload();
  await h.settle();
  assert.equal(h.panel().querySelectorAll(".memory-detail-link-item").length, 1, "le souvenir lié est arrivé dans le panneau");
  assert.equal(Element.active.textContent, "Annuler", "le focus est resté sur Annuler");
  assert.deepEqual(h.mutations(), [], "rien n'a été oublié");
});
const FAR = "Même début de phrase très long pour que deux souvenirs se ressemblent après troncature, vraiment très long";
test("deux souvenirs liés au texte tronqué identique : le focus reste sur celui qui l'avait, après une relecture", async () => {
  const start = [fact("m", "grokbot", "private", "Principal", "automatic", "c1"), fact("b", "grokbot", "private", `${FAR} B`, "automatic", "c1"), fact("c", "grokbot", "private", `${FAR} C`, "automatic", "c1")];
  const h = await harness({ start });
  h.item("m").click();
  const items = () => h.panel().querySelectorAll(".memory-detail-link-item");
  assert.equal(items()[0].textContent, items()[1].textContent, "les deux boutons portent le même libellé");
  items()[1].focus();
  const wanted = Element.active.dataset.factId;
  h.net.touch("m");
  h.reload();
  await h.settle();
  assert.equal(Element.active.dataset.factId, wanted, "le focus est resté sur le même souvenir lié");
});
test("le souvenir lié focalisé disparaît : le focus va au titre du panneau", async () => {
  const h = await harness({ start: CHAIN });
  h.item("l1").click();
  const target = h.panel().querySelectorAll(".memory-detail-link-item")[0];
  target.focus();
  h.net.drop(target.dataset.factId);
  h.reload();
  await h.settle();
  assert.equal(Element.active.classes.includes("memory-detail-title"), true);
});
test("un clic sur un souvenir lié ouvre ce souvenir dans le panneau", async () => {
  const h = await harness({ start: CHAIN });
  h.item("l1").click();
  h.panel().querySelectorAll(".memory-detail-link-item").find((item) => item.textContent === "Déjeune à midi").click();
  assert.equal(h.text("text"), "Déjeune à midi");
  assert.deepEqual(linkedTexts(h), ["Aime le café noir", "Garde-meuble fermé le soir"]);
});
test("un texte lié reste du texte, et un souvenir lié oublié ensuite sort de « Lié à » après la relecture", async () => {
  const h = await harness({ start: [fact("x1", "grokbot", "private", "<b>gras</b>", "automatic", "c1"), fact("x2", "grokbot", "private", "Deuxième", "automatic", "c1"), fact("x3", "grokbot", "private", "Troisième", "automatic", "c1")] });
  h.item("x2").click();
  assert.deepEqual(linkedTexts(h), ["<b>gras</b>", "Troisième"]);
  h.net.drop("x3");
  h.reload();
  await h.settle();
  assert.deepEqual(linkedTexts(h), ["<b>gras</b>"]);
  h.net.drop("x1");
  h.reload();
  await h.settle();
  assert.equal(h.panel().querySelector(".memory-detail-links"), null, "plus aucun souvenir lié");
});
test("styles : jetons seulement, aucun texte sous 11,5 px", () => {
  const css = fs.readFileSync(`${__dirname}/memory-detail.css`, "utf8");
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b|rgb\(|hsl\(/);
  for (const [, size] of css.matchAll(/font-size:\s*(\d+(?:\.\d+)?)px/g)) assert.ok(Number(size) >= 11.5, `${size}px`);
});

test("styles : le texte d'un résultat reçoit le clic (le graphe le laissait traverser), sans que les étiquettes de groupe le reçoivent", () => {
  const graph = fs.readFileSync(`${__dirname}/memory-graph.css`, "utf8"), detail = fs.readFileSync(`${__dirname}/memory-detail.css`, "utf8");
  assert.match(graph, /\.memory-graph-label[^{}]*\{[^}]*pointer-events:\s*none/, "le graphe garde ses étiquettes de groupe traversables");
  assert.match(detail, /^\.memory-graph-point \.memory-graph-label \{ pointer-events: auto; \}$/m, "plus spécifique que la règle du graphe : elle l'emporte, quel que soit l'ordre des feuilles");
});
