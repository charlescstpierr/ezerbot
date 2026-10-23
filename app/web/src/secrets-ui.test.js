import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const STALE = "Ce formulaire n’a pas été accepté.";
const SECRET = "phrase secrete du panneau";
const flush = () => new Promise((resolve) => setImmediate(resolve));

class Element {
  constructor(tag, document) {
    this.tagName = tag;
    this.document = document;
    this.children = [];
    this.listeners = {};
    this.dataset = {};
    this.hidden = false;
    this.className = "";
    this.value = "";
    this.type = "";
    this.name = "";
    this.disabled = false;
    this.attributes = {};
  }
  get parentElement() { return this.parent; }
  append(...children) {
    for (const child of children) {
      child.remove();
      child.parent = this;
      this.children.push(child);
    }
  }
  replaceChildren(...children) {
    for (const child of [...this.children]) child.remove();
    this.text = "";
    this.append(...children);
  }
  remove() {
    if (this.parent) this.parent.children = this.parent.children.filter((child) => child !== this);
    this.parent = null;
  }
  set textContent(text) { this.replaceChildren(); this.text = String(text); }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute(name, value) { this.attributes[name] = String(value); if (name === "type") this.type = String(value); }
  getAttribute(name) { return this.attributes[name]; }
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { if (!this.disabled) return this.listeners.click?.(); }
  focus() { this.document.activeElement = this; }
  querySelectorAll(selector) {
    const matches = (el) => {
      if (selector.startsWith(".")) return selector.slice(1).split(".").every((cls) => el.className.split(" ").includes(cls));
      const name = selector.match(/\[name="([^"]+)"\]/)?.[1];
      if (name) return el.name === name || el.getAttribute("name") === name;
      return el.tagName === selector;
    };
    return this.children.flatMap((child) => [...(matches(child) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
}

function mount() {
  const document = { createElement: (tag) => new Element(tag, document), getElementById: () => null, activeElement: null };
  const root = new Element("div", document);
  const store = {};
  const requests = [];
  let registration;
  const state = { forms: { storage: { backend: "keychain", label: "Trousseau macOS" }, bot_secrets: {} }, bots: [{ id: "grokbot", name: "Ezerbot" }, { id: "bot-b", name: "Bot B" }] };
  const panels = {
    state,
    register(value) { registration = value; },
    notify() {},
    async api(path, body) {
      requests.push({ path, body: body && { ...body } });
      if (path === "/api/secrets/put") {
        const item = { name: body.name.trim().toUpperCase(), description: body.description.trim(), created_at: 1_700_000_000_000, updated_at: 1_700_000_100_000, value: body.value };
        store[body.bot_id] = [item];
        const { value: _value, ...visible } = item;
        return { secrets: [visible] };
      }
      if (path === "/api/secrets/delete") {
        delete store[body.bot_id];
        return { secrets: [] };
      }
      throw new Error(`route inattendue ${path}`);
    },
    async refresh() {
      state.forms.bot_secrets = Object.fromEntries(Object.entries(store).map(([id, items]) => [id, items.map(({ value: _value, ...item }) => item)]));
      registration.onState(state);
    },
  };
  const window = { GrokbotPanels: panels, dispatchEvent() {}, setTimeout, clearTimeout };
  runInNewContext(readFileSync(new URL("../../ui/panels/secrets.js", import.meta.url), "utf8"), { document, window, setTimeout, clearTimeout });
  const bot = { id: "grokbot", name: "Ezerbot", deleted: false };
  registration.botSection(bot, root);
  const button = (text, scope = root) => scope.querySelectorAll("button").find((el) => el.textContent === text);
  return { root, document, requests, registration, state, button, window };
}

test("le panneau vide dit Aucun secret, ajoute sans montrer la valeur, et ignore l’autre bot", async () => {
  const panel = mount();
  assert.match(panel.root.textContent, /Aucun secret/);
  assert.equal(panel.button("Ajouter un secret").textContent, "Ajouter un secret");
  assert.doesNotMatch(panel.root.textContent, /Stockage des secrets|ChatGPT|Continuer avec ChatGPT/);
  panel.button("Ajouter un secret").click();
  const name = panel.root.querySelector('[name="secret-name"]');
  const description = panel.root.querySelector('[name="secret-description"]');
  const value = panel.root.querySelector('[name="secret-value"]');
  assert.equal(value.type, "password");
  name.value = "cle_panneau";
  description.value = "Jeton du panneau";
  value.value = SECRET;
  await panel.button("Enregistrer").click();
  await flush();
  assert.equal(panel.requests[0].path, "/api/secrets/put");
  assert.equal(panel.requests[0].body.bot_id, "grokbot");
  assert.equal(panel.requests[0].body.value, SECRET);
  assert.equal(value.value, "");
  assert.match(panel.root.textContent, /CLE_PANNEAU/);
  assert.match(panel.root.textContent, /Jeton du panneau/);
  assert.match(panel.root.textContent, /\d{2}\/\d{2}\/\d{4}/);
  assert.doesNotMatch(panel.root.textContent, new RegExp(SECRET));
  assert.doesNotMatch(panel.root.textContent, /Stockage des secrets|ChatGPT/);
  const other = new Element("div", panel.document);
  panel.registration.botSection({ id: "bot-b", name: "Bot B", deleted: false }, other);
  assert.match(other.textContent, /Aucun secret/);
  assert.doesNotMatch(other.textContent, /CLE_PANNEAU|Jeton du panneau/);
  await panel.button("Supprimer").click();
  await flush();
  assert.equal(panel.requests.at(-1).path, "/api/secrets/delete");
  assert.equal(panel.requests.at(-1).body.bot_id, "grokbot");
  assert.match(panel.root.textContent, /Aucun secret/);
  assert.doesNotMatch(panel.root.textContent, /CLE_PANNEAU/);
});

test("un formulaire de secret périmé affiche la phrase exacte et n’écrit pas la valeur", async () => {
  const document = { createElement: (tag) => new Element(tag, document), getElementById: () => null };
  const calls = [];
  const panels = {
    state: { forms: {} },
    register(value) { this.panel = value; },
    async api(path, body) {
      calls.push({ path, body });
      throw new Error(STALE);
    },
    async refresh() {},
    notify() {},
  };
  const window = { GrokbotPanels: panels, dispatchEvent() {}, setTimeout, clearTimeout, Event: class Event { constructor(type) { this.type = type; } } };
  runInNewContext(readFileSync(new URL("../../ui/panels/forms.js", import.meta.url), "utf8"), { document, window, setTimeout, clearTimeout, Event: window.Event });
  const message = {
    id: "entry-1",
    kind: "secret",
    payload: {
      entry_id: "entry-1",
      route: { target_conversation_id: "conv", requesting_bot_id: "grokbot" },
      kind: { kind: "secret", name: "CLE_API", purpose: "Test" },
      status: { status: "pending" },
    },
  };
  const handled = await panels.panel.onCardAction(message, "forms.secret", { value: SECRET }, { conversation: { id: "conv" }, state: panels.state });
  assert.equal(handled, true);
  assert.equal(window.GrokbotForms.view("entry-1").error, STALE);
  assert.equal(calls[0].path, "/api/forms/submit");
  assert.equal(Object.hasOwn(calls[0].body, "value"), false, "la valeur est retirée du corps après l’envoi");
  panels.api = async () => { throw new Error(SECRET); };
  await panels.panel.onCardAction(message, "forms.secret", { value: SECRET }, { conversation: { id: "conv" }, state: panels.state });
  assert.notEqual(window.GrokbotForms.view("entry-1").error, SECRET);
  const forms = readFileSync(new URL("../../ui/panels/forms.js", import.meta.url), "utf8");
  assert.match(forms, /Stockage des secrets/);
  assert.match(forms, /identifiants ChatGPT sont gérés séparément/);
});

test("un formulaire de site envoie les valeurs au canal privé puis les oublie", async () => {
  const document = { createElement: (tag) => new Element(tag, document), getElementById: () => null };
  const calls = [];
  const panels = { state: { forms: {} }, register(value) { this.panel = value; },
    async api(path, body) { calls.push({ path, body, sent: JSON.stringify(body) }); return { accepted: true, receipt: { wake: { status: "pending" } } }; },
    async refresh() {}, notify() {} };
  const window = { GrokbotPanels: panels, dispatchEvent() {}, setTimeout, clearTimeout, Event: class {} };
  runInNewContext(readFileSync(new URL("../../ui/panels/forms.js", import.meta.url), "utf8"), { document, window, setTimeout, clearTimeout, Event: window.Event });
  const message = { kind: "form", payload: { entry_id: "site", route: { target_conversation_id: "conv" }, kind: { kind: "site_form", origin: "https://example.org" } } };
  await panels.panel.onCardAction(message, "forms.site", { values: { password: SECRET } }, { conversation: { id: "conv" } });
  assert.equal(calls[0]?.path, "/api/forms/site-submit");
  assert.match(calls[0].sent, new RegExp(SECRET));
  assert.equal(calls[0].body.origin, "https://example.org");
  assert.equal(Object.hasOwn(calls[0].body, "values"), false);
  assert.doesNotMatch(JSON.stringify(window.GrokbotForms.view("site")), new RegExp(SECRET));
});
