# Réunions et routines

Les réunions règlent la durée de conservation de l’audio gardé. Les routines s’ouvrent dans leur propre dialogue, par bot, depuis Paramètres.

## Sub-features

- `reunions-retention` montre le sélecteur `#meeting-retention` (1, 7 et 30 jours).
- `routines-ouvrir` ouvre le dialogue Routines du bot par défaut.
- `routines-liste` montre le titre Routines et le choix du bot, sans créer de routine.
- `routines-regle` montre, sous le choix du bot, la règle de rattrapage : extinction ou veille rattrapées dans l’ordre, fenêtre de 60 minutes sinon, une pause ne se rattrape pas.
- `routines-webhook` (hors parcours groupé) : une routine au déclencheur Webhook montre, dans le dialogue Routines, la publication Tailscale Funnel (URL `https`, rappel Bearer, jeton masqué) ou la raison sans URL. Voir « Gotchas ».

## How to get to it (user POV)

- Ouvrir Paramètres (`#settings-button`).
- Faire défiler jusqu’à « Réunions et rétention audio ».
- Choisir « Gérer les routines par bot » dans la section Routines.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`.
- Base neuve : aucune routine créée. Ne pas en créer pendant la preuve.

- **Rétention.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("settings-button").click(); const node = document.getElementById("meeting-retention"); node.scrollIntoView({block:"center"}); return {heading: node.closest("section, .setting-section").querySelector("h3").textContent, values: [...node.options].map((option) => option.value)}; }'`. Le heading contient `Réunions`. Les valeurs sont `1`, `7`, `30`.
- **Capture réunions.** Run `screenshot <preuves>/spheres/reunions-routines/reunions-<theme>-<largeur>x<hauteur>.png` pour chaque thème et chaque taille.
- **Ouvrir les routines.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { const button = [...document.querySelectorAll("[data-panel=routines] button")].find((node) => node.textContent.includes("Gérer les routines")); button.click(); return true; }'`.
- **Dialogue.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => ({open: document.querySelector("dialog.routines-dialog").open, title: document.getElementById("routines-title").textContent, bot: document.querySelector("[aria-label=\\"Bot des routines\\"]").value})'`. `open` est vrai, le titre est `Routines`, le bot sélectionné est `grokbot`.
- **Règle de rattrapage.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => document.querySelector("dialog.routines-dialog .routines-note").textContent'`. Le texte commence par `Les routines horaires partent quand le moteur local est ouvert` et finit par `Une pause ne se rattrape pas.`
- **Capture routines.** Run `screenshot <preuves>/spheres/reunions-routines/routines-<theme>-<largeur>x<hauteur>.png`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere reunions-routines`.

## Gotchas

- Le bouton « Enregistrer une réunion » du compositeur (`#meeting-start`) demande le micro. Cette sphère ne le presse pas : la preuve est le réglage de rétention, pas une capture audio.
- Changer `#meeting-retention` enregistre tout de suite. Ne pas changer la valeur pour une simple capture.
- « Nouvelle routine » crée une routine. Ne pas le presser.
- Le dialogue Routines couvre Paramètres. Le fermer par son bouton « Fermer » avant de revenir à l’accueil.
- Sous 560 px de haut, les deux sections ne tiennent pas ensemble. D’où deux captures, pas une capture unique déclarée suffisante.
- `routines-webhook` crée une routine : le parcours groupé ne la touche pas. Pour la voir, lancer le wrapper de debug avec `GROKBOT_FAKE_TAILSCALE` (script dans le `HOME` jetable) et `GROKBOT_WEBHOOK_PORT` (port libre) ; un debug n'appelle jamais le vrai Tailscale. Le faux script répond à `status --json`, `funnel …` et `serve status --json`. L'approbation de la publication est une carte dans la conversation Approbations.
