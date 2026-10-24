# Lecture locale des tâches

`GET /api/taches` renvoie `{ "items": [...] }`. Cet accès est en lecture seule : il ne lance ni tâche, ni requête réseau, ni décision, et ne consomme pas les Notifications. Aucun écran n'est ajouté.

Chaque ligne contient :

- `id` : identité stable préfixée par la source ;
- `kind` : `shell`, `delegation`, `task` (y compris un tour de conversation), `routine` ;
- `bot_id`, `conversation_id` : bot et conversation d'origine (conversation parente pour une délégation) ;
- `title` : libellé court fixe, sans contenu du travail ;
- `status` : `pending`, `running`, `finished`, `failed`, `stopped` ;
- `started_at` : heure de début en millisecondes Unix ;
- `group` : `decision` (À décider), `running` (En cours), `recent` (Terminées récemment) ;
- `request` : `null` ou `{ "id", "kind", "card_id" }`, où `kind` vaut `approval`, `form` ou `secret`. La conversation de cette ligne est celle où la carte de demande est affichée.

Les décisions en attente arrivent en premier (les plus anciennes d'abord), puis les travaux en cours (les plus récents d'abord), puis les **20 dernières terminaisons**, triées par heure de changement d'état décroissante. Les identités départagent les égalités. Les travaux en file et les parents attendant leurs enfants restent dans En cours. Un tour terminé sans déclaration de fin de tâche n'est pas une réussite : la tâche du bot est Arrêtée ; pour une routine ou une sous-tâche non réconciliée, c'est un Échec. Une demande remplace la ligne de son tour, de sa délégation ou de sa routine tant qu'elle attend ; chaque demande garde son identité de carte. Une commande de fond reste indépendante du tour qui l'a lancée.

Les lignes sont calculées depuis les journaux propriétaires, jamais depuis les événements non lus. Les demandes décidées, expirées, remplacées et les cartes de secret d'un démarrage précédent ne sont plus À décider. Les bots, conversations et destinations de délégation supprimés sont exclus. La récupération au démarrage existante marque les commandes interrompues Arrêtées ; un tour interrompu ne peut pas être présenté comme En cours, même si son graphe de délégation ou son occurrence n'a pas encore été réconcilié.

Un journal local d'horodatages suit les transitions d'état, indépendamment des lectures et des redémarrages. Pour les données antérieures à cette migration, dont les journaux ne conservaient pas l'heure de fin, l'heure de début sert de repli jusqu'au prochain changement d'état. Pour une ancienne commande de fond sans heure propre, c'est l'heure de début du tour source.

La réponse n'inclut ni arguments ou sorties d'outils, ni objectifs, ni résultats de sous-tâches, ni noms ou valeurs de secrets, ni texte de formulaire. Les demandes ne transportent que leur nature et leurs identifiants pour retrouver la carte dans le fil.

Tests ciblés : `cargo test -p grokbot-wrapper --locked --test tasks`.
