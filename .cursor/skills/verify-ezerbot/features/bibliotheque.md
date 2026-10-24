# Bibliothèque

La bibliothèque est la liste des fichiers de contexte d’un bot, ouverte depuis « Modifier Ezerbot ». Une base neuve n’a aucun fichier.

## Sub-features

- `bibliotheque-vide` affiche le plafond par fichier (20 Mo), le nombre maximal de fichiers (« 0 sur 20 fichiers »), la zone de dépôt et « Aucun fichier pour l’instant. ».
- `bibliotheque-liste` montre un fichier déposé avec son format, sa taille, sa date et, si le texte a été coupé, son avertissement.
- `bibliotheque-retrait` demande une confirmation avant de retirer un fichier.

## How to get to it (user POV)

- Ouvrir Paramètres, puis dans la liste Bots choisir « Modifier Ezerbot » (`[aria-label="Modifier Ezerbot"]`).
- Faire défiler jusqu’à « Bibliothèque » (`[aria-label="Bibliothèque du bot"]`).

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`, la pilule Modèle `#composer-tools [data-picker="model"]` est présente.
- Aucun fichier déposé. La sphère ne dépose ni ne retire rien.

- **Ouvrir le bot.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("settings-button").click(); document.querySelector("[aria-label=\\"Modifier Ezerbot\\"]").click(); return document.getElementById("bot-dialog").open; }'`. Attendre que la section se charge.
- **Section.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { const section = document.querySelector("[aria-label=\\"Bibliothèque du bot\\"]"); section.scrollIntoView({block:"start"}); return section.innerText; }'`. Le texte contient `Plafond : 20 Mo par fichier · 0 sur 20 fichiers.` et `Aucun fichier pour l’instant.`.
- **Capture.** Run `screenshot <preuves>/spheres/bibliotheque/<theme>-<largeur>x<hauteur>.png` pour sombre, clair, `1140x780` et `820x560`, avec le dialogue du bot ouvert sur Bibliothèque.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere bibliotheque`.

## Gotchas

- « Retirer » supprime le fichier après confirmation : ne pas confirmer pendant une capture.
- Déposer un fichier passe par le sélecteur natif du Mac. Pour un état rempli, appeler `/api/library` depuis `eval` plutôt que d’ouvrir le sélecteur.
- Fermer le dialogue avec `[data-close-dialog]` sans enregistrer.
