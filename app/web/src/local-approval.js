import { createElement as h, Fragment } from "react";

export function SensitiveWarning({ preview }) {
  return preview?.donnees_ezerbot && h("p", { role: "alert", className: "text-sm gb-local-sensitive-warning" }, "Ce chemin appartient aux données d’Ezerbot (identifiants, historique). Refusez sauf certitude.");
}

export function LocalApprovalPreview({ tool, arguments: args }) {
  if (!args || typeof args !== "object") return null;
  const preview = args.apercu_local;
  if (tool === "write_file") {
    const replacing = Boolean(args.expected_sha256);
    const missing = !preview || typeof preview.lignes_ajoutees !== "number";
    const count = (n, word) => `${n} ligne${n <= 1 ? "" : "s"} ${word}${n <= 1 ? "" : "s"}`;
    return h(Fragment, null,
      h("p", { className: "text-sm" }, `${replacing ? "Remplacement" : "Création"} : `, h("code", null, args.path)),
      preview?.cible_reelle && preview.cible_reelle !== args.path && h("p", { className: "text-sm gb-local-real-target" }, "Destination réelle : ", h("code", null, preview.cible_reelle)),
      missing ? h("p", { role: "alert", className: "text-sm" }, "Aperçu des modifications indisponible. Refusez cette demande et faites relire le fichier.")
        : h("p", { className: "text-sm gb-local-diff-summary" }, `Modifications : +${count(preview.lignes_ajoutees, "ajoutée")}${replacing ? ` / −${count(preview.lignes_supprimees, "supprimée")}` : ""} · ${count(preview.lignes_proposees, "proposée")}.`),
      preview?.empreinte_actuelle_ok === false && h("p", { role: "alert", className: "text-sm" }, "Le fichier a changé : l’empreinte ne correspond plus. Refusez cette demande et faites relire le fichier."),
      preview?.source === "lecture_approuvee" && h("p", { className: "text-xs" }, "Comparaison fondée sur la dernière lecture autorisée de ce bot. L’empreinte sera revérifiée avant écriture."),
      preview?.extrait && h("details", { className: "gb-local-diff", open: true }, h("summary", { className: "text-sm" }, "Changements proposés"), h("pre", { className: "gb-local-command gb-local-file-content" }, ...preview.extrait.split("\n").filter(line => replacing || !line.startsWith("@@")).map((line, i, all) => h("span", { key: i, className: line.startsWith("@@") ? "gb-local-diff-hunk" : line.startsWith("+") ? "gb-local-diff-added" : line.startsWith("-") ? "gb-local-diff-removed" : undefined }, line + (i < all.length - 1 ? "\n" : ""))))),
      preview?.diff_simplifie && h("p", { className: "text-xs" }, "Comparaison simplifiée : fichier trop grand pour une comparaison minimale. Les compteurs portent sur le bloc remplacé."),
      preview?.tronque && h("p", { className: "text-xs" }, "Extrait des modifications tronqué (200 lignes ou 16 Kio maximum). Les ajouts, leur début et leur fin restent représentés."),
      preview?.fin_de_ligne_modifiee && h("p", { className: "text-xs" }, "Le saut de ligne final est modifié."),
      preview?.format_fin_de_ligne_modifie && h("p", { className: "text-xs" }, "Le format des fins de ligne est modifié (CRLF/LF)."),
      !replacing && h("p", { className: "text-xs" }, "Aucun fichier existant ne sera écrasé."),
      h("details", null, h("summary", { className: "text-sm" }, "Nouveau contenu complet"), h("pre", { className: "gb-local-command gb-local-file-content" }, args.content)),
    );
  }
  if (["read_file", "list_directory"].includes(tool)) {
    const target = preview?.cible_reelle;
    return h(Fragment, null,
      h("p",{className:"text-sm"},tool==="read_file" ? "Lecture : " : "Liste : ",h("code",null,args.path)),
      target && target !== args.path && h("p", { className: "text-sm gb-local-real-target" }, "Lien symbolique → ", h("code", null, target), preview.hors_dossiers_accordes ? " (hors des dossiers accordés)" : ""),
      h("p", { className: "text-xs" }, preview?.donnees_ezerbot ? "Ces données exigent toujours une carte individuelle ; un dossier accordé ne supprime pas cette protection." : preview?.existe === false && preview?.dans_dossier_accorde ? "Ce fichier n’existe pas encore ; la lecture demande votre accord." : "Hors des dossiers accordés, chaque lecture requiert cette carte. Pour éviter ces cartes, accordez le dossier dans Paramètres › Terminal et fichiers sur ce Mac."),
      h(SensitiveWarning, { preview }),
    );
  }
  return null;
}

export function LocalApprovalWarning({ tool, scope }) {
  if (scope !== "global" || !["run_shell", "write_file", "document_action"].includes(tool)) return null;
  if (tool === "document_action") return h("p", { role: "alert", className: "my-2 text-xs gb-local-global-warning" }, "Attention : ce bot pourra ouvrir des documents et enregistrer de nouveaux fichiers n’importe où sur votre Mac sans vous demander.");
  const action = tool === "run_shell" ? "exécuter toute commande, sans sandbox," : "créer ou remplacer tout fichier";
  return h("p", { role: "alert", className: "my-2 text-xs gb-local-global-warning" }, `Attention : ce bot pourra ${action} sur votre Mac sans vous demander.`);
}

export function localApprovalBlocked(tool, args) {
  return tool === "write_file" && (!args?.apercu_local || typeof args.apercu_local.lignes_ajoutees !== "number" || args.apercu_local.empreinte_actuelle_ok === false);
}
