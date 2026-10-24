# Voix par bot

Chaque bot a une voix, une vitesse (de 0,5 à 2) et une langue de voix (ou Auto), réglées dans « Modifier <bot> », section Voix. Sans choix, deux bots n’annoncent pas la même voix tant qu’il en existe deux. Cette sphère tourne sur le build de développement : le faux adaptateur propose deux voix, « Voix française de test (Canada) » et « Voix française de test (France) ».

## Sub-features

- `voix-ezerbot` montre la section Voix d’Ezerbot : « Voix de Ezerbot : Voix française de test (Canada) · 1× · langue Auto », la mention « Voix attribuée automatiquement » et la vitesse qui monte jusqu’à 2.
- `voix-chef` montre celle de Chef, un second bot sans choix : une autre voix que celle d’Ezerbot.
- `voix-refus` saisit 2,5 dans la vitesse de Chef et enregistre : « Vitesse refusée : choisissez une vitesse de 0,5 à 2. La vitesse du bot reste inchangée. », et la ligne « Voix de Chef » garde la vitesse précédente.
- `voix-mode-voix` (tests Node, pas capture) : le mode voix d’une conversation parle avec la voix, la vitesse et la langue de son bot.

## How to get to it (user POV)

- Ouvrir Paramètres, puis dans la liste Bots choisir « Modifier Ezerbot » ou « Modifier Chef » (`[aria-label="Modifier Chef"]`).
- Faire défiler jusqu’à « Voix » (`.voice-bot-settings`).

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur le build de développement (`state.voice.debug_fake` vrai), la pilule Modèle `#composer-tools [data-picker="model"]` est présente.
- Un second bot : le parcours crée « Chef » (`bot-chef`) par `/api/bots`, sans effet s’il existe déjà.

- **Créer le bot.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => window.GrokbotPanels.api("/api/bots", {id: "bot-chef", name: "Chef", color: "orange", instructions: "Réponds en recettes."})'`.
- **Voix attribuées.** Run `eval '() => window.GrokbotPanels.state.voice.bots'` : `grokbot` et `bot-chef` ont deux `voice` différentes, prises dans `window.GrokbotPanels.state.voice.voices`.
- **Section.** Ouvrir Paramètres, cliquer `[aria-label="Modifier Chef"]`, lire `.voice-bot-current` : « Voix de Chef : … · 1× · langue Auto ».
- **Refus.** Mettre `[name="bot-voice-speed"]` à `2.5` (événement `input`), cliquer « Enregistrer la voix » : `.voice-settings-notice` dit la vitesse refusée et `.voice-bot-current` garde la vitesse précédente.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere voix-bots`. Les douze PNG (`ezerbot-`, `chef-` et `refus-`, deux thèmes, deux tailles) existent.

## Gotchas

- Le faux adaptateur ne prononce rien : il enregistre la voix, la vitesse et la langue reçues (`recorded_voices`). L’écoute réelle d’une voix se vérifie sur le Mac.
- « Enregistrer la voix » est un choix de Charles : le bot garde alors cette voix, même si une autre devient libre.
- Le bot « Chef » reste dans la base jetable après le parcours.
- Fermer le dialogue avec `[data-close-dialog]` sans enregistrer.
