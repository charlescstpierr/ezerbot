# Accueil

L’accueil est la conversation vide : la marque Ezerbot, le titre « Nouvelle conversation », la question « Une idée en tête » et le champ « Votre message ».

## Sub-features

- `accueil-marque` affiche la marque et ramène à la conversation vide.
- `accueil-compositeur` montre le champ « Votre message » sans envoi.
- `accueil-theme` applique Clair et Sombre, puis les relit.

## How to get to it (user POV)

- Ouvrir l’URL annoncée par le wrapper.
- Choisir la marque « Ezerbot, accueil » (`#home-link`).
- Choisir « Nouvelle conversation » (`#new-chat`) lorsque l’accueil a été quitté.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur cette instance.
- `#mode-note` commence par « Quota de votre abonnement ChatGPT ».
- Aucun dialogue ouvert.

- **Ouvrir.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> open <url>`. Le titre du document est `Ezerbot`.
- **Attendre le compte fictif.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => document.getElementById("mode-note").textContent.startsWith("Quota de votre abonnement ChatGPT")'`. `result:` vaut `true`.
- **Choisir le thème.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("settings-button").click(); document.querySelector("[data-theme-choice=dark]").click(); return true; }'`. `document.documentElement.dataset.theme` devient `dark` et le bouton Sombre a `aria-pressed="true"`.
- **Fermer les Paramètres.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.querySelector("#settings-dialog .dialog-footer [data-close-dialog]").click(); return document.getElementById("settings-dialog").open; }'`. `result:` vaut `false`.
- **Relire le thème.** Rouvrir Paramètres. Le bouton `[data-theme-choice="dark"]` a encore `aria-pressed="true"`. Fermer.
- **Taille.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> resize 1140 780`, puis la même chose pour `820 560`. `innerWidth` et `innerHeight` égalent la taille demandée.
- **Accueil visible.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => ({heading: document.querySelector("#welcome h2").innerText, title: document.getElementById("conversation-title").textContent, editor: !!document.querySelector(".gb-editor[aria-label=\\"Votre message\\"]")})'`. Le titre de conversation est `Nouvelle conversation`, le heading contient `Une idée en tête`, l’éditeur est présent.
- **Capture.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> screenshot <preuves>/spheres/accueil/dark-1140x780.png`. Répéter pour `dark-820x560`, `light-1140x780` et `light-820x560` après le bouton Clair. `--canvas` vaut `#0e0f12` en sombre et `#ffffff` en clair.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere accueil`. Les quatre PNG existent.

## Gotchas

- Le bouton Système suit le thème de l’OS. La preuve n’utilise que Clair et Sombre.
- `#welcome` est masqué dès qu’une conversation a des messages. Revenir par `#home-link` avant la capture.
- À 760 px et moins, la barre latérale se replie. 820 px reste au-dessus de ce seuil.
- Un clic dans le compositeur déplace le foyer. Ne pas y envoyer de touche pour prouver l’accueil.
- `localStorage` du thème vit dans le profil Chrome du run, pas dans le profil du capitaine.
