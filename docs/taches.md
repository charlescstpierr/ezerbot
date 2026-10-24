# Lecture locale des tâches

`GET /api/taches` renvoie `{ "items": [...] }`. Cet accès est en lecture seule : il ne lance ni tâche, ni requête réseau, ni décision, et ne consomme pas les Notifications. Aucun écran n'est ajouté.

Chaque ligne contient :

- `id` : identité stable préfixée par la source ;
- `kind` : `shell`, `delegation`, `task` (y compris un tour de conversation), `routine`, `question` ;
- `bot_id`, `conversation_id` : bot et conversation d'origine (conversation parente pour une délégation) ;
- `title` : libellé court fixe, sans contenu du travail ;
- `status` : `pending`, `running`, `finished`, `failed`, `stopped` ;
- `started_at` : heure de début en millisecondes Unix ;
- `group` : `decision` (À décider), `running` (En cours), `recent` (Terminées récemment) ;
- `question` : `null`, sauf pour une ligne `question` (texte du bot, masqué puis tronqué à 140 caractères) ;
- `request` : `null` ou `{ "id", "kind", "card_id" }`, où `kind` vaut `approval`, `form` ou `secret`. La conversation de cette ligne est celle où la carte de demande est affichée.

Les décisions en attente arrivent en premier (les plus anciennes d'abord), puis les travaux en cours (les plus récents d'abord), puis les **20 dernières terminaisons**, triées par heure décroissante. Les identités départagent les égalités. Les travaux en file et les parents attendant leurs enfants restent dans En cours. Un tour terminé sans déclaration de fin de tâche n'est pas une réussite : la tâche du bot est Arrêtée ; pour une routine ou une sous-tâche non réconciliée, c'est un Échec. Une demande remplace la ligne de son tour, de sa délégation ou de sa routine tant qu'elle attend ; chaque demande garde son identité de carte. Une commande de fond reste indépendante du tour qui l'a lancée.

Une ligne `question` (À décider, `pending`, une par conversation) existe tant que Charles n'a pas répondu à l'appel de l'outil `attendre_reponse` du bot : c'est le dernier appel de la conversation, et il est ouvert tant qu'aucun message de Charles ni aucune remise à zéro du fil ne le suit dans l'ordre du fil. Rien n'est écrit pour elle : la ligne se lit dans le journal du moteur (arguments de l'appel). Ezerbot ne devine aucune question depuis le texte d'une réponse. Une sous-tâche ne peut pas appeler l'outil. Les secrets du coffre sont masqués dans la question entière avant la troncature.

Les lignes sont calculées depuis les journaux propriétaires, jamais depuis les événements non lus. Les demandes décidées, expirées, remplacées et les cartes de secret d'un démarrage précédent ne sont plus À décider. Les bots, conversations et destinations de délégation supprimés sont exclus ; une sous-tâche n'est visible que si la chaîne de ses délégations remonte à un tour visible : sa ligne, ses commandes de fond (en cours ou terminées), les tours de son fil de tâche (approbation tardive) et ses demandes, même routées vers un autre fil, disparaissent avec la conversation d'origine, y compris pour une sous-tâche de sous-tâche. La récupération au démarrage existante marque les commandes interrompues Arrêtées ; un tour interrompu ne peut pas être présenté comme En cours, même si son graphe de délégation ou son occurrence n'a pas encore été réconcilié.

Le moteur n'est jamais écrit par cette lecture : ni table, ni déclencheur sur ses journaux. Les journaux sources ne conservent pas d'heure de fin ; les Terminées récemment sont donc triées par l'heure de début déjà enregistrée (début du tour, du tour source pour une commande de fond, heure prévue pour une routine sans tour).

Un tour, une sous-tâche ou une routine suspendu sur une demande qui n'est plus valable (décidée, expirée, remplacée, ou carte de secret d'un démarrage précédent) ne peut plus avancer : il apparaît Arrêté dans Terminées récemment, jamais En cours ni masqué. Une demande décidée dont le réveil attend encore d'être livré au moteur garde son tour En cours jusqu'à la reprise. Cette règle est calculée à la lecture.

La réponse n'inclut ni arguments ou sorties d'outils (hors la question ci-dessus), ni objectifs, ni résultats de sous-tâches, ni noms ou valeurs de secrets, ni texte de formulaire. Les demandes ne transportent que leur nature et leurs identifiants pour retrouver la carte dans le fil.

Tests ciblés : `cargo test -p grokbot-wrapper --locked --test tasks --test question`.
