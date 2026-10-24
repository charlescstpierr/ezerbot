# Appel vocal

L’appel vocal est une conversation en direct, à côté du mode voix : un bouton « Appel vocal » dans l’en-tête de la conversation, un écran qui dit que l’audio est transmis au fournisseur de voix le temps de l’appel, un transcript qu’on affiche ou masque, et, en quittant, ce transcript rangé dans le fil sous « Appel vocal ». Cette sphère tourne sur le build de développement : le fournisseur est une boucle (route `/api/voice/call/debug`), sans microphone ni réseau.

## Sub-features

- `avant` montre la conversation avant l’appel (le bouton « Appel vocal » dans l’en-tête) après avoir vérifié que le mode voix, lancé seul, dit encore « L’audio reste sur ce Mac. » et que l’appel est refusé pendant le mode voix.
- `appel` montre l’appel lancé : « Appel en cours · parlez, le bot vous écoute », la phrase « L’audio de cet appel est transmis au fournisseur de voix le temps de l’appel. Il n’est pas gardé sur le Mac. », le bouton « Afficher le transcript », et le bouton Mode voix grisé (« Indisponible pendant l’appel vocal »).
- `transcript` montre le transcript affiché pendant que le bot parle (« Le bot parle · parlez ou Échap pour couper sa voix »), après avoir vérifié que parler par-dessus puis Échap coupent la voix.
- `fil` montre le fil après Quitter : la carte « Appel vocal » garde les lignes, sans lecteur audio.
- `appel-mode-voix` (tests Node et Rust, pas capture) : l’appel est refusé pendant le mode voix, et le mode voix pendant l’appel ; la PWA n’a pas de bouton ; fermer l’app termine l’appel et le transcript écrit reste.

## How to get to it (user POV)

- Ouvrir une conversation, puis cliquer sur l’icône de téléphone de l’en-tête (`.voice-call-toggle`, nom accessible « Appel vocal »), à gauche du menu « Actions de la conversation ».
- Sous 900 px de large, l’icône quitte l’en-tête pour laisser sa largeur au titre : le même geste est l’entrée « Appel vocal » (`.voice-call-menu-item`) du menu « Actions de la conversation ».
- Sans conversation ouverte ou sans fournisseur de voix, le bouton est grisé et dit pourquoi.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur le build de développement (`state.voice_call.debug_fake` vrai), la pilule Modèle `#composer-tools [data-picker="model"]` est présente.
- Une conversation ouverte (le parcours en crée une neuve à chaque passage).

- **Lancer.** Cliquer `.voice-call-toggle` : `.voice-call-bar` apparaît, `.voice-call-state` dit « Appel en cours · parlez, le bot vous écoute ».
- **Faire parler.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => window.GrokbotPanels.api("/api/voice/call/debug", {command: "feed", token: window.GrokbotPanels.state.voice_call.active.id, text: "Bonjour", final_result: true})'` : le fournisseur de boucle répond « Je vous ai entendu : Bonjour » pendant quatre secondes.
- **Couper la voix.** Un `feed` sans `final_result` pendant que le bot parle, ou un évènement `keydown` `Escape` sur `document`, ramène l’état à « Appel en cours ».
- **Quitter.** Cliquer « Quitter » dans `.voice-call-bar` : `[data-card-kind="voice_call"]` apparaît dans le fil avec les lignes.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere voix-appel`. Les seize PNG (`avant-`, `appel-`, `transcript-` et `fil-`, deux thèmes, deux tailles) existent.

## Gotchas

- Le fournisseur de boucle ne prononce rien et n’entend rien : il répond par un texte fixe. Entendre le bot, parler dans un vrai micro et le secret d’un vrai fournisseur se vérifient sur le Mac de Charles avec un fournisseur configuré. Une version de production sans fournisseur grise le bouton.
- Le secret éphémère ne s’affiche nulle part : ne pas le chercher dans l’écran, il ne sort pas de `/api/voice/call/start`.
- Chaque passage crée une conversation neuve : les conversations de test restent dans la base jetable.
