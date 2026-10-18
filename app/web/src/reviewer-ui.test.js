import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import { after, test } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

// Exercise the shipping settings panel, including DOM replacement on refresh.
class Element {
  constructor(tag) {
    this.tagName = tag; this.children = []; this.listeners = {}; this.attributes = {};
    this.value = ""; this.className = "";
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; this.text = ""; }
  set textContent(text) { this.text = text; this.children = []; }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener(name, callback) { this.listeners[name] = callback; }
  focus() { this.focused = true; }
}
const descendants = (element) => [element, ...element.children.flatMap(descendants)];
const find = (root, predicate) => descendants(root).find(predicate);
const byClass = (root, name) => find(root, (node) => node.className.split(" ").includes(name));
const field = (root, key) => find(root, (node) => node.name === `reviewer-${key}`);
const button = (root, text) => find(root, (node) => node.tagName === "button" && node.textContent === text);
const form = (root) => byClass(root, "reviewer-settings");
const submit = (root) => form(root).listeners.submit({ preventDefault() {} });
const input = (root) => form(root).listeners.input();

function settingsPanel() {
  const root = new Element("div");
  let registration;
  let remote = { config: { enabled: true, allow: ["Autorise la lecture."], block: [] }, version: "1" };
  const requests = [];
  const notifications = [];
  const panels = {
    state: { forms: { rules: [], reviewer: remote }, bots: [] },
    register(value) { registration = value; },
    notify(text) { notifications.push(text); },
    async api(path, body) {
      requests.push({ path, body });
      if (panels.failSave) throw new Error("never show remote exception text");
      remote = { config: body.config, version: String(Number(remote.version) + 1) };
      return remote;
    },
    async refresh() {
      if (panels.failRefresh) throw new Error("offline");
      panels.state = { ...panels.state, forms: { ...panels.state.forms, reviewer: remote } };
      registration.onState(panels.state);
      return panels.state;
    },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/forms.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels, clearTimeout() {}, setTimeout() {}, dispatchEvent() {} },
    document: { createElement: (tag) => new Element(tag) }, Event: class {},
  });
  registration.settingsSection(root);
  return { root, panels, registration, requests, notifications, setRemote(value) { remote = value; } };
}

test("save confirmation survives the real refresh/rebuild inside the modal, without a hidden toast", async () => {
  const f = settingsPanel();
  const previous = form(f.root);
  field(f.root, "allow").value = "Autorise la lecture dans ~/Documents/projets.";
  input(f.root);
  await submit(f.root);
  assert.notEqual(form(f.root), previous);
  assert.match(byClass(f.root, "reviewer-notice").textContent, /^Règles enregistrées\./);
  assert.equal(byClass(f.root, "reviewer-notice").attributes.role, "status");
  assert.equal(f.notifications.length, 0);
  f.panels.state.bots = [{ id: "a", name: "Nouveau nom" }];
  f.registration.onState(f.panels.state);
  assert.match(byClass(f.root, "reviewer-notice").textContent, /^Règles enregistrées\./);
  input(f.root);
  assert.equal(byClass(f.root, "reviewer-notice").textContent, "");
});

test("reload discards a draft and visibly confirms the saved rules were reloaded", async () => {
  const f = settingsPanel();
  field(f.root, "allow").value = "Brouillon"; input(f.root);
  f.setRemote({ config: { enabled: false, allow: ["Demande avant toute suppression."], block: [] }, version: "2" });
  await button(f.root, "Recharger les règles enregistrées").listeners.click();
  assert.equal(field(f.root, "allow").value, "Demande avant toute suppression.");
  assert.equal(field(f.root, "enabled").checked, false);
  assert.equal(byClass(f.root, "reviewer-notice").textContent, "Règles rechargées.");
});

test("limit errors identify the exact field, are associated alerts and clear on editing", async () => {
  const f = settingsPanel();
  field(f.root, "block").value = Array.from({ length: 21 }, () => "Refuse la suppression.").join("\n");
  input(f.root); await submit(f.root);
  const bad = field(f.root, "block");
  assert.equal(bad.attributes["aria-invalid"], "true");
  assert.equal(field(f.root, "allow").attributes["aria-invalid"], "false");
  const error = find(f.root, (node) => node.id === bad.attributes["aria-describedby"]);
  assert.equal(error.attributes.role, "alert");
  assert.match(error.textContent, /^Limite dépassée/);
  assert.equal(bad.focused, true);
  assert.equal(f.requests.length, 0);
  bad.value = "Refuse la suppression."; input(f.root);
  assert.equal(bad.attributes["aria-invalid"], "false");
  assert.equal(error.textContent, "");
});

test("save failures remain visible alerts, while a failed follow-up read preserves acknowledged success", async () => {
  const f = settingsPanel(); f.panels.failSave = true;
  await submit(f.root);
  assert.equal(byClass(f.root, "reviewer-notice").attributes.role, "alert");
  assert.ok(byClass(f.root, "reviewer-notice").className.includes("reviewer-error"));
  assert.match(byClass(f.root, "reviewer-notice").textContent, /^Enregistrement non confirmé/);
  assert.ok(!f.root.textContent.includes("remote exception"));
  f.panels.failSave = false; f.panels.failRefresh = true;
  input(f.root); await submit(f.root);
  assert.equal(byClass(f.root, "reviewer-notice").attributes.role, "status");
  assert.match(byClass(f.root, "reviewer-notice").textContent, /^Règles enregistrées\. L’actualisation/);
  assert.equal(button(f.root, "Enregistrer les règles").disabled, false);
});

test("standing rules precede the separated reviewer, whose switch and grouped actions follow app styling", () => {
  const f = settingsPanel();
  const empty = find(f.root, (node) => node.tagName === "p" && node.textContent.includes("Aucune règle « Toujours »"));
  assert.ok(f.root.children.indexOf(empty) < f.root.children.indexOf(form(f.root)));
  const toggle = byClass(f.root, "setting-toggle");
  assert.equal(toggle.children[1], field(f.root, "enabled"));
  assert.equal(field(f.root, "enabled").attributes.role, "switch");
  assert.equal(toggle.children[2].className, "toggle-track");
  const actions = byClass(f.root, "reviewer-actions");
  assert.equal(actions.children.length, 2);
  assert.equal(actions.children[0].className, "primary-button");
  assert.equal(actions.children[1].className, "secondary-button");
});

// Render the actual JSX through Vite's transform rather than inspecting source
// strings. Middleware mode starts no listening server, and no dependency is added.
const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)), configFile: false,
  plugins: [react()], server: { middlewareMode: true }, logLevel: "error",
  optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());
const { Card } = await vite.ssrLoadModule("/src/cards/approval.jsx");
function card(status, decision, args = { path: "~/Documents/projets/notes.txt" }, pending = false) {
  globalThis.window = { GrokbotForms: { view() {} }, GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(Card, { message: {
      id: "approval-test", kind: "approval", payload: {
        entry_id: "approval-test", status: { status }, route: { requesting_bot_id: "grokbot" },
        kind: { kind: "approval", tool: "read_file", expires_at: Date.now() + 60000 }, arguments: args,
        review: { pending, started_at: Date.now(), answer: { decision, reason: "Cette action correspond à votre règle." } },
      },
    } }));
  } finally { delete globalThis.window; }
}

test("automatic allow and deny both expose the redacted target plus an expandable exact action", () => {
  for (const [status, decision] of [["resolved", "allow"], ["denied", "deny"]]) {
    const html = card(status, decision, { path: "~/Documents/projets/notes.txt", credential: "[...]" });
    assert.ok(html.includes("~/Documents/projets/notes.txt"));
    assert.ok(html.includes("Voir l’action complète"));
    assert.ok(html.includes('aria-label="Arguments de l’action"'));
    assert.ok(html.includes("[...]"));
    assert.equal(html.includes("gb-approval-denied"), status === "denied");
    assert.equal(html.includes("gb-status-alert"), status === "denied");
  }
  assert.ok(card("resolved", "allow", null).includes("Aperçu de l’action indisponible."));
});

test("human fallback is attributed to the reviewer; pending review has decorative progress without disabling decisions", () => {
  const fallback = card("pending", "ask");
  assert.ok(fallback.includes("Relecteur : "));
  assert.ok(!fallback.includes("gb-reviewer-progress"));
  const progress = card("pending", "ask", { path: "~/Documents/projets/a.txt" }, true);
  assert.ok(progress.includes('class="gb-reviewer-progress" aria-hidden="true"'));
  assert.ok(progress.includes("Relecteur : "));
  assert.ok(progress.includes("Autoriser une fois"));
  assert.ok(!progress.includes("disabled="));
});
