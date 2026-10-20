import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { memoryPresentation, collapseMemoryReceipts } from "./memory-presentation.js";

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
  let current = initial, registration, refreshes = 0, refreshFailure = false, memoryData = {};
  const requests = [], root = new Element("div");
  const state = { bots: [{ id: "bot", name: "Ezerbot" }], provider: { connected: true, models_ready: true }, memory_auto: { available: true, revision: 1 }, conversations: [{ messages: [{ kind: "memory_auto", payload: { ...initial, active: true } }] }] };
  const panels = {
    state, register(value) { registration = value; }, notify() {},
    async api(path, body) {
      requests.push(path);
      if (path === "/api/memory") return { facts: current ? [current] : [], ...memoryData };
      if (path === "/api/memory/forget") { current = null; return { forgotten: [body.fact_id] }; }
      if (path === "/api/memory/correct") { current = { ...current, fact_id: "fact-two", text: body.text }; return { fact: current }; }
      if (path === "/api/memory/promote") { current.scope = "team"; return { created: [current.fact_id], already_in_team: [], kept_private_count: 0, shared_count: 1 }; }
      if (path === "/api/memory/automatic/undo") throw new Error("Ce fait a changé.");
      throw new Error(path);
    },
    async refresh() {
      refreshes++;
      if (refreshFailure) throw new Error("Lecture de l’état indisponible.");
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
  refreshes = 0; // Initial revision reconciliation is not an action refresh.
  return { root, state, panels, registration, requests, cleanup, failRefresh: () => { refreshFailure = true; }, setCurrent: (fact) => { current = fact; }, setMemoryData: (data) => { memoryData = data; }, refreshes: () => refreshes, card: () => state.conversations[0].messages[0] };
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

test("panel searches accent-insensitively, filters scope/source and warns at saturation", async () => {
  const f = await fixture();
  try {
    f.setCurrent({ fact_id: "fact-boreal", bot_id: "bot", text: "Projet Boréal", scope: "private", learned_at: 1, provenance: { source: "automatic" } });
    f.setMemoryData({ automatic: { saturated: true, catalogue_chars: 6200 }, sharing_classes: { "fact-boreal": "work" }, profile: { version: 12, sections: [{ title: "Projets et travail", count: 1, facts: [{ text: "Projet Boréal" }] }], log: [{ version: 12, action: "learned", changed_at: 1 }] } });
    button(f.root, "Actualiser").listeners.click(); await flush();
    assert.match(byClass(f.root, "memory-saturation").textContent, /6 000/);
    assert.equal(byClass(f.root, "memory-saturation").hidden, false);
    assert.match(byClass(f.root, "memory-profile").textContent, /v12/);
    const search = walk(f.root).find((el) => el.type === "search");
    search.value = "boreal"; search.listeners.input();
    assert.match(byClass(f.root, "memory-list").textContent, /Projet Boréal/);
    search.value = "introuvable"; search.listeners.input();
    assert.match(byClass(f.root, "memory-list").textContent, /Aucun résultat/);
    search.value = ""; search.listeners.input();
    const [scope, source] = walk(f.root).filter((el) => el.tagName === "select");
    scope.value = "team"; scope.listeners.change(); assert.match(byClass(f.root, "memory-list").textContent, /Aucun résultat/);
    scope.value = ""; source.value = "user_stated"; source.listeners.change(); assert.match(byClass(f.root, "memory-list").textContent, /Aucun résultat/);
    source.value = "automatic"; source.listeners.change(); assert.match(byClass(f.root, "memory-list").textContent, /Projet Boréal/);
    const all = byClass(f.root, "memory-select-all").children[0]; all.checked = true; all.listeners.change();
    assert.ok(button(f.root, "Partager la sélection (1)"));
  } finally { f.cleanup(); }
});

test("personal facts are excluded by default and consent-card confirmation carries server counts", async () => {
  const f = await fixture();
  try {
    f.setMemoryData({ sharing_classes: { "fact-one": "personal" } });
    button(f.root, "Actualiser").listeners.click(); await flush();
    assert.equal(byClass(f.root, "memory-select-all").children[0].disabled, true);
    assert.equal(byClass(f.root, "memory-fact-choice").children[0].checked, false);
    const offered = { kind: "memory_share", payload: { bot_id: "bot", facts: [{ fact_id: "fact-one" }] } };
    let report;
    assert.equal(await f.registration.onCardAction(offered, "share", { fact_ids: ["fact-one"], onConfirmed: (value) => { report = value; } }), true);
    assert.equal(report.shared_count, 1);
    await assert.rejects(f.registration.onCardAction(offered, "share", { fact_ids: ["forged"] }), /Sélection invalide/);
  } finally { f.cleanup(); }
});

test("memory receipts distinguish corrections, restore undo and the owning group bot", () => {
  const updated = memoryPresentation({ active: true, replaced: true, text: "Je code en Rust." }, "Éclaireur", true);
  assert.equal(updated.label, "Mis à jour par Éclaireur :");
  assert.equal(updated.quote, "« Je code en Rust. »");
  assert.equal(memoryPresentation({ active: true }).label, "Mémorisé :");
  assert.equal(memoryPresentation({ active: true }, "Ezerbot", true).label, "Mémorisé par Ezerbot :");
  const undo = memoryPresentation({ active: false, cancelled: true, replaced: true, restored_active: true, text: "Texte ancien" });
  assert.match(undo.receipt, /l’ancien fait est rétabli/);
  assert.equal(undo.quote, "");
  assert.equal(memoryPresentation({ active: false, cancelled: true, replaced: true, restored_active: false }).receipt, "Mise à jour annulée.");
});

test("inactive group receipts retain bot attribution, including cancelled updates", () => {
  assert.equal(memoryPresentation({ cancelled: true }, "Éclaireur", true).receipt, "Mémorisation annulée (Éclaireur).");
  assert.equal(memoryPresentation({ cancelled: true, replaced: true, restored_active: true }, "Rédacteur", true).receipt, "Mise à jour annulée — l’ancien fait est rétabli (Rédacteur).");
  assert.equal(memoryPresentation({ updated: true }, "Ezerbot", true).receipt, "Ce fait a été mis à jour (Ezerbot).");
  const shared = memoryPresentation({ scope_changed: true, active: false, text: "Ancien texte privé" });
  assert.equal(shared.receipt, "Ce fait a changé de portée. Consultez la mémoire du bot.");
  assert.equal(shared.quote, "");
});

test("a revision delivered with a stale conversation frame triggers one follow-up state read", async () => {
  const f = await fixture();
  try {
    f.setCurrent({ ...f.card().payload, text: "Fait reçu à la fin du tour." });
    f.state.memory_auto.revision++;
    f.registration.onState(f.state);
    await flush();
    assert.equal(f.refreshes(), 1);
    assert.equal(f.card().payload.text, "Fait reçu à la fin du tour.");
    await flush(); assert.equal(f.refreshes(), 1); // No idle polling loop.
  } finally { f.cleanup(); }
});

test("confirmed mutations remain confirmed when only the state refresh fails", async () => {
  const f = await fixture();
  try {
    f.failRefresh();
    button(f.root, "Oublier…").listeners.click(); button(f.root, "Confirmer l’oubli").listeners.click();
    await flush();
    assert.equal(f.root.textContent.includes("Action non confirmée"), false);
    assert.ok(f.root.textContent.includes("Fait oublié."));
    await assert.rejects(f.registration.onCardAction(f.card(), "undo"), /Ce fait a changé/);
  } finally { f.cleanup(); }
  const share = await fixture();
  try {
    share.failRefresh(); const check = byClass(share.root, "memory-fact-choice").children[0]; check.checked = true; check.listeners.change();
    button(share.root, "Partager la sélection (1)").listeners.click(); await flush();
    assert.ok(share.root.textContent.includes("Partage terminé."));
    assert.ok(!share.root.textContent.includes("Partage interrompu"));
  } finally { share.cleanup(); }
});

test("group turns keep three receipts visible and retain individual actions inside one disclosure", () => {
  const receipts = Array.from({ length: 6 }, (_, index) => ({ id: `memory-${index}`, kind: "memory_auto", payload: { memory_root_id: "human-root", bot_id: `bot-${index % 3}` } }));
  const next = { id: "next", role: "user", body: "Autre tour" };
  const otherRoot = { id: "other", kind: "memory_auto", payload: { memory_root_id: "other-root" } };
  const input = [...receipts, next, otherRoot], result = collapseMemoryReceipts(input, true);
  assert.equal(result.filter((m) => m.kind === "memory_auto").length, 4);
  const disclosure = result.find((m) => m.kind === "memory_overflow");
  assert.deepEqual(disclosure.memories, receipts.slice(3));
  assert.deepEqual(disclosure.groups.map((group) => group.bot_id), ["bot-0", "bot-1", "bot-2"]);
  assert.equal(disclosure.groups[1].memories[0], receipts[4]);
  // Nonadjacent receipts of one bot still share one heading and original IDs.
  const interleaved = collapseMemoryReceipts([...receipts, { ...receipts[3], id: "last" }], true).find((m) => m.kind === "memory_overflow");
  assert.deepEqual(interleaved.groups[0].memories.map((m) => m.id), ["memory-3", "last"]);
  assert.equal(result.at(-2), next); assert.equal(result.at(-1), otherRoot);
  assert.equal(collapseMemoryReceipts(input, false), input);
});

test("settings expose unavailable small models without repeating one description per bot", async () => {
  const f = await fixture();
  try {
    f.state.memory_auto.available = false;
    const settings = new Element("div");
    f.registration.settingsSection(settings);
    assert.match(byClass(settings, "memory-auto-availability").textContent, /aucun petit modèle/);
    assert.equal(walk(settings).filter((el) => el.tagName === "small").length, 0);
    const row = byClass(settings, "memory-auto-toggle");
    assert.match(row.className, /is-paused/); assert.match(row.textContent, /en pause/);
    assert.equal(row.querySelector("input").checked, true);
  } finally { f.cleanup(); }
});
