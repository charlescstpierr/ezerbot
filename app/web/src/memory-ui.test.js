import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { memoryPresentation } from "./memory-presentation.js";

class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.dataset = {}; this.listeners = {}; this.value = ""; }
  append(...items) { for (const child of items) { child.parent = this; this.children.push(child); } }
  replaceChildren(...items) { this.children = []; this.text = ""; this.append(...items); }
  set textContent(text) { this.text = text; this.children = []; }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute(name, value) { this[name] = value; }
  addEventListener(name, action) { this.listeners[name] = action; }
  focus() {}
  closest(tag) { return this.tagName === tag ? this : this.parent?.closest(tag); }
  remove() { this.parent.children = this.parent.children.filter((child) => child !== this); }
  querySelector(selector) { return walk(this).find((child) => selector.startsWith(".") ? child.className?.split(" ").includes(selector.slice(1)) : child.tagName === selector); }
}
const walk = (root) => [root, ...root.children.flatMap(walk)];
const byClass = (root, name) => walk(root).find((el) => el.className?.split(" ").includes(name));
const button = (root, text) => walk(root).find((el) => el.tagName === "button" && el.textContent === text);
const flush = () => new Promise((resolve) => setImmediate(resolve));

async function fixture() {
  const initial = { fact_id: "fact-one", bot_id: "bot", text: "Je préfère le thé vert.", scope: "private", learned_at: 1, provenance: { source: "automatic" } };
  let current = initial, registration, refreshes = 0;
  const requests = [], root = new Element("div");
  const state = { bots: [{ id: "bot", name: "Ezerbot" }], provider: { connected: true, models_ready: true }, memory_auto: { available: true, revision: 1 }, conversations: [{ messages: [{ kind: "memory_auto", payload: { ...initial, active: true } }] }] };
  const panels = {
    state, register(value) { registration = value; }, notify() {},
    async api(path, body) {
      requests.push(path);
      if (path === "/api/memory") return { facts: current ? [current] : [] };
      if (path === "/api/memory/forget") { current = null; return { forgotten: [body.fact_id] }; }
      if (path === "/api/memory/correct") { current = { ...current, fact_id: "fact-two", text: body.text }; return { fact: current }; }
      if (path === "/api/memory/promote") { current.scope = "team"; return { created: [current.fact_id], already_in_team: [], kept_private_count: 0 }; }
      if (path === "/api/memory/automatic/undo") throw new Error("Ce fait a changé.");
      throw new Error(path);
    },
    async refresh() {
      refreshes++;
      state.conversations[0].messages[0].payload = { ...(current || initial), active: !!current, text: current?.text || "" };
      registration.onState(state);
    },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/memory.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels }, setTimeout, clearTimeout, HTMLInputElement: Element,
    document: { createElement: (tag) => new Element(tag), createTextNode: (text) => { const el = new Element("#text"); el.textContent = text; return el; } },
  });
  registration.onState(state);
  const cleanup = registration.botSection(state.bots[0], root);
  await flush();
  return { root, state, panels, registration, requests, cleanup, setCurrent: (fact) => { current = fact; }, refreshes: () => refreshes, card: () => state.conversations[0].messages[0] };
}

test("forgetting through the real panel refreshes the thread into an inactive, text-free receipt", async () => {
  const f = await fixture();
  try {
    button(f.root, "Oublier…").listeners.click();
    button(f.root, "Confirmer l’oubli").listeners.click();
    await flush();
    assert.ok(f.requests.includes("/api/memory/forget"));
    assert.equal(f.refreshes(), 1);
    assert.equal(f.card().payload.active, false);
    assert.equal(f.card().payload.text, "");
    assert.equal(memoryPresentation(f.card().payload).receipt, "Ce fait a été oublié.");
    assert.equal(memoryPresentation(f.card().payload).quote, "");
  } finally { f.cleanup(); }
});

test("editing through the panel refreshes the existing thread card immediately", async () => {
  const f = await fixture();
  try {
    button(f.root, "Modifier").listeners.click();
    byClass(f.root, "memory-editor").querySelector("textarea").value = "Je préfère le thé avec du miel.";
    button(f.root, "Enregistrer la modification").listeners.click();
    await flush();
    assert.equal(f.refreshes(), 1);
    assert.equal(f.card().payload.text, "Je préfère le thé avec du miel.");
    assert.equal(memoryPresentation(f.card().payload).quote, "« Je préfère le thé avec du miel. »");
  } finally { f.cleanup(); }
});

test("sharing refreshes the thread and a rejected card action also reconciles its stale payload", async () => {
  const f = await fixture();
  try {
    const check = byClass(f.root, "memory-fact-choice").children[0];
    check.checked = true; check.listeners.change();
    button(f.root, "Partager la sélection (1)").listeners.click();
    await flush();
    assert.equal(f.refreshes(), 1);
    assert.equal(f.card().payload.scope, "team");
    const stale = f.card(); f.setCurrent(null);
    await assert.rejects(f.registration.onCardAction(stale, "undo"), /Ce fait a changé/);
    assert.equal(f.refreshes(), 2);
    assert.equal(f.card().payload.active, false);
  } finally { f.cleanup(); }
});

test("memory receipts distinguish corrections, restore undo and the owning group bot", () => {
  const updated = memoryPresentation({ active: true, replaced: true, text: "Je code en Rust." }, "Éclaireur", true);
  assert.equal(updated.label, "Mis à jour par Éclaireur :");
  assert.equal(updated.quote, "« Je code en Rust. »");
  assert.equal(memoryPresentation({ active: true }).label, "Mémorisé :");
  assert.equal(memoryPresentation({ active: true }, "Ezerbot", true).label, "Mémorisé par Ezerbot :");
  const undo = memoryPresentation({ active: false, cancelled: true, replaced: true, text: "Texte ancien" });
  assert.match(undo.receipt, /l’ancien fait est rétabli/);
  assert.equal(undo.quote, "");
});

test("settings expose unavailable small models without repeating one description per bot", async () => {
  const f = await fixture();
  try {
    f.state.memory_auto.available = false;
    const settings = new Element("div");
    f.registration.settingsSection(settings);
    assert.match(byClass(settings, "memory-auto-availability").textContent, /aucun petit modèle/);
    assert.equal(walk(settings).filter((el) => el.tagName === "small").length, 0);
  } finally { f.cleanup(); }
});
