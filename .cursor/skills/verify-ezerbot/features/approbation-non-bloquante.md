# Approbation non bloquante

Le réglage « Continuer sans attendre mes approbations » (Paramètres) décide si un appel protégé suspend le tour du bot (dix minutes, comme avant) ou si le bot continue le reste du travail pendant que la carte d’approbation attend (24 h par défaut). Rien n’est exécuté sans la décision de l’utilisateur.

## Sub-features

- `approbation-reglage` montre la section `.waiting-settings` : l’interrupteur `waiting-task` (actif par défaut, pour les routines et les tâches de fond), `waiting-chat` (« Aussi dans le chat direct », inactif par défaut), `waiting-validity` (1 h, 6 h, 24 h, 7 jours ; 24 h par défaut) et `waiting-reminder` (30 min, 1 h, 3 h, Jamais ; 1 h par défaut), avec la note sur les envois sous le nom de l’utilisateur et les suppressions.
- `approbation-inactif` coupe l’interrupteur : les trois autres commandes sont grisées et l’aide dit que le bot s’arrête et attend dix minutes, comme avant. Le réglage est relu depuis l’état du moteur après l’enregistrement.
- Le tour qui continue (résultat « En attente d’approbation », carte encore en attente, expiration sans effet) est prouvé par les tests Rust `crates/wrapper/tests/forms.rs` et `crates/engine/src/run_loop.rs`, pas par cette sphère : le faux OpenAI n’appelle pas d’outil protégé.

## How to get to it (user POV)

- Choisir le bouton du bas de barre « Paramètres de l’application » (`#settings-button`).
- Faire défiler jusqu’à « Approbations non bloquantes », au-dessus des règles en langage naturel.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`.
- Badge `ChatGPT`.
- Base neuve : le réglage est à ses valeurs par défaut.

- **Ouvrir Paramètres.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }'`. `result:` vaut `true`.
- **Lire le réglage.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { const q = (n) => document.querySelector(`.waiting-settings [name="${n}"]`); return [q("waiting-task").checked, q("waiting-chat").checked, q("waiting-validity").value, q("waiting-reminder").value]; }'`. `result:` vaut `[true,false,"24h","1h"]` sur une base neuve.
- **Couper le réglage.** Cliquer `.waiting-settings [name="waiting-task"]`. Une fois l’état relu, `waiting-validity`, `waiting-reminder` et `waiting-chat` sont `disabled`, et la section dit « pendant 10 minutes, comme avant ».
- **Capture.** Pour chaque thème et chaque taille (`1140x780`, `820x560`), `actif-<thème>-<taille>.png` puis `inactif-<thème>-<taille>.png`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere approbation-non-bloquante`. Les huit PNG (`actif` et `inactif`, deux thèmes, deux tailles) arrivent sous `spheres/approbation-non-bloquante/`.

## Gotchas

- L’interrupteur est un `input` transparent posé sur sa piste : le clic programmatique (`input.click()`) suffit, il déclenche l’enregistrement puis la relecture de l’état, qui reconstruit la section. Re-chercher les éléments après chaque clic.
- Chaque changement enregistre tout le réglage d’un coup (`POST /api/approvals/waiting`). Une durée hors liste est refusée par le moteur.
- Le rappel (`waiting-reminder`) est seulement enregistré ici : sa livraison arrive avec le ticket suivant de la série.
- La sphère laisse le réglage coupé après la capture `inactif` ; la préparation suivante le rallume. Une base jetable n’en garde aucune trace utile.
- L’iPhone n’a pas ce réglage : il reste sur le Mac.
