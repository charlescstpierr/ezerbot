// Run with: node --test app/ui/panels/updates.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class Element {
  constructor(tag, document) {
    this.ownerDocument = document;
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.dataset = {};
    this.attributes = {};
    this.listeners = {};
    this.hidden = false;
    this.disabled = false;
    this.className = "";
  }
  append(...children) { this.children.push(...children); }
  insertBefore(node, before) {
    const index = this.children.indexOf(before);
    if (index < 0) this.children.push(node);
    else this.children.splice(index, 0, node);
  }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  addEventListener(name, action) { this.listeners[name] = action; }
  querySelector(selector) {
    const all = [this, ...this.querySelectorAll("*")];
    if (selector === ".icon") return all.find((el) => el.className.split(" ").includes("icon")) || null;
    return null;
  }
  querySelectorAll() {
    return this.children.flatMap((child) => child instanceof Element ? [child, ...child.querySelectorAll("*")] : []);
  }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map((child) => child?.textContent ?? String(child)).join(""); }
}

function harness(api) {
  const button = new Element("button");
  button.className = "account-button";
  const icon = new Element("svg");
  icon.className = "icon";
  button.append(icon);
  const document = {
    createElement: (tag) => new Element(tag, document),
    getElementById: (id) => id === "settings-button" ? button : null,
  };
  const panels = { api, register(panel) { this.panel = panel; } };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document });
  vm.runInContext(fs.readFileSync(`${__dirname}/updates.js`, "utf8"), context);
  const section = document.createElement("section");
  panels.panel.settingsSection(section);
  return { panels, section, button };
}

const installed = {
  version: "0.1.0",
  installed_commit: "a".repeat(40),
  installed_short: "aaaaaaa",
  phase: "idle",
  available: false,
  remote_commit: null,
  remote_short: null,
  message: "",
  log: "",
  running: false,
};

test("la section affiche la version et ne propose la mise à jour que si main est plus récent", async () => {
  const calls = [];
  const ui = harness(async (path) => {
    calls.push(path);
    return { ...installed, phase: "recherche", message: "Recherche des mises à jour…", running: true };
  });
  ui.panels.panel.onState({ updates: installed });
  assert.match(ui.section.textContent, /Mises à jour/);
  assert.match(ui.section.textContent, /Version installée 0\.1\.0 · aaaaaaa/);
  assert.match(ui.section.textContent, /Rechercher les mises à jour/);
  assert.match(ui.section.textContent, /conversations/);
  const update = ui.section.querySelectorAll().find((el) => el.textContent === "Mettre à jour et redémarrer");
  assert.equal(update.hidden, true);
  assert.equal(ui.button.querySelectorAll().some((el) => el.className.includes("update-badge") && el.hidden), true);

  ui.panels.panel.onState({
    updates: {
      ...installed,
      available: true,
      phase: "disponible",
      remote_short: "bbbbbbb",
      message: "Une mise à jour est disponible (bbbbbbb).",
    },
  });
  assert.equal(update.hidden, false);
  const badge = ui.button.querySelectorAll().find((el) => el.className.includes("update-badge"));
  assert.equal(badge.hidden, false);
  assert.equal(badge.attributes["aria-label"], "Mise à jour disponible");

  await ui.section.querySelectorAll().find((el) => el.textContent === "Rechercher les mises à jour").listeners.click();
  assert.deepEqual(calls, ["/api/updates/check"]);
  assert.match(ui.section.textContent, /Recherche des mises à jour/);

  ui.panels.panel.onState({
    updates: { ...installed, running: true, phase: "compilation", message: "Compilation…" },
  });
  assert.match(ui.section.textContent, /Compilation…/);
  assert.equal(ui.panels.panel.wantsPolling(), true);

  ui.panels.panel.onState({
    updates: {
      ...installed,
      phase: "erreur",
      message: "La compilation a échoué. L’application actuelle est restée en place.",
      log: "error[E0425]: rustc\n",
    },
  });
  assert.match(ui.section.textContent, /restée en place/);
  assert.match(ui.section.textContent, /E0425/);
  assert.equal(ui.panels.panel.wantsPolling(), false);
});
