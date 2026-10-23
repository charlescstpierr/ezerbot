# Tâches de fond

Une commande lancée par le bot en arrière-plan reste une carte du fil : En cours, avec la sortie et Arrêter, puis Terminée et son code. Cette sphère couvre la saisie d’une ligne sur l’entrée standard de la tâche, depuis un champ de la carte.

## Sub-features

- `invite` montre la carte En cours devant l’invite « Continuer ? [oui/non] », sa sortie ouverte, le bouton Arrêter et le champ « Saisie pour la tâche » avec Envoyer.
- `saisie` montre la même carte après l’envoi de « oui » : Terminée, code 0, la sortie contient la ligne recopiée « oui » puis « reponse=oui », et le champ a disparu. Une saisie envoyée ensuite à cette tâche est refusée en français (« Cette tâche n’est plus en cours : la saisie n’a pas été envoyée. ») sans changer la sortie.
- `mot-de-passe` montre une tâche qui demande « Mot de passe : » : la ligne proposée n’est pas écrite, la carte le dit (« Ce programme demande un mot de passe : la saisie n’a pas été envoyée. »), la sortie ne la contient pas, la ligne reste dans le champ et la tâche reste En cours.
- `controle-ecran` (tests Rust, pas capture) : le refus d’un champ mot de passe par le contrôle de l’écran (SCR-2) n’est pas touché par cette sphère.

## How to get to it (user POV)

- Dans une conversation où le shell est accordé au bot, demander une commande longue qui attend une réponse. Après « Autoriser une fois », la carte du fil passe En cours et montre le champ de saisie sous Arrêter.
- Écrire une ligne dans le champ, puis Envoyer (ou Entrée). La ligne apparaît dans la sortie de la carte, que « Afficher la sortie » ouvre.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok` sur cette instance, badge `#mode-badge-text` égal à `ChatGPT`.
- Le faux OpenAI répond à « Lance le script de saisie » et à « Lance le script du mot de passe » par un appel `run_shell` en arrière-plan (dossier `/tmp`), puis par « La tâche tourne. ».

- **Lancer.** Run `bash .cursor/skills/verify-ezerbot/scripts/launch.sh --evidence <preuves> --sphere taches-de-fond`. Sans `--sphere taches-de-fond`, le wrapper répond « OK » à tout et aucune commande ne part.
- **Demander.** Cliquer `#new-chat`, saisir `Lance le script de saisie` dans `.gb-editor[aria-label="Votre message"]`, cliquer `#send-button`, puis « Autoriser une fois » sur `[data-card-kind="approval"]`.
- **Attendre la carte.** `[data-card-kind="tool"][data-local-state="en cours"]` contient `.gb-local-input input` et la sortie « Continuer ? [oui/non] ».
- **Envoyer.** Saisir `oui` dans `.gb-local-input input` (événement `input`), soumettre le formulaire `.gb-local-input` : l’état devient `terminé`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere taches-de-fond`. Les douze PNG (`invite-`, `saisie-` et `mot-de-passe-`, deux thèmes, deux tailles) existent.

## Gotchas

- Chaque passage ouvre une conversation neuve et arrête d’abord les tâches encore En cours des passages précédents.
- Le faux OpenAI ne prouve pas ce que le vrai modèle lance : il prouve la carte, l’approbation, le processus réel et le refus. Un programme qui demande un mot de passe sans le dire dans son invite reçoit encore la ligne : on refuse la classe connue, on ne devine pas tous les secrets.
- Les PNG montrent la carte, pas le processus : « le processus a lu la ligne » se vérifie par la ligne `reponse=oui`, écrite par le programme lui-même.
