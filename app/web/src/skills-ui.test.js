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

function panel(fileCount = 1, withDemonstration = false) {
  const root = new Element("div");
  const skill = { skill_id: "skill_internal_123", version: 1, name: "resume", display_name: "Résumé express", description: "En trois puces.", file_count: fileCount };
  const content = { ...skill, body: "Texte original.", editable_body: "Texte original.", files: [{ path: "exemple.md", content: "Texte annexe <script>non exécuté</script>" }] };
  let registration;
  const requests = [];
  const panels = {
    state: { skills: { by_bot: { bot: [skill] } }, demonstrations: { links: withDemonstration ? [{ skill_id: skill.skill_id, id: "demo" }] : [] } },
    register(value) { registration = value; },
    async api(path) { requests.push(path); return path === "/api/skills/import" ? { report: [{ status: { status: "written", skill_id: skill.skill_id } }] } : { versions: [{ ...skill, content }] }; },
    async refresh() {}, notify() {},
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/skills.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels }, TextEncoder, btoa,
    document: { createElement: (tag) => new Element(tag), getElementById: () => root },
  });
  registration.botSection({ id: "bot" }, root);
  return { root, skill, panels, requests, registration };
}

test("skill list keeps the human title and version but not the internal id", () => {
  const { root, skill } = panel();
  const row = byClass(root, "skills-row");
  assert.equal(row.children[0].textContent, "Résumé expressEn trois puces.Version 1 · 1 annexe");
  assert.ok(!row.textContent.includes(skill.skill_id));
  // Keep the durable id for edit routing, not for display.
  assert.equal(byClass(row, "skills-row-actions").children[0].dataset.skillId, skill.skill_id);
  assert.equal(byClass(root, "skills-history-content").textContent, "");
});

test("skill actions share an aligned group and source links have distinct accessible names", () => {
  const { root } = panel(1, true);
  const actions = byClass(root, "skills-row-actions");
  assert.equal(actions.children.length, 2);
  assert.equal(actions.children[1]["aria-label"], "Voir la démonstration : Résumé express");
  assert.equal(byClass(root, "skills-row").children[0].children.some(child => child.tagName === "button"), false);
});

test("Show the bot stays locked with other skill actions while an import is pending", async () => {
  const { root } = panel();
  const demonstrate = descendants(root).find(el => el.tagName === "button" && el.textContent === "Montrer au bot");
  const input = descendants(root).find(el => el.name === "skills-files");
  let release;
  input.files = [{ name: "note.md", size: 5, arrayBuffer: () => new Promise(resolve => { release = resolve; }) }];
  input.listeners.change();
  assert.equal(demonstrate.disabled, true);
  release(new TextEncoder().encode("Texte").buffer);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(demonstrate.disabled, false);
});

test("starting a demonstration gives persistent feedback inside the retained bot editor", () => {
  const { root, panels, registration } = panel();
  const demonstrate = descendants(root).find(el => el.tagName === "button" && el.textContent === "Montrer au bot");
  panels.state.demonstrations.session = { id: "demo", bot_id: "bot", status: "recording" };
  registration.onState();
  assert.equal(demonstrate.textContent, "Démonstration en cours…");
  assert.equal(demonstrate.title, "Ouvrir les contrôles de la démonstration");
  assert.equal(demonstrate.disabled, false);
});

test("skill list shows a bounded companion-file count", () => {
  const { root } = panel(2);
  assert.ok(byClass(root, "skills-row").textContent.includes("Version 1 · 2 annexes"));
});

test("skill editor hides the id while still loading its exact version history", async () => {
  const { root, skill, requests } = panel();
  byClass(root, "skills-row-actions").children[0].listeners.click();
  await new Promise((resolve) => setImmediate(resolve));
  const dialog = byClass(root, "dialog skills-dialog");
  assert.equal(dialog.open, true);
  assert.ok(requests[0].includes(`skill_id=${skill.skill_id}`));
  const fields = byClass(dialog, "skills-fields");
  assert.equal(fields.children.find((child) => child.tagName === "p").textContent, "Version 1");
  assert.ok(!dialog.textContent.includes(skill.skill_id));
  assert.equal(fields.children.find((child) => child.tagName === "input").value, "Résumé express");
  assert.equal(byClass(dialog, "skills-history").children[1].children[0].textContent, "Version 1");
  const files = byClass(dialog, "skills-companion-files");
  assert.equal(files.hidden, false);
  assert.ok(files.textContent.includes("exemple.md"));
  assert.ok(files.textContent.includes("<script>non exécuté</script>"));
  assert.equal(requests.length, 1); // Bodies/files are loaded only on opening.
  const history = byClass(dialog, "skills-history");
  history.children[1].value = "1";
  history.children[1].listeners.change();
  const oldFiles = byClass(history, "skills-companion-files");
  assert.equal(oldFiles.children[1].tagName, "details");
  assert.equal(oldFiles.children[1].children[0].textContent, "exemple.md");
  assert.equal(oldFiles.children[1].children[1].textContent, "Texte annexe <script>non exécuté</script>");
  assert.ok(!byClass(history.children[2], "skills-version-preview").textContent.includes("exemple.md"));
});
