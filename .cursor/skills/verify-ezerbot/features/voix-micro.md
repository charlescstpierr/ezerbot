# Micro coupé

Le bouton icône micro des deux barres vocales (mode voix et appel vocal) coupe et rend le micro pour de vrai : « Couper le micro » / « Réactiver le micro » (`aria-pressed`), ⌘⇧M pendant une session. Micro coupé, la barre dit « Micro coupé · le bot ne vous entend pas » et la transcription en direct se vide. Quitter puis relancer rend le micro. Cette sphère tourne sur le build de développement : le faux pont vocal (`/api/voice/session/debug`) et le faux appel (`/api/voice/call/debug`) jouent `mute` et `unmute` et renvoient `muted`.

## Sub-features

- `mode-ecoute` montre le mode voix à l’écoute : « À l’écoute · parlez pour envoyer », bouton « Couper le micro » relâché, `muted` faux au faux pont.
- `mode-coupe` montre le mode voix après un clic sur le bouton : « Micro coupé · le bot ne vous entend pas », bouton « Réactiver le micro » enfoncé, transcription masquée, `muted` vrai au faux pont, une phrase entendue en sourdine ignorée. ⌘⇧M rend puis recoupe le micro.
- `appel-ecoute` montre l’appel à l’écoute, après avoir quitté puis relancé le mode voix (micro actif, `muted` faux).
- `appel-coupe` montre l’appel micro coupé, par clic puis par ⌘⇧M, `muted` vrai au faux appel.

## How to get to it (user POV)

- Lancer le mode voix (icône casque du compositeur) ou l’appel vocal (icône téléphone de l’en-tête) : le bouton micro est dans la barre qui apparaît, avant « Interrompre » et « Quitter ».
- ⌘⇧M, avec une session active seulement.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur le build de développement (`state.voice.debug_fake` et `state.voice_call.debug_fake` vrais), la pilule Modèle `#composer-tools [data-picker="model"]` est présente.
- Une conversation neuve (le parcours la crée et quitte toute session restante).

- **Couper.** Cliquer `.voice-session-bar .voice-mute-toggle` (ou `.voice-call-bar .voice-mute-toggle`) : `.voice-session-state` dit « Micro coupé · le bot ne vous entend pas », `aria-pressed` vaut `true`.
- **Lire le pont, appel vocal.** `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => window.GrokbotPanels.api("/api/voice/call/debug", {command: "status", token: window.GrokbotPanels.state.voice_call.active.id}).then(s => s.muted)'` : `true` micro coupé. `voice_call.active` vaut `null` hors appel, et donc en mode voix seul.
- **Lire le pont, mode voix.** Le jeton du mode voix n’est pas dans l’état de l’écran : poser le relevé avant de lancer le mode voix (`eval '() => { const api = window.GrokbotPanels.api; window.__muteTap = {}; window.GrokbotPanels.api = function (path, body) { if (path === "/api/voice/session/debug" && body?.token) window.__muteTap.token = body.token; return api.apply(this, arguments); }; }'`), lancer le mode voix, puis `eval '() => window.GrokbotPanels.api("/api/voice/session/debug", {command: "status", token: window.__muteTap.token}).then(s => s.muted)'` : `true` micro coupé. Ne pas lire le pont après « Quitter » : le faux pont refuse (409).
- **Clavier.** `eval '() => document.dispatchEvent(new KeyboardEvent("keydown", {key: "M", metaKey: true, shiftKey: true, bubbles: true, cancelable: true}))'` bascule ; sans session, rien ne se passe.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere voix-micro`. Les seize PNG (`mode-ecoute-`, `mode-coupe-`, `appel-ecoute-` et `appel-coupe-`, deux thèmes, deux tailles) existent.

## Gotchas

- Le faux pont ne capture aucun son : la fermeture réelle du micro (témoin orange de macOS éteint) se vérifie sur le Mac de Charles avec la version de production.
- Le jeton du mode voix ne s’affiche pas : le parcours lit celui que l’écran envoie au faux pont.
- Chaque passage crée une conversation neuve : les conversations de test restent dans la base jetable.
