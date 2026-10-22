# Images générées

Quand on demande un dessin, le bot appelle l’outil `generate_image` sur la session ChatGPT : l’indicateur « Génération d’une image… » laisse place à l’image, qui s’agrandit dans la visionneuse comme une pièce jointe.

## Sub-features

- `images-indicateur` montre « Génération d’une image… » pendant l’appel.
- `images-resultat` montre l’image seule dans la carte (`.gb-generated-image-card[data-image-state="done"]`).
- `images-visionneuse` agrandit l’image, avec « Enregistrer l’image… » et « Copier l’image ».
- `images-connexion` montre la carte « Connectez votre compte ChatGPT » sans session.

## How to get to it (user POV)

- Dans une conversation, écrire une demande de dessin, par exemple « Dessine un phare au crépuscule », puis envoyer.
- Cliquer l’image pour l’agrandir.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur cette instance.
- Badge `#mode-badge-text` égal à `ChatGPT`.
- Le faux OpenAI répond à une demande contenant « dessine » par un appel `generate_image`, à l’appel d’image par un PNG de dégradé, puis par « Voici l’image. ».

- **Lancer.** Run `bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence <preuves> --sphere images`. Sans `--sphere images`, le wrapper répond « OK » à tout et l’image n’est jamais demandée.
- **Envoyer.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("new-chat").click(); return true; }'`, saisir `Dessine un phare au crépuscule` dans `.gb-editor[aria-label="Votre message"]`, puis cliquer `#send-button`.
- **Attendre l’image.** `.gb-generated-image-card[data-image-state="done"] img` est chargée (`naturalWidth` > 0) et la carte contient un seul `.gb-image-cell`.
- **Agrandir.** Cliquer `.gb-image-cell` : `dialog.gb-image-viewer` s’ouvre avec les boutons « Enregistrer l’image… » et « Copier l’image ».
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere images`. Les quatre PNG existent.

## Gotchas

- La conversation créée par le premier passage est réutilisée pour les autres thèmes et tailles ; l’image n’est demandée qu’une fois.
- Le faux OpenAI ne prouve pas l’hôte réel : il vérifie la route `/backend-api/codex/responses` et le corps `image_generation`, pas l’acceptation par ChatGPT.
- Sans session, l’appel n’est jamais émis ; ce chemin n’est pas dans le parcours groupé.
