# Approbation tardive

Une carte d’approbation qui a survécu à la fin du tour du bot (« Continuer sans attendre mes approbations », ticket précédent) reçoit sa décision plus tard. Approuvée, elle fait repartir le bot dans la même conversation avec l’action exacte qu’elle liait, une seule fois ; refusée, elle le prévient sans rien exécuter ; expirée, elle est grise et ne peut plus être approuvée.

## Sub-features

- `approbation-tardive-approuvee` : après « Autoriser une fois » sur une carte dont le tour est fini, la carte dit « Autorisé une fois » et « Le bot reprend dans la conversation. », le bot répond « Tu as approuvé : envoyer le résumé… » et une seule carte d’outil porte le résultat « Résumé envoyé ».
- `approbation-tardive-expiree` : la carte `data-request-status="expired"` porte `gb-approval-expired`, n’a aucun bouton et dit « Cette demande a expiré. Aucune action n’a été faite. » ; rien n’est parti et le bot ne reprend pas.
- Le refus, la double décision (Mac et iPhone), l’empreinte différente et le redémarrage sont prouvés par les tests Rust `crates/wrapper/tests/forms.rs` et `crates/wrapper/tests/local.rs`, pas par cette sphère.

## How to get to it (user POV)

- Dans une conversation neuve, demander « Envoie le résumé ». Le chat direct continue sans attendre (réglage « Aussi dans le chat direct »), le bot dit qu’il attend l’accord et la carte « Approbation demandée » reste en attente.
- Choisir « Autoriser une fois » sur la carte, longtemps après la fin du tour.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`.
- Badge `ChatGPT`.
- La préparation de la sphère repasse en « Demander avant d'agir » (`/api/yolo/mode` sur `ask`, le Mode YOLO actif par défaut lèverait la carte), permet le shell du bot et règle `/api/approvals/waiting` sur `{task:true, chat:true, validity:"24h", reminder:"1h"}`.

- **Demander.** Nouvelle conversation (`#new-chat`), texte « Envoie le résumé ». La page contient « J’attends son accord pour envoyer le résumé. » et une carte `[data-card-kind="approval"][data-request-status="pending"]` avec « Autoriser une fois ». Le texte « Résumé envoyé » est absent.
- **Approuver tard.** Cliquer « Autoriser une fois ». Attendre `[data-request-status="resolved"]`, « Tu as approuvé : envoyer le résumé » et une seule carte `[data-card-kind="tool"]` contenant « Résumé envoyé ».
- **Expirer.** Le temps ne passe pas dans la sphère : l’échéance de la carte en attente est ramenée à une seconde avant maintenant dans `forms_requests` de la base jetable (`run.json`, clé `db`). Le tick de l’hôte (500 ms) l’expire ; `GrokbotPanels.refresh()` relit l’état.
- **Capture.** Pour chaque thème et chaque taille (`1140x780`, `820x560`), `approuvee-<thème>-<taille>.png` (la carte « Autorisé une fois »), `reprise-<thème>-<taille>.png` (le bas du même fil : résultat puis réponse du bot) et `expiree-<thème>-<taille>.png`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere approbation-tardive`. Les douze PNG arrivent sous `spheres/approbation-tardive/`.

## Gotchas

- Le faux OpenAI répond à « Envoie le résumé » par un appel `run_shell` (`printf 'Résumé envoyé'`), puis « J’attends son accord… » tant que le résultat est « En attente », et « Tu as approuvé : envoyer le résumé. C’est parti. » une fois le résultat réel reçu.
- Le tour de suite est lancé par le tick de l’hôte, pas par le clic : compter jusqu’à une demi-seconde de plus.
- L’échéance modifiée dans la base n’est pas une façon de contourner l’app : c’est l’horloge de la sphère. Ne jamais le faire sur la base du capitaine.
- Chaque combinaison thème et taille ouvre sa propre conversation et sa propre carte.
