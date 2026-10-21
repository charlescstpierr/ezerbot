// Run with: node --test app/ui/panels/updates.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

class Element {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.dataset = {};
    this.attributes = {};
    this.listeners = {};
    this.hidden = false;
    this.disabled = false;
    this.className = "";
    this.parentElement = null;
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
  const bottom = new Element("div");
  bottom.className = "sidebar-bottom";
  const button = new Element("button");
  button.className = "account-button";
  const icon = new Element("svg");
  icon.className = "icon";
  button.append(icon);
  bottom.append(button);
  const document = {
    createElement: (tag) => new Element(tag),
    getElementById: (id) => id === "settings-button" ? button : null,
  };
  const panels = { api, register(panel) { this.panel = panel; } };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document });
  vm.runInContext(fs.readFileSync(`${__dirname}/updates.js`, "utf8"), context);
  const section = document.createElement("section");
  panels.panel.settingsSection(section);
  return { panels, section, button, bottom };
}

const installed = {
  version: "0.1.0",
  installed_commit: "a".repeat(40),
  installed_short: "aaaaaaa",
  phase: "a_jour",
  available: false,
  remote_commit: null,
  remote_short: null,
  message: "Ezerbot est à jour.",
  log: "",
  running: false,
};

function cornerOf(ui) {
  return ui.bottom.querySelectorAll().find((el) => el.className.split(" ").includes("update-corner"));
}

function labelOf(corner) {
  return corner.querySelectorAll().find((el) => el.className.includes("update-corner-label")).textContent;
}

test("le bouton de barre reste caché tant qu’aucune mise à jour n’est disponible", async () => {
  const calls = [];
  const ui = harness(async (path) => {
    calls.push(path);
    if (path.endsWith("/apply")) {
      return { ...installed, available: true, running: true, phase: "telechargement", message: "Téléchargement…" };
    }
    return { ...installed, phase: "recherche", message: "Recherche des mises à jour…", running: true };
  });
  ui.panels.panel.onState({ updates: installed });
  const corner = cornerOf(ui);
  assert.equal(corner.hidden, true);
  assert.equal(ui.button.querySelectorAll().some((el) => el.className.includes("update-badge")), false);
  assert.match(ui.section.textContent, /Mises à jour/);
  assert.match(ui.section.textContent, /Version installée 0\.1\.0 · aaaaaaa/);
  const settingsUpdate = ui.section.querySelectorAll().find((el) => el.textContent === "Mettre à jour et redémarrer");
  assert.equal(settingsUpdate.hidden, true);
  assert.equal(settingsUpdate.className.includes("primary-button"), false);
  assert.equal(settingsUpdate.className.includes("secondary-button"), true);

  ui.panels.panel.onState({
    updates: {
      ...installed,
      available: true,
      phase: "disponible",
      remote_short: "bbbbbbb",
      message: "Une mise à jour est disponible (bbbbbbb).",
    },
  });
  assert.equal(corner.hidden, false);
  assert.equal(labelOf(corner), "Mettre à jour");
  assert.equal(corner.attributes["aria-label"], "Mettre à jour");
  assert.equal(corner.className.includes("is-running"), false);
  assert.equal(settingsUpdate.hidden, false);
  assert.equal(ui.bottom.children[0], corner);
  assert.equal(ui.bottom.children[1], ui.button);

  await corner.listeners.click();
  assert.deepEqual(calls, ["/api/updates/apply"]);
  assert.equal(corner.hidden, false);
  assert.equal(labelOf(corner), "Téléchargement…");
  assert.equal(corner.className.includes("is-running"), true);
  assert.equal(corner.disabled, true);
  assert.equal(ui.panels.panel.wantsPolling(), true);

  ui.panels.panel.onState({
    updates: { ...installed, available: true, running: true, phase: "compilation", message: "Compilation…" },
  });
  assert.equal(labelOf(corner), "Compilation…");
  assert.match(ui.section.textContent, /Compilation…/);
  assert.equal(ui.section.textContent.includes("Mettre à jour et redémarrer") ? settingsUpdate.hidden : true, true);

  ui.panels.panel.onState({
    updates: {
      ...installed,
      available: true,
      phase: "erreur",
      message: "La compilation a échoué. L’application actuelle est restée en place.",
      log: "error[E0425]: rustc\n",
    },
  });
  assert.equal(corner.hidden, false);
  assert.equal(labelOf(corner), "Mettre à jour");
  assert.equal(corner.className.includes("is-running"), false);
  assert.match(ui.section.textContent, /restée en place/);
  assert.match(ui.section.textContent, /E0425/);
  assert.equal(ui.panels.panel.wantsPolling(), false);

  ui.panels.panel.onState({ updates: installed });
  assert.equal(corner.hidden, true);
  assert.equal(ui.panels.panel.wantsPolling(), false);
});
