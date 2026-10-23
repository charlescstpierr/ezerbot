# Sources citées

Une réponse du bot qui cite `[n]` montre ces numéros comme des pastilles, avec la source de chacun. La sphère dépose trois fichiers dans la bibliothèque du bot, demande « Cite tes sources » et lit la réponse du faux OpenAI, qui cite `[1]`, `[2]`, `[3]` et un `[7]` que rien ne soutient.

## Sub-features

- `sources-pastilles` rend chaque `[n]` d’une source du tour en pastille cliquable (`.gb-cite-pill`) et la ligne repliable « Sources 3 » (`.gb-sources`) sous la réponse, dépliée pour la capture.
- `sources-carte` ouvre au clic la carte de la source : type, titre, expéditeur ou auteur, date ou plage, « Ouvrir la source » et « Fermer ».
- `sources-introuvable` rend `[7]` barré et gris (`.gb-cite-missing`), avec l’infobulle « Source introuvable ».
- `sources-inaccessible` : une source retirée de la bibliothèque affiche « Ce fichier n’est plus dans la bibliothèque. » sans rien casser.

## How to get to it (user POV)

- Envoyer un message dont la réponse cite des sources, par exemple après une lecture de courriel, de Drive ou de la bibliothèque.
- Cliquer sur une pastille, ou déplier « Sources » sous la réponse.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`. La sphère garde le faux OpenAI du run : lui seul sait citer `[1]` à `[3]` et `[7]`.
- La sphère dépose `loyer.md`, `budget.md` et `bail.txt` dans la bibliothèque du bot et les retire pour la dernière capture. Elle part d’une conversation neuve.

- **Envoyer.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => document.querySelectorAll(".gb-assistant .gb-cite-pill").length'`. Après la réponse, le résultat est `3`, avec un `.gb-cite-missing` et `.gb-sources-count` égal à `3`.
- **Captures.** `pastilles-…` (bulle et ligne dépliée), `carte-…`, `introuvable-…`, `inaccessible-…`, en sombre et en clair, à `1140x780` et `820x560`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere sources-citees`.

## Gotchas

- Le faux OpenAI écrit « 1 200 dollars » et non « 1 200 $ » : deux `$` dans une réponse deviennent une formule mathématique et avalent les `[n]` qu’ils encadrent.
- « Ouvrir la source » d’un fichier de bibliothèque ouvre la fiche du bot. Un lien Gmail ou Drive s’ouvre dans le navigateur du Mac ; la sphère n’a pas de compte Google, elle ne l’exerce pas.
- La sphère retire les trois fichiers de la bibliothèque à la dernière capture et les redépose au passage suivant.
