// Run with: test-gate node --test app/ui/panels/memory-review.test.cjs
// « À revoir » (#270) : halo orange dans le graphe, panneau « Encore vrai ? » avec Oui / Modifier / Oublier.
// Le moteur est simulé : `needs_review` vient de lui, l'interface ne recalcule aucun âge.
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
  getAttribute(key) { return this.attributes[key]; }
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

const OLD = Date.parse("2026-08-01T10:00:00Z"), RECENT = Date.parse("2026-10-20T10:00:00Z");
const fact = (id, bot, scope, text, extra = {}) => ({ fact_id: id, bot_id: bot, scope, text, provenance: { source: "user_stated", conversation_id: null }, learned_at: RECENT, reconfirmed_at: null, needs_review: false, ...extra });
const FACTS = [
  fact("f1", "grokbot", "private", "Aime le café <b>noir</b>", { learned_at: OLD, needs_review: true }),
  fact("f2", "chef", "team", "L’équipe déjeune à midi"),
  fact("f3", "chef", "topic", "Le garde-meuble ferme à 18 h", { learned_at: OLD, reconfirmed_at: Date.parse("2026-08-20T10:00:00Z"), needs_review: true, topic_id: "t1", topic_name: "Déménagement" }),
];
const NOT_FOUND = "Ce souvenir n’existe plus : rien n’a été reconfirmé.";

// `gone` : ids que le moteur a déjà oubliés sans que l'écran le sache (404 à la confirmation) ; `down` : la route répond par une erreur réseau.
async function harness({ facts: seed = FACTS, down = false } = {}) {
  const sidebar = new Element("aside"), main = new Element("main"), dock = new Element("div"), bots = new Element("div"), keys = [], registered = [];
  bots.className = "sidebar-bots"; dock.className = "composer-dock"; sidebar.append(bots); main.append(dock);
  const nodes = { sidebar, "conversation-title": new Element("h1"), "workspace-label": new Element("button") };
  const document = { createElement: (tag) => new Element(tag), createElementNS: (_, tag) => new Element(tag), getElementById: (id) => nodes[id] || null, querySelector: (s) => (s === ".main-panel" ? main : s === "dialog[open]" ? null : dock), addEventListener: (_, action) => keys.push(action), get activeElement() { return Element.active; }, body: new Element("body") };
  let facts = seed.map((item) => ({ ...item })), n = 0;
  const calls = { api: [] }, net = { down, gone: new Set(), hold: null, flag: (id) => { facts = facts.map((item) => (item.fact_id === id ? { ...item, needs_review: true } : item)); } };
  const panels = {
    register: (panel) => registered.push(panel), leaveConversation() {}, refresh: async () => {}, openConversation: () => false,
    async api(path, body) {
      calls.api.push([path, JSON.parse(JSON.stringify(body || {}))]);
      if (path === "/api/memory/all") return { facts: facts.map((item) => ({ ...item })) };
      await net.hold;
      if (net.down) throw new Error("La connexion au moteur local est interrompue.");
      if (path === "/api/memory/reconfirm") {
        if (net.gone.has(body.fact_id)) { facts = facts.filter((item) => item.fact_id !== body.fact_id); throw Object.assign(new Error(NOT_FOUND), { status: 404 }); }
        facts = facts.map((item) => (item.fact_id === body.fact_id ? { ...item, needs_review: false, reconfirmed_at: Date.now() } : item));
        return { fact: facts.find((item) => item.fact_id === body.fact_id) };
      }
      if (path === "/api/memory/correct") { const old = facts.find((item) => item.fact_id === body.fact_id), next = { ...old, fact_id: `n${++n}`, text: body.text, learned_at: Date.now(), needs_review: false }; facts = [next, ...facts.filter((item) => item !== old)]; return { fact: next }; }
      facts = facts.filter((item) => item.fact_id !== body.fact_id);
      return { forgotten: [body.fact_id] };
    },
  };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document, setInterval: () => 0, JSON, Number, String, Date, Promise, Object, Array, Boolean, Math, Set, Map });
  for (const file of ["memory-detail.js", "memory-filter.js", "memory-graph.js", "memory-review.js", "memory-space.js"]) vm.runInContext(fs.readFileSync(`${__dirname}/${file}`, "utf8"), context);
  const settle = async () => { for (let i = 0; i < 4; i += 1) await new Promise((resolve) => setImmediate(resolve)); };
  Element.active = null;
  const state = { bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }], memory_auto: { revision: 1 } };
  registered[0].onState(state, {});
  await settle();
  sidebar.querySelector(".memory-entry").click();
  await settle();
  const h = {
    calls, net, settle, main, state,
    tab: (view) => main.querySelector(`.is-${view}`).click(),
    point: (id) => main.querySelector(`.memory-graph-point[data-id="${id}"]`),
    halos: () => main.querySelectorAll(".memory-graph-point").filter((point) => point.querySelector(".memory-graph-halo")).map((point) => point.attributes["data-id"]),
    panel: () => main.querySelector(".memory-review"),
    rows: () => main.querySelectorAll(".memory-review-item").map((row) => row.dataset.reviewId),
    row: (id) => main.querySelector(`.memory-review-item[data-review-id="${id}"]`),
    buttons: (id) => h.row(id).querySelectorAll("button").map((b) => b.textContent),
    press: (id, label) => { const b = h.row(id).querySelectorAll("button").find((x) => x.textContent === label); b.focus(); b.click(); }, // un clic focalise le bouton
    escape: () => keys.forEach((key) => key({ key: "Escape", preventDefault() {} })),
    detail: () => main.querySelector(".memory-detail"),
    detailButton: (label) => h.detail().querySelectorAll("button").find((b) => b.textContent === label),
    mutations: () => calls.api.filter(([path]) => path !== "/api/memory/all"),
    reload: () => registered[0].onState({ ...state, memory_auto: { revision: Math.random() } }, {}),
  };
  return h;
}

test("un souvenir à revoir a un halo dans le graphe et une ligne dans le panneau, avec sa raison ; les autres n'ont ni l'un ni l'autre", async () => {
  const h = await harness();
  h.tab("graph");
  assert.deepEqual(h.halos(), ["f1", "f3"], "le halo suit le drapeau du moteur, pas l'âge");
  assert.ok(h.point("f1").classes.includes("is-review") && !h.point("f2").classes.includes("is-review"));
  assert.match(h.point("f1").attributes["aria-label"], /à revoir/, "lisible sans la couleur");
  assert.equal(h.panel().hidden, false);
  assert.deepEqual(h.rows(), ["f1", "f3"], "du plus ancien au plus récent, sans f2");
  const row = h.row("f1");
  assert.equal(row.querySelector(".memory-review-text").textContent, "Aime le café <b>noir</b>", "le texte reste du texte");
  assert.match(row.querySelector(".memory-review-reason").textContent, /Appris le .*2026.*jamais reconfirmé/);
  assert.match(h.row("f3").querySelector(".memory-review-reason").textContent, /Dernière confirmation le .*2026/, "un souvenir déjà reconfirmé cite sa dernière confirmation");
  assert.match(row.textContent, /Encore vrai \?/);
  assert.deepEqual(h.buttons("f1"), ["Oui", "Modifier", "Oublier…"]);
  assert.deepEqual(h.buttons("f3"), ["Oui", "Oublier…"], "un souvenir de sujet ne se modifie pas");
  assert.match(h.panel().querySelector(".memory-review-title").textContent, /À revoir · 2 souvenirs/);
  assert.ok(h.main.querySelectorAll(".memory-graph-key").some((key) => /À revoir/.test(key.textContent)), "la légende nomme l'orange");
  h.tab("list");
  assert.deepEqual(h.rows().length, 2, "le panneau est aussi là dans la Liste");
});

test("aucun souvenir à revoir : ni halo, ni panneau, ni entrée de légende", async () => {
  const h = await harness({ facts: FACTS.map((item) => ({ ...item, needs_review: false })) });
  h.tab("graph");
  assert.deepEqual([h.halos(), h.panel().hidden, h.rows()], [[], true, []]);
  assert.ok(!h.main.querySelectorAll(".memory-graph-key").some((key) => /À revoir/.test(key.textContent)));
});

test("Oui envoie /api/memory/reconfirm, puis le halo et la ligne disparaissent ; rien d'autre n'est écrit", async () => {
  const h = await harness();
  h.tab("graph");
  h.press("f1", "Oui");
  assert.ok(h.row("f1").querySelectorAll("button").every((b) => b.disabled), "occupé : aucun second geste");
  h.press("f1", "Oui");
  await h.settle();
  assert.deepEqual(h.mutations(), [["/api/memory/reconfirm", { fact_id: "f1" }]], "une seule requête, sans texte ni bot");
  assert.deepEqual([h.halos(), h.rows(), h.point("f1").classes.includes("is-review")], [["f3"], ["f3"], false]);
  assert.equal(h.panel().querySelector(".memory-review-note").textContent, "Souvenir confirmé.");
  assert.equal(Element.active, h.panel().querySelector(".memory-review-title"), "la ligne focalisée a disparu : le focus va au titre du panneau");
  assert.equal(h.main.querySelectorAll(".memory-graph-point").length, 3, "le souvenir reste dans le graphe");
  h.press("f3", "Oui");
  await h.settle();
  assert.deepEqual([h.halos(), h.rows(), h.panel().hidden === false, h.panel().querySelector(".memory-review-title").textContent], [[], [], true, "À revoir"], "le dernier confirmé laisse le message, plus de ligne");
});

test("404 (souvenir oublié entre-temps) : le message du moteur s'affiche, la liste est relue, le reste fonctionne", async () => {
  const h = await harness();
  h.tab("graph");
  h.net.gone.add("f1");
  h.press("f1", "Oui");
  await h.settle();
  assert.equal(h.panel().querySelector(".memory-review-error").textContent, NOT_FOUND);
  assert.deepEqual([h.rows(), h.point("f1"), h.main.querySelectorAll(".memory-graph-point").length, h.halos()], [["f3"], null, 2, ["f3"]], "le souvenir disparu quitte le graphe et le panneau");
  assert.equal(h.panel().querySelector(".memory-review-error").attributes.role, "alert");
  h.press("f3", "Oui");
  await h.settle();
  assert.deepEqual([h.panel().querySelector(".memory-review-error").textContent, h.halos()], ["", []], "l'action suivante efface l'erreur et aboutit");
});

test("erreur réseau : l'erreur s'affiche, le souvenir reste à revoir et les boutons redeviennent utilisables", async () => {
  const h = await harness();
  h.tab("graph");
  h.net.down = true;
  h.press("f1", "Oui");
  await h.settle();
  assert.match(h.panel().querySelector(".memory-review-error").textContent, /interrompue/);
  assert.deepEqual([h.rows(), h.halos(), h.row("f1").querySelectorAll("button").some((b) => b.disabled)], [["f1", "f3"], ["f1", "f3"], false]);
  h.net.down = false;
  h.press("f1", "Oui");
  await h.settle();
  assert.deepEqual([h.rows(), h.panel().querySelector(".memory-review-error").textContent], [["f3"], ""]);
});

test("Modifier ouvre le panneau de détail déjà en édition : l'enregistrement passe par /api/memory/correct et la ligne disparaît", async () => {
  const h = await harness();
  h.tab("graph");
  h.press("f1", "Modifier");
  const field = h.detail().querySelector(".memory-detail-field");
  assert.equal(field.value, "Aime le café <b>noir</b>", "le texte est prêt à être corrigé");
  assert.deepEqual(h.mutations(), [], "rien n'est envoyé avant Enregistrer");
  field.value = "Aime le café au lait";
  field.fire("input");
  h.detailButton("Enregistrer").click();
  await h.settle();
  assert.deepEqual(h.mutations(), [["/api/memory/correct", { bot_id: "grokbot", fact_id: "f1", text: "Aime le café au lait" }]]);
  assert.deepEqual([h.rows(), h.halos()], [["f3"], ["f3"]], "le souvenir corrigé est neuf : plus à revoir");
});

test("Oublier ouvre la confirmation du panneau de détail ; annuler n'envoie rien ; confirmer oublie (route de sujet pour un fait de sujet)", async () => {
  const h = await harness();
  h.tab("list");
  h.press("f1", "Oublier…");
  assert.match(h.detail().querySelector(".memory-detail-warning").textContent, /effacé pour de bon/);
  h.detailButton("Annuler").click();
  assert.deepEqual([h.mutations(), h.rows()], [[], ["f1", "f3"]]);
  h.press("f1", "Oublier…");
  h.detailButton("Oublier ce souvenir").click();
  await h.settle();
  assert.deepEqual(h.mutations(), [["/api/memory/forget", { bot_id: "grokbot", fact_id: "f1" }]]);
  assert.deepEqual(h.rows(), ["f3"]);
  h.press("f3", "Oublier…");
  h.detailButton("Oublier ce souvenir").click();
  await h.settle();
  assert.deepEqual(h.mutations().slice(1), [["/api/topics/memory/forget", { fact_id: "f3" }]]);
  assert.equal(h.panel().hidden, true, "plus rien à revoir : le panneau s'efface");
});

test("le panneau suit les filtres de l'écran ; le halo reste sur tout le graphe", async () => {
  const h = await harness();
  h.tab("graph");
  const field = h.main.querySelector('[aria-label="Filtrer par bot"]');
  field.value = "grokbot";
  field.fire("change");
  assert.deepEqual(h.rows(), ["f1"], "seuls les souvenirs du bot filtré");
});

test("la relecture périodique fait apparaître un souvenir devenu à revoir : halo et ligne ensemble", async () => {
  const h = await harness({ facts: FACTS.map((item) => ({ ...item, needs_review: false })) });
  h.tab("graph");
  assert.deepEqual([h.panel().hidden, h.halos()], [true, []]);
  h.net.flag("f2");
  h.reload();
  await h.settle();
  assert.deepEqual([h.panel().hidden, h.rows(), h.halos()], [false, ["f2"], ["f2"]]);
});

test("styles : jetons --verify en clair et en sombre, lisibles (4,5:1), aucune couleur codée en dur, aucun texte sous 11,5 px", () => {
  const styles = fs.readFileSync(`${__dirname}/../styles.css`, "utf8");
  const cut = styles.indexOf('[data-theme="dark"]'), [light, dark] = [styles.slice(0, cut), styles.slice(cut, styles.indexOf('[data-accent="orange"]'))];
  const token = (block, name) => (block.match(new RegExp(`--${name}: (#[0-9a-f]{6});`)) || [])[1];
  const luminance = (hex) => { const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
  const ratio = (a, b) => { const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x); return (hi + 0.05) / (lo + 0.05); };
  for (const block of [light, dark]) {
    for (const name of ["verify", "verify-bg", "pane"]) assert.ok(token(block, name), `--${name} défini`);
    assert.ok(ratio(token(block, "verify"), token(block, "verify-bg")) >= 4.5, "texte orange sur le fond du panneau");
    assert.ok(ratio(token(block, "verify"), token(block, "pane")) >= 4.5, "texte orange sur une carte");
  }
  assert.notEqual(token(light, "verify"), token(dark, "verify"), "une teinte par thème");
  for (const file of ["memory-review.css", "memory-graph.css"]) {
    const css = fs.readFileSync(`${__dirname}/${file}`, "utf8");
    assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b|rgb\(|hsl\(/, file);
    for (const [, size] of css.matchAll(/font-size:\s*(?:calc\()?(\d+(?:\.\d+)?)px/g)) assert.ok(Number(size) >= 11.5, `${file} ${size}px`);
  }
  assert.match(fs.readFileSync(`${__dirname}/memory-graph.css`, "utf8"), /memory-graph-halo[^{]*\{[^}]*var\(--verify\)/, "le halo prend la couleur du jeton");
});

test("fermer le panneau de détail ouvert depuis « À revoir » rend le focus au bouton d'où l'on vient (Fermer, Annuler puis Échap)", async () => {
  const h = await harness();
  h.tab("graph");
  const button = (id, label) => h.row(id).querySelectorAll("button").find((b) => b.textContent === label);
  h.press("f1", "Modifier");
  h.detailButton("Fermer").click();
  assert.equal(Element.active, button("f1", "Modifier"), "Modifier, puis Fermer");
  h.press("f1", "Oublier…");
  h.escape();
  assert.equal(Element.active, button("f1", "Oublier…"), "Oublier, puis Échap");
  h.point("f2").click();
  h.escape();
  assert.equal(Element.active, h.point("f2"), "ouvert depuis le graphe, le focus revient toujours au point");
});
