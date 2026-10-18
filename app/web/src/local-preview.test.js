import assert from "node:assert/strict";
import { test } from "node:test";
import { createElement as h } from "react";
import { renderToStaticMarkup as render } from "react-dom/server";
import { LocalApprovalPreview, LocalApprovalWarning, localApprovalBlocked } from "./local-approval.js";
import { LocalResult } from "./local-result.js";
import { localStatusLabel, localFailureView, localLimits } from "./local-call.js";

const args = { path: "/tmp/existant.txt", content: "un\ndeux\nNOUVEAU\nquatre\n", expected_sha256: "a".repeat(64), apercu_local: {
  lignes_ajoutees: 1, lignes_supprimees: 1, lignes_proposees: 4,
  extrait: " un\n deux\n-trois\n+NOUVEAU\n quatre\n", empreinte_actuelle_ok: true,
} };
test("replacement approval renders actual removed/added lines and exact counts", () => {
  const html=render(h(LocalApprovalPreview,{tool:"write_file",arguments:args}));
  assert.match(html,/Diff : \+1 ligne ajoutée \/ −1 ligne supprimée · 4 lignes proposées/);
  assert.ok(html.includes("-trois\n"));assert.ok(html.includes("+NOUVEAU\n"));
  assert.equal(localApprovalBlocked("write_file",args),false);
  const stale={...args,apercu_local:{...args.apercu_local,empreinte_actuelle_ok:false}};
  assert.equal(localApprovalBlocked("write_file",stale),true);
  assert.match(render(h(LocalApprovalPreview,{tool:"write_file",arguments:stale})),/empreinte ne correspond plus/);
  assert.equal(localApprovalBlocked("write_file",{path:"/tmp/ancien"}),true);
});
test("empty creation is zero lines and cached reads are described honestly",()=>{
  const empty={path:"/tmp/vide",content:"",expected_sha256:null,apercu_local:{lignes_ajoutees:0,lignes_supprimees:0,lignes_proposees:0}};
  const creation=render(h(LocalApprovalPreview,{tool:"write_file",arguments:empty}));
  assert.match(creation,/0 ligne proposée/);assert.ok(!creation.includes("−0"));
  const cached={...args,apercu_local:{...args.apercu_local,source:"lecture_approuvee",empreinte_actuelle_ok:null}};
  assert.match(render(h(LocalApprovalPreview,{tool:"write_file",arguments:cached})),/dernière lecture autorisée/);
});
test("read approval shows the real symlink target as escaped text",()=>{
  const html=render(h(LocalApprovalPreview,{tool:"read_file",arguments:{path:"/tmp/accorde/lien",apercu_local:{cible_reelle:"/tmp/hors/<secret>",hors_dossiers_accordes:true}}}));
  assert.match(html,/Lien symbolique →/);assert.match(html,/hors des dossiers accordés/);
  assert.ok(html.includes("&lt;secret&gt;"));assert.ok(!html.includes("<secret>"));
  const protectedHtml=render(h(LocalApprovalPreview,{tool:"read_file",arguments:{path:"/tmp/app/identifiants",apercu_local:{donnees_ezerbot:true}}}));
  assert.match(protectedHtml,/identifiants, historique/);assert.match(protectedHtml,/Refusez sauf certitude/);
  assert.match(protectedHtml,/toujours une carte individuelle/);assert.ok(!protectedHtml.includes("Pour éviter ces cartes"));
});
test("write previews show a canonical parent destination and app-data warning",()=>{
  const html=render(h(LocalApprovalPreview,{tool:"write_file",arguments:{...args,path:"/tmp/lien/note.txt",apercu_local:{...args.apercu_local,cible_reelle:"/tmp/app/<note>.txt",donnees_ezerbot:true}}}));
  assert.match(html,/Destination réelle/);assert.ok(html.includes("/tmp/app/&lt;note&gt;.txt"));
  assert.match(html,/identifiants, historique/);assert.match(html,/Refusez sauf certitude/);
});

test("global shell and file permissions carry specific warnings",()=>{
  assert.match(render(h(LocalApprovalWarning,{tool:"run_shell",scope:"global"})),/exécuter toute commande, sans sandbox/);
  assert.match(render(h(LocalApprovalWarning,{tool:"write_file",scope:"global"})),/créer ou remplacer tout fichier/);
  assert.equal(render(h(LocalApprovalWarning,{tool:"run_shell",scope:"subject"})),"");
});
test("file results render readable content, names/types and write metadata, not JSON",()=>{
  const read=render(h(LocalResult,{name:"read_file",output:{content:"première\nseconde\n<script>",sha256:"123"}}));
  assert.ok(read.includes("première\nseconde\n&lt;script&gt;"));assert.match(read,/Empreinte/);assert.ok(!read.split("<details>")[0].includes('"content"'));
  const list=render(h(LocalResult,{name:"list_directory",output:{entries:[{name:"a.txt",kind:"fichier"}],truncated:true}}));
  assert.match(list,/a.txt — fichier/);assert.match(list,/Liste limitée/);
  const write=render(h(LocalResult,{name:"write_file",output:{bytes:42,sha256:"123"}}));
  assert.match(write,/42 octets écrits/);assert.ok(!write.split("<details>")[0].includes('"bytes"'));
});
test("failures distinguish refusal, missing files and unknown external effects",()=>{
  assert.equal(localFailureView("L’approbation a été refusée", "denied").label,"Refusée");
  assert.equal(localFailureView("Fichier ou dossier introuvable.").label,"Erreur");
  for(const reason of ["La demande a été refusée, a expiré ou a été annulée.","Une autorisation permanente refuse cette action."]) assert.equal(localFailureView(reason).label,"Refusée");
  const reviewed=localFailureView("La règle interdit cette suppression.","denied");
  assert.equal(reviewed.label,"Refusée");assert.match(reviewed.message,/La règle interdit cette suppression/);
  const disabled=localFailureView("Cet outil est désactivé pour ce bot.","cancelled","cancelled");
  assert.equal(disabled.label,"Désactivé");assert.match(disabled.message,/désactivé/);
  const unknown=localFailureView("Résultat inconnu après interruption. Vérifiez son effet.");
  assert.equal(unknown.label,"Résultat inconnu");assert.equal(unknown.tone,"warning");assert.match(unknown.message,/Vérifiez/);
  assert.equal(localLimits({timeout_ms:100,output_limit:1024}),"Délai : 0,1 s · Sortie cumulée : 1 Kio maximum");
  assert.equal(localLimits({}),"Délai : 30 s · Sortie cumulée : 128 Kio maximum");
});
test("all shell status labels are consistent French headings",()=>{
  for(const status of ["terminé","échec","annulé","délai dépassé"]) assert.match(localStatusLabel(status),/^[A-ZÉ]/);
  assert.equal(localStatusLabel("annulé"),"Annulée");assert.equal(localStatusLabel("délai dépassé"),"Délai dépassé");
});
