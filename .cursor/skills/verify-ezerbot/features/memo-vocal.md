# Mémo vocal

Un mémo envoyé se lit à la vitesse choisie, de 0,5 à 2, et sa transcription se lit mot à mot : un clic sur un mot place la lecture au début de ce mot. Cette sphère tourne sur le build de développement : le faux adaptateur enregistre un mémo de vingt secondes (scénario `long_memo`) dont chaque mot a un début, sauf le cinquième.

## Sub-features

- `memo-avant` montre le mémo dans le fil : lecteur, champ « Vitesse » à `1` et la transcription mot à mot.
- `memo-vitesse` passe la vitesse à `2` : le lecteur suit (`playbackRate` vaut 2). Saisir `3` est refusé : « Vitesse refusée : choisissez une vitesse de 0,5 à 2. La vitesse reste 2. » et le champ garde `2`. `0,5` est accepté.
- `memo-saut` clique un mot du milieu : la lecture reprend à l’instant de ce mot (`data-start-ms`), à la vitesse choisie.
- `memo-sans-temps` clique le mot sans horodatage : « Ce mot n’a pas d’horodatage : la lecture continue. » et la position ne bouge pas.

## How to get to it (user POV)

- Envoyer un mémo vocal (case « Mémo vocal » du compositeur, puis le bouton micro), ouvrir le fil : le message porte le lecteur et la transcription.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur le build de développement (`state.voice.debug_fake` vrai), badge `ChatGPT`.

- **Mémo.** Le parcours crée le mémo par `/api/voice/debug` (`start` avec `purpose: "memo"` et `scenario: "long_memo"`, puis `stop`), `/api/voice/memos` avec les `words` reçus, puis `/api/send` avec le mémo en pièce jointe.
- **Vitesse.** Saisir dans `.gb-memo-speed input` (événement `input`, puis `blur`) : `document.querySelector(".gb-voice-memo audio").playbackRate` suit, `.gb-memo-refusal` dit le refus.
- **Saut.** Cliquer `button.gb-memo-word` : `audio.currentTime` vaut `data-start-ms / 1000`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere memo-vocal`. Les seize PNG (`avant-`, `vitesse-`, `saut-` et `sans-temps-`, deux thèmes, deux tailles) existent.

## Gotchas

- L’audio du faux mémo est silencieux : la vitesse et le saut se prouvent par `playbackRate` et `currentTime`, pas à l’oreille. L’écoute réelle se vérifie sur le Mac avec un vrai mémo.
- Les horodatages réels viennent de la reconnaissance du Mac (segments de Speech) : le faux adaptateur les répartit à intervalles réguliers.
- Le mémo reste dans la base jetable après le parcours.
