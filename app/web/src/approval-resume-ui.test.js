import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";

class Element {
  constructor(tag) {
    this.tagName = tag; this.children = []; this.listeners = {}; this.attributes = {};
    this.className = ""; this.hidden = false; this.dataset = {}; this.id = "";
  }
  append(...children) { this.children.push(...children); }
  before(node) { this.previous = node; }
  set textContent(text) { this.text = text; this.children = []; }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener(name, callback) { this.listeners[name] = callback; }
}

const NOW = Date.now();
const approval = (id, left = 3_600_000, status = "pending") => ({
  entry_id: id, status: { status }, kind: { kind: "approval", tool: "NE_JAMAIS_AFFICHER", expires_at: NOW + left },
});

// forms.js runs as the real panel script; only the page around it is a stand-in.
function app() {
  const scroll = new Element("section");
  let registration;
  runInNewContext(readFileSync(new URL("../../ui/panels/forms.js", import.meta.url), "utf8"), {
    window: {
      GrokbotPanels: { state: { bots: [] }, register(value) { registration = value; }, notify() {}, async api() {}, async refresh() {} },
      clearTimeout() {}, setTimeout() {}, dispatchEvent() {},
    },
    document: {
      createElement: (tag) => new Element(tag),
      createElementNS: (_, tag) => new Element(tag),
      getElementById: (id) => (id === "chat-scroll" ? scroll : undefined),
    },
    Event: class {}, Date,
  });
  return { scroll, state: (requests) => registration.onState({ forms: { requests }, conversations: [] }) };
}

test("at the first reading, a short banner says how many requests wait, above the conversation", () => {
  const f = app();
  f.state([approval("a"), approval("b"), approval("done", 3_600_000, "resolved")]);
  const banner = f.scroll.previous;
  assert.equal(banner.id, "approval-banner");
  assert.equal(banner.attributes.role, "status");
  assert.equal(banner.children[0].textContent, "2 demandes attendent votre décision");
  assert.equal(banner.hidden, false);
  assert.doesNotMatch(banner.textContent, /NE_JAMAIS/);
  // Tâches (#211) does not exist yet: no link to it.
  assert.equal(banner.children.filter((child) => child.tagName === "a").length, 0);
  assert.doesNotMatch(banner.textContent, /Voir dans Tâches/);
});

test("a single waiting request is said in the singular, and the count follows the decisions", () => {
  const f = app();
  f.state([approval("a"), approval("b")]);
  f.state([approval("a"), approval("b", 3_600_000, "denied")]);
  assert.equal(f.scroll.previous.children[0].textContent, "1 demande attend votre décision");
  f.state([approval("a", 3_600_000, "resolved")]);
  assert.equal(f.scroll.previous.hidden, true);
});

test("the cross closes it for good; an expired request or an empty first reading shows nothing", () => {
  const f = app();
  f.state([approval("a")]);
  const banner = f.scroll.previous;
  banner.children[1].listeners.click();
  assert.equal(banner.hidden, true);
  f.state([approval("a"), approval("b")]);
  assert.equal(banner.hidden, true);
  assert.equal(banner.children[1].attributes["aria-label"], "Fermer le rappel des demandes en attente");

  const none = app();
  none.state([approval("old", -1000), approval("done", 3_600_000, "expired")]);
  assert.equal(none.scroll.previous, undefined);
  // A request that arrives while the app is in use is not a « reprise »: its notification speaks.
  none.state([approval("fresh")]);
  assert.equal(none.scroll.previous, undefined);
});
