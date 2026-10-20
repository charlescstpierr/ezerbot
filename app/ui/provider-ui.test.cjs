const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { runInNewContext } = require("node:vm");
const { test } = require("node:test");

const source = readFileSync(__dirname + "/app.js", "utf8");
const urls = source.slice(source.indexOf("  function usageUrl()"), source.indexOf("  const effortNames"));
const rendering = source.slice(source.indexOf("  function renderPicker()"), source.indexOf("  async function providerAction("));

function mount(backend, connected = false) {
  let document;
  class Element {
    constructor(tag) { this.tagName = tag; this.children = []; this.listeners = {}; this.className = ""; }
    append(...children) { for (const child of children) { child.parent = this; this.children.push(child); } }
    contains(node) { return node === this || this.children.some((child) => child.contains(node)); }
    replaceChildren(...children) {
      if (this.children.some((child) => child.contains(document.activeElement))) document.activeElement = document.body;
      this.children = []; this.text = ""; this.append(...children);
    }
    set textContent(text) { this.replaceChildren(); this.text = String(text); }
    get textContent() { return (this.text || "") + this.children.map((child) => child.textContent).join(" "); }
    prepend(child) { child.parent = this; this.children.unshift(child); }
    addEventListener(name, handler) { this.listeners[name] = handler; }
    focus() { document.activeElement = this; }
    showModal() { this.open = true; }
    querySelectorAll(selector) {
      const matches = (node) => {
        if (selector.startsWith(".")) return node.className.split(" ").includes(selector.slice(1));
        if (selector.startsWith("input[")) return node.tagName === "input" && [...selector.matchAll(/\[(\w+)=([^\]]+)\]/g)].every(([, key, value]) => node[key] === value);
        return node.tagName === selector;
      };
      return this.children.flatMap((child) => [...(matches(child) ? [child] : []), ...child.querySelectorAll(selector)]);
    }
    querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  }
  const nodes = new Map();
  document = { body: new Element("body"), activeElement: null, createTextNode: (text) => element("text", "", text), querySelectorAll: () => [signin] };
  document.activeElement = document.body;
  function element(tag, className = "", text = "") { const node = new Element(tag); node.className = className; node.text = text; return node; }
  for (const id of ["mode-badge-text", "plan-banner", "composer-tools", "picker", "mode-note", "provider-section", "plan-dialog", "plan-title"]) {
    const node = element("div"); nodes.set(id, node); document.body.append(node);
  }
  const signin = element("button"); nodes.get("plan-banner").append(element("span"), signin);
  nodes.get("plan-dialog").append(element("p", "delete-description"), element("a"));
  const state = { provider: { backend, connected, signin: "idle", welcomed: true, email: "qa@example.test", plan: "plus", connections: {}, models: [{ slug: "model", name: "Modèle", efforts: [], tiers: [] }] }, preferences: { model: "model" } };
  const context = {
    document, state, element, $: (id) => nodes.get(id), provider: () => state.provider,
    currentBot: () => ({ name: "Ezerbot" }), pill: () => element("button"), option: () => element("button"),
    effortName: () => "Équilibré", signIn() {}, signOut() {}, providerAction() {}, refresh() {},
    pickerKind: connected ? "model" : null,
    closePicker() { context.pickerKind = null; nodes.get("picker").hidden = true; },
  };
  runInNewContext(urls + rendering, context);
  context.renderProvider();
  return { context, document, nodes, state };
}

for (const connected of [false, true]) {
  test(`Codex copy and every usage link stay Codex-specific (${connected ? "connected" : "signed out"})`, () => {
    const { context, nodes } = mount("codex", connected);
    assert.equal(context.usageUrl(), "https://chatgpt.com/codex/settings/usage");
    for (const id of ["plan-banner", "provider-section", "picker", "plan-dialog", "mode-note"]) {
      assert.doesNotMatch(nodes.get(id).textContent, /abonnement|partage/i, id);
      for (const link of nodes.get(id).querySelectorAll("a")) assert.equal(link.href, context.usageUrl());
    }
    assert.match(nodes.get("provider-section").textContent, connected ? /Forfait : Plus/ : /quota Codex de votre forfait/);
  });
  test(`Sharing keeps its own copy and general usage URL (${connected ? "connected" : "signed out"})`, () => {
    const { context, nodes } = mount("sharing", connected);
    assert.equal(context.usageUrl(), "https://chatgpt.com/settings/usage");
    for (const id of ["provider-section", "picker", "plan-dialog", "mode-note"]) {
      for (const link of nodes.get(id).querySelectorAll("a")) assert.equal(link.href, context.usageUrl());
    }
    assert.match(nodes.get("plan-dialog").textContent, /partage d’abonnement/);
  });
}

test("a closed picker does not retain stale text or usage links from the previous backend", () => {
  const { context, nodes, state } = mount("sharing", true);
  assert.match(nodes.get("picker").textContent, /Partage d’abonnement/);
  context.pickerKind = null;
  state.provider.backend = "codex";
  context.renderProvider();
  assert.equal(nodes.get("picker").children.length, 0);
  assert.equal(nodes.get("picker").hidden, true);
});

test("conversation action links track the selected backend instead of capturing the initial URL", () => {
  let backend = "codex";
  const context = { provider: () => ({ backend }), threadMascot: () => () => {}, cardAction() {} };
  const actions = source.slice(source.indexOf("  const thread = {"), source.indexOf("  function render(followLatest"));
  runInNewContext(urls + actions + "\nglobalThis.actions = thread;", context);
  assert.equal({ ...context.actions }.usageUrl, "https://chatgpt.com/codex/settings/usage");
  backend = "sharing";
  assert.equal({ ...context.actions }.usageUrl, "https://chatgpt.com/settings/usage");
  backend = "codex";
  assert.equal({ ...context.actions }.usageUrl, "https://chatgpt.com/codex/settings/usage");
});

test("connection radio focus survives a selection response and subsequent polling renders", () => {
  const { context, nodes, state, document } = mount("codex");
  nodes.get("provider-section").querySelector("input[name=connection-backend][value=sharing]").focus();
  state.provider.backend = "sharing";
  for (let i = 0; i < 3; i++) {
    context.renderProvider();
    assert.equal(document.activeElement.name, "connection-backend");
    assert.equal(document.activeElement.value, "sharing");
    assert.equal(document.activeElement.checked, true);
  }
});

test("the sign-in banner stays hidden while either ChatGPT account is connected", () => {
  const { context, nodes, state } = mount("codex");
  assert.equal(nodes.get("plan-banner").hidden, false);
  state.provider.connections = { codex: false, sharing: true };
  context.renderProvider();
  assert.equal(nodes.get("plan-banner").hidden, true);
});
