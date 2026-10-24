// Run with: node --test app/ui/panels/memory-add.test.cjs
// Le formulaire « Ajouter un souvenir » et l’écran Mémoire, chargés ensemble comme dans le bundle des panneaux.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class Element {
  constructor(tag) { Object.assign(this, { tagName: tag, children: [], classes: [], dataset: {}, attributes: {}, listeners: {}, hidden: false, disabled: false, value: "", id: "" }); this.classList = { toggle: (name, on) => { this.classes = this.classes.filter((c) => c !== name); if (on) this.classes.push(name); } }; }
  append(...kids) { for (const kid of kids) { if (kid instanceof Element) kid.parentElement = this; this.children.push(kid); } }
  insertBefore(kid, before) { kid.parentElement = this; const i = this.children.indexOf(before); if (i < 0) this.children.push(kid); else this.children.splice(i, 0, kid); }
  replaceChildren(...kids) { this.children = []; this.append(...kids); }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  removeAttribute(key) { delete this.attributes[key]; }
  addEventListener(name, action) { (this.listeners[name] ||= []).push(action); }
  fire(name, extra = {}) { const event = { target: this, preventDefault() { this.prevented = true; }, stopPropagation() {}, ...extra }; for (const action of this.listeners[name] || []) action(event); return event; }
  click() { this.fire("click"); }
  focus() { Element.active = this; }
  querySelectorAll(selector) { const all = (el) => el.children.filter((kid) => kid instanceof Element).flatMap((kid) => [kid, ...all(kid)]); return all(this).filter((kid) => (selector[0] === "." ? kid.classes.includes(selector.slice(1)) : kid.id === selector.slice(1))); }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  set className(value) { this.classes = String(value).split(/\s+/).filter(Boolean); }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map((kid) => (kid instanceof Element ? kid.textContent : String(kid))).join(""); }
}
const STATE = {
  bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "chef", name: "Chef" }, { id: "vieux", name: "Ancien", deleted: true }],
  topics: { items: [{ id: "t1", name: "Déménagement" }, { id: "t2", name: "Vacances" }] },
  memory_auto: { revision: 1 },
};

// `engine(body)` répond à la place du moteur : un souvenir ajouté, ou une erreur.
async function open({ state = STATE, engine, lost = false } = {}) {
  const sidebar = new Element("aside"), main = new Element("main"), dock = new Element("div");
  const bots = new Element("div"); bots.className = "sidebar-bots"; sidebar.append(bots);
  dock.className = "composer-dock"; main.append(dock);
  const nodes = { sidebar, "conversation-title": new Element("h1"), "workspace-label": new Element("button") };
  const document = { title: "", createElement: (tag) => new Element(tag), getElementById: (id) => nodes[id] || null, querySelector: (selector) => (selector === ".main-panel" ? main : dock) };
  const facts = [{ fact_id: "f1", bot_id: "grokbot", scope: "private", text: "Aime le café", learned_at: Date.now() }];
  const calls = { added: [], read: 0, toasts: [] }, registered = [];
  const panels = {
    register(panel) { registered.push(panel); return true; },
    async api(path, body) {
      if (path === "/api/memory/all") { calls.read += 1; return { facts: [...facts] }; }
      assert.equal(path, "/api/memory/add");
      calls.added.push(JSON.parse(JSON.stringify(body))); // hors du contexte vm, pour comparer en profondeur
      const fact = await (engine ? engine(body) : { fact_id: `n${calls.added.length}`, bot_id: body.bot_id, scope: body.scope, text: body.text.trim(), learned_at: Date.now() });
      facts.unshift(fact);
      if (lost) throw new Error("La connexion au moteur local est interrompue."); // enregistré, mais la réponse se perd
      return { fact };
    },
    leaveConversation() {},
    notify: (message) => calls.toasts.push(message),
  };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document, setInterval: () => 0, JSON, Number, String, Date, Promise, Object, Array, Boolean });
  for (const file of ["memory-add.js", "memory-space.js"]) vm.runInContext(fs.readFileSync(`${__dirname}/${file}`, "utf8"), context);
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  for (const panel of registered) panel.onState(state, {});
  await settle();
  sidebar.querySelector(".memory-entry").click();
  await settle();
  main.querySelector(".memory-add-open").click();
  const field = (id) => main.querySelector(`#memory-add-${id}`), form = main.querySelector(".memory-add");
  return {
    calls, settle, nodes, form, field, opener: main.querySelector(".memory-add-open"),
    submit: main.querySelector(".primary-button"),
    items: () => main.querySelectorAll(".memory-space-text").map((item) => item.textContent),
    count: () => sidebar.querySelector(".memory-entry-count").textContent,
    error: () => main.querySelector(".memory-add-error").textContent,
    set(id, value) { field(id).value = value; field(id).fire("input"); field(id).fire("change"); },
    send: () => form.fire("submit"),
    async add(bot, text) { this.set("bot", bot); this.set("text", text); this.send(); await settle(); },
  };
}

test("le bouton ouvre le formulaire : texte focalisé, Valider attend un texte et un bot, bots supprimés exclus", async () => {
  const h = await open();
  assert.deepEqual([h.form.hidden, Element.active], [false, h.field("text")]);
  assert.deepEqual(h.field("bot").children.map((o) => o.textContent), ["Choisir un bot", "Ezerbot", "Chef"]);
  assert.equal(h.field("topic").parentElement.hidden, true, "le sujet n’est demandé que pour la portée sujet");
  h.set("text", "   \n ");
  h.set("bot", "grokbot");
  assert.equal(h.submit.disabled, true, "texte blanc");
  h.set("text", "Le code du portail est 4821");
  assert.equal(h.submit.disabled, false);
  // Un seul bot : choisi d’office. Sans sujet : la portée sujet n’est pas proposée.
  const alone = await open({ state: { ...STATE, bots: [STATE.bots[0]], topics: { items: [] } } });
  alone.set("text", "Un fait");
  assert.deepEqual([alone.field("bot").value, alone.submit.disabled], ["grokbot", false]);
  assert.equal(alone.field("scope").children.find((o) => o.value === "topic").disabled, true);
});

test("valider appelle /api/memory/add puis relit la liste : le souvenir apparaît, compteur +1, message court", async () => {
  const h = await open();
  assert.equal(h.count(), "1");
  const reads = h.calls.read;
  h.set("bot", "chef");
  h.set("text", "  Le code du portail est 4821 ");
  h.send();
  h.send(); // un double envoi n’ajoute qu’un souvenir : Valider est inactif pendant l’envoi
  assert.equal(h.submit.disabled, true);
  await h.settle();
  assert.deepEqual(h.calls.added, [{ bot_id: "chef", text: "  Le code du portail est 4821 ", scope: "private" }]);
  assert.equal(h.calls.read, reads + 1, "la liste est relue, pas complétée à la main");
  assert.deepEqual(h.items(), ["Le code du portail est 4821", "Aime le café"]);
  assert.deepEqual([h.count(), h.nodes["conversation-title"].textContent, h.calls.toasts], ["2", "Mémoire · 2 souvenirs", ["Souvenir ajouté."]]);
  assert.deepEqual([h.form.hidden, h.field("text").value], [true, ""]);
});

test("erreur du moteur : message affiché tel quel, texte conservé, rien d’ajouté, on peut revalider", async () => {
  const h = await open({ engine: async () => { throw Object.assign(new Error("Un souvenir tient entre 1 et 2 000 caractères."), { status: 400 }); } });
  const long = "x".repeat(2001);
  await h.add("grokbot", long);
  assert.equal(h.calls.added.length, 1, "le moteur tranche : le formulaire n’impose pas lui-même 2 000 caractères");
  assert.equal(h.error(), "Un souvenir tient entre 1 et 2 000 caractères.");
  assert.equal(h.field("text").value, long);
  assert.equal(h.form.hidden || h.submit.disabled, false);
  assert.deepEqual([h.items(), h.count(), h.calls.toasts], [["Aime le café"], "1", []]);
});

test("réponse perdue après l’enregistrement : la liste relue montre le souvenir avant tout nouvel essai, le texte reste", async () => {
  const h = await open({ lost: true });
  await h.add("grokbot", "Déjà enregistré");
  assert.deepEqual(h.items(), ["Déjà enregistré", "Aime le café"]);
  assert.deepEqual([h.error(), h.field("text").value, h.count()], ["La connexion au moteur local est interrompue.", "Déjà enregistré", "2"]);
});

test("la portée sujet envoie topic_id ; la portée privée n’en envoie pas", async () => {
  const h = await open();
  h.set("bot", "grokbot");
  h.set("text", "Le garde-meuble ferme à 18 h");
  h.set("scope", "topic");
  assert.equal(h.field("topic").parentElement.hidden, false);
  assert.equal(h.submit.disabled, true, "un sujet doit être choisi");
  h.set("topic", "t2");
  h.send();
  await h.settle();
  h.opener.click();
  h.set("scope", "private");
  await h.add("grokbot", "Autre fait");
  assert.deepEqual(h.calls.added, [
    { bot_id: "grokbot", text: "Le garde-meuble ferme à 18 h", scope: "topic", topic_id: "t2" },
    { bot_id: "grokbot", text: "Autre fait", scope: "private" },
  ]);
});

test("Échap ferme le formulaire en gardant le texte, le focus revient au bouton", async () => {
  const h = await open();
  h.set("text", "Brouillon");
  h.form.fire("keydown", { key: "a" });
  assert.equal(h.form.hidden, false);
  assert.equal(h.form.fire("keydown", { key: "Escape" }).prevented, true);
  assert.deepEqual([h.form.hidden, Element.active], [true, h.opener]);
  h.opener.click();
  assert.equal(h.field("text").value, "Brouillon");
});
