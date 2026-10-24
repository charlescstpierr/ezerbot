// Run with: node --test app/ui/panels/pages.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class ClassList {
  constructor(el) { this.el = el; }
  add(...names) { for (const name of names) if (!this.contains(name)) this.el.classes.push(name); }
  remove(...names) { this.el.classes = this.el.classes.filter((name) => !names.includes(name)); }
  contains(name) { return this.el.classes.includes(name); }
  toggle(name, force) {
    const on = force === undefined ? !this.contains(name) : force;
    if (on) this.add(name); else this.remove(name);
    return on;
  }
}
class Element {
  constructor(tag, document) {
    this.ownerDocument = document;
    this.tagName = String(tag).toLowerCase();
    this.children = [];
    this.classes = [];
    this.classList = new ClassList(this);
    this.dataset = {};
    this.attributes = {};
    this.listeners = {};
    this.hidden = false;
    this.value = "";
    this.disabled = false;
    this.type = "";
    this.id = "";
    this.style = {};
    this.offsetWidth = 90;
    this.offsetHeight = 30;
    this.rect = { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
  }
  get firstChild() { return this.children[0]; }
  contains(node) { return node === this || this.children.some((child) => child === node || (child instanceof Element && child.contains(node))); }
  getBoundingClientRect() { return this.rect; }
  scrollIntoView() { this.scrolled = true; }
  append(...children) {
    for (const child of children) {
      if (child instanceof Element) child.parentElement = this;
      this.children.push(child);
    }
  }
  insertBefore(node, before) {
    if (node instanceof Element) node.parentElement = this;
    const index = this.children.indexOf(before);
    if (index < 0) this.children.push(node);
    else this.children.splice(index, 0, node);
  }
  replaceChildren(...children) {
    this.children = [];
    this.append(...children);
  }
  setAttribute(key, value) { this.attributes[key] = String(value); if (key === "id") this.id = String(value); if (key === "class") this.classes = String(value).split(/\s+/).filter(Boolean); }
  getAttribute(key) { return this.attributes[key]; }
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { if (!this.disabled) this.listeners.click?.(); }
  focus() { this.focused = true; this.ownerDocument.activeElement = this; }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  querySelectorAll(selector) {
    const matches = (el) => {
      if (selector.startsWith("#")) return el.id === selector.slice(1);
      if (selector.startsWith(".")) return el.classes.includes(selector.slice(1));
      return el.tagName === selector.toLowerCase();
    };
    return this.children.filter((child) => child instanceof Element).flatMap((child) => [...(matches(child) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  set className(value) { this.classes = String(value).split(/\s+/).filter(Boolean); }
  get className() { return this.classes.join(" "); }
  set textContent(text) { this.replaceChildren(String(text)); }
  get textContent() { return this.children.map((child) => child?.textContent ?? String(child)).join(""); }
}
const documentOf = (nodes) => {
  const document = {
    createElement: (tag) => new Element(tag, document),
    createTextNode: (text) => ({ textContent: String(text), children: [] }),
    getElementById: (id) => nodes[id] || null,
    querySelector: (selector) => Object.values(nodes).find((node) => node.querySelector(selector) || (selector.startsWith(".") && node.classes.includes(selector.slice(1)) ? node : null)) || null,
  };
  for (const node of Object.values(nodes)) node.ownerDocument = document;
  return document;
};
const descendants = (element) => [element, ...element.children.filter((child) => child instanceof Element).flatMap(descendants)];
const buttons = (root) => descendants(root).filter((el) => el.tagName === "button");

const readyDocument = {
  states: [{ name: "offre", initial: "Essentiel" }],
  view: {
    type: "element", tag: "article", children: [
      { type: "element", tag: "h1", children: [{ type: "text", text: "Trois forfaits" }] },
      { type: "element", tag: "p", children: [{ type: "text", text: "Offre choisie : " }, { type: "state", name: "offre" }] },
      { type: "element", tag: "table", children: [
        { type: "element", tag: "thead", children: [{ type: "element", tag: "tr", children: [
          { type: "element", tag: "th", children: [{ type: "text", text: "Offre" }] },
          { type: "element", tag: "th", children: [{ type: "text", text: "Prix" }] },
        ] }] },
        { type: "element", tag: "tbody", children: [{ type: "map", items: [["Essentiel", "9 €"], ["Confort", "19 €"], ["Famille", "29 €"]], node: {
          type: "element", tag: "tr", children: [
            { type: "element", tag: "td", children: [{ type: "field", index: 0 }] },
            { type: "element", tag: "td", children: [{ type: "field", index: 1 }] },
            { type: "element", tag: "td", children: [{ type: "element", tag: "button", click: { state: "offre", index: 0 }, children: [{ type: "text", text: "Choisir" }] }] },
          ],
        } }] },
      ] },
    ],
  },
};

function harness(page) {
  const sidebar = new Element("aside");
  const list = new Element("nav");
  list.id = "conversation-list";
  const main = new Element("main");
  main.classes = ["main-panel"];
  const chat = new Element("section");
  chat.id = "chat-scroll";
  const composer = new Element("div");
  composer.classes = ["composer-dock"];
  const area = new Element("div");
  area.classes = ["composer-area"];
  const form = new Element("form");
  form.id = "composer-form";
  const editor = new Element("div");
  form.append(editor);
  area.append(form);
  composer.append(area);
  main.append(chat, composer);
  sidebar.append(list);
  const nodes = { sidebar, "conversation-list": list, "chat-scroll": chat, "main-panel": main };
  const document = documentOf(nodes);
  for (const el of [area, form, editor]) el.ownerDocument = document;
  document.querySelector = (selector) => {
    if (selector === ".main-panel") return main;
    if (selector === ".composer-dock") return composer;
    if (selector === ".composer-area") return area;
    if (selector === "#composer-input [contenteditable='true']") return editor;
    return main.querySelector(selector) || sidebar.querySelector(selector);
  };
  const selection = {
    rangeCount: 0, isCollapsed: true, text: "", range: null,
    getRangeAt() { return this.range; },
    toString() { return this.text; },
    removeAllRanges() { this.change(0, ""); },
    change(count, text, range = null) { this.rangeCount = count; this.isCollapsed = !count; this.text = text; this.range = range; document.listeners.selectionchange?.(); },
  };
  document.listeners = {};
  document.activeElement = null;
  document.addEventListener = (name, action) => { document.listeners[name] = action; };
  document.getSelection = () => selection;
  const requests = [];
  let current = { ...page };
  const panels = {
    state: { pages: { by_bot: { grokbot: [{ id: page.id, bot_id: "grokbot", conversation_id: "fil", title: page.title, filename: page.filename, status: page.status }] } } },
    register(value) { this.panel = value; },
    async api(path, body) {
      requests.push({ path, body });
      if (path === "/api/pages/rename") {
        current = { ...current, title: body.title };
        panels.state.pages.by_bot.grokbot[0].title = body.title;
        return { page: current };
      }
      if (path === `/api/pages/${page.id}`) return { page: current };
      if (panels.others?.[path]) return { page: panels.others[path] };
      throw new Error(`adresse inattendue ${path}`);
    },
    async refresh() {},
    notify() {},
    openConversation() { return true; },
  };
  vm.runInNewContext(fs.readFileSync(require("node:path").join(__dirname, "pages.js"), "utf8"), {
    window: { GrokbotPanels: panels },
    document,
  });
  return {
    panels,
    requests,
    main,
    chat,
    sidebar,
    area,
    editor,
    selection,
    current: () => current,
    use(next) { current = next; },
  };
}

test("la Page s’affiche à côté du fil, se renomme, s’ouvre en pleine page, et ne propose pas de partage", async () => {
  const page = { id: "page_ready", title: "Trois forfaits", filename: "forfaits.page.tsx", status: "ready", diagnostic: null, preview: "Trois forfaits Essentiel Confort Famille Choisir", document: readyDocument };
  const { panels, requests, main, chat, sidebar } = harness(page);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  const surface = `${sidebar.textContent}\n${main.textContent}`;
  assert.match(surface, /Pages/);
  assert.match(surface, /Trois forfaits/);
  assert.match(surface, /Essentiel/);
  assert.match(surface, /Confort/);
  assert.match(surface, /Famille/);
  assert.equal(buttons(main).some((button) => button.textContent === "Ouvrir en pleine page"), true);
  assert.equal(descendants(main).some((el) => el.tagName === "input" && (el.getAttribute("aria-label") === "Renommer" || el.attributes["aria-label"] === "Renommer")), true);
  assert.doesNotMatch(surface, /Partager|lien public|URL publique|Copier le lien/);
  const choisir = buttons(main).filter((button) => button.textContent === "Choisir");
  assert.equal(choisir.length, 3);
  choisir[1].click();
  assert.match(main.textContent, /Offre choisie : Confort/);
  buttons(main).find((button) => button.textContent === "Ouvrir en pleine page").click();
  assert.equal(main.classList.contains("page-full"), true);
  assert.equal(chat.hidden, true);
  const rename = descendants(main).find((el) => el.tagName === "input");
  rename.value = "Mes forfaits";
  rename.listeners.change?.();
  rename.listeners.blur?.();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(requests.some((call) => call.path === "/api/pages/rename" && call.body.title === "Mes forfaits"), true);
  assert.match(sidebar.textContent, /Mes forfaits/);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(sidebar.textContent, /Mes forfaits/);
  assert.equal(requests.some((call) => String(call.path).includes("share") || String(call.path).includes("publish")), false);
});

function nestedDocument(depth) {
  let view = { type: "text", text: "profond" };
  for (let level = 0; level < depth; level += 1) {
    view = { type: "element", tag: "div", children: [view] };
  }
  return { states: [], view };
}

function canvasOf(main) {
  return descendants(main).find((el) => el.classes.includes("page-canvas"));
}

test("une Page de 64 niveaux s’affiche encore", async () => {
  const page = { id: "page_cap", title: "Profonde", filename: "profonde.page.tsx", status: "ready", diagnostic: null, preview: "profond", document: nestedDocument(64) };
  const { panels, main } = harness(page);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(main.textContent, /profond/);
  assert.equal(canvasOf(main).children.length > 0, true);
});

test("une Page de 65 niveaux est refusée avant de peindre le DOM", async () => {
  const page = { id: "page_over", title: "Profonde", filename: "profonde.page.tsx", status: "ready", diagnostic: null, preview: "profond", document: nestedDocument(65) };
  const { panels, main } = harness(page);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(main.textContent, /imbrication trop profonde/);
  assert.equal(canvasOf(main).children.length, 0);
});

test("une Page déjà enregistrée plus profonde que 64 niveaux est refusée sans construire le DOM", async () => {
  const page = { id: "page_deep", title: "Profonde", filename: "profonde.page.tsx", status: "ready", diagnostic: null, preview: "profond", document: nestedDocument(9000) };
  const { panels, main } = harness(page);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(main.textContent, /imbrication trop profonde/);
  assert.equal(canvasOf(main).children.length, 0);
  assert.equal(descendants(main).some((el) => el.tagName === "div" && el.classes.includes("page-canvas") && el.children.length > 0), false);
});

test("un useState se réinitialise quand la source change et garde le clic si la version est la même", async () => {
  const broken = {
    id: "page_ready",
    title: "Trois forfaits",
    filename: "forfaits.page.tsx",
    status: "error",
    diagnostic: "ligne 6, caractère 7 : balise « td » non fermée",
    preview: "",
    document: null,
    source_version: "casse",
  };
  const ready = {
    id: "page_ready",
    title: "Trois forfaits",
    filename: "forfaits.page.tsx",
    status: "ready",
    diagnostic: null,
    preview: "Offre choisie : Essentiel",
    source_version: "corrige",
    document: {
      states: [{ name: "offre", initial: "Essentiel" }],
      view: {
        type: "element",
        tag: "article",
        children: [
          { type: "element", tag: "p", children: [{ type: "text", text: "Offre choisie : " }, { type: "state", name: "offre" }] },
          { type: "element", tag: "button", click: { state: "offre", value: "Confort" }, children: [{ type: "text", text: "Choisir" }] },
        ],
      },
    },
  };
  const { panels, main, use } = harness(broken);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(main.textContent, /ligne 6, caractère 7/);
  use(ready);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(main.textContent, /Offre choisie : Essentiel/);
  buttons(main).find((button) => button.textContent === "Choisir").click();
  assert.match(main.textContent, /Offre choisie : Confort/);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(main.textContent, /Offre choisie : Confort/);
  use({ ...ready, source_version: "encore" });
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(main.textContent, /Offre choisie : Essentiel/);
});

test("une compilation cassée montre la ligne et le caractère, pas une Page vide réussie", async () => {
  const page = { id: "page_broken", title: "Trois forfaits", filename: "forfaits.page.tsx", status: "error", diagnostic: "ligne 6, caractère 7 : balise « td » non fermée", preview: "", document: null };
  const { panels, main, sidebar } = harness(page);
  await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
  assert.match(main.textContent, /ligne 6, caractère 7/);
  assert.equal(descendants(main).some((el) => el.tagName === "table"), false);
  assert.equal(buttons(main).some((button) => button.textContent === "Choisir"), false);
  assert.match(sidebar.textContent, /Pages/);
  assert.doesNotMatch(`${sidebar.textContent}\n${main.textContent}`, /Partager|lien public|URL publique/);
});

for (const [levels, text] of [[14, "x"], [14, ""], [8, "é".repeat(3000)], [40, "x"]]) {
  test(`les maps imbriquées respectent le budget cumulé (${levels}, ${text.length})`, async () => {
    let view = { type: "text", text };
    for (let i = 0; i < levels; i++) {
      view = { type: "element", tag: "div", children: [{ type: "map", items: [[1], [2]], node: view }] };
    }
    const { panels, main } = harness({ id: "budget", title: "Budget", status: "ready", document: { states: [], view } });
    await panels.panel.onState(panels.state, { bot: { id: "grokbot" }, conversation: { id: "fil" } });
    assert.match(main.textContent, /la Page dépasse le budget de rendu/);
    assert.equal(canvasOf(main).children.length, 0);
  });
}

const ctxOf = (conversation, bot = "grokbot") => ({ bot: { id: bot }, conversation: conversation ? { id: conversation } : null });

test("un rafraîchissement identique ne reconstruit pas la Page et garde le focus", async () => {
  const page = { id: "page_ready", title: "Trois forfaits", filename: "forfaits.page.tsx", status: "ready", diagnostic: null, preview: "", source_version: "v1", document: readyDocument };
  const { panels, main } = harness(page);
  await panels.panel.onState(panels.state, ctxOf("fil"));
  const before = buttons(canvasOf(main));
  await panels.panel.onState(panels.state, ctxOf("fil"));
  const after = buttons(canvasOf(main));
  assert.equal(after.length, 3);
  assert.equal(after.every((button, index) => button === before[index]), true);
  after[1].click();
  assert.match(main.textContent, /Offre choisie : Confort/);
});

test("une Page écrite par un autre participant d’un groupe s’affiche avec le fil du groupe", async () => {
  const page = { id: "page_ready", title: "Trois forfaits", filename: "forfaits.page.tsx", status: "ready", diagnostic: null, preview: "", source_version: "v1", document: readyDocument };
  const { panels, main, sidebar } = harness(page);
  panels.state.pages.by_bot = {
    coordinateur: [],
    grokbot: [{ id: "page_ready", bot_id: "grokbot", conversation_id: "groupe", title: "Trois forfaits", filename: "forfaits.page.tsx", status: "ready" }],
  };
  await panels.panel.onState(panels.state, ctxOf("groupe", "coordinateur"));
  assert.match(sidebar.textContent, /Trois forfaits/);
  assert.match(main.textContent, /Offre choisie : Essentiel/);
  await panels.panel.onState(panels.state, ctxOf("autre", "coordinateur"));
  assert.doesNotMatch(sidebar.textContent, /Trois forfaits/);
});

test("une Page dont la conversation a disparu reste ouvrable, sans l’afficher à côté d’un autre fil", async () => {
  const page = { id: "page_ready", title: "Trois forfaits", filename: "forfaits.page.tsx", status: "ready", diagnostic: null, preview: "", source_version: "v1", document: readyDocument };
  const { panels, main, sidebar } = harness(page);
  panels.state.conversations = [{ id: "autre" }];
  await panels.panel.onState(panels.state, ctxOf("autre"));
  assert.equal(descendants(main).some((el) => el.classes.includes("page-canvas") && el.children.length > 0), false);
  const link = descendants(sidebar).find((el) => el.tagName === "button" && el.classes.includes("page-link"));
  assert.ok(link);
  link.click();
  await new Promise((resolve) => setImmediate(resolve));
  assert.match(main.textContent, /Offre choisie : Essentiel/);
  await panels.panel.onState(panels.state, ctxOf("autre"));
  assert.match(main.textContent, /Offre choisie : Essentiel/);
  await panels.panel.onState(panels.state, ctxOf("encore-un-autre"));
  assert.equal(descendants(main).some((el) => el.classes.includes("page-canvas") && el.children.length > 0), false);
});

test("une navigation refusée pendant une réponse prévient au lieu d’afficher la Page à côté d’un autre fil", async () => {
  const page = { id: "page_ready", title: "Trois forfaits", filename: "forfaits.page.tsx", status: "ready", diagnostic: null, preview: "", source_version: "v1", document: readyDocument };
  const { panels, main, sidebar } = harness(page);
  const notices = [];
  panels.notify = (message) => notices.push(message);
  panels.openConversation = () => false;
  panels.state.conversations = [{ id: "fil" }, { id: "autre" }];
  await panels.panel.onState(panels.state, ctxOf("autre"));
  descendants(sidebar).find((el) => el.tagName === "button" && el.classes.includes("page-link")).click();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(notices.length, 1);
  assert.equal(descendants(main).some((el) => el.classes.includes("page-canvas") && el.children.length > 0), false);
});

const cell = (text) => ({ type: "element", tag: "td", children: [{ type: "text", text }] });
const forfaits = {
  id: "page_forfaits", title: "Trois forfaits", filename: "forfaits.page.tsx", status: "ready", diagnostic: null,
  preview: "Trois forfaitsForfait A9 €Forfait B19 €Forfait C29 €", source_version: "a".repeat(64),
  document: { states: [], view: { type: "element", tag: "article", children: [
    { type: "element", tag: "h1", children: [{ type: "text", text: "Trois forfaits" }] },
    { type: "element", tag: "table", children: [{ type: "element", tag: "tbody", children: [
      { type: "element", tag: "tr", children: [cell("Forfait A"), cell("9 €")] },
      { type: "element", tag: "tr", children: [cell("Forfait B"), cell("19 €")] },
    ] }] },
    { type: "element", tag: "p", children: [{ type: "text", text: `${"long ".repeat(100)}fin` }] },
  ] } },
};
const textNode = (root, text) => {
  for (const child of root.children) {
    if (!(child instanceof Element)) { if (child.textContent === text) return child; continue; }
    const found = textNode(child, text);
    if (found) return found;
  }
  return null;
};
const byClass = (root, name) => descendants(root).find((el) => el.classes.includes(name));

async function openForfaits() {
  const h = harness(forfaits);
  await h.panels.panel.onState(h.panels.state, ctxOf("fil"));
  const stage = byClass(h.main, "page-stage");
  stage.rect = { left: 400, top: 60, right: 800, bottom: 660, width: 400, height: 600 };
  byClass(h.main, "page-canvas").rect = { left: 400, top: 120, right: 800, bottom: 660, width: 400, height: 540 };
  const comment = byClass(h.main, "page-comment");
  const quoteBar = byClass(h.area, "gb-quote-chip");
  const inCanvas = () => textNode(canvasOf(h.main), "Forfait B");
  const select = (text, container = textNode(canvasOf(h.main), text) || inCanvas(), rect = { left: 450, top: 200, right: 520, bottom: 220, width: 70, height: 20 }) =>
    h.selection.change(1, text, { startContainer: container, endContainer: container, getBoundingClientRect: () => rect });
  return { ...h, stage, comment, quoteBar, select, anchor: () => JSON.parse(JSON.stringify(h.panels.panel.sendExtras())) };
}

test("sélectionner « Forfait B » affiche « Commenter » près de la sélection ; le clic pose la puce au-dessus de la boîte de message", async () => {
  const { comment, quoteBar, area, select, selection, editor, anchor } = await openForfaits();
  assert.equal(comment.hidden, true);
  select("Forfait B");
  assert.equal(comment.hidden, false);
  assert.equal(comment.textContent, "Commenter");
  assert.deepEqual([comment.style.left, comment.style.top], ["50px", "166px"]);
  assert.equal(quoteBar.hidden, true);
  comment.click();
  assert.equal(quoteBar.hidden, false);
  assert.equal(area.children[0], quoteBar);
  assert.equal(quoteBar.textContent, "Page « Trois forfaits » — « Forfait B »");
  assert.equal(comment.hidden, true);
  assert.equal(selection.rangeCount, 0);
  assert.equal(editor.focused, true);
  assert.deepEqual(anchor(), { page_anchor: { page_id: "page_forfaits", source_version: "a".repeat(64), passage: "Forfait B" } });
});

test("l’envoi transmet la Page, la version lue par le panneau et le passage rogné ; la puce part avec le message", async () => {
  const { comment, quoteBar, panels, select, selection, anchor } = await openForfaits();
  select("  Forfait B\n");
  comment.click();
  assert.equal(anchor().page_anchor.passage, "Forfait B");
  panels.panel.composer.onSent({ messageId: "m1", extras: { mentions: [] } });
  assert.equal(quoteBar.hidden, false, "un envoi sans ancre ne retire pas la puce");
  panels.panel.composer.onSent({ messageId: "m2", extras: anchor() });
  assert.equal(quoteBar.hidden, true);
  assert.deepEqual(anchor(), {});
  assert.equal(selection.rangeCount, 0);
});

test("la croix retire la puce : le message part sans ancre", async () => {
  const { comment, quoteBar, select, anchor } = await openForfaits();
  select("Forfait B");
  comment.click();
  const cross = buttons(quoteBar)[0];
  assert.equal(cross.attributes["aria-label"], "Retirer la citation");
  cross.click();
  assert.equal(quoteBar.hidden, true);
  assert.equal(quoteBar.children.length, 0);
  assert.deepEqual(anchor(), {});
});

test("une sélection de plus de 500 caractères ne donne ni bouton ni ancre ; 500 passent", async () => {
  const { comment, select, anchor } = await openForfaits();
  const long = `${"long ".repeat(100)}fin`;
  select(long);
  assert.equal(long.length > 500, true);
  assert.equal(comment.hidden, true);
  assert.deepEqual(anchor(), {});
  const exact = "é".repeat(500);
  select(exact);
  assert.equal(comment.hidden, false);
  select("é".repeat(501));
  assert.equal(comment.hidden, true);
});

test("une sélection vide, hors de la Page ou qui déborde du canevas ne donne pas de bouton", async () => {
  const { comment, select, selection, sidebar, main } = await openForfaits();
  select("   ");
  assert.equal(comment.hidden, true);
  select("Forfait B");
  assert.equal(comment.hidden, false);
  selection.change(0, "");
  assert.equal(comment.hidden, true);
  select("Pages", sidebar.children[0]);
  assert.equal(comment.hidden, true);
  const inside = textNode(canvasOf(main), "Forfait B");
  selection.change(1, "Forfait B et la suite", { startContainer: inside, endContainer: sidebar, getBoundingClientRect: () => ({ left: 450, top: 200, bottom: 220 }) });
  assert.equal(comment.hidden, true);
  select("Forfait B", inside, { left: 450, top: 20, right: 520, bottom: 40, width: 70, height: 20 });
  assert.equal(comment.hidden, true, "une sélection défilée hors du canevas n’a pas de bouton flottant");
});

test("au clavier, « Commenter » est un bouton après le canevas et garde la sélection quand il prend le focus", async () => {
  const { comment, quoteBar, stage, select, selection, anchor } = await openForfaits();
  const order = stage.children;
  assert.equal(order.indexOf(comment) > order.indexOf(byClass(stage, "page-canvas")), true);
  assert.equal(comment.tagName, "button");
  assert.equal(comment.type, "button");
  assert.equal(comment.attributes.tabindex, undefined);
  select("Forfait B");
  comment.focus();
  selection.change(0, "");
  assert.equal(comment.hidden, false, "Tab vers le bouton ne le fait pas disparaître");
  comment.click();
  assert.equal(quoteBar.textContent, "Page « Trois forfaits » — « Forfait B »");
  assert.equal(anchor().page_anchor.passage, "Forfait B");
});

test("changer de fil retire la puce, et « Commenter » depuis la pleine page revient au fil pour écrire", async () => {
  const h = await openForfaits();
  h.select("Forfait B");
  buttons(h.main).find((button) => button.textContent === "Ouvrir en pleine page").click();
  assert.equal(h.main.classList.contains("page-full"), true);
  h.comment.click();
  assert.equal(h.main.classList.contains("page-full"), false);
  assert.equal(h.quoteBar.hidden, false);
  await h.panels.panel.onState(h.panels.state, ctxOf("autre"));
  assert.equal(h.quoteBar.hidden, true);
  assert.deepEqual(h.anchor(), {});
});

test("un clic sur la puce du fil montre la Page ; une Page disparue prévient", async () => {
  const h = await openForfaits();
  h.panels.state.conversations = [{ id: "fil" }, { id: "autre" }];
  const opened = [];
  h.panels.openConversation = (id) => { opened.push(id); return true; };
  const notices = [];
  h.panels.notify = (message) => notices.push(message);
  h.stage.scrolled = false;
  h.panels.panel.openPage("page_forfaits");
  assert.deepEqual(opened, []);
  assert.equal(h.stage.scrolled, true);
  h.panels.panel.openPage("page_disparue");
  assert.deepEqual(notices, ["Cette Page n’existe plus."]);
  await h.panels.panel.onState(h.panels.state, ctxOf("autre"));
  assert.equal(h.stage.hidden, true);
  h.panels.panel.openPage("page_forfaits");
  assert.deepEqual(opened, ["fil"]);
});

test("ouvrir une autre Page du même fil retire la citation de la première avant tout envoi", async () => {
  const other = { id: "page_autre", title: "Autre", filename: "autre.page.tsx", status: "ready", diagnostic: null, preview: "Autre", source_version: "b".repeat(64), document: readyDocument };
  const h = await openForfaits();
  h.panels.others = { "/api/pages/page_autre": other };
  h.panels.state.conversations = [{ id: "fil" }];
  h.panels.state.pages.by_bot.grokbot.push({ id: "page_autre", bot_id: "grokbot", conversation_id: "fil", title: "Autre", filename: "autre.page.tsx", status: "ready" });
  await h.panels.panel.onState(h.panels.state, ctxOf("fil"));
  h.select("Forfait B");
  h.comment.click();
  assert.equal(h.anchor().page_anchor.page_id, "page_forfaits");
  descendants(h.sidebar).find((el) => el.tagName === "button" && el.dataset.pageId === "page_autre").click();
  assert.equal(h.quoteBar.hidden, true);
  assert.deepEqual(h.anchor(), {});
});

test("dans une sous-tâche déléguée, « Commenter » n’est pas proposé : son envoi n’emporte pas d’ancre", async () => {
  const h = harness(forfaits);
  h.panels.state.conversations = [{ id: "fil", kind: "task" }];
  await h.panels.panel.onState(h.panels.state, ctxOf("fil"));
  const comment = byClass(h.main, "page-comment");
  byClass(h.main, "page-stage").rect = { left: 400, top: 60, right: 800, bottom: 660, width: 400, height: 600 };
  byClass(h.main, "page-canvas").rect = { left: 400, top: 120, right: 800, bottom: 660, width: 400, height: 540 };
  const node = textNode(canvasOf(h.main), "Forfait B");
  h.selection.change(1, "Forfait B", { startContainer: node, endContainer: node, getBoundingClientRect: () => ({ left: 450, top: 200, right: 520, bottom: 220, width: 70, height: 20 }) });
  assert.equal(comment.hidden, true);
});

test("500 caractères en comptant les emoji comme le moteur : 500 passent, 501 non", async () => {
  const { comment, select, anchor } = await openForfaits();
  select("😀".repeat(500));
  assert.equal(comment.hidden, false);
  comment.click();
  assert.equal([...anchor().page_anchor.passage].length, 500);
  select("😀".repeat(501));
  assert.equal(comment.hidden, true);
});

test("le rognage suit `str::trim` de Rust : U+0085 est retiré, U+FEFF est gardé", async () => {
  const { comment, select, anchor } = await openForfaits();
  select(`\u0085${"a".repeat(500)}`);
  assert.equal(comment.hidden, false, "500 caractères une fois U+0085 retiré");
  comment.click();
  assert.equal(anchor().page_anchor.passage, "a".repeat(500));
  select("\uFEFFabc");
  comment.click();
  assert.equal(anchor().page_anchor.passage, "\uFEFFabc");
});

test("une sélection sur plusieurs cellules et paragraphes donne le bouton et part telle quelle", async () => {
  const { comment, select, anchor } = await openForfaits();
  select("Trois forfaits\n\nForfait A\t9 €\nForfait B");
  assert.equal(comment.hidden, false);
  comment.click();
  assert.equal(anchor().page_anchor.passage, "Trois forfaits\n\nForfait A\t9 €\nForfait B");
});
