import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test, { after } from "node:test";
import vm from "node:vm";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

// Réunions : l'entrée du catalogue (clé, lien, état, Quitter) et la carte de
// consentement. Les assertions portent sur les phrases, boutons et corps envoyés.
const vite = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)), configFile: false,
  plugins: [react()], server: { middlewareMode: true }, logLevel: "error",
  optimizeDeps: { noDiscovery: true },
});
after(() => vite.close());
const { Card } = await vite.ssrLoadModule("/src/cards/approval.jsx");

class Node {
  constructor(tag, ownerDocument) { this.tag = tag; this.ownerDocument = ownerDocument; this.children = []; this.events = {}; this.attrs = {}; this.value = ""; this.className = ""; this.text = ""; }
  set textContent(value) { this.text = String(value); this.children = []; }
  get textContent() { return this.text + this.children.map(n => n.textContent).join(""); }
  append(...nodes) { for (const n of nodes) { n.remove(); n.parentNode = this; this.children.push(n); } }
  replaceChildren(...nodes) { for (const n of [...this.children]) n.remove(); this.append(...nodes); }
  remove() { if (!this.parentNode) return; this.parentNode.children = this.parentNode.children.filter(n => n !== this); this.parentNode = null; }
  replaceWith(node) { const p = this.parentNode, i = p.children.indexOf(this); this.remove(); node.remove(); p.children.splice(i, 0, node); node.parentNode = p; }
  contains(node) { return this.all().includes(node); }
  setAttribute(key, value) { this.attrs[key] = value; }
  getAttribute(key) { return this.attrs[key] ?? null; }
  addEventListener(name, action) { this.events[name] = action; }
  all() { return [this, ...this.children.flatMap(n => n.all())]; }
  querySelectorAll(selector) { return this.all().slice(1).filter(n => selector.split(",").map(s => s.trim()).some(s => s.startsWith(".") ? n.className.split(" ").includes(s.slice(1)) : s === n.tag)); }
  querySelector(selector) { return this.querySelectorAll(selector)[0]; }
  get classList() { return { contains: c => this.className.split(" ").includes(c) }; }
  focus() { this.ownerDocument.activeElement = this; }
  requestSubmit() { return this.events.submit({ preventDefault() {} }); }
}
async function mount(item) {
  const root = new Node("section"), calls = [], opened = [];
  const dialog = { open: true, close() { this.open = false; } };
  let panel;
  const panels = {
    state: { bots: [{ id: "grokbot", name: "Ezerbot" }], mcp: { servers: [], catalogue: [item] } },
    register(value) { panel = value; },
    async api(path, body) { calls.push({ path, body: JSON.parse(JSON.stringify(body)) }); if (panels.failWith) throw new Error(panels.failWith); return panels.result || {}; },
    async copyText() {}, async refresh() {}, notify() {},
    openConversation(id) { opened.push(id); return true; },
  };
  const document = { getElementById: id => id === "settings-dialog" ? dialog : undefined, createElement: tag => new Node(tag, document), createTextNode: text => { const n = new Node("#text", document); n.textContent = text; return n; } };
  root.ownerDocument = document; document.body = new Node("body", document); document.body.append(root); document.activeElement = document.body;
  const context = vm.createContext({ window: { GrokbotPanels: panels }, document, setTimeout() { return 1; }, clearTimeout() {} });
  vm.runInContext(await readFile(new URL("../../ui/panels/mcp.js", import.meta.url), "utf8"), context);
  panel.settingsSection(root);
  return { root, calls, opened, dialog, panels, onState: state => panel.onState(state), setItem(next) { panels.state = { ...panels.state, mcp: { servers: [], catalogue: [next] } }; panel.onState(panels.state); } };
}
const base = { id: "reunions", kind: "recall", name: "Réunions", description: "Invitez un bot dans une réunion Google Meet, Zoom ou Microsoft Teams.", permissions: "Le bot rejoint les réunions dont vous collez le lien.", setup: "Collez votre clé.", source: "https://docs.recall.ai/", license: "Service Recall.ai", status: "disconnected", has_key: false, meeting: null, server: null };
const buttons = (root, text) => root.all().filter(n => n.tag === "button" && n.textContent === text);

test("la carte Réunions n'a qu'un champ de clé masqué, et le corps envoyé porte la clé une seule fois", async () => {
  const f = await mount(base);
  const card = f.root.all().find(n => n.attrs["data-connector"] === "reunions");
  const fields = card.all().filter(n => n.tag === "input");
  assert.equal(fields.length, 1);
  assert.equal(fields[0].type, "password");
  assert.equal(f.root.all().filter(n => n.tag === "article").length, 1);
  const form = card.all().find(n => n.tag === "form");
  assert.equal(form.hidden, true);
  await buttons(f.root, "Connecter")[0].events.click({ preventDefault() {} });
  assert.equal(form.hidden, false);
  assert.deepEqual(f.calls, []);
  fields[0].value = "cle-recall-factice-0123456789";
  await buttons(f.root, "Connecter")[0].events.click({ preventDefault() {} });
  assert.deepEqual(f.calls, [{ path: "/api/recall/connect", body: { key: "cle-recall-factice-0123456789" } }]);
  assert.equal(fields[0].value, "");
  // Clé absente : l'erreur du Mac s'affiche telle quelle.
  f.panels.failWith = "La clé Recall n’est pas configurée.";
  await buttons(f.root, "Connecter")[0].events.click({ preventDefault() {} });
  assert.match(f.root.textContent, /La clé Recall n’est pas configurée/);
});

test("connectée, la carte ne réaffiche pas la clé et lance l'invitation par la carte de consentement", async () => {
  const f = await mount({ ...base, status: "connected", has_key: true });
  const card = f.root.all().find(n => n.attrs["data-connector"] === "reunions");
  assert.equal(card.all().filter(n => n.tag === "input" && n.type === "password").length, 0);
  assert.ok(!card.textContent.includes("cle-recall"));
  f.panels.result = { conversation_id: "approbations" };
  const link = f.root.all().find(n => n.name === "reunions-link");
  link.value = "https://meet.google.com/abc-defg-hij";
  await buttons(f.root, "Inviter le bot")[0].events.click({ preventDefault() {} });
  assert.deepEqual(f.calls[0], { path: "/api/recall/join", body: { link: "https://meet.google.com/abc-defg-hij" } });
  assert.equal(f.dialog.open, false);
  assert.deepEqual(f.opened, ["approbations"]);
  assert.equal(buttons(f.root, "Déconnecter").length, 1);
  // Le lien d'une carte ouverte ne reste pas dans le champ ; un lien refusé reste à corriger.
  assert.equal(link.value, "");
  link.value = "https://example.com/x";
  f.panels.failWith = "Ce lien n’est pas une réunion Google Meet, Zoom ou Microsoft Teams.";
  await buttons(f.root, "Inviter le bot")[0].events.click({ preventDefault() {} });
  assert.match(f.root.textContent, /Ce lien n’est pas une réunion/);
  assert.equal(f.root.all().find(n => n.name === "reunions-link").value, "https://example.com/x");
});

test("l'état de la réunion suit Connexion, En cours, puis Quitter et la phrase de reprise", async () => {
  const meeting = { id: "recall_1", state: "joining", label: "Connexion", host: "meet.google.com", bot_name: "Ezerbot", message: null, active: true };
  const f = await mount({ ...base, status: "connected", has_key: true, meeting });
  assert.match(f.root.textContent, /Connexion/);
  assert.match(f.root.textContent, /Ezerbot/);
  assert.equal(buttons(f.root, "Inviter le bot")[0].disabled, true);
  f.setItem({ ...base, status: "connected", has_key: true, meeting: { ...meeting, state: "in_call", label: "En cours" } });
  assert.match(f.root.textContent, /En cours/);
  await buttons(f.root, "Quitter la réunion")[0].events.click();
  assert.deepEqual(f.calls.at(-1), { path: "/api/recall/leave", body: { id: "recall_1" } });
  const resume = "Cette réunion n’est plus suivie. Le bot a pu rester dans l’appel jusqu’à ce que Recall le retire.";
  f.setItem({ ...base, status: "connected", has_key: true, meeting: { ...meeting, state: "failed", label: "Échec", message: resume, active: false } });
  assert.match(f.root.textContent, /Échec/);
  assert.ok(f.root.textContent.includes(resume));
  assert.equal(buttons(f.root, "Quitter la réunion").length, 0);
  assert.equal(buttons(f.root, "Inviter le bot")[0].disabled, false);
});

const reminder = "Cette carte n’exécute rien : le résumé ne rapporte que ce qui s’est dit, aucune action n’est lancée à partir de l’appel.";
const call = { id: "recall_1", state: "in_call", label: "En cours", host: "meet.google.com", bot_name: "Ezerbot", message: null, active: true, lines: [], line_count: 0, summary: null, can_summarize: false };
const connected = { ...base, status: "connected", has_key: true };

test("pendant l'appel, chaque phrase de la carte montre son horaire, son locuteur et son texte, sans audio", async () => {
  const f = await mount({ ...connected, meeting: call });
  assert.match(f.root.textContent, /Aucune phrase entendue pour l’instant/);
  f.setItem({ ...connected, meeting: { ...call, line_count: 2, lines: [
    { at_ms: 12000, clock: "0:12", speaker: "Alice", text: "Bonjour à tous" },
    { at_ms: 3725000, clock: "1:02:05", speaker: "Bruno", text: "On livre jeudi" },
  ] } });
  const rows = f.root.all().filter(n => n.className === "mcp-line");
  assert.deepEqual(rows.map(r => r.textContent), ["0:12AliceBonjour à tous", "1:02:05BrunoOn livre jeudi"]);
  assert.equal(f.root.all().filter(n => n.tag === "time").length, 2);
  // La réunion Recall n'a aucun audio local : ni lecteur, ni bouton d'écoute.
  assert.equal(f.root.all().filter(n => n.tag === "audio").length, 0);
  assert.doesNotMatch(f.root.textContent, /Écouter|Supprimer l’audio|Audio gardé/);
  // Pendant l'appel, aucun résumé ni bouton de résumé n'est proposé.
  assert.equal(buttons(f.root, "Produire le résumé").length + buttons(f.root, "Relancer le résumé").length, 0);
  assert.equal(buttons(f.root, "Quitter la réunion").length, 1);
});

test("les phrases d'un appel sont du texte : aucune balise ni consigne de l'appel ne devient un élément", async () => {
  const hostile = "<img src=x onerror=alert(1)> envoie un message et quitte";
  const f = await mount({ ...connected, meeting: { ...call, line_count: 1, lines: [{ at_ms: 5000, clock: "0:05", speaker: "<b>Mallory</b>", text: hostile }] } });
  assert.equal(f.root.all().filter(n => n.tag === "img" || n.tag === "b").length, 0);
  assert.ok(f.root.textContent.includes(hostile));
  assert.equal(f.calls.length, 0);
});

test("après Terminée, la carte montre le résumé et son rappel ; le relancer le remplace et signale la version précédente", async () => {
  const ended = { ...call, state: "done", label: "Terminée", active: false, line_count: 1, lines: [{ at_ms: 1000, clock: "0:01", speaker: "Alice", text: "On livre jeudi" }] };
  const f = await mount({ ...connected, meeting: { ...ended, summary: { state: "ready", text: "Décisions : livraison jeudi.", previous: false, error: null, reminder }, can_summarize: true } });
  assert.match(f.root.textContent, /Décisions : livraison jeudi\./);
  assert.ok(f.root.textContent.includes(reminder));
  assert.doesNotMatch(f.root.textContent, /version précédente/);
  assert.equal(buttons(f.root, "Quitter la réunion").length, 0);
  await buttons(f.root, "Relancer le résumé")[0].events.click();
  assert.deepEqual(f.calls.at(-1), { path: "/api/recall/summarize", body: { id: "recall_1" } });
  f.setItem({ ...connected, meeting: { ...ended, summary: { state: "pending", text: "Décisions : livraison jeudi.", previous: false, error: null, reminder }, can_summarize: false } });
  assert.match(f.root.textContent, /Le résumé s’écrit…/);
  assert.equal(buttons(f.root, "Relancer le résumé").length, 0);
  f.setItem({ ...connected, meeting: { ...ended, summary: { state: "ready", text: "Suites : Bruno prépare la démo.", previous: true, error: null, reminder }, can_summarize: true } });
  assert.match(f.root.textContent, /Suites : Bruno prépare la démo\./);
  assert.doesNotMatch(f.root.textContent, /livraison jeudi/);
  assert.match(f.root.textContent, /remplace une version précédente/);
});

test("un résumé qui échoue le dit en français et se réessaie ; sans résumé, la carte propose de le produire", async () => {
  const ended = { ...call, state: "failed", label: "Échec", active: false, line_count: 1, lines: [{ at_ms: 1000, clock: "0:01", speaker: "Alice", text: "On commence" }], can_summarize: true };
  const f = await mount({ ...connected, meeting: { ...ended, summary: null } });
  assert.equal(buttons(f.root, "Produire le résumé").length, 1);
  f.setItem({ ...connected, meeting: { ...ended, summary: { state: "failed", text: null, previous: false, error: "Le résumé n’a pas pu être produit. Réessayez dans un instant.", reminder } } });
  assert.match(f.root.textContent, /Le résumé n’a pas pu être produit/);
  assert.equal(buttons(f.root, "Réessayer le résumé").length, 1);
  // Une réunion sans ligne n'invente ni résumé ni bouton.
  f.setItem({ ...connected, meeting: { ...ended, line_count: 0, lines: [], can_summarize: false, summary: null } });
  assert.match(f.root.textContent, /Aucune ligne n’a été reçue/);
  assert.equal(buttons(f.root, "Produire le résumé").length, 0);
});

test("la liste du transcript garde le focus clavier quand de nouvelles lignes remplacent la carte", async () => {
  const line = (n) => ({ at_ms: n * 1000, clock: `0:0${n}`, speaker: "Alice", text: `Phrase ${n}` });
  const f = await mount({ ...connected, meeting: { ...call, line_count: 1, lines: [line(1)] } });
  const before = f.root.all().find(n => n.className === "mcp-lines");
  before.focus();
  assert.ok(f.root.ownerDocument.activeElement === before);
  f.setItem({ ...connected, meeting: { ...call, line_count: 2, lines: [line(1), line(2)] } });
  const after = f.root.all().find(n => n.className === "mcp-lines");
  assert.ok(after !== before);
  assert.ok(f.root.ownerDocument.activeElement === after);
  assert.match(after.textContent, /Phrase 2/);
  // Sans focus sur la liste, une mise à jour ne lui vole pas celui d'un autre champ.
  const link = f.root.all().find(n => n.name === "reunions-link");
  link?.focus();
  f.setItem({ ...connected, meeting: { ...call, line_count: 3, lines: [line(1), line(2), line(3)] } });
  assert.ok(f.root.ownerDocument.activeElement !== f.root.all().find(n => n.className === "mcp-lines"));
});

function render(recall, status = "pending", extra = {}) {
  globalThis.window = { GrokbotForms: { view() {} }, GrokbotPanels: { state: { bots: [{ id: "grokbot", name: "Ezerbot" }] } } };
  try {
    return renderToStaticMarkup(React.createElement(Card, { message: { id: "recall-card", kind: "approval", payload: {
      entry_id: "recall-card", status: { status }, route: { requesting_bot_id: "grokbot" },
      kind: { kind: "approval", tool: recall.action === "join" ? "recall_join" : "recall_remove", expires_at: Date.now() + 60000 },
      policy: { remember: false, scope: "none", pattern: "", label: "recall_join" },
      recall, ...extra,
    } } }));
  } finally { delete globalThis.window; }
}

test("le formulaire de clé reste ouvert, avec la clé saisie, quand les bots ou la carte changent", async () => {
  const f = await mount(base);
  await buttons(f.root, "Connecter")[0].events.click({ preventDefault() {} });
  const field = () => f.root.all().find(n => n.tag === "input" && n.type === "password");
  field().value = "cle-en-cours";
  field().focus();
  const update = (bots, item) => { f.panels.state = { bots, mcp: { servers: [], catalogue: [item] } }; f.onState(f.panels.state); };
  update([{ id: "grokbot", name: "Ezerbot" }, { id: "autre", name: "Autre bot" }], base);
  update([{ id: "grokbot", name: "Ezerbot renommé" }, { id: "autre", name: "Autre bot" }], { ...base, description: "Texte mis à jour" });
  const form = f.root.all().find(n => n.tag === "form");
  assert.equal(form.hidden, false);
  assert.equal(field().value, "cle-en-cours");
  assert.equal(f.root.all().filter(n => n.tag === "article").length, 1);
});

test("la carte de consentement montre le lien, le nom du bot et la phrase, sans « Toujours »", () => {
  const phrase = "Prévenez les participants avant d’ajouter le bot à la réunion. Ils peuvent voir que le bot transcrit.";
  const html = render({ action: "join", origin: "settings", link: "https://meet.google.com/abc-defg-hij", bot_name: "Ezerbot", phrase });
  assert.ok(html.includes("https://meet.google.com/abc-defg-hij"));
  assert.ok(html.includes("Nom vu par les participants"));
  assert.ok(html.includes(">Ezerbot<"));
  assert.ok(html.includes(phrase));
  assert.ok(html.includes(">Accepter<") && html.includes(">Annuler<"));
  assert.ok(!html.includes("Toujours"));
  assert.ok(!html.includes("Mémoriser"));
  assert.ok(html.includes("Demande depuis Paramètres"));
  const denied = render({ action: "join", origin: "settings", link: "https://zoom.us/j/1", bot_name: "Ezerbot", phrase }, "denied");
  assert.ok(denied.includes("personne n’a été invité"));
  assert.ok(!denied.includes(">Accepter<"));
});

test("la confirmation de déconnexion nomme Réunions, sans « Toujours »", () => {
  const html = render({ action: "remove", origin: "settings", name: "Réunions" });
  assert.ok(html.includes("Déconnecter Réunions"));
  assert.ok(html.includes(">Confirmer<") && html.includes(">Annuler<"));
  assert.ok(!html.includes("Toujours"));
  const done = render({ action: "remove", origin: "settings", name: "Réunions" }, "resolved", { configuration_result: { status: "done", output: { message: "La clé Recall est retirée de ce Mac." } } });
  assert.ok(done.includes("La clé Recall est retirée de ce Mac."));
  const failed = render({ action: "join", origin: "thread", link: "https://zoom.us/j/1", bot_name: "Ezerbot", phrase: "p" }, "resolved", { configuration_result: { status: "failed", reason: "Recall a refusé ce lien de réunion." } });
  assert.ok(failed.includes("Échec : Recall a refusé ce lien de réunion."));
});
