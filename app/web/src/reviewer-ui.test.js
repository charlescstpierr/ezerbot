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

const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)), configFile: false,
  plugins: [react()], server: { middlewareMode: true }, logLevel: "error",
  optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());
const { Card } = await vite.ssrLoadModule("/src/cards/approval.jsx");
const { Card: ToolCard, TaskInputView, settleDraft, openConnectors } = await vite.ssrLoadModule("/src/cards/tool.jsx");
const { ScreenCard } = await vite.ssrLoadModule("/src/screen.jsx");
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

test("starting a server added by address shows the address it will contact, not a command or arguments", () => {
  globalThis.window={GrokbotForms:{view(){}},GrokbotPanels:{state:{bots:[{id:"grokbot",name:"Ezerbot"}]}}};
  try {
    const html=renderToStaticMarkup(React.createElement(Card,{message:{id:"remote",kind:"approval",payload:{
      entry_id:"remote",status:{status:"pending"},route:{requesting_bot_id:"grokbot"},
      kind:{kind:"approval",tool:"mcp_start_remote",expires_at:Date.now()+60000},
      connector:{server:"Serveur distant",tool:"Démarrer le connecteur",origin:"settings"},
      arguments:{serveur:"Serveur distant",commande:"https://exemple.fr/mcp",arguments:[],variables_privées:["ACCESS_TOKEN"],transport:"https",action:"Démarrer le connecteur"},
    }}}));
    assert.ok(html.includes("Adresse du service"));assert.ok(html.includes("https://exemple.fr/mcp"));
    assert.ok(!html.includes(">Commande<"));assert.ok(!html.includes("Aucun argument"));
    assert.ok(html.includes("valeurs masquées"));
  } finally {delete globalThis.window;}
});

test("a GitHub disconnection names the account and offers Confirmer and Annuler, never Toujours", () => {
  globalThis.window={GrokbotForms:{view(){}},GrokbotPanels:{state:{bots:[{id:"grokbot",name:"Ezerbot"}]}}};
  const card=(tool,compte)=>renderToStaticMarkup(React.createElement(Card,{message:{id:"github",kind:"approval",payload:{
    entry_id:"github",status:{status:"pending"},route:{requesting_bot_id:"grokbot"},
    kind:{kind:"approval",tool,expires_at:Date.now()+60000},
    connector:{server:"GitHub",tool:tool.startsWith("mcp_remove_")?"Déconnecter GitHub":"Connecter GitHub",origin:"settings"},
    policy:{remember:false,protection:null,scope:"none"},
    arguments:{commande:"https://api.github.com",arguments:[],variables_privées:["ACCESS_TOKEN","REFRESH_TOKEN"],catalogue:{transport:"https",service:"GitHub",licence:"Service GitHub",permissions:"Issues",portées:"aucune",connexion:"Un code s’affiche.",déconnexion:compte?`Le compte GitHub ${compte} sera déconnecté.`:"Les jetons sont supprimés.",...(compte?{compte}:{})}},
  }}}));
  try {
    const html=card("mcp_remove_github","charles-test");
    assert.ok(html.includes("Déconnecter le compte GitHub charles-test ?"));
    assert.ok(html.includes("Compte : <strong>charles-test</strong>"));
    assert.ok(html.includes(">Confirmer<")&&html.includes(">Annuler<"));
    for(const word of ["Toujours","Autoriser une fois","Refuser","Mémoriser"]) assert.ok(!html.includes(word),word);
    const connect=card("mcp_start_github");
    assert.ok(connect.includes("Autoriser une fois")&&!connect.includes(">Confirmer<"));
  } finally {delete globalThis.window;}
});

test("the webhook publication card names Funnel, the public path and the private iPhone address, and shows its result", () => {
  globalThis.window={GrokbotForms:{view(){}},GrokbotPanels:{state:{bots:[{id:"grokbot",name:"Ezerbot"}]}}};
  try {
    for (const enabled of [true,false]) {
      const html=renderToStaticMarkup(React.createElement(Card,{message:{id:"webhooks",kind:"approval",payload:{
        entry_id:"webhooks",status:{status:"pending"},route:{requesting_bot_id:"grokbot"},
        kind:{kind:"approval",tool:"routines_webhook_publish",expires_at:Date.now()+60000},
        arguments:{enabled},policy:{remember:false,scope:"none",pattern:"",label:"routines_webhook_publish"},
      }}}));
      assert.ok(html.includes(enabled?"Publier les webhooks sur Internet":"Retirer la publication des webhooks"));
      if(enabled){assert.ok(html.includes("Tailscale Funnel"));assert.ok(html.includes("jeton Bearer"));assert.ok(html.includes("iPhone"));}
      assert.ok(html.includes("Autoriser une fois"));assert.ok(!html.includes("Toujours autoriser"));assert.ok(!html.includes("agent de routines"));
    }
    const done=renderToStaticMarkup(React.createElement(Card,{message:{id:"webhooks",kind:"approval",payload:{
      entry_id:"webhooks",status:{status:"resolved"},route:{requesting_bot_id:"grokbot"},
      kind:{kind:"approval",tool:"routines_webhook_publish"},arguments:{enabled:true},
      configuration_result:{status:"failed",reason:"Le tailnet a refusé Funnel."},
    }}}));
    assert.ok(done.includes("Échec : Le tailnet a refusé Funnel."));
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

test("a background shell card stays En cours with Arrêter, then shows Terminée or Arrêtée", () => {
  const call = { call_id: "c", name: "run_shell", arguments: { command: "sleep 90 && echo fini", cwd: "/tmp", background: true, timeout_ms: 100 } };
  const html = (output) => renderToStaticMarkup(React.createElement(ToolCard, { message: { id: "m", kind: "tool", payload: { run_id: "run", call, outcome: { status: "done", output } } } }));
  const running = html({ status: "en cours", background: true, stdout: "partiel", stderr: "", exit_code: null, truncated: false });
  assert.match(running, /En cours/); assert.match(running, />Arrêter</); assert.ok(running.includes("partiel"));
  assert.match(running, /sans plafond de temps/); assert.ok(!running.includes("Code de sortie"));
  const done = html({ status: "terminé", background: true, stdout: "fini\n", stderr: "", exit_code: 0, truncated: false });
  assert.match(done, /Terminé/); assert.match(done, /Code de sortie : 0/); assert.ok(done.includes("fini")); assert.ok(!done.includes(">Arrêter</"));
  const stopped = html({ status: "arrêtée", background: true, stdout: "", stderr: "", exit_code: null, truncated: true });
  assert.match(stopped, /Arrêtée/); assert.match(stopped, /Sortie tronquée/); assert.ok(!stopped.includes("Code de sortie"));
  const failed = html({ status: "échec", background: true, stdout: "", stderr: "raté", exit_code: 3 });
  assert.match(failed, /Code de sortie non nul/); assert.match(failed, /Code de sortie : 3/);
});

test("only a running background card takes a line, and a refusal stays on the card", () => {
  const call = { call_id: "c", name: "run_shell", arguments: { command: "./demande.sh", cwd: "/tmp", background: true } };
  const html = (output) => renderToStaticMarkup(React.createElement(ToolCard, { message: { id: "m", kind: "tool", payload: { run_id: "run", call, outcome: { status: "done", output } } } }));
  const running = html({ status: "en cours", background: true, stdout: "Continuer ? ", stderr: "" });
  assert.match(running, /<input[^>]*aria-label="Saisie pour la tâche"/);
  assert.match(running, />Envoyer<\/button>/);
  assert.ok(running.indexOf(">Arrêter<") < running.indexOf("Saisie pour la tâche"));
  for (const status of ["terminé", "échec", "arrêtée"]) {
    const settled = html({ status, background: true, stdout: "", stderr: "", exit_code: status === "terminé" ? 0 : null });
    assert.ok(!settled.includes("Saisie pour la tâche"), status);
    assert.ok(!settled.includes(">Envoyer<"), status);
  }
  const foreground = renderToStaticMarkup(React.createElement(ToolCard, { message: { id: "f", kind: "tool", payload: { run_id: "run", call: { ...call, arguments: { ...call.arguments, background: false } }, outcome: { status: "done", output: { status: "terminé", exit_code: 0, stdout: "", stderr: "" } } } } }));
  assert.ok(!foreground.includes("Saisie pour la tâche"));
  const refused = renderToStaticMarkup(React.createElement(TaskInputView, { line: "hunter2", notice: "Ce programme demande un mot de passe : la saisie n’a pas été envoyée. Saisissez-le vous-même, hors d’Ezerbot.", sending: false }));
  assert.match(refused, /role="alert"/); assert.match(refused, /mot de passe : la saisie n’a pas été envoyée/);
  assert.match(refused, /value="hunter2"/);
  globalThis.window = { EzerbotIPhone: { remote: true } };
  try { assert.ok(!html({ status: "en cours", background: true, stdout: "", stderr: "" }).includes("Saisie pour la tâche"), "l’iPhone n’envoie pas de saisie"); }
  finally { delete globalThis.window; }
});

test("the task field empties after its line is sent but keeps what was typed meanwhile", () => {
  assert.equal(settleDraft("oui", "oui"), "");
  assert.equal(settleDraft("oui, et ensuite", "oui"), "oui, et ensuite");
  assert.equal(settleDraft("", ""), "");
});

test("interrupted tool cards are stopped, not left running or waiting", () => {
  const local = renderToStaticMarkup(React.createElement(ToolCard, { message: { id: "stopped", kind: "tool", payload: {
    run_id: "run", run_status: "interrupted", call: { call_id: "call", name: "run_shell", arguments: {command:"sleep 20",cwd:"/tmp"} },
  } } }));
  assert.doesNotMatch(local, /En cours/);
  assert.doesNotMatch(local, /Annuler l’exécution/);
  assert.match(local, /Arrêtée/);
  const generic = renderToStaticMarkup(React.createElement(ToolCard, { message: { id: "generic", kind: "tool", payload: {
    run_id: "run", run_status: "interrupted", call: { call_id: "call", name: "lookup_thing", arguments: {} },
  } } }));
  assert.doesNotMatch(generic, /Résultat en attente/);
  assert.match(generic, /Non exécuté/);
  globalThis.window = { addEventListener() {}, removeEventListener() {}, GrokbotEngine: { version: () => 1, run: () => ({ status: "interrupted" }) } };
  try {
    const live = renderToStaticMarkup(React.createElement(ToolCard, { message: { id: "live", kind: "tool", payload: {
      run_id: "run", run_status: "running", call: { call_id: "call", name: "lookup_thing", arguments: {} },
    } } }));
    assert.doesNotMatch(live, /Résultat en attente/);
    assert.match(live, /Non exécuté/);
  } finally { delete globalThis.window; }
});

test("an interrupted document card does not promise a later result", () => {
  const html = renderToStaticMarkup(React.createElement(ToolCard, { message: { id: "doc", kind: "tool", payload: {
    run_id: "run", run_status: "interrupted", call: { call_id: "call", name: "document_action", arguments: { action: "save", path: "/tmp/note.txt" } },
  } } }));
  assert.match(html, /Arrêtée/);
  assert.match(html, /Aucun résultat n’a été enregistré avant l’arrêt/);
  assert.doesNotMatch(html, /Le résultat sera affiché ici/);
  assert.doesNotMatch(html, /En cours/);
  assert.doesNotMatch(html, /Annuler l’exécution/);
});

test("generic approval retains its code identifier instead of a local-action label", () => {
  const html = card("pending", "ask", {}, false, "test_generic_tool");
  assert.match(html, /Outil : <code>test_generic_tool<\/code>/);
  assert.ok(!html.includes("Action : test_generic_tool"));
  assert.ok(!html.includes("data-follow-actions"));
});

test("a remembered shell command is listed by its text and revoked as that pattern", async () => {
  const rule = { ...standingRule("run_shell", true, "pattern"), args_digest: "printf oui" };
  const f = settingsPanel({ rules: [rule] });
  assert.match(ruleRows(f.root)[0].textContent, /Terminal sur votre Mac · Toujours autoriser · Cette commande · printf oui/);
  await button(ruleRows(f.root)[0], "Retirer la règle").listeners.click();
  assert.equal(f.requests[0].path, "/api/approvals/rules/revoke");
  assert.equal(f.requests[0].body.tool, "run_shell");
  assert.equal(f.requests[0].body.args_digest, "printf oui");
  assert.equal(f.requests[0].body.scope, "pattern");
});

function cardWithPolicy(policy, tool, args) {
  globalThis.window = { GrokbotForms: { view() {} }, GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(Card, { message: {
      id: "approval-policy", kind: "approval", payload: {
        entry_id: "approval-policy", status: { status: "pending" }, route: { requesting_bot_id: "grokbot" },
        kind: { kind: "approval", tool, expires_at: Date.now() + 60000 }, arguments: args, policy,
      },
    } }));
  } finally { delete globalThis.window; }
}

test("the approval card offers Toujours autoriser for a rememberable command", () => {
  const html = cardWithPolicy(
    { remember: true, scope: "pattern", pattern: "printf oui", label: "printf oui" },
    "run_shell",
    { command: "printf oui", cwd: "/tmp" },
  );
  assert.ok(html.includes('data-always-scope="pattern"'));
  assert.ok(html.indexOf("Toujours autoriser") >= 0);
  assert.ok(html.indexOf("Toujours autoriser") < html.indexOf("Mémoriser une règle"));
  assert.ok(html.includes("Autoriser une fois"));
});

test("a background task card names the bot, the action and the shortened prompt", () => {
  const summary = `${"Préparer le dossier ".repeat(12)}… (raccourci)`;
  const base = {
    entry_id: "approval-bg", status: { status: "pending" }, route: { requesting_bot_id: "grokbot" },
    kind: { kind: "approval", tool: "delegate", expires_at: Date.now() + 60000 },
    arguments: { objective: "Préparer le dossier" },
    policy: { remember: true, scope: "action", pattern: "", label: "Exécuter une tâche sur l'ordinateur" },
    background_task: { kind: "Exécuter une tâche sur l'ordinateur", summary, bot_name: "Ezerbot" },
  };
  const render = (payload) => {
    globalThis.window = { GrokbotForms: { view() {} }, GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
    try {
      return renderToStaticMarkup(React.createElement(Card, { message: { id: "approval-bg", kind: "approval", payload } }));
    } finally { delete globalThis.window; }
  };
  const pending = render(base);
  assert.ok(pending.includes("Autoriser Ezerbot à exécuter une tâche en arrière-plan ?"));
  assert.ok(pending.includes("gb-background-kind"));
  assert.match(pending, /Exécuter une tâche sur l(?:'|&#x27;)ordinateur/);
  assert.ok(pending.includes("… (raccourci)"));
  assert.ok(pending.includes("Autoriser une fois"));
  assert.ok(pending.includes('data-always-scope="global"'));
  assert.ok(pending.includes("Toujours autoriser"));
  assert.ok(pending.includes("Refuser"));
  assert.ok(!pending.includes("gb-status-ok"));
  const once = render({ ...base, status: { status: "resolved" }, decision: "allow_once" });
  assert.ok(once.includes("gb-status-ok"));
  assert.ok(once.includes("Autorisé une fois"));
  assert.ok(!once.includes("Toujours autoriser"));
  const always = render({ ...base, status: { status: "resolved" }, decision: { always_allow: { scope: "global" } } });
  assert.ok(always.includes("gb-status-ok"));
  assert.ok(always.includes("Toujours autorisé"));
  const refused = render({ ...base, status: { status: "denied" }, decision: "deny" });
  assert.ok(refused.includes("gb-status-alert"));
  assert.ok(refused.includes("Refusé"));
  assert.ok(!refused.includes("gb-status-ok"));
  const protectedLaunch = render({
    ...base,
    policy: { remember: false, protection: "destructive", scope: "none", pattern: "", label: "Suppression à confirmer" },
    background_task: undefined,
  });
  assert.ok(protectedLaunch.includes("Confirmation d’une suppression"));
  assert.ok(!protectedLaunch.includes("Toujours autoriser"));
});

test("a background computer rule is revoked from Settings as that action type", async () => {
  const rule = { ...standingRule("background_computer", true, "global"), args_digest: "" };
  const f = settingsPanel({ rules: [rule] });
  assert.match(ruleRows(f.root)[0].textContent, /Tâche en arrière-plan · Exécuter une tâche sur l'ordinateur/);
  assert.match(ruleRows(f.root)[0].textContent, /Ce type d’action/);
  await button(ruleRows(f.root)[0], "Retirer la règle").listeners.click();
  assert.equal(f.requests[0].path, "/api/approvals/rules/revoke");
  assert.equal(f.requests[0].body.tool, "background_computer");
  assert.equal(f.requests[0].body.args_digest, "");
  assert.equal(f.requests[0].body.scope, "global");
  assert.equal(f.requests[0].body.requesting_bot_id, "grokbot");
});

test("outbound sends and deletions stay on a confirmation card without Toujours autoriser", () => {
  const outbound = cardWithPolicy(
    { remember: false, protection: "outbound", scope: "none", pattern: "", label: "Envoi sous votre nom" },
    "mcp_gmail_send",
    { to: "a@exemple.fr" },
  );
  assert.ok(outbound.includes("Envoi sous votre nom"));
  assert.ok(outbound.includes("Cet envoi part sous votre nom."));
  assert.ok(!outbound.includes("Toujours autoriser"));
  const removal = cardWithPolicy(
    { remember: false, protection: "destructive", scope: "none", pattern: "", label: "Suppression à confirmer" },
    "run_shell",
    { command: "rm note", cwd: "/tmp" },
  );
  assert.ok(removal.includes("Confirmation d’une suppression"));
  assert.ok(removal.includes("Cette suppression demande une confirmation à chaque fois."));
  assert.ok(!removal.includes("Toujours autoriser"));
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

test("resolved permission cards show the green decision for every action type", () => {
  globalThis.window = { GrokbotForms: { view() {} } };
  try {
    for (const tool of ["test_generic_tool", "run_shell", "mcp_start_test", "routines_background_configure", "routines_webhook_publish", "propose_skill"]) {
      for (const [decision, label] of [["allow_once", "Autorisé une fois"], [{ always_allow: { scope: "global" } }, "Toujours autorisé"]]) {
        const html = renderToStaticMarkup(React.createElement(Card, { message: {
          id: "allowed", kind: "approval", payload: {
            entry_id: "allowed", route: { requesting_bot_id: "bot" },
            status: { status: "resolved" }, kind: { kind: "approval", tool }, decision,
          },
        } }));
        assert.match(html, new RegExp(`class="gb-status-ok"[^>]*>${label}<`), tool);
      }
    }
  } finally { delete globalThis.window; }
});

test("automatic permission grants show a green one-time authorization", () => {
  assert.match(card("resolved", "allow"), /class="gb-status-ok"[^>]*>Autorisé une fois</);
});

test("screen grants show their one-time authorization in green inside the muted footer", () => {
  const html = renderToStaticMarkup(React.createElement(ScreenCard, { message: {
    id: "screen-allowed", payload: { approval_status: "resolved", run_status: "running", call: { name: "computer_click", arguments: { x: 1, y: 2 } } },
  } }));
  assert.match(html, /<footer class="screen-decision"><span class="gb-status-ok"[^>]*>Autorisé une fois<\/span><\/footer>/);
});
