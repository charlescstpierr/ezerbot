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
  }
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
  focus() {}
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
  main.append(chat, composer);
  sidebar.append(list);
  const nodes = { sidebar, "conversation-list": list, "chat-scroll": chat, "main-panel": main };
  const document = documentOf(nodes);
  document.querySelector = (selector) => {
    if (selector === ".main-panel") return main;
    if (selector === ".composer-dock") return composer;
    return main.querySelector(selector) || sidebar.querySelector(selector);
  };
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
