# Mémoire

La mémoire automatique est un réglage par bot dans Paramètres. La mémoire du bot est la liste de ses faits, ouverte depuis « Modifier Ezerbot ». Une base neuve n’a aucun fait.

## Sub-features

- `memoire-reglage` montre « Mémoire automatique » et l’interrupteur du bot Ezerbot.
- `memoire-liste` ouvre la section « Mémoire du bot » avec la recherche.
- `memoire-vide` affiche zéro fait actif, sans en inventer.
- `memoire-espace` : l’entrée « Mémoire » sous « Rechercher » (`#memory-entry`, compteur `.memory-entry-count`) ouvre `#memory-space` (« Espace personnel / Mémoire · N souvenirs », Graphe « bientôt » / Liste lue depuis `GET /api/memory/all`). Pas encore une sphère de `drive.sh` : se pilote à la main avec `browser.sh eval`.

## How to get to it (user POV)

- Ouvrir Paramètres, section « Mémoire automatique ».
- Dans la liste Bots, choisir « Modifier Ezerbot » (`[aria-label="Modifier Ezerbot"]`).
- Faire défiler jusqu’à « Mémoire ».

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`, la pilule Modèle `#composer-tools [data-picker="model"]` est présente, catalogue chargé (le faux catalogue contient `gpt-test-mini`, donc la mémoire automatique n’affiche pas « aucun petit modèle »).
- Aucun fait appris. Ne pas envoyer de message pendant cette sphère : un envoi pourrait créer un fait.

- **Réglage.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("settings-button").click(); const panel = document.querySelector("[data-panel=memory]"); return {heading: panel.querySelector("h3").textContent, toggle: !!panel.querySelector("input[name=automatic-grokbot]")}; }'`. Le heading est `Mémoire automatique` et l’interrupteur existe.
- **Ouvrir le bot.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.querySelector("[aria-label=\\"Modifier Ezerbot\\"]").click(); return document.getElementById("bot-dialog").open; }'`. Attendre que le dialogue soit ouvert : le clic est synchrone, le contenu mémoire se charge ensuite.
- **Liste.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { const section = document.querySelector("[aria-label=\\"Mémoire du bot\\"]"); section.scrollIntoView({block:"start"}); return {search: !!document.querySelector("[aria-label=\\"Rechercher dans la mémoire\\"]"), text: section.innerText}; }'`. La recherche existe. Le texte contient `0 sur 0 faits actifs`.
- **Capture.** Run `screenshot <preuves>/spheres/memoire/<theme>-<largeur>x<hauteur>.png` pour sombre, clair, `1140x780` et `820x560`, avec le dialogue du bot ouvert sur Mémoire.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere memoire`.

## Gotchas

- L’interrupteur de mémoire automatique enregistre dès le clic. Ne pas le basculer pour une capture.
- « Partager la sélection » et « Oublier… » modifient les faits. Les laisser inactifs.
- Sans `gpt-test-mini` (ou un slug `mini`, `nano` ou `gpt-6-luna`), le réglage dit que l’extraction est indisponible. C’est alors un échec de fixture, pas une preuve de la liste.
- Le champ de recherche est dans le dialogue du bot, pas dans la barre latérale. `#search` cherche une conversation.
- Fermer le dialogue avec `[data-close-dialog]` sans enregistrer si le formulaire a été touché. Cette sphère ne modifie pas le nom.
- Sous 720 px de haut, l’aperçu du bot se compacte. La section Mémoire reste le critère, pas la taille de la mascotte.
