import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";

class Element {
  constructor(tag) {
    this.tagName = tag; this.children = []; this.listeners = {}; this.attributes = {};
    this.value = ""; this.className = ""; this.disabled = false; this.checked = false;
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; this.text = ""; }
  set textContent(text) { this.text = text; this.children = []; }
  get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(""); }
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener(name, callback) { this.listeners[name] = callback; }
  focus() {}
}
const descendants = (element) => [element, ...element.children.flatMap(descendants)];
const named = (root, name) => descendants(root).find((node) => node.name === name);

function settingsPanel(waiting = { task: true, chat: false, validity: "24h", reminder: "1h" }) {
  const root = new Element("div");
  let registration;
  const requests = [];
  let remote = waiting;
  const panels = {
    state: { forms: { rules: [], waiting: remote }, bots: [] },
    register(value) { registration = value; },
    notify() {},
    async api(path, body) {
      requests.push({ path, body });
      if (panels.fail) throw new Error("never show remote exception text");
      remote = body;
      return remote;
    },
    async refresh() {
      panels.state = { ...panels.state, forms: { ...panels.state.forms, waiting: remote } };
      registration.onState(panels.state);
      return panels.state;
    },
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/forms.js", import.meta.url), "utf8"), {
    window: { GrokbotPanels: panels, clearTimeout() {}, setTimeout() {}, dispatchEvent() {} },
    document: { createElement: (tag) => new Element(tag), getElementById: () => undefined }, Event: class {},
  });
  registration.settingsSection(root);
  return { root, panels, requests };
}
// The panel runs in another realm: compare plain JSON, not prototypes.
const plain = (value) => JSON.parse(JSON.stringify(value));
const change = async (control, patch) => { Object.assign(control, patch); await control.listeners.change(); };

test("the setting shows the defaults: on for tasks, off for the direct chat, 24 h and 1 h", () => {
  const { root } = settingsPanel();
  assert.equal(named(root, "waiting-task").checked, true);
  assert.equal(named(root, "waiting-chat").checked, false);
  assert.equal(named(root, "waiting-validity").value, "24h");
  assert.equal(named(root, "waiting-reminder").value, "1h");
  const text = root.textContent;
  assert.match(text, /Continuer sans attendre mes approbations/);
  assert.match(text, /Rien n’est exécuté sans votre feu vert/);
  assert.match(text, /envois sous votre nom et les suppressions demandent toujours une confirmation/);
  assert.deepEqual(
    named(root, "waiting-validity").children.map((option) => option.value),
    ["1h", "6h", "24h", "7d"],
  );
});

test("turning it off greys the choices and explains the 10-minute wait", () => {
  const { root } = settingsPanel({ task: false, chat: false, validity: "6h", reminder: "never" });
  assert.equal(named(root, "waiting-validity").disabled, true);
  assert.equal(named(root, "waiting-reminder").disabled, true);
  assert.equal(named(root, "waiting-chat").disabled, true);
  assert.match(root.textContent, /attend votre réponse pendant 10 minutes, comme avant/);
});

test("a change saves the whole setting, and switching off also clears the direct chat", async () => {
  const f = settingsPanel({ task: true, chat: true, validity: "24h", reminder: "1h" });
  await change(named(f.root, "waiting-validity"), { value: "7d" });
  assert.deepEqual(plain(f.requests.at(-1)), {
    path: "/api/approvals/waiting",
    body: { task: true, chat: true, validity: "7d", reminder: "1h" },
  });
  await change(named(f.root, "waiting-task"), { checked: false });
  assert.deepEqual(plain(f.requests.at(-1).body), { task: false, chat: false, validity: "7d", reminder: "1h" });
  assert.equal(named(f.root, "waiting-validity").disabled, true);
});

test("a failed save says so without exposing the error and restores the saved value", async () => {
  const f = settingsPanel();
  f.panels.fail = true;
  await change(named(f.root, "waiting-validity"), { value: "1h" });
  assert.match(f.root.textContent, /Réglage non enregistré/);
  assert.doesNotMatch(f.root.textContent, /never show/);
  assert.equal(named(f.root, "waiting-validity").value, "24h");
});
