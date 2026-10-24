# Approbation : le rappel

Une demande d’approbation qui attend ne doit pas être oubliée. Après le délai choisi dans Paramètres (« Me rappeler une demande en attente » : 30 min, 1 h, 3 h ou Jamais), Ezerbot envoie un seul rappel, avec les mêmes réglages et le même Ne pas déranger que les autres notifications. La carte du fil dit depuis combien de temps elle attend et quand elle expire, et à la reprise de l’app un bandeau dit combien de demandes attendent. Le rappel n’approuve rien et ne contient ni outil, ni argument, ni secret.

## Sub-features

- `approbation-rappel-notification` : le rappel « Une demande attend votre décision depuis 1 h » est une ligne de la liste Notifications (`dialog.notifications-dialog`, bouton `.notifications-button`), type « Action requise », sans nom d’outil ni argument. Elle n’est lue d’avance que si le fil de la carte est à l’écran et au premier plan.
- `approbation-rappel-attente` : au-dessus de la carte, la ligne grise `[data-approval-wait]` dit « En attente de vous depuis 2 h · expire dans 22 h » ; l’en-tête de la carte ne répète plus « En attente de vous ». Les boutons de décision sont inchangés.
- `approbation-rappel-bandeau` : à la reprise de l’app, `#approval-banner` dit « 1 demande attend votre décision » (« N demandes attendent votre décision » au pluriel), avec une croix pour le fermer. Le lien « Voir dans Tâches » viendra avec l’onglet Tâches : le bandeau n’en a pas encore.
- Le réglage du délai lui-même est prouvé par la sphère `approbation-non-bloquante`. L’unicité du rappel, les délais 30 min / 1 h / 3 h, « Jamais », Ne pas déranger, les notifications coupées et la demande décidée avant le délai sont prouvés par les tests Rust `crates/wrapper/tests/notifications.rs`, pas par cette sphère : l’alerte macOS elle-même ne s’ouvre pas dans le navigateur.

## How to get to it (user POV)

- Dans une conversation neuve, demander « Envoie le résumé ». Le chat direct continue sans attendre (réglage « Aussi dans le chat direct »), le bot dit qu’il attend l’accord et la carte « Approbation demandée » reste en attente.
- Laisser passer le délai du rappel, puis ouvrir Notifications : la ligne du rappel ouvre le fil sur la carte.
- Fermer puis rouvrir Ezerbot : le bandeau est en haut du fil.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`.
- La pilule Modèle `#composer-tools [data-picker="model"]` est présente.
- La préparation de la sphère permet le shell du bot, règle `/api/approvals/waiting` sur `{task:true, chat:true, validity:"24h", reminder:"1h"}` et fait expirer les cartes laissées par la combinaison précédente.

- **Demander.** Nouvelle conversation (`#new-chat`), texte « Envoie le résumé ». Une carte `[data-card-kind="approval"][data-request-status="pending"]` attend.
- **Faire passer le temps.** Le temps ne passe pas dans la sphère : la carte est vieillie de deux heures dans `wrapper_messages` et son échéance repoussée à 24 h après sa création dans `forms_requests` de la base jetable (`run.json`, clé `db`). Un autre fil (`#new-chat`) est alors à l’écran, pour que le rappel ne soit pas lu d’avance.
- **Rappel.** Cliquer `.notifications-button` : la liste contient « Une demande attend votre décision depuis 1 h », sans « run_shell » ni « printf ». Capture `rappel-<thème>-<taille>.png`.
- **Attente.** Cliquer cette ligne : le fil de la carte s’ouvre et `[data-approval-wait]` précède la carte. Capture `attente-<thème>-<taille>.png`.
- **Bandeau.** Recharger la page (`location.reload()`), attendre `#approval-banner` non masqué. Capture `bandeau-<thème>-<taille>.png`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere approbation-rappel`. Les douze PNG arrivent sous `spheres/approbation-rappel/`.
- **Mode avant.** `EZERBOT_VERIFY_BASELINE=1` rejoue les mêmes gestes sans attendre les éléments nouveaux : il sert à photographier un produit qui n’a pas encore le rappel, pour comparer.

## Gotchas

- Le bandeau ne naît qu’à la première lecture de l’état de la page : sans rechargement, il n’apparaît pas.
- La ligne d’ancienneté lit l’heure de la carte (`created_at` du message) : une carte dont l’heure est inconnue garde son en-tête habituel, sans la ligne.
- Le rappel est une notification de type « approbation » : il suit l’interrupteur « Autorisations demandées », Ne pas déranger et « Recevoir les alertes ». Coupés, la ligne reste dans la liste mais aucune alerte système n’est envoyée.
- L’échéance modifiée dans la base n’est pas une façon de contourner l’app : c’est l’horloge de la sphère. Ne jamais le faire sur la base du capitaine.
- Chaque combinaison thème et taille ouvre sa propre conversation et sa propre carte ; les cartes de la combinaison précédente sont expirées par la préparation.
