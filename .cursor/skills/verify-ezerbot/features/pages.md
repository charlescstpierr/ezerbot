# Pages

Une Page est un tableau interactif que le bot écrit en `.page.tsx` : Ezerbot la compile sur le Mac et l’affiche à côté du fil, avec une entrée « Pages » dans la barre latérale.

## Sub-features

- `pages-avant` montre le fil d’une conversation neuve, sans Page.
- `pages-apres` montre la Page « Trois forfaits » dans `.page-stage`, à côté du fil, et son entrée dans la liste « Pages ».
- `pages-choix` change l’offre choisie au clic sur « Choisir » (`button.page-choice`) sans perdre le focus.
- `pages-commenter` : sélectionner « Confort » dans `.page-canvas` (souris, ou Maj+flèches après un premier glissé) montre `.page-comment` « Commenter » ; le clic pose `.composer-area > .gb-quote-chip` (« Page « Trois forfaits » — « Confort » », croix `.gb-quote-remove`) ; l’envoi ajoute la même puce `.gb-user .gb-quote-chip` dans la bulle et une ancre dans `GET /api/pages/:id` ; plus de 500 caractères, une sélection hors canevas ou un texte absent de l’aperçu (« Offre choisie : Confort » après « Choisir ») : pas de bouton ; une sélection sur plusieurs cellules voisines (« Confort\t19 € ») est acceptée, chaque suite de blancs comptant pour un espace, et l’ancre garde le passage tel qu’envoyé ; un passage dont un blanc manque ou est en trop (« Con fort », « Confort19 € », envoyé directement à `POST /api/send`) est refusé en 400 (« Ce passage n’apparaît pas dans le texte affiché de la Page. ») ; pendant un envoi lent, la croix puis « Commenter » sur le même passage pose une nouvelle puce qui reste à la réponse du premier envoi et part avec le message suivant, avec son ancre.
- `pages-texte` affiche les prix écrits avec un échappement Unicode comme « 9 € » (jamais « 9u20ac »), et les références JSX `&lt;` et `&#8364;` s’affichent « < » et « € ».

## How to get to it (user POV)

- Dans une conversation, écrire « Fais-moi une page des forfaits », puis envoyer.
- La Page apparaît à côté du fil ; son titre se renomme dans la barre de la Page et « Ouvrir en pleine page » l’agrandit.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur cette instance.
- La pilule Modèle `#composer-tools [data-picker="model"]` est présente.
- Le faux OpenAI répond à une demande contenant « une page des forfaits » par un appel `write_page`, puis par « Voici la Page. ».

- **Lancer.** Run `bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence <preuves> --sphere pages`. Sans `--sphere pages`, le wrapper répond « OK » à tout et aucune Page n’est écrite.
- **Envoyer.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("new-chat").click(); return true; }'`, saisir `Fais-moi une page des forfaits` dans `.gb-editor[aria-label="Votre message"]`, puis cliquer `#send-button`.
- **Attendre la Page.** `.page-stage` n’est plus `hidden` et contient trois `.page-canvas button.page-choice`.
- **Choisir.** Cliquer le deuxième bouton : le texte devient « Offre choisie : Confort » et le focus reste sur ce bouton.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere pages`. Les huit PNG (`avant-` et `apres-`, deux thèmes, deux tailles) existent.

## Gotchas

- Chaque passage ouvre une conversation neuve : la Page `forfaits.page.tsx` du bot est réécrite et suit la nouvelle conversation. Seul le premier « avant » est sans entrée dans la liste « Pages ».
- Le faux OpenAI ne prouve pas ce que le vrai modèle écrit : il vérifie la compilation locale, l’affichage et le clic.
