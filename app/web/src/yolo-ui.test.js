import assert from "node:assert/strict";
import { after, test } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createTestServer } from "./vite-test-server.js";
import react from "@vitejs/plugin-react";
import { automaticLine, isAutomatic, sourceLabel } from "./yolo-view.js";

const vite = await createTestServer({ plugins: [react()] });
after(() => vite.close());
const { Card } = await vite.ssrLoadModule("/src/cards/tool.jsx");

const yolo = { kind: "yolo", source: "global" };
const render = (payload) => renderToStaticMarkup(React.createElement(Card, { message: { id: "m", kind: "tool", payload: { run_status: "running", ...payload } } }));
const shell = { call_id: "c", name: "run_shell", arguments: { command: "ls -la ~/Documents", cwd: "/Users/vous", timeout_ms: 30000, output_limit: 131072 } };
const done = (output) => ({ status: "done", output });

test("une commande de terminal auto-approuvée devient une carte compacte « Fait automatiquement »", () => {
  const html = render({ call: shell, authorized_by: yolo, outcome: done({ status: "terminé", exit_code: 0, stdout: "a\nb", stderr: "" }) });
  assert.match(html, /gb-auto-card/);
  assert.match(html, /class="gb-auto-badge">Fait automatiquement</);
  assert.match(html, /Terminal sur votre Mac/);
  assert.match(html, /Commande exécutée : ls -la ~\/Documents</);
  assert.match(html, /Code de sortie : 0/);
  assert.match(html, /title="Réglage global"/);
  assert.match(html, /Afficher la sortie/);
  assert.doesNotMatch(html, /Annuler l’exécution/);
});

test("une écriture de fichier auto-approuvée dit ce qui a été écrit", () => {
  const html = render({
    call: { call_id: "w", name: "write_file", arguments: { path: "/Users/vous/notes.txt", content: "bonjour", expected_sha256: null } },
    authorized_by: { kind: "yolo", source: "bot" },
    outcome: done({ path: "/Users/vous/notes.txt", bytes: 7, sha256: "a".repeat(64) }),
  });
  assert.match(html, /Fait automatiquement/);
  assert.match(html, /Fichier créé : \/Users\/vous\/notes\.txt · 7 octets/);
  assert.match(html, /title="Réglage de ce bot"/);
});

test("une commande en cours, une tâche de fond et un échec restent lisibles", () => {
  const running = render({ call: shell, authorized_by: yolo });
  assert.match(running, /Commande en cours : ls -la/);
  assert.match(running, /Annuler l’exécution/);
  const background = render({
    call: { ...shell, arguments: { ...shell.arguments, background: true } }, run_id: "run", authorized_by: yolo,
    outcome: done({ status: "en cours", background: true }),
  });
  assert.match(background, /Commande lancée en tâche de fond/);
  assert.match(background, />Arrêter</);
  const failed = render({ call: shell, authorized_by: yolo, outcome: { status: "failed", reason: "Mode YOLO : les données et les identifiants d’Ezerbot restent hors de portée des outils." } });
  assert.match(failed, /Commande non exécutée : ls -la/);
  assert.match(failed, /hors de portée des outils/);
});

test("un connecteur auto-approuvé affiche le serveur et l’outil dans une carte repliée", () => {
  const html = render({
    call: { call_id: "mcp", name: "mcp_notion_abc", arguments: { query: "x" } }, authorized_by: yolo,
    connector: { server: "Notion", tool: "search" }, outcome: done({ content: [{ type: "text", text: "trois pages" }] }),
  });
  assert.match(html, /Fait automatiquement · Appel terminé/);
  assert.match(html, /Appel effectué : Notion · search/);
});

test("une carte non automatique ne porte aucune mention « Fait automatiquement »", () => {
  const manual = render({ call: shell, authorized_by: { kind: "allow_once", request_id: "a" }, outcome: done({ status: "terminé", exit_code: 0, stdout: "", stderr: "" }) });
  assert.doesNotMatch(manual, /Fait automatiquement/);
  assert.doesNotMatch(manual, /gb-auto-card/);
  assert.match(manual, /class="gb-local-command"/);
  const rule = render({ call: shell, authorized_by: { kind: "rule", rule_id: "r" }, outcome: done({ status: "terminé", exit_code: 0, stdout: "", stderr: "" }) });
  assert.match(rule, /règle « Toujours »/);
  assert.doesNotMatch(rule, /Fait automatiquement/);
});

test("un geste d’écran auto-approuvé n’attend pas une carte d’approbation pour s’afficher", () => {
  const call = { call_id: "s", name: "computer_screenshot", arguments: {} };
  const html = render({ call, authorized_by: yolo });
  assert.match(html, /Fait automatiquement/);
  assert.match(html, /Geste en cours d’exécution/);
  const manual = render({ call, approval_status: "pending" });
  assert.equal(manual, "");
});

test("les descriptions viennent des arguments de l’appel et gardent un texte inerte", () => {
  assert.equal(isAutomatic(yolo), true);
  assert.equal(isAutomatic({ kind: "rule" }), false);
  assert.equal(isAutomatic(undefined), false);
  assert.equal(sourceLabel({ source: "bot" }), "Réglage de ce bot");
  assert.equal(sourceLabel({ source: "global" }), "Réglage global");
  assert.equal(automaticLine({ call: { name: "read_file", arguments: { path: "/a/b" } }, outcome: done({}) }), "Fichier lu : /a/b");
  assert.equal(automaticLine({ call: { name: "list_directory", arguments: { path: "/a" } }, outcome: done({}) }), "Dossier listé : /a");
  assert.equal(automaticLine({ call: { name: "delegate", arguments: { objective: "Trier   la\nboîte" } }, outcome: done({}) }), "Tâche déléguée : Trier la boîte");
  assert.equal(automaticLine({ call: { name: "run_shell", arguments: { command: "x".repeat(300) } }, outcome: done({ status: "terminé" }) }).length < 200, true);
  assert.equal(automaticLine({ call: { name: "run_shell", arguments: null } }), "Commande en cours : ");
  assert.match(renderToStaticMarkup(React.createElement("p", null, automaticLine({ call: { name: "run_shell", arguments: { command: "<script>alert(1)</script>" } } }))), /&lt;script&gt;/);
});
