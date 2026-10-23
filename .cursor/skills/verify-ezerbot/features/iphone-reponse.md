# Réponse depuis la notification iPhone

Quand le bot répond et que la PWA n’est pas ouverte sur la conversation, la notification porte le nom du bot et un corps générique. Répondre ouvre la feuille de réponse de cette conversation, champ prêt ; Envoyer pose le texte dans le fil, une seule fois. Cette sphère joue la feuille sur la page du Mac : même panneau, mêmes phrases que sur la PWA.

## Sub-features

- `avant` montre le fil avec la réponse du bot, sans feuille.
- `feuille` montre la feuille « Répondre à <bot> » : champ prêt, bouton Envoyer.
- `envoyee` envoie « Vu depuis la notification » en pressant deux fois Envoyer : la feuille se ferme et le fil contient ce message une fois.

## How to get to it (user POV)

- Sur l’iPhone, toucher Répondre sur la notification d’un bot (ou ouvrir `/?notification=<id>&reply=1`, ce que fait un toucher quand la PWA est fermée).

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`, badge `ChatGPT`.

- **Réponse du bot.** Le parcours crée une conversation, envoie « Bonjour, qui es-tu ? », attend la réponse du faux bot, puis revient à l’accueil : la notification `reply:<run>` reste non lue.
- **Feuille.** `window.EzerbotNotifications.activate("reply:<run>", {reply: true})` ouvre `dialog.reply-sheet`.
- **Envoi.** `form.requestSubmit()` deux fois : le fil garde un seul message de l’utilisateur avec ce texte.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere iphone-reponse`. Les douze PNG (`avant-`, `feuille-` et `envoyee-`, deux thèmes, deux tailles) existent.

## Gotchas

- La notification push elle-même (titre au nom du bot, corps générique) est prouvée par les tests `reply_push_is_titled_with_the_bot_and_carries_no_reply_text` et `iphone.test.cjs` ; aucun vrai service push ni vrai iPhone ici.
- Le parcours « Mac éteint » ne se joue pas avec ce harnais : arrêter le wrapper arrêterait aussi la preuve.
- Chaque passage crée sa propre conversation dans la base jetable.
