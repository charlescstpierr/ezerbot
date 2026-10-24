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
  for (const id of ["plan-banner", "composer-tools", "picker", "mode-note", "provider-section", "plan-dialog", "plan-title"]) {
    const node = element("div"); nodes.set(id, node); document.body.append(node);
  }
  const signin = element("button"); nodes.get("plan-banner").append(element("span"), signin);
  nodes.get("plan-dialog").append(element("p", "delete-description"), element("a"));
  const state = { provider: { backend, connected, signed_in: extra.signed_in ?? connected, connection_test: extra.connection_test || {}, signin: "idle", welcomed: true, email: "qa@example.test", plan: "plus", connections: extra.connections || {}, models: [{ slug: "model", name: "Modèle", efforts: [], tiers: [] }] }, preferences: { model: "model" } };
  const context = {
    document, state, element, $: (id) => nodes.get(id), provider: () => state.provider,
    currentBot: () => ({ name: "Ezerbot" }), pill: () => element("button"), option: () => element("button"),
    signIn() {}, signOut() {}, providerAction() {}, refresh() {},
    pickerKind: connected ? "model" : null,
    closePicker() { context.pickerKind = null; nodes.get("picker").hidden = true; },
  };
  runInNewContext(urls + rendering, context);
  context.renderProvider();
  return { context, document, nodes, state };
}

for (const connected of [false, true]) {
  for (const backend of ["codex", "sharing"]) test(`one ChatGPT subscription even with stale backend data (${backend}, ${connected})`, () => {
    const { context, nodes } = mount(backend, connected);
    assert.equal(context.usageUrl(), "https://chatgpt.com/codex/settings/usage");
    assert.equal(nodes.get("provider-section").querySelectorAll("input").length, 0);
    for (const id of ["plan-banner", "provider-section", "picker", "plan-dialog", "mode-note"]) {
      assert.doesNotMatch(nodes.get(id).textContent, /partage d’abonnement|Passer à Codex|autre mode|clé API distincte/i, id);
      for (const link of nodes.get(id).querySelectorAll("a")) assert.equal(link.href, context.usageUrl());
    }
    assert.match(nodes.get("provider-section").textContent, connected ? /Forfait : Plus/ : /sans clé API ni autre abonnement/);
    const buttons = nodes.get("provider-section").querySelectorAll("button");
    assert.equal(buttons.filter(button => button.textContent === "Se connecter").length, connected ? 0 : 1);
  });
}

test("a closed picker does not retain stale text or usage links from the previous backend", () => {
  const { context, nodes, state } = mount("sharing", true);
  assert.match(nodes.get("picker").textContent, /Quota de votre abonnement ChatGPT/);
  context.pickerKind = null;
  state.provider.backend = "codex";
  context.renderProvider();
  assert.equal(nodes.get("picker").children.length, 0);
  assert.equal(nodes.get("picker").hidden, true);
});

test("conversation action links track the selected backend instead of capturing the initial URL", () => {
  let backend = "codex";
  let signedIn = 0;
  const context = {
    provider: () => ({ backend }),
    threadMascot: () => () => {},
    cardAction() {},
    signIn() { signedIn += 1; },
  };
  const actions = source.slice(source.indexOf("  const thread = {"), source.indexOf("  function render(followLatest"));
  runInNewContext(urls + actions + "\nglobalThis.actions = thread;", context);
  assert.equal({ ...context.actions }.usageUrl, "https://chatgpt.com/codex/settings/usage");
  backend = "sharing";
  assert.equal({ ...context.actions }.usageUrl, "https://chatgpt.com/codex/settings/usage");
  backend = "codex";
  assert.equal({ ...context.actions }.usageUrl, "https://chatgpt.com/codex/settings/usage");
  context.actions.onSignIn();
  assert.equal(signedIn, 1);
  assert.doesNotMatch(source, /signInCodex/);
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

function signInControls(root) {
  return root.querySelectorAll("button").filter((button) => button.className.split(/\s+/).includes("chatgpt-button") || /via Codex|Sign in with ChatGPT/.test(button.textContent));
}

test("Réglages shows exactly one ChatGPT sign-in control and no second connection option", () => {
  for (const backend of ["codex", "sharing"]) {
    const signedOut = mount(backend, false);
    const section = signedOut.nodes.get("provider-section");
    const controls = signInControls(section);
    assert.equal(controls.length, 1);
    assert.equal(controls[0].textContent, "Se connecter");
    assert.equal(section.querySelectorAll("input").length, 0);
    assert.doesNotMatch(section.textContent, /via Codex|Sign in with ChatGPT/);
    const signedIn = mount(backend, true);
    const connected = signedIn.nodes.get("provider-section");
    assert.equal(signInControls(connected).length, 0);
    assert.match(connected.textContent, /Se déconnecter/);
    assert.doesNotMatch(connected.textContent, /via Codex|Sign in with ChatGPT/);
  }
});

test("no second connection choice returns after polling stale backend state", () => {
  const { context, nodes, state } = mount("codex");
  state.provider.backend = "sharing";
  for (let i = 0; i < 3; i++) {
    context.renderProvider();
    assert.equal(nodes.get("provider-section").querySelectorAll("input").length, 0);
    assert.doesNotMatch(nodes.get("provider-section").textContent, /Passer à Codex|Sign in with ChatGPT/);
  }
});

test("only the single ChatGPT sign-in state can hide the banner", () => {
  const { context, nodes, state } = mount("codex");
  assert.equal(nodes.get("plan-banner").hidden, false);
  state.provider.connections = { codex: false, sharing: true };
  context.renderProvider();
  assert.equal(nodes.get("plan-banner").hidden, false);
  state.provider.signed_in = true;
  context.renderProvider();
  assert.equal(nodes.get("plan-banner").hidden, true);
});

test("a signed-in account is not shown as Codex until OpenAI answers", () => {
  const failed = mount("codex", false, { signed_in: true, connection_test: { state: "failed", message: "probe refused" } });
  assert.match(failed.nodes.get("mode-note").textContent, /Vous pouvez envoyer, ou réessayer le test/);
  assert.match(failed.nodes.get("provider-section").textContent, /Envoyez votre question, ou réessayez le test/);
  assert.doesNotMatch(failed.nodes.get("provider-section").textContent, /Aucune réponse/);
  assert.equal(failed.nodes.get("composer-tools").textContent, "Non connecté");
  const retry = failed.nodes.get("provider-section").querySelectorAll("button").find((button) => button.textContent === "Tester la connexion");
  assert.equal(retry.disabled, false);
  const running = mount("codex", false, { signed_in: true, connection_test: { state: "running" } });
  assert.equal(running.nodes.get("composer-tools").textContent, "Vérification");
  assert.match(running.nodes.get("mode-note").textContent, /Votre question part sans attendre/);
  assert.match(running.nodes.get("provider-section").textContent, /L’envoi n’est pas bloqué/);
  assert.doesNotMatch(running.nodes.get("provider-section").textContent, /Aucune réponse/);
  assert.doesNotMatch(source, /Aucune réponse n’est produite avant cette confirmation|Aucune réponse tant que ce n’est pas confirmé/);
  assert.equal(running.nodes.get("provider-section").querySelectorAll("button").find((button) => /Test/.test(button.textContent)).disabled, true);
  const ready = mount("codex", true, { connection_test: { state: "ok", message: "OK — ChatGPT répond." } });
  assert.equal(ready.nodes.get("composer-tools").textContent.includes("Local"), false);
  assert.match(ready.nodes.get("provider-section").textContent, /Forfait : Plus/);
});

test("signed out, nothing says Local: the composer status is empty", () => {
  const { nodes } = mount("codex");
  assert.equal(nodes.get("composer-tools").textContent, "");
});
