# Suite de l'enquête sur Grok Bot 0.70.2

La demande de goal suivante est traitée dans [les neuf sujets approfondis](goal-nine-topics.md). Elle ajoute contexte/mémoire/apprentissage, exécution/autonomie, permissions/secrets et ressources/traçabilité avec leurs ledgers. La présente passe conserve les premiers chemins cron, ordinateur, compaction, délégation, runtime et personnalité.

Cette passe approfondit les routines, les ordinateurs, la compaction, la délégation et les runtimes. Elle ajoute une comparaison des capacités compilées avec les rapports précédents. Le client sauvegardé révèle les chemins d'appel et leurs frontières. Le moteur distant et le provisioning de production restent absents.

La demande suivante ajoute [la personnalité des bots](bot-personality-detail.md) et [le cycle de délégation](delegation-lifecycle-deep.md). La voie vocale possède un prompt local qui utilise la description du bot comme contrainte de rôle et de voix. Le prompt texte complet reste absent.

Les recherches ciblées parcourent 600 fichiers JavaScript. L'inventaire historique de 484 entrées RPC sert à repérer les méthodes peu documentées. Une absence de nom ou de candidat d'appel reste bornée à ces recherches; elle ne prouve pas l'absence de fonction dans le produit.

## Comment il configure les cronjobs

Une routine contient un nom, un prompt et un déclencheur. Le client transmet création, modification et test à la gateway. Aucun `crontab` installé sur le Mac n'est observé dans ces chemins. La réponse contient un record JSON et un `filePath`; son contenu distant et son écrivain restent inconnus.

Le schéma inclut cron, plusieurs providers d'événements et des groupes de listeners. Le calendrier compilé sait lire cinq champs cron, `CRON_TZ` ou `TZ` et des intervalles `@every`. Les horaires exacts et flexibles ont des présentations distinctes. La présence du calendrier ne prouve pas un scheduler distant; le caller actif du calcul cron de prochaine occurrence n'est pas établi.

La routine peut exécuter son propre prompt. Une skill reste optionnelle. Le rattrapage des retards, l'heure d'été, la concurrence et la déduplication distante restent inconnus. Voir [routines et cron](cron-detail.md).

## Comment il rejoint son ordinateur

Le backend fournit ou localise une box, avec gateway, URLs VNC et tokens. Le coordinator transmet les commandes à cette gateway et adapte les URLs VNC main et fork. Electron affiche le bureau dans un webview noVNC.

Le bouton de prise de contrôle change le booléen local qui autorise la saisie dans le viewer. Ce callback ne prouve pas un verrou exclusif ou une pause du bot côté serveur. Le handoff possède un autre parcours de reprise, dont `EndGrokBotBoxHandoff` avec agent, request et trigger.

Une mise à jour d'image demande une reconstruction avec conservation des données. Reset et recover appellent force-recreate. Ces noms et commandes ne prouvent pas quels fichiers le backend conserve ou supprime. Voir [ordinateur cloud et VNC](computer-detail.md).

## Comment il utilise le Mac de l'utilisateur

Un daemon distinct reçoit des demandes de la box ou du backend. Son registre actif sert shell en flux, shell en arrière-plan, lecture et liste de fichiers. Il ajoute transferts, Messages et découverte de configuration d'outils IA. La racine de fichiers contrôle les symlinks mais ne confine pas les commandes shell.

Messages utilise un helper macOS par socket. La nouvelle inspection native confirme ses déclarations de permissions et des signatures Swift pour snapshots, pièces jointes, contacts et préparation d'envoi. Des indices de capture et de contrôle graphique existent aussi dans le binaire. Le constructeur actif du daemon ne les enregistre pas comme outil computer-use. Voir [exécution locale](execution.md:81) et [helper natif](native-helper-detail.md).

## Comment il compacte

Le package livre une configuration de résumé spéculatif, un contrat `pre_compact`, une action de résumé partagée et des résumés de mémoire d'équipe. La compaction effective reste partiellement inconnue.

70 % est une valeur de repli compilée. Aucun consommateur Grok Bot de cette configuration n'est établi dans cette passe. Ce n'est donc pas un seuil effectif démontré. Le hook déclare tokens, fenêtre de contexte, messages concernés et modèle, mais son exécution locale n'est pas démontrée.

Pagination du transcript, mémoire et nouvelle conversation restent distinctes de la compaction. Le prompt de résumé, les messages conservés et les checkpoints demandent des preuves du moteur distant. Voir [compaction et runtimes](compaction-runtimes.md).

## Comment il délègue

Le client lit les sous-agents, suit leurs événements, rattache les tâches aux sessions et crée des rooms avec bots et humains. Le protocole déclare messages de bot à bot, tours de room, nonces, deadlines et résultats SENT, PASS, TIMEOUT ou CANCELLED.

Le choix du destinataire, le contexte transmis, les budgets et l'ordonnanceur restent distants. Les noms d'envoi et de tours ciblés ne sont retrouvés que dans les descripteurs. Le suivi des sous-agents a, lui, des appels actifs.

Les agents de code cloud constituent une autre famille. Le client appelle `ListWorkersForManager`, compte des workers CREATING ou RUNNING et lit PR, diffs, résumés et artefacts. Ce modèle manager/workers est distinct des sous-agents Grok. Voir [délégation](other-capabilities-detail.md:30).

## Comment il gère les runtimes

Le main autorise le compte et possède les services système. Le renderer dispose d'un port vers un coordinator séparé. Ce coordinator compose gateway, tail serveur, supervision du daemon et providers WebAuthn et cookies.

Le changement de compte versionne les transitions, révoque les ports et attend la sortie de l'ancien coordinator. Si cette sortie n'est pas confirmée au délai prévu, le remplacement reste bloqué. Une résolution tardive peut permettre la reprise.

Le routage distingue `box`, `temporal` et `unsupported`. La relance du coordinator utilise un backoff; le daemon possède heartbeat et leases. Annuler un RPC, interrompre un run et arrêter un shell background ont des portées différentes. Ces mécanismes prouvent la gestion des connexions client, pas une reprise du modèle au token interrompu. Voir [runtimes](compaction-runtimes.md).

## Les autres capacités mieux établies

Les nouvelles traces couvrent comptes financiers connectés et réparation, avatars générés par IA, workers cloud et édition ou partage de documents et canvas. Elles précisent meetings, démonstrations save/discard, propositions d'apps hébergées, dépenses et credentials. L'image retrouvée concerne des avatars carrés; les liaisons bancaires ont un parcours hébergé et un état de complétion non confirmée. Voir [autres capacités](other-capabilities-detail.md:80).

## Couverture de cette passe

L'approfondissement demandé ensuite ajoute [l'admission native au socket](native-peer-deep.md). Huit fonctions x86_64 ont été suivies jusqu'aux branches de refus et aux appels Security. Les métadonnées natives et ce désassemblage restent distincts d'une observation du helper en fonctionnement.

| Question | Statut | Ce qui reste |
|---|---|---|
| Personnalité des bots | Partiellement répondue | Prompt vocal et configuration retracés; prompt texte complet distant |
| Configuration des cronjobs | Partiellement répondue | Scheduler, stockage et garanties d'occurrences distants |
| Accès à l'ordinateur du bot | Répondue statiquement côté client | Provisioning, isolation et contrôle serveur |
| Accès au Mac de l'utilisateur | Partiellement répondue | Corps natifs et voie graphique effectivement active |
| Compaction | Partiellement répondue | Algorithme, seuil effectif, prompt et checkpoints |
| Délégation | Partiellement répondue | Choix des enfants, contexte hérité et propagation des arrêts |
| Runtimes | Répondue statiquement côté client | Internes des moteurs distants |
| Autres capacités | Inventaire comparé et nouvelles traces ciblées | Branches dynamiques, compte réel, serveur et autres plateformes |

Aucun scénario authentifié ni code Rust produit n'a été ajouté par cette passe. Les changements préexistants du wrapper restent hors de cette enquête. Les inconnues ne sont pas remplacées par des choix de reconstruction présentés comme des faits.
