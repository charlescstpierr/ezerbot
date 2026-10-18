import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const flush = () => new Promise((resolve) => setImmediate(resolve));
class Element {
  constructor(tag, document) { this.tagName = tag; this.document = document; this.children = []; this.listeners = {}; this.dataset = {}; this.hidden = false; this.className = ""; }
  get isConnected() { return this.connected || Boolean(this.parent?.isConnected); }
  get parentElement() { return this.parent; }
  get classList() { return { toggle: (name, enabled) => { const names = new Set(this.className.split(" ").filter(Boolean)); if (enabled) names.add(name); else names.delete(name); this.className = [...names].join(" "); } }; }
  append(...children) { for (const child of children) { child.remove(); child.parent = this; this.children.push(child); } }
  replaceChildren(...children) { for (const child of [...this.children]) child.remove(); this.text = ""; this.append(...children); }
  after(child) { child.remove(); const parent = this.parent; child.parent = parent; parent.children.splice(parent.children.indexOf(this) + 1, 0, child); }
  remove() { if (this.parent) this.parent.children = this.parent.children.filter((child) => child !== this); this.parent = null; }
  set textContent(text) { this.replaceChildren(); this.text = String(text); }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute() {}
  addEventListener(name, action) { this.listeners[name] = action; }
  click() { if (!this.disabled) return this.listeners.click?.(); }
  focus() { this.document.activeElement = this; }
  scrollIntoView() { this.scrolled = true; }
  querySelectorAll(selector) {
    const matches = (el) => selector.startsWith(".") ? selector.slice(1).split(".").every((cls) => el.className.split(" ").includes(cls)) : el.tagName === selector;
    return this.children.flatMap((child) => [...(matches(child) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
}
const fact = (id = "fact-one") => ({ id, bot_name: "Archiviste", bot_id: "bot", text: `Mémoire privée ${id}`, status: "active" });
const empty = () => ({ usage: { images: 0, documents: 0, screen: 0, memory: 0, image_count: 0, document_count: 0, screen_count: 0, fact_count: 0, forget_marker_count: 0 }, orphan_facts: [] });
function mount({ initial = empty(), read, preview = { token: "preview", count: 2, bytes: 20 }, result = { deleted: 2, bytes: 20, retained: 0, already_deleted: 0 } } = {}) {
  const document = { createElement: (tag) => new Element(tag, document), getElementById: () => dialog };
  const dialog = new Element("dialog", document); dialog.open = true;
  const root = new Element("div", document); root.connected = true;
  const data = initial, requests = [];
  let registration, reads = 0;
  const panels = {
    register(value) { registration = value; }, refresh: async () => {},
    api: async (path, body) => {
      requests.push({ path, body });
      if (path === "/api/storage") return read ? read(++reads, data) : structuredClone(data);
      if (path === "/api/storage/preview") return preview;
      if (path === "/api/storage/clean") return result;
      if (path === "/api/storage/facts/delete") { data.orphan_facts = data.orphan_facts.filter((f) => f.id !== body.fact_id); return { deleted: 1 }; }
      throw new Error(`Unexpected route ${path}`);
    },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/storage.js", import.meta.url), "utf8"), {
    document, window: { GrokbotPanels: panels, GrokbotChat: { formatAttachmentSize: (bytes) => `${bytes} o` } }, MutationObserver: class { observe() {} },
  });
  registration.settingsSection(root);
  const state = (deleted = false) => registration.onState({ bots: [{ id: "bot", name: "Archiviste", deleted }] });
  state();
  return { root, document, data, requests, state, registration, readCount: () => requests.filter((r) => r.path === "/api/storage").length };
}
const button = (root, text) => root.querySelectorAll("button").find((el) => el.textContent === text);
const confirmation = (root) => root.querySelectorAll(".storage-confirmation").find((el) => !el.hidden);

test("deleting a bot while Settings stays open refreshes private facts without Actualiser", async () => {
  const p = mount(); await flush();
  assert.equal(p.root.querySelectorAll(".storage-fact").length, 0);
  p.data.orphan_facts = [fact()]; p.state(true); await flush();
  assert.equal(p.readCount(), 2);
  assert.equal(p.root.querySelectorAll(".storage-fact").length, 1);
  assert.match(p.root.textContent, /Mémoire privée fact-one/);
  p.state(true); await flush(); assert.equal(p.readCount(), 2, "unchanged polls do not repeatedly scan storage");
});

test("deleting a thread while Settings stays open refreshes screen usage without Actualiser", async () => {
  const data=empty();data.usage.screen=43700;data.usage.screen_count=2;
  const p=mount({initial:data});await flush();
  const state=(conversations)=>p.registration.onState({bots:[{id:"bot",name:"Archiviste",deleted:false}],conversations});
  state([{id:"thread",deleted:false}]);await flush();
  assert.match(p.root.textContent,/2 captures/);
  data.usage.screen=0;data.usage.screen_count=0;
  state([]);await flush();
  assert.match(p.root.textContent,/0 capture/);
  const reads=p.readCount();state([]);await flush();
  assert.equal(p.readCount(),reads,"unchanged conversation lists do not repeatedly scan storage");
});
test("an old storage response cannot overwrite a bot-deletion refresh queued in flight", async () => {
  let resolveOld;
  const p = mount({ read: (n, data) => n === 1 ? new Promise((resolve) => { resolveOld = resolve; }) : structuredClone(data) });
  p.data.orphan_facts = [fact()]; p.state(true);
  resolveOld(empty()); await flush();
  assert.equal(p.readCount(), 2);
  assert.equal(p.root.querySelectorAll(".storage-fact").length, 1);
});

test("fact confirmation is local, focuses Annuler, preserves its origin and defers automatic refresh", async () => {
  const data = empty(); data.orphan_facts = [fact()];
  const p = mount({ initial: data }); await flush();
  const row = p.root.querySelector(".storage-fact"); const origin = button(row, "Supprimer");
  origin.click();
  const card = confirmation(p.root);
  assert.equal(row.parent.children[row.parent.children.indexOf(row) + 1], card);
  assert.equal(p.document.activeElement.textContent, "Annuler");
  await button(card, "Annuler").click();
  assert.equal(p.document.activeElement, origin);
  origin.click(); p.state(true); await flush(); assert.equal(p.readCount(), 1, "do not erase a user's confirmation");
  await button(confirmation(p.root), "Annuler").click(); await flush();
  assert.equal(p.readCount(), 2);
});

test("deleting a fact keeps its result in the facts section and returns focus to its heading", async () => {
  const data = empty(); data.orphan_facts = [fact()];
  const p = mount({ initial: data }); await flush();
  button(p.root.querySelector(".storage-fact"), "Supprimer").click();
  await button(confirmation(p.root), "Confirmer la suppression").click();
  assert.equal(p.root.querySelectorAll(".storage-fact").length, 0);
  assert.equal(p.root.querySelector(".storage-fact-notice").textContent, "Fait privé supprimé.");
  assert.equal(p.document.activeElement.textContent, "Mémoire des bots supprimés");
});

test("memory explains retained forget markers and zero counts use the singular", async () => {
  const data = empty(); data.usage.memory = 387; data.usage.forget_marker_count = 3;
  const p = mount({ initial: data }); await flush();
  const details = p.root.querySelector(".storage-usage").querySelectorAll("small").map((el) => el.textContent);
  assert.deepEqual(details, ["0 fichier", "0 document", "0 capture", "0 fait · 3 marqueurs d’oubli"]);
});

test("document usage includes generated originals, previews and opening caches", async () => {
  const data=empty();data.usage.document_count=7;data.usage.documents=18200;
  const p=mount({initial:data});await flush();
  assert.match(p.root.textContent,/7 documents/);
  assert.match(p.root.textContent,/fichiers générés, aperçus et copies locales d’ouverture/);
  assert.doesNotMatch(p.root.textContent,/sans copie du fichier d’origine/);
});

test("a second-window cleanup is described as already cleaned, not retained", async () => {
  const p = mount({ result: { deleted: 0, bytes: 0, retained: 0, already_deleted: 2 } }); await flush();
  await button(p.root, "Nettoyer").click();
  assert.match(confirmation(p.root).textContent, /2 éléments inutilisés/);
  assert.equal(p.document.activeElement.textContent, "Annuler");
  await button(confirmation(p.root), "Confirmer le nettoyage").click();
  assert.equal(p.root.querySelector(".storage-notice").textContent, "Déjà nettoyé ; rien d’autre à supprimer.");
  assert.equal(p.document.activeElement.textContent, "Nettoyer");
});
