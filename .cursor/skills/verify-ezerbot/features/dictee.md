# Dictée et microphone

Le réglage Microphone choisit l’appareil qu’Ezerbot écoute. La dictée pose le texte dans le brouillon, à relire, sans rien envoyer. Cette sphère tourne sur le build de développement : un faux adaptateur remplace le microphone, la liste est « Microphone intégré (test) » et « Microphone USB (test) ».

## Sub-features

- `microphone-liste` montre, dans Paramètres, la section Microphone, la phrase « Choisissez le microphone qu’Ezerbot écoute quand vous lui parlez. » et trois choix : le micro par défaut du Mac et les deux micros de test. Le choix est marqué « Choisi ».
- `microphone-repli` retire le micro choisi : la phrase « Le microphone choisi n’est plus disponible. Ezerbot utilise le microphone par défaut. » apparaît et le choix revient au micro par défaut.
- `dictee-ecoute` démarre la dictée avec `#voice-microphone` : l’état dit « Écoute en cours · Microphone intégré (test) · Échap pour arrêter » et la durée s’affiche sur une échelle de `5:00`.
- `dictee-brouillon` arrête la dictée : l’état dit « Transcription ajoutée au brouillon. Relisez-la avant d’envoyer. », le texte est dans `.gb-editor` et aucun message n’est parti.
- `dictee-raccourci` ⌘ D démarre ou arrête la dictée, un appui maintenu d’une demi-seconde parle jusqu’au relâchement (couverts par les tests Node, pas par la capture : le harnais n’envoie pas ⌘ D).

## How to get to it (user POV)

- Ouvrir Paramètres (`#settings-button`), section Microphone, choisir un micro dans la liste.
- Dans le compositeur, cliquer le bouton micro (`#voice-microphone`), ou presser ⌘ D, puis relire le texte dans le brouillon.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur le build de développement (`state.voice.debug_fake` vrai).
- Aucune conversation ouverte : la dictée sans mémo ne crée pas de conversation.

- **Liste.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("settings-button").click(); return [...document.querySelectorAll(".voice-microphone-option")].map((node) => [node.dataset.id, node.dataset.selected]); }'`. Les ids sont `` (vide), `fake-built-in`, `fake-usb`.
- **Choisir.** Cliquer l’entrée `fake-usb` : `data-selected` passe à `true` et « Choisi » s’affiche.
- **Retirer le micro.** Run `eval '() => window.GrokbotPanels.api("/api/voice/debug", {command: "configure", unplugged: ["fake-usb"]})'`, fermer puis rouvrir Paramètres : la phrase de repli s’affiche dans `.voice-settings-notice` de la section Microphone. Rebrancher avec `unplugged: []`.
- **Dicter.** Cliquer `#voice-microphone` : `.voice-status` commence par « Écoute en cours · Microphone intégré (test) » et `.voice-time` finit par `/ 5:00`. Cliquer encore : « Transcription ajoutée au brouillon ».
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere dictee`. Les seize PNG (`microphone-`, `repli-`, `ecoute-` et `brouillon-`, deux thèmes, deux tailles) existent.

## Gotchas

- Le choix du micro est enregistré tout de suite. Le parcours le remet à `fake-usb`, puis le repli le vide : la base jetable en garde l’état final.
- Le texte de la dictée reste dans le brouillon d’un passage à l’autre ; il n’est jamais envoyé.
- Le faux adaptateur ne prouve ni la permission du microphone, ni la reconnaissance du Mac, ni le choix réel d’un appareil : ces points se vérifient sur le Mac avec un vrai micro.
- Ne pas presser « Mémo vocal » : il crée une conversation et joint un audio.
