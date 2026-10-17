import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// The skills panel uses the shell's DOM, not React. Exercise its actual list and
// editor rendering with the small DOM surface it needs; no browser dependency.
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.dataset = {}; this.listeners = {}; this.value = ""; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; this.text = ""; }
  set textContent(text) { this.text = text; this.children = []; }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute(name, value) { this[name] = value; }
  addEventListener(name, action) { this.listeners[name] = action; }
  cloneNode() { return new Element(this.tagName); }
  showModal() { this.open = true; }
  close() { this.open = false; this.listeners.close?.(); }
  focus() {}
  setSelectionRange() {}
}
const descendants = (element) => [element, ...element.children.flatMap(descendants)];
const byClass = (element, name) => descendants(element).find((child) => child.className === name);

function panel() {
  const root = new Element("div");
  const skill = { skill_id: "skill_internal_123", version: 1, name: "resume", display_name: "Résumé express", description: "En trois puces." };
  const content = { ...skill, body: "Texte original.", editable_body: "Texte original." };
  let registration;
  const requests = [];
  const panels = {
    state: { skills: { by_bot: { bot: [skill] } } },
    register(value) { registration = value; },
    async api(path) { requests.push(path); return { versions: [{ ...skill, content }] }; },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/skills.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels }, TextEncoder,
    document: { createElement: (tag) => new Element(tag), getElementById: () => root },
  });
  registration.botSection({ id: "bot" }, root);
  return { root, skill, panels, requests };
}

test("skill list keeps the human title and version but not the internal id", () => {
  const { root, skill } = panel();
  const row = byClass(root, "skills-row");
  assert.equal(row.children[0].textContent, "Résumé expressEn trois puces.Version 1");
  assert.ok(!row.textContent.includes(skill.skill_id));
  // Keep the durable id for edit routing, not for display.
  assert.equal(row.children[1].dataset.skillId, skill.skill_id);
});

test("skill editor hides the id while still loading its exact version history", async () => {
  const { root, skill, requests } = panel();
  byClass(root, "skills-row").children[1].listeners.click();
  await new Promise((resolve) => setImmediate(resolve));
  const dialog = byClass(root, "dialog skills-dialog");
  assert.equal(dialog.open, true);
  assert.ok(requests[0].includes(`skill_id=${skill.skill_id}`));
  const fields = byClass(dialog, "skills-fields");
  assert.equal(fields.children.find((child) => child.tagName === "p").textContent, "Version 1");
  assert.ok(!dialog.textContent.includes(skill.skill_id));
  assert.equal(fields.children.find((child) => child.tagName === "input").value, "Résumé express");
  assert.equal(byClass(dialog, "skills-history").children[1].children[0].textContent, "Version 1");
});
