# Paramètres et connecteurs

Les Paramètres regroupent le compte, l’apparence et les sections ajoutées par les panneaux. Les connecteurs sont la section MCP : catalogue, serveurs personnels et formulaire « Ajouter un serveur MCP ».

## Sub-features

- `parametres-ouvrir` ouvre le dialogue Paramètres depuis le bas de la barre latérale.
- `connecteurs-section` montre le titre Connecteurs et l’ajout d’un serveur, sans en enregistrer.
- `connecteurs-vide` laisse la liste des serveurs personnels vide sur une base neuve.

## How to get to it (user POV)

- Choisir le bouton du bas de barre « Paramètres de l’application » (`#settings-button`).
- Faire défiler jusqu’à la section Connecteurs.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`.
- Badge `ChatGPT`.
- Base neuve : aucun serveur MCP enregistré.

- **Ouvrir Paramètres.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }'`. `result:` vaut `true`. Le titre `#settings-title` est `Paramètres`.
- **Aller aux connecteurs.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { const panel = document.querySelector("[data-panel=mcp]"); panel.scrollIntoView({block:"start"}); return panel.querySelector("h3").textContent; }'`. `result:` vaut `"Connecteurs"`.
- **Formulaire d’ajout.** Le `summary` « Ajouter un serveur MCP » est dans `[data-panel="mcp"]`. Ne pas le soumettre.
- **Liste vide.** `[data-panel="mcp"] .mcp-list` ne contient pas d’article de serveur personnalisé.
- **Capture.** Pour chaque thème et chaque taille (`1140x780`, `820x560`), Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> resize <largeur> <hauteur>` puis `screenshot <preuves>/spheres/parametres-connecteurs/<theme>-<largeur>x<hauteur>.png`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere parametres-connecteurs`.

## Gotchas

- Le dialogue est modal. Un clic sur la marque ne ferme pas Paramètres ; utiliser `[data-close-dialog]` ou « Terminé ».
- La section est montée dans `#settings-extensions` avec `data-panel="mcp"`. Chercher ce panneau, pas un lien de navigation séparé.
- Enregistrer un connecteur lancerait une commande locale. Cette sphère ne soumet pas le formulaire.
- Le catalogue peut être long. La capture doit avoir fait défiler le titre Connecteurs dans le dialogue, pas seulement ouvert Paramètres en haut.
