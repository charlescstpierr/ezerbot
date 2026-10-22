import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import { after, test } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";
import { localToolLabel } from "./local-call.js";

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

function settingsPanel({ rules = [], mcp, localUi = { toolLabel: localToolLabel } } = {}) {
  const root = new Element("div");
  const settingsButton = new Element("button");
  let registration;
  let remote = { config: { enabled: true, allow: ["Autorise la lecture."], block: [] }, version: "1" };
  const requests = [];
  const notifications = [];
  const panels = {
    state: { forms: { rules, reviewer: remote }, bots: [], mcp },
    register(value) { registration = value; },
    notify(text) { notifications.push(text); },
    async api(path, body) {
      requests.push({ path, body });
      if (panels.failSave) throw new Error("never show remote exception text");
      if (path === "/api/approvals/rules/revoke") {
        panels.state.forms.rules = panels.state.forms.rules.filter((rule) => !(
          rule.route.requesting_bot_id === body.requesting_bot_id
          && rule.route.target_bot_id === body.target_bot_id
          && rule.tool === body.tool && rule.args_digest === body.args_digest && rule.scope === body.scope
        ));
        return { rules: panels.state.forms.rules };
      }
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
    window: { GrokbotPanels: panels, GrokbotLocalUi: localUi, clearTimeout() {}, setTimeout() {}, dispatchEvent() {} },
    document: { createElement: (tag) => new Element(tag), getElementById: (id) => id === "settings-button" ? settingsButton : undefined }, Event: class {},
  });
  registration.settingsSection(root);
  return { root, settingsButton, panels, registration, requests, notifications, setRemote(value) { remote = value; } };
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

const fallbackStorage = { backend: "file_fallback", label: "Trousseau macOS indisponible", notice: "Les nouveaux secrets ne sont pas enregistrés." };
function updateStorage(f, storage) {
  f.panels.state.forms.storage = storage;
  f.registration.onState(f.panels.state);
}

test("storage warning is a dedicated alert in a separated block and marks Paramètres outside the modal", () => {
  const f = settingsPanel(); updateStorage(f, fallbackStorage);
  const block = byClass(f.root, "storage-settings");
  const notice = byClass(block, "storage-notice");
  assert.equal(notice.attributes.role, "alert");
  assert.equal(notice.children[0].tagName, "strong");
  assert.match(notice.textContent, /^Le trousseau macOS est indisponible\./);
  assert.ok(f.root.children.indexOf(block) < f.root.children.findIndex(e => e.textContent === "Autorisations permanentes"));
  const marker = byClass(f.settingsButton, "storage-warning-marker");
  assert.equal(marker.hidden, false);
  assert.match(marker.attributes["aria-label"], /ne sont pas enregistrés/);
  updateStorage(f, { backend: "keychain", label: "Trousseau macOS" });
  assert.equal(marker.hidden, true);
  assert.equal(byClass(f.root, "storage-notice"), undefined);
});

test("signature pending is neutral information, not a failure alert or an unlock invitation", () => {
  const f = settingsPanel();
  updateStorage(f, { backend: "file", label: "Fichier local protégé", notice: "Le trousseau attend une signature stable.", notice_kind: "info" });
  const notice = byClass(f.root, "storage-notice");
  assert.equal(notice.attributes.role, "status");
  assert.ok(notice.className.includes("storage-info"));
  assert.match(notice.textContent, /^Activation du trousseau en attente/);
  assert.equal(button(f.root, "Vérifier le stockage"), undefined);
  assert.equal(byClass(f.settingsButton, "storage-warning-marker").hidden, true);
});

test("storage check confirms refusal and recovery inside the modal, persisting through a real refresh", async () => {
  const f = settingsPanel(); updateStorage(f, fallbackStorage);
  await button(f.root, "Vérifier le stockage").listeners.click();
  assert.match(byClass(f.root, "storage-feedback").textContent, /^Le trousseau est toujours inaccessible/);
  f.panels.refresh = async () => updateStorage(f, { backend: "keychain", label: "Trousseau macOS" });
  await button(f.root, "Vérifier le stockage").listeners.click();
  assert.match(byClass(f.root, "storage-feedback").textContent, /^Trousseau macOS rétabli\.$/);
  assert.equal(byClass(f.root, "storage-feedback").attributes.role, "status");
  assert.equal(button(f.root, "Vérifier le stockage"), undefined);
  f.registration.onState(f.panels.state);
  assert.match(byClass(f.root, "storage-feedback").textContent, /^Trousseau macOS rétabli/);
  assert.equal(f.notifications.length, 0); // A body toast is hidden behind the native modal.
});

test("storage check blocks duplicate clicks, indicates progress and retains an actionable error", async () => {
  const f = settingsPanel(); updateStorage(f, fallbackStorage);
  let reject;
  let calls = 0;
  f.panels.refresh = () => { calls++; return new Promise((_, failure) => { reject = failure; }); };
  const pending = button(f.root, "Vérifier le stockage").listeners.click();
  assert.equal(button(f.root, "Vérifier le stockage").disabled, true);
  assert.match(byClass(f.root, "storage-feedback").textContent, /en cours/);
  await button(f.root, "Vérifier le stockage").listeners.click();
  assert.equal(calls, 1);
  reject(new Error("private remote details")); await pending;
  assert.match(byClass(f.root, "storage-feedback").textContent, /ne peut pas être vérifié/);
  assert.ok(!f.root.textContent.includes("private remote"));
  assert.equal(button(f.root, "Vérifier le stockage").disabled, false);
});

const standingRule = (tool, allow, scope = "global") => ({
  route: { requesting_bot_id: "grokbot", target_bot_id: "grokbot" },
  tool, allow, scope, args_digest: scope === "global" ? "" : "a".repeat(64),
});
const ruleRows = (root) => root.children.filter((row) => button(row, "Retirer la règle"));
const connector = { servers: [{ id: "atelier", name: "Atelier MCP", tools: [{ namespace: "mcp_atelier_echo", name: "echo" }] }] };

for (const mcpFirst of [false, true]) {
  test(`mixed local/MCP standing rules all render and revoke without losing the reviewer (MCP first: ${mcpFirst})`, async () => {
    const local = standingRule("run_shell", true);
    const mcp = standingRule("mcp_atelier_echo", false, "subject");
    const rules = mcpFirst ? [mcp, local] : [local, mcp];
    // Execute the shipping panel with rules already present: the old code throws
    // after its first row and never reaches the reviewer editor.
    const f = settingsPanel({ rules, mcp: connector });
    assert.equal(ruleRows(f.root).length, 2);
    assert.match(ruleRows(f.root)[rules.indexOf(local)].textContent, /Terminal sur votre Mac · Toujours autoriser · Toutes les actions/);
    assert.match(ruleRows(f.root)[rules.indexOf(mcp)].textContent, /Atelier MCP · echo · Toujours refuser · Arguments identiques/);
    const details = f.root.children.filter((node) => node.tagName === "details");
    assert.equal(details.length, 1);
    assert.equal(find(details[0], (node) => node.tagName === "code").textContent, "run_shell");
    for (const [i, rule] of rules.entries()) {
      assert.ok(form(f.root));
      assert.ok(field(f.root, "enabled"));
      assert.ok(button(f.root, "Enregistrer les règles"));
      await button(ruleRows(f.root)[0], "Retirer la règle").listeners.click();
      assert.deepEqual({ ...f.requests[i].body }, {
        requesting_bot_id: "grokbot", target_bot_id: "grokbot", tool: rule.tool,
        args_digest: rule.args_digest, scope: rule.scope,
      });
      assert.equal(f.requests[i].path, "/api/approvals/rules/revoke");
      assert.equal(ruleRows(f.root).length, 1 - i);
      assert.ok(form(f.root));
    }
    assert.match(f.root.textContent, /Aucune règle « Toujours »/);
    field(f.root, "allow").value = "Demandez confirmation.";
    input(f.root); await submit(f.root);
    assert.equal(field(f.root, "allow").value, "Demandez confirmation.");
    assert.match(byClass(f.root, "reviewer-notice").textContent, /^Règles enregistrées/);
    assert.equal(f.notifications.length, 0);
  });
}

test("multiple local rules, MCP lifecycle and unknown tools preserve every row and the reviewer with or without local UI", () => {
  const rules = [standingRule("run_shell", true), standingRule("write_file", false), standingRule("mcp_start_atelier", true), standingRule("other_tool", false)];
  for (const localUi of [{ toolLabel: localToolLabel }, null]) {
    const f = settingsPanel({ rules, mcp: connector, localUi });
    assert.equal(ruleRows(f.root).length, rules.length);
    assert.ok(form(f.root));
    assert.match(f.root.textContent, /Atelier MCP · Démarrer \/ redémarrer/);
    assert.match(f.root.textContent, /other_tool · Toujours refuser/);
  }
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
const { Card: ToolCard, openConnectors } = await vite.ssrLoadModule("/src/cards/tool.jsx");
const { Card: SecretCard } = await vite.ssrLoadModule("/src/cards/secret.jsx");

test("connector lifecycle cards show the provider-specific action and its result, not a bare resume receipt", () => {
  globalThis.window={GrokbotForms:{view(){}},GrokbotPanels:{state:{bots:[{id:"grokbot",name:"Ezerbot"}]}}};
  try {
    for (const remove of [false,true]) {
      const html=renderToStaticMarkup(React.createElement(Card,{message:{id:"connector",kind:"approval",payload:{
        entry_id:"connector",status:{status:"resolved"},route:{requesting_bot_id:"grokbot"},
        kind:{kind:"approval",tool:remove?"mcp_remove_fixture":"mcp_start_fixture",expires_at:Date.now()+60000},
        connector:{server:"Notion",tool:remove?"Déconnecter Notion":"Connecter Notion",origin:"settings"},
        arguments:{commande:"https://mcp.notion.com/mcp",arguments:[],variables_privées:[],catalogue:{transport:"https",service:"Notion",licence:"Service officiel",permissions:"Pages choisies",portées:"Choisies pendant le consentement Notion",connexion:"Le serveur est contacté par HTTPS.",déconnexion:"Les jetons sont supprimés."}},
        configuration_result:{status:"done",output:{message:remove?"Déconnecté sur ce Mac.":"Connexion en attente dans le navigateur."}},
      }}}));
      assert.ok(html.includes(remove?"Déconnecter Notion":"Connecter Notion"));
      assert.ok(html.includes(remove?"Déconnecté sur ce Mac.":"Connexion en attente dans le navigateur."));
      assert.ok(!html.includes("Repris"));assert.ok(!html.includes("CLIENT_SECRET"));assert.ok(!html.includes("Google"));
      assert.equal(html.includes("Adresse du service"),!remove);assert.ok(!html.includes(">Commande<"));
    }
  } finally {delete globalThis.window;}
});

test("managed Google approvals describe the adapter without inventing an empty argument list", () => {
  globalThis.window={GrokbotForms:{view(){}},GrokbotPanels:{state:{bots:[{id:"grokbot",name:"Ezerbot"}]}}};
  try {
    for (const managed of [false,true]) {
      const args={commande:managed?"uvx · workspace-mcp 2.1.1":"python3",arguments:[],variables_privées:[],...(managed?{catalogue:{transport:"stdio",service:"Gmail",connexion:"Lancement avec uvx et l’adaptateur Ezerbot (identifiants en mémoire)."}}:{})};
      const html=renderToStaticMarkup(React.createElement(Card,{message:{id:"launch",kind:"approval",payload:{entry_id:"launch",status:{status:"pending"},route:{requesting_bot_id:"grokbot"},kind:{kind:"approval",tool:"mcp_start_fixture",expires_at:Date.now()+60000},connector:{server:managed?"Gmail":"Atelier",tool:"Démarrer",origin:"settings"},arguments:args}}}));
      assert.equal(html.includes(">Arguments<"),!managed);assert.equal(html.includes("Aucun argument."),!managed);
      if(managed) assert.ok(html.includes("adaptateur Ezerbot"));
    }
  } finally {delete globalThis.window;}
});

test("connector recovery reveals the affected card and focuses its single action", () => {
  const focus=[],scroll=[];
  const target={dataset:{connector:"gmail"},scrollIntoView(){scroll.push("gmail");},querySelector(){return{focus(){focus.push("reconnect");}};}};
  const unrelated={dataset:{connector:"notion"}};
  const section={querySelectorAll(){return[unrelated,target];},scrollIntoView(){scroll.push("section");}};
  const dialog={open:false};
  globalThis.document={getElementById(id){return id==="settings-dialog"?dialog:{click(){dialog.open=true;}};},querySelector(){return section;}};
  globalThis.window={};globalThis.requestAnimationFrame=fn=>fn();
  try {openConnectors({catalogue_id:"gmail"});assert.equal(dialog.open,true);assert.deepEqual(scroll,["gmail"]);assert.deepEqual(focus,["reconnect"]);}
  finally {delete globalThis.document;delete globalThis.window;delete globalThis.requestAnimationFrame;}
});

function secretMessage(status, kind = { kind: "secret", name: "CLE_API", purpose: "Test" }) {
  return {
    id: "secret-test", kind: "secret", payload: {
      entry_id: "secret-test", status: { status }, route: { requesting_bot_id: "grokbot" }, kind,
    },
  };
}

test("secret cards warn about current fallback only while the form is still open", () => {
  for (const backend of ["file_fallback", "keychain", "file"]) {
    globalThis.window = {
      GrokbotForms: { view() {}, storage: () => ({ backend }) },
      GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } },
    };
    try {
      const pending = renderToStaticMarkup(React.createElement(SecretCard, { message: secretMessage("pending") }));
      assert.equal(pending.includes("gb-secret-storage-warning"), backend === "file_fallback");
      if (backend === "file_fallback") assert.ok(pending.includes("Les nouveaux secrets ne sont pas enregistrés"));
      assert.ok(pending.includes("selon Paramètres"));
      assert.ok(!pending.includes("envoyée au coffre local"));
      const resolved = renderToStaticMarkup(React.createElement(SecretCard, { message: secretMessage("resolved") }));
      assert.ok(!resolved.includes("gb-secret-storage-warning"));
      assert.ok(!resolved.includes("selon Paramètres"));
      assert.ok(resolved.includes("Secret CLE_API enregistré"));
      assert.ok(!resolved.includes("type=\"password\""));
    } finally { delete globalThis.window; }
  }
});

test("a conversation secret card takes one masked value and then names the recorded secret", () => {
  globalThis.window = {
    GrokbotForms: { view() {}, storage: () => ({ backend: "keychain" }) },
    GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } },
  };
  try {
    const pasted = "phrase secrete du formulaire";
    const pending = renderToStaticMarkup(React.createElement(SecretCard, { message: secretMessage("pending", { kind: "secret", name: "STRIPE_KEY", purpose: "Clé API Stripe" }) }));
    assert.ok(pending.includes("Clé API Stripe"));
    assert.ok(pending.includes("STRIPE_KEY"));
    assert.ok(pending.includes("type=\"password\""));
    assert.ok(pending.includes(">Enregistrer<"));
    assert.ok(!pending.includes("Enregistrer le secret"));
    assert.ok(!pending.includes(pasted));
    assert.ok(!pending.includes("Secret STRIPE_KEY enregistré"));
    const resolved = renderToStaticMarkup(React.createElement(SecretCard, { message: secretMessage("resolved", { kind: "secret", name: "STRIPE_KEY", purpose: "Clé API Stripe" }) }));
    assert.ok(resolved.includes("Secret STRIPE_KEY enregistré"));
    assert.ok(!resolved.includes("type=\"password\""));
    assert.ok(!resolved.includes(">Enregistrer<"));
    assert.ok(!resolved.includes(pasted));
    assert.ok(!resolved.includes("selon Paramètres"));
  } finally { delete globalThis.window; }
});
function card(status, decision, args = { path: "~/Documents/projets/notes.txt" }, pending = false, tool = "test_read_resource") {
  globalThis.window = { GrokbotForms: { view() {} }, GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(Card, { message: {
      id: "approval-test", kind: "approval", payload: {
        entry_id: "approval-test", status: { status }, route: { requesting_bot_id: "grokbot" },
        kind: { kind: "approval", tool, expires_at: Date.now() + 60000 }, arguments: args,
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

test("local cards combine reviewer status, exact action and protected preview without automatic framing", () => {
  const read = { path: "/tmp/app/identifiants.json", apercu_local: { donnees_ezerbot: true, cible_reelle: "/tmp/app/identifiants.json" } };
  const html = card("pending", "ask", read, false, "read_file");
  for (const text of ["Relecteur : ", "Lecture : ", "identifiants, historique", "Autoriser une fois", "/tmp/app/identifiants.json"]) assert.ok(html.includes(text));
  const command = card("pending", "ask", {command:"printf bonjour",cwd:"/tmp"}, false, "run_shell");
  assert.ok(command.includes("Relecteur : "));assert.ok(command.includes("printf bonjour"));
  assert.ok(!card("resolved", "allow", read, false, "read_file").includes("Approuvé automatiquement"));
});

test("protected writes render one sensitive warning immediately before the decision controls", () => {
  const html = card("pending", "ask", { path: "/tmp/app/new.txt", content: "test", expected_sha256: null,
    apercu_local: { donnees_ezerbot: true, lignes_ajoutees: 1, lignes_supprimees: 0, lignes_proposees: 1, extrait: "@@ -0,0 +1,1 @@\\n+test\\n" } }, false, "write_file");
  assert.match(html, /data-follow-actions="true"/);
  assert.equal(html.split('gb-local-sensitive-warning').length - 1, 1);
  assert.match(html, /Refusez sauf certitude\.<\/p><fieldset/);
  assert.ok(html.indexOf('gb-local-sensitive-warning') > html.indexOf("Empreinte et expiration"));
});

test("running local cards publish their committed phase and a visible stop affordance", () => {
  const html = renderToStaticMarkup(React.createElement(ToolCard, { message: { id: "running", kind: "tool", payload: {
    run_id: "run", run_status: "running", call: { call_id: "call", name: "run_shell", arguments: {command:"sleep 20",cwd:"/tmp"} },
  } } }));
  assert.match(html, /data-local-state="running"/);
  assert.match(html, /gb-local-stop/);assert.match(html, /Annuler l’exécution/);
});

test("generic approval retains its code identifier instead of a local-action label", () => {
  const html = card("pending", "ask", {}, false, "test_generic_tool");
  assert.match(html, /Outil : <code>test_generic_tool<\/code>/);
  assert.ok(!html.includes("Action : test_generic_tool"));
  assert.ok(!html.includes("data-follow-actions"));
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
