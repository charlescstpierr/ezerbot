# Paramètres et connecteurs

Les Paramètres regroupent le compte, l’apparence et les sections ajoutées par les panneaux. Les connecteurs sont la section MCP : catalogue, serveurs personnels et formulaire « Ajouter un serveur MCP ».

## Sub-features

- `parametres-ouvrir` ouvre le dialogue Paramètres depuis le bas de la barre latérale.
- `connecteurs-section` montre le titre Connecteurs et l’ajout d’un serveur, sans en enregistrer.
- `connecteurs-vide` laisse la liste des serveurs personnels vide sur une base neuve.
- `connecteurs-reunions` montre l’entrée Réunions du catalogue : un seul champ de clé (mot de passe) et le bouton Connecter, sans OAuth.
- `connecteurs-recherche` filtre le catalogue : « git » ne laisse que GitHub, une recherche vide rend toutes les cartes, une chaîne absente affiche « Aucun service ne correspond à … ».
- `connecteurs-mcp-https` : le formulaire « Ajouter un serveur MCP » propose « Commande locale » (par défaut) ou « Adresse HTTPS » (adresse, jeton en champ mot de passe, case « sur ce Mac » visible seulement pour une adresse `http:`). Un serveur ajouté par adresse montre son adresse, « Jeton privé conservé dans le coffre », jamais le jeton, et « Connecter… » ouvre l’approbation avec « Adresse du service ».
- `connecteurs-github` montre la carte GitHub sans champ d’identifiant ni de secret ; sans `client_id` de build, elle dit que l’application GitHub d’Ezerbot n’est pas configurée.

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
- **Entrée Réunions.** `[data-panel="mcp"] [data-connector="reunions"]` contient un seul `input` de type `password` et le bouton « Connecter ». Ne pas le soumettre : la clé et les liens de réunion ne passent que par un fournisseur de boucle (`GROKBOT_FAKE_RECALL`), jamais par cette sphère.
- **Recherche.** Le champ `[data-panel="mcp"] [name="catalogue-search"]` filtre par nom et description, sans tenir compte de la casse ni des accents. Saisir `git` (événement `input`) : seul `[data-connector="github"]` n’est pas `hidden`. Vider : toutes les cartes reviennent. Saisir `zzzz-absent` : le `[role="status"]` de l’en-tête du catalogue dit qu’aucun service ne correspond.
- **Serveur par adresse.** Choisir la case `input[type=radio][aria-label="Adresse HTTPS"]` : `[name=url]`, `[name=token]` (mot de passe) et, après une adresse `http://127.0.0.1…`, `[name=on-this-mac]` apparaissent ; le champ `[name=command]` disparaît. Un test de bout en bout exige un serveur MCP HTTP local sur `127.0.0.1` (non fourni par la preuve automatisée) : `prove.sh` ne soumet pas ce formulaire.
- **Liste vide.** `[data-panel="mcp"] .mcp-list` ne contient pas d’article de serveur personnalisé.
- **Capture.** Pour chaque thème et chaque taille (`1140x780`, `820x560`), Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> resize <largeur> <hauteur>` puis `screenshot <preuves>/spheres/parametres-connecteurs/<theme>-<largeur>x<hauteur>.png`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere parametres-connecteurs`.

## Gotchas

- Le dialogue est modal. Un clic sur la marque ne ferme pas Paramètres ; utiliser `[data-close-dialog]` ou « Terminé ».
- La section est montée dans `#settings-extensions` avec `data-panel="mcp"`. Chercher ce panneau, pas un lien de navigation séparé.
- Enregistrer un connecteur lancerait une commande locale. Cette sphère ne soumet pas le formulaire.
- La carte GitHub se connecte par un code à 8 caractères validé chez GitHub avec un vrai compte : la preuve automatisée n’y va pas. Sans `EZERBOT_GITHUB_CLIENT_ID`, un clic sur Connecter est refusé avec une phrase, sans carte d’approbation.
- Le catalogue peut être long. La capture doit avoir fait défiler le titre Connecteurs dans le dialogue, pas seulement ouvert Paramètres en haut.
