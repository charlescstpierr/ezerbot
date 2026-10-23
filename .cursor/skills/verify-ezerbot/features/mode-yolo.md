# Mode YOLO

Le Mode YOLO (tous les droits) est activé par défaut : le terminal et les fichiers s'ouvrent sans réglage, les approbations partent seules, et chaque action reste visible dans le fil (carte « Fait automatiquement ») et dans un journal d'activité des Paramètres.

## Sub-features

- `yolo-parametres` montre la section « Mode YOLO (tous les droits) » des Paramètres : les deux modes (`[data-yolo-mode]`, `aria-pressed`), les garde-fous qui restent toujours, un mode par bot et le journal d'activité.
- `yolo-terminal` montre la carte d'une commande de terminal faite sans demander : `article[data-auto="yolo"]`, badge « Fait automatiquement », « Commande exécutée : ls -1 » et son code de sortie.
- `yolo-ecriture` montre la carte d'une écriture de fichier faite sans demander : « Fichier créé : …/Notes-du-jour.txt », et le fichier existe dans le dossier de travail jeté du run.
- `yolo-retour` revient à « Demander avant d'agir » d'un clic : les cartes d'approbation reviennent, les réglages Terminal et Fichiers d'avant reprennent effet.
- `yolo-garde-fous` refuse sans carte tout ce qui touche les données d'Ezerbot, même en Mode YOLO.

## How to get to it (user POV)

- Ouvrir Paramètres (`#settings-button`) : la section « Mode YOLO (tous les droits) » (`[data-panel="yolo"]`) est la première après les bots.
- Dans une conversation, écrire « Liste mon dossier Documents », puis « Écris mes notes du jour » : chaque outil s'exécute sans carte d'approbation.
- Pour demander de nouveau avant d'agir, cliquer « Demander avant d'agir » dans la même section.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur cette instance.
- Badge `#mode-badge-text` égal à `ChatGPT`.
- Le faux OpenAI répond à « Liste mon dossier Documents » par un appel `run_shell` (`ls -1`) et à « Écris mes notes du jour » par un appel `write_file` dans le dossier `Documents` du run, à côté du HOME. Il ne répond ainsi que si le lancement a reçu `--sphere mode-yolo`.

- **Lancer.** Run `bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence <preuves> --sphere mode-yolo`. Le dossier `Documents` du run reçoit quelques fichiers d'exemple et l'écriture s'y fait : tout ce que contient le HOME jeté est la base d'Ezerbot, que le Mode YOLO refuse aux outils.
- **Vérifier le défaut.** `window.GrokbotPanels.state.yolo.mode` vaut `yolo` sur une base neuve, sans avoir touché aux réglages Terminal ou Fichiers.
- **Envoyer.** Run `bash .cursor/skills/verify-ezerbot/scripts/browser.sh --evidence <preuves> eval '() => { document.getElementById("new-chat").click(); return true; }'`, saisir `Liste mon dossier Documents` dans `.gb-editor[aria-label="Votre message"]`, cliquer `#send-button`, puis recommencer avec `Écris mes notes du jour`.
- **Attendre.** Deux `article[data-auto="yolo"]` portent chacun `.gb-auto-badge` « Fait automatiquement » et aucun `[data-card-kind="approval"]` n'est présent.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere mode-yolo`. Les douze PNG (`parametres-`, `terminal-` et `ecriture-`, deux thèmes, deux tailles) existent. Sur une build d'avant le Mode YOLO, les mêmes demandes s'arrêtent à leur carte d'approbation : c'est la preuve « avant ».

## Gotchas

- Le garde-fou du terminal refuse les commandes qui nomment les données d'Ezerbot, et sur macOS le shell tourne sous `sandbox-exec`, privé de ces données seulement : il n'isole pas le reste du Mac. Un `cwd` dans le HOME jeté est refusé, c'est voulu.
- Le contrôle de l'écran exige toujours une session démarrée par l'utilisateur, même en Mode YOLO : cette sphère ne le couvre pas.
- Le faux OpenAI ne prouve pas ce que le vrai modèle décide : il vérifie l'admission sans carte, l'écriture sur le disque et l'affichage.
