# Modèles

Le menu Modèle du compositeur liste le catalogue du compte. Avec le faux OpenAI, l’entrée visible est « GPT Test ». Le modèle caché `internal` ne doit pas apparaître.

## Sub-features

- `modeles-pilule` affiche la pilule Modèle seulement une fois le compte fictif connecté.
- `modeles-menu` ouvre `#picker` titré Modèle et montre GPT Test.
- `modeles-cache` n’affiche pas le modèle de visibilité `hide`.

## How to get to it (user POV)

- Rester sur l’accueil, compte connecté.
- Choisir la pilule « Modèle » dans la barre d’écriture (`[data-picker="model"]`).

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` et la pilule Modèle `#composer-tools [data-picker="model"]` est présente.
- Le faux catalogue répond sur `GET /v1/models`.
- Paramètres fermés, pour laisser voir le compositeur.

- **Attendre la pilule.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { const pill = document.querySelector("[data-picker=model]"); return pill && !pill.disabled && pill.getAttribute("aria-label"); }'`. `result:` contient `Modèle` et n’est pas `null`. Tant que le texte dit `Chargement…`, attendre.
- **Ouvrir le menu.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.querySelector("[data-picker=model]").click(); return document.getElementById("picker").hidden; }'`. `result:` vaut `false`.
- **Lire les entrées.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => ({title: document.querySelector("#picker h4").textContent, items: [...document.querySelectorAll("#picker [role=menuitemradio]")].map((node) => node.innerText)})'`. Le titre est `Modèle`. Les items contiennent `GPT Test` et ne contiennent pas `Internal`.
- **Capture.** Pour sombre et clair, à `1140x780` et `820x560`, Run `resize` puis `screenshot <preuves>/spheres/modeles/<theme>-<largeur>x<hauteur>.png` avec le menu ouvert.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere modeles`.

## Gotchas

- Sans jeton fictif, la pilule Modèle n’existe pas. Ne pas simuler le menu en écrivant dans le DOM.
- Un second clic sur la pilule ferme le menu : le document bascule `pickerKind`. Ne cliquer que si `#picker` est `hidden`.
- Un clic ailleurs dans la page ferme le menu. Capturer tout de suite après l’ouverture.
- À 1100 px et moins, le libellé `<small>` de la pilule est masqué. Le nom accessible et le menu restent la preuve.
- « Modèles de bots » (`[data-panel="templates"]`, « Parcourir ») est une autre entrée. Cette sphère prouve le catalogue du compte, pas la galerie de bots. Ne pas la compter comme faite si seul « Parcourir » a été ouvert.
