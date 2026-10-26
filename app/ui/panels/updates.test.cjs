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
    this.id = "";
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
  before(node) { this.parentElement?.insertBefore(node, this); }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  addEventListener(name, action) { this.listeners[name] = action; }
  querySelector(selector) {
    const all = [this, ...this.querySelectorAll("*")];
    if (selector === ".dialog-footer") return all.find((el) => el.className.split(" ").includes("dialog-footer")) || null;
    if (selector === ".icon") return all.find((el) => el.className.split(" ").includes("icon")) || null;
    return null;
  }
  querySelectorAll() {
    return this.children.flatMap((child) => child instanceof Element ? [child, ...child.querySelectorAll("*")] : []);
  }
  set textContent(text) { this.children = [String(text)]; }
  get textContent() { return this.children.map((child) => child?.textContent ?? String(child)).join(""); }
}

function harness(api, { lateHost = false } = {}) {
  const bottom = new Element("div");
  bottom.className = "rail-bottom";
  const host = new Element("div");
  let hostMounted = !lateHost;
  const button = new Element("button");
  button.className = "account-button";
  const icon = new Element("svg");
  icon.className = "icon";
  button.append(icon);
  bottom.append(button);
  const dialog = new Element("dialog");
  dialog.id = "settings-dialog";
  const footer = new Element("div");
  footer.className = "dialog-footer";
  dialog.append(footer);
  const document = {
    createElement: (tag) => new Element(tag),
    getElementById: (id) => {
      if (id === "settings-button") return button;
      if (id === "settings-dialog") return dialog;
      if (id === "sidebar-updates" && hostMounted) return host;
      return null;
    },
  };
  const panels = { api, register(panel) { this.panel = panel; } };
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document });
  vm.runInContext(fs.readFileSync(`${__dirname}/updates.js`, "utf8"), context);
  const section = document.createElement("section");
  panels.panel.settingsSection(section);
  return { panels, section, button, bottom, host, dialog, footer, mountHost() { hostMounted = true; } };
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
  last_attempt: "aucune",
  last_error: "",
};

const failure = "La compilation a échoué. L’application actuelle est restée en place.";

function byClass(root, name) {
  return root.querySelectorAll().find((el) => el.className.split(" ").includes(name));
}

function cornerOf(ui) { return byClass(ui.host, "update-corner"); }
function actionOf(corner) { return byClass(corner, "update-corner-action"); }
function labelOf(corner) { return byClass(corner, "update-corner-label").textContent; }
function errorOf(corner) { return byClass(corner, "update-corner-error"); }
function errorText(corner) { return byClass(corner, "update-corner-error-text").textContent; }

test("une compilation ratée s’affiche en rouge dans le coin et reste dans les Réglages", async () => {
  const calls = [];
  const ui = harness(async (path) => {
    calls.push(path);
    if (path.endsWith("/apply")) {
      return { ...installed, available: true, running: true, phase: "telechargement", message: "Téléchargement…", last_attempt: "en_cours", last_error: "" };
    }
    return installed;
  });
  assert.equal(ui.dialog.children[0], ui.section);
  assert.equal(ui.dialog.children[1], ui.footer);
  assert.equal(ui.section.id, "settings-updates");
  ui.panels.panel.onState({ updates: installed });
  const corner = cornerOf(ui);
  const action = actionOf(corner);
  assert.equal(corner.hidden, true);
  assert.equal(ui.button.querySelectorAll().some((el) => el.className.includes("update-badge")), false);
  assert.match(ui.section.textContent, /Mises à jour/);
  assert.match(ui.section.textContent, /Version actuelle 0\.1\.0 · aaaaaaa/);
  assert.match(ui.section.textContent, /Dernière tentative : aucune/);
  assert.equal(ui.section.textContent.includes("Rechercher les mises à jour"), false);
  assert.equal(ui.section.textContent.includes("Mettre à jour et redémarrer"), false);
  const css = fs.readFileSync(`${__dirname}/updates.css`, "utf8");
  assert.match(css, /\.update-corner-error \{[^}]*font-size: 11\.5px/);
  assert.match(css, /\.update-corner-error \{[^}]*color: var\(--danger-text\)/);

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
  assert.equal(action.hidden, false);
  assert.equal(errorOf(corner).hidden, true);
  assert.equal(labelOf(corner), "Mettre à jour");
  assert.equal(action.attributes["aria-label"], "Mettre à jour");
  assert.equal(corner.className.includes("is-running"), false);
  assert.deepEqual(ui.bottom.children, [ui.button]);
  assert.deepEqual(ui.host.children, [corner]);

  await action.listeners.click();
  assert.deepEqual(calls, ["/api/updates/hold", "/api/updates/apply"]);
  assert.equal(corner.hidden, false);
  assert.equal(labelOf(corner), "Téléchargement…");
  assert.equal(corner.className.includes("is-running"), true);
  assert.equal(action.disabled, true);
  assert.equal(ui.panels.panel.wantsPolling(), true);
  assert.match(ui.section.textContent, /Dernière tentative : en cours/);

  ui.panels.panel.onState({
    updates: { ...installed, available: true, running: true, phase: "compilation", message: "Compilation…", last_attempt: "en_cours" },
  });
  assert.equal(labelOf(corner), "Compilation…");
  assert.match(ui.section.textContent, /Dernière tentative : en cours/);

  ui.panels.panel.onState({
    updates: {
      ...installed,
      available: true,
      phase: "erreur",
      message: failure,
      last_attempt: "echec",
      last_error: failure,
      log: "error[E0425]: rustc\n",
    },
  });
  assert.equal(corner.hidden, false);
  assert.equal(action.hidden, true);
  assert.equal(errorOf(corner).hidden, false);
  assert.equal(errorText(corner), failure);
  assert.equal(action.attributes.title || "", "");
  assert.equal(byClass(corner, "update-corner-dismiss").attributes["aria-label"], "Fermer");
  assert.match(ui.section.textContent, /Dernière tentative : échec/);
  assert.match(ui.section.textContent, /restée en place/);
  assert.equal(ui.section.textContent.includes("E0425"), false);
  assert.equal(ui.panels.panel.wantsPolling(), false);

  byClass(corner, "update-corner-dismiss").listeners.click();
  assert.equal(errorOf(corner).hidden, true);
  assert.equal(action.hidden, false);
  assert.equal(labelOf(corner), "Mettre à jour");
  assert.match(ui.section.textContent, /Dernière tentative : échec/);
  assert.match(ui.section.textContent, /restée en place/);

  ui.panels.panel.onState({ updates: installed });
  assert.equal(corner.hidden, true);
  assert.equal(ui.panels.panel.wantsPolling(), false);
});

test("le redémarrage attend discrètement une réunion ou un enregistrement", async () => {
  const calls = [];
  const ui = harness(async (path, body) => {
    calls.push({ path, body });
    return {
      ...installed,
      available: true,
      running: true,
      phase: "attente",
      message: "Le redémarrage suivra la fin de l’enregistrement.",
      relaunch: "attente",
      last_attempt: "attente",
      last_error: "",
    };
  });
  ui.panels.meetingActive = () => true;
  ui.panels.panel.onState({
    updates: {
      ...installed,
      available: true,
      running: true,
      phase: "redemarrage",
      message: "Redémarrage…",
      last_attempt: "reussie",
    },
  });
  const corner = cornerOf(ui);
  const action = actionOf(corner);
  assert.equal(corner.hidden, false);
  assert.equal(labelOf(corner), "Après l’enregistrement");
  assert.equal(action.attributes["aria-label"], "Après l’enregistrement");
  assert.equal(action.disabled, true);
  assert.equal(corner.className.includes("is-running"), false);
  assert.equal(errorOf(corner).hidden, true);
  assert.equal(calls.some((call) => call.path === "/api/updates/hold" && call.body?.active === true), true);
  await action.listeners.click();
  assert.equal(calls.some((call) => call.path === "/api/updates/apply"), false);

  ui.panels.meetingActive = () => false;
  ui.panels.panel.onState({
    updates: {
      ...installed,
      available: true,
      running: true,
      phase: "attente",
      message: "Le redémarrage suivra la fin de l’enregistrement.",
      last_attempt: "attente",
    },
  });
  assert.equal(labelOf(corner), "Après l’enregistrement");
  assert.equal(action.attributes.title, "Le redémarrage suivra la fin de l’enregistrement.");
  assert.match(ui.section.textContent, /Dernière tentative : après l’enregistrement/);
  assert.equal(ui.panels.panel.wantsPolling(), true);
});

test("le premier état de capture est toujours envoyé, même sans capture", async () => {
  const calls = [];
  const ui = harness(async (path, body) => {
    calls.push({ path, body });
    return { ...installed, available: false, running: false, phase: "idle", message: "" };
  });
  ui.panels.panel.onState({ updates: { ...installed, available: false, running: false, phase: "idle" } });
  ui.panels.panel.onState({ updates: { ...installed, available: false, running: false, phase: "idle" } });
  const holds = calls.filter((call) => call.path === "/api/updates/hold");
  assert.equal(holds.length, 1);
  assert.equal(holds[0].body.active, false);
});

test("un hôte tardif reçoit une seule action sans déplacer Réglages", () => {
  const ui = harness(async () => installed, { lateHost: true });
  ui.panels.panel.onState({ updates: { ...installed, available: true, phase: "disponible" } });
  assert.equal(cornerOf(ui), undefined);
  ui.mountHost();
  for (const phase of ["disponible", "telechargement", "erreur", "a_jour"]) {
    ui.panels.panel.onState({ updates: { ...installed, available: phase !== "a_jour", phase, running: phase === "telechargement", message: phase === "erreur" ? failure : "" } });
    assert.equal(ui.host.children.length, 1);
    assert.deepEqual(ui.bottom.children, [ui.button]);
  }
  assert.equal(cornerOf(ui).hidden, true);
});
