const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { runInNewContext } = require("node:vm");
const { test } = require("node:test");

const source = readFileSync(__dirname + "/app.js", "utf8");
const urls = source.slice(source.indexOf("  function usageUrl()"), source.indexOf("  const systemTheme"));
const rendering = source.slice(source.indexOf("  function renderPicker()"), source.indexOf("  async function providerAction("));

function mount(backend, connected = false, extra = {}) {
  let document;
  class Element {
    constructor(tag) { this.tagName = tag; this.children = []; this.listeners = {}; this.className = ""; this.dataset = {}; this.attributes = {}; }
    get classList() {
      const node = this;
      return {
        add(...names) {
          const classes = new Set(node.className.split(/\s+/).filter(Boolean));
          for (const name of names) classes.add(name);
          node.className = [...classes].join(" ");
        },
        remove(...names) { node.className = node.className.split(/\s+/).filter((name) => name && !names.includes(name)).join(" "); },
        contains(name) { return node.className.split(/\s+/).includes(name); },
      };
    }
    setAttribute(name, value) { this.attributes[name] = String(value); }
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
  const state = { provider: { backend, connected, signed_in: extra.signed_in ?? connected, connection_test: extra.connection_test || {}, signin: "idle", welcomed: true, email: "qa@example.test", plan: "plus", connections: extra.connections || {}, models: [{ slug: "model", name: "Modèle", efforts: [], tiers: [] }] }, preferences: { model: "model" } };
  const context = {
    document, state, element, $: (id) => nodes.get(id), provider: () => state.provider,
    currentBot: () => ({ name: "Ezerbot" }), pill: () => element("button"), option: () => element("button"),
    signIn() {}, signInCodex() {}, signOut() {}, providerAction() {}, refresh() {},
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

test("effort picker displays actual wire levels and submits the selected value", async () => {
  const { context, nodes, state } = mount("codex", true);
  const calls = [];
  context.icon = () => context.element("svg");
  context.notify = (message) => assert.fail(message);
  context.api = async (path, body) => { calls.push({ path, body }); return { preferences: body }; };
  const pickerControls = source.slice(source.indexOf("  function pill("), source.indexOf("  function renderPicker("));
  runInNewContext(pickerControls, context);
  const descriptions = ["Léger", "Équilibré", "Élevé", "Très élevé", "Maximum"];
  state.provider.models = [{ slug: "gpt-6.1-sol", name: "GPT-6.1 Sol", tiers: [], efforts: ["low", "medium", "high", "xhigh", "max"].map((effort, i) => ({ effort, description: descriptions[i] })) }];
  state.preferences = { model: "gpt-6.1-sol", effort: "medium" };
  context.pickerKind = "effort";
  nodes.get("picker").scrollHeight = 400;
  nodes.get("picker").clientHeight = 300;
  context.renderProvider();
  assert.equal(nodes.get("picker").querySelector(".picker-scroll-hint").hidden, false);
  nodes.get("picker").clientHeight = 500;
  context.renderProvider();
  assert.equal(nodes.get("picker").querySelector(".picker-scroll-hint").hidden, true);
  assert.equal(nodes.get("composer-tools").querySelectorAll("b")[1].textContent, "medium");
  assert.equal(nodes.get("composer-tools").querySelectorAll("button")[0].title, "Modèle : GPT-6.1 Sol");
  const options = nodes.get("picker").querySelectorAll("button");
  assert.deepEqual(options.map((option) => option.children[0].text), ["low", "medium", "high", "xhigh", "max"]);
  assert.deepEqual(options.map((option) => option.querySelector("small").textContent), descriptions);
  assert.equal(options[1].attributes["aria-checked"], "true");
  await options[3].listeners.click();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/api/preferences");
  assert.equal(calls[0].body.effort, "xhigh");
  assert.equal(state.preferences.effort, "xhigh");
  assert.equal(nodes.get("composer-tools").querySelectorAll("b")[1].textContent, "xhigh");
  assert.doesNotMatch(nodes.get("composer-tools").textContent, /Par défaut/);
});

test("sharing renders all three values with accessible labels and submits Fast", async () => {
  const { context, nodes, state } = mount("sharing", true);
  const calls = [];
  context.icon = () => context.element("svg");
  context.notify = (message) => assert.fail(message);
  context.api = async (path, body) => { calls.push({ path, body }); return { preferences: body }; };
  runInNewContext(source.slice(source.indexOf("  function pill("), source.indexOf("  function renderPicker(")), context);
  state.provider.models = [{ slug: "gpt-6.1-sol", name: "GPT-6.1 Sol", efforts: [{ effort: "medium", description: "Équilibré" }], tiers: [{ id: "priority", name: "Fast" }] }];
  state.preferences = { model: "gpt-6.1-sol", effort: "medium", fast: false };
  context.pickerKind = "speed";
  context.renderProvider();
  const buttons = nodes.get("composer-tools").querySelectorAll("button");
  assert.deepEqual(buttons.map((button) => button.querySelector("b").textContent), ["GPT-6.1 Sol", "medium", "Standard"]);
  assert.deepEqual(buttons.map((button) => button.attributes["aria-label"]), ["Modèle : GPT-6.1 Sol", "Effort : medium", "Vitesse : Standard"]);
  await nodes.get("picker").querySelectorAll("button")[1].listeners.click();
  assert.equal(calls[0].path, "/api/preferences");
  assert.equal(calls[0].body.fast, true);
  const speed = nodes.get("composer-tools").querySelectorAll("button")[2];
  assert.equal(speed.querySelector("b").textContent, "Fast");
  assert.equal(speed.attributes["aria-label"], "Vitesse : Fast");
});

test("changing picker kind resets scroll without resetting a same-kind refresh", () => {
  const { context, nodes } = mount("codex", true);
  const picker = nodes.get("picker");
  picker.scrollTop = 43;
  context.pickerKind = "effort";
  context.renderPicker();
  assert.equal(picker.scrollTop, 0);
  assert.equal(picker.dataset.kind, "effort");
  picker.scrollTop = 21;
  context.renderPicker();
  assert.equal(picker.scrollTop, 21);
  context.pickerKind = "model";
  context.renderPicker();
  assert.equal(picker.scrollTop, 0);
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

test("a signed-in account is not shown as Codex until OpenAI answers", () => {
  const failed = mount("codex", false, { signed_in: true, connection_test: { state: "failed", message: "probe refused" } });
  assert.equal(failed.nodes.get("mode-badge-text").textContent, "Non connecté");
  assert.match(failed.nodes.get("mode-note").textContent, /Aucune réponse n’est produite/);
  assert.match(failed.nodes.get("provider-section").textContent, /Aucune réponse n’est produite/);
  assert.equal(failed.nodes.get("composer-tools").textContent, "Non connecté");
  const retry = failed.nodes.get("provider-section").querySelectorAll("button").find((button) => button.textContent === "Tester la connexion");
  assert.equal(retry.disabled, false);
  const running = mount("codex", false, { signed_in: true, connection_test: { state: "running" } });
  assert.equal(running.nodes.get("mode-badge-text").textContent, "Vérification");
  assert.match(running.nodes.get("mode-note").textContent, /Aucune réponse tant que ce n’est pas confirmé/);
  assert.equal(running.nodes.get("provider-section").querySelectorAll("button").find((button) => /Test/.test(button.textContent)).disabled, true);
  const ready = mount("codex", true, { connection_test: { state: "ok", message: "OK — ChatGPT répond." } });
  assert.equal(ready.nodes.get("mode-badge-text").textContent, "Codex");
  assert.match(ready.nodes.get("provider-section").textContent, /Forfait : Plus/);
});
