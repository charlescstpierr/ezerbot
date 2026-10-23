# Images de référence et cadre

Quand Charles joint une photo et demande « Le même style, en 16:9 », le bot appelle `generate_image` avec la photo en référence et le cadre demandé : l’image large apparaît dans le fil à côté de la photo, dans son propre rapport.

## Sub-features

- `images-reference-photo` montre la photo jointe dans le message de Charles.
- `images-reference-large` montre l’image générée dans `.gb-generated-image-card[data-image-state="done"]`, affichée dans le rapport de l’image reçue (jamais rognée en 4:3).

## How to get to it (user POV)

- Dans une conversation, joindre une photo, écrire « Le même style, en 16:9 », puis envoyer.
- Un cadre hors de 1:1, 16:9, 9:16, 4:3 et 3:4 est refusé par une phrase française, sans image.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur cette instance.
- Badge `#mode-badge-text` égal à `ChatGPT`.
- Le faux OpenAI répond à une demande contenant « le même style » par `list_delegation_context`, puis par `generate_image` avec l’identifiant de la photo et `16:9`, et rend un PNG 288×192 pour la taille `1536x1024`.

- **Lancer.** Run `bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence <preuves> --sphere images-reference`. Sans cette sphère, le wrapper répond « OK » à tout et l’image n’est jamais demandée.
- **Envoyer.** Le parcours groupé crée la conversation, importe une photo peinte dans un canevas, attend sa légende, puis envoie « Le même style, en 16:9 » avec la photo.
- **Attendre l’image.** La carte de l’image générée est chargée, la photo aussi, et le rapport affiché de l’image générée égale son rapport naturel (1,5).
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere images-reference`. Les quatre PNG existent.

## Gotchas

- La conversation créée par le premier passage est réutilisée pour les autres thèmes et tailles.
- Le faux OpenAI ne prouve pas l’hôte réel : le 16:9 part en `1536x1024` (rapport 3:2), le plus proche cadre accepté par `gpt-image-1.5`.
- Le faux OpenAI n’inspecte pas l’image de référence reçue ; ce contrat est couvert par `cargo test -p grokbot-wrapper --test images`.
