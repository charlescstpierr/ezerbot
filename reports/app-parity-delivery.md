# Rendre notre app capable du travail de Grok Bot

L’objectif utilisateur couvre les capacités réelles, y compris les groupes, les échanges entre bots, les outils et l’usage depuis l’iPhone. L’analyse du produit original reste une source de référence. Une fonctionnalité ne sera déclarée livrée qu’après vérification de son parcours dans notre app.

Le code relu le 17 octobre 2026 possède déjà un wrapper Mac, un historique SQLite, des profils de bots et un adaptateur ChatGPT en streaming. Il conserve un mode simulé sans connexion. L’état d’un compte utilisateur vivant n’a pas été consulté. `wrapper_conversations.bot_id`, `Conversation.bot_id`, la création et `start_generation` limitent actuellement le fil à un bot. La requête du fournisseur transmet un historique rôle/texte; son lecteur de streaming traite les deltas texte, sans cycle interbot implémenté dans ce chemin.

| Lot | Résultat utilisable et critère de fin |
|---|---|
| Conversations à plusieurs bots | Ajouter des membres à un fil existant, mentions par ID, auteurs distincts, tours durables et délégation bornée dans le groupe; vérifier en API et navigateur. |
| Moteur d’agent et outils | Boucle modèle/outils, progression, interruption, shell/fichiers/navigateur et erreurs explicites; terminer de vrais scénarios avec critères de résultat. |
| Contrôle du Mac | Helper indépendant, permissions OS, admission, contrôle humain et annulation; vérifier sur un Mac de test. Les noms de RPC de l’original ne suffisent pas. |
| Mémoire et contexte | Mémoire scoped, correction/suppression, résumés et compaction mesurables; vérifier ce que chaque bot reçoit effectivement. |
| Routines, skills et connecteurs | Exécution planifiée durable, fuseaux, occurrences, OAuth/MCP et révocation, versions de skills; vérifier pannes et effets réels. |
| Fichiers et médias | Pièces jointes, documents exportables, dictée, voix et démonstrations; vérifier les formats et permissions concernés. |
| Ordinateur distant et iPhone | Exécution indépendante du Mac ouvert, identité commune, transport authentifié, synchronisation et notifications; vérifier sur les deux appareils. |
| Partage et exploitation | Bots d’équipe, rôles, invitations, budgets, sauvegarde/restauration, mises à jour et intégrations de publication/dépenses si retenues; vérifier chaque fournisseur séparément. |

Ces lots regroupent les [45 familles historiques](parity.md). Ils ne sont pas un pourcentage terminé et ne promettent pas les algorithmes propriétaires de l’original. Les corps serveur inconnus doivent être remplacés par nos propres décisions de conception et vérifiés dans notre système.

Le premier lot possède une [proposition concrète](../docs/superpowers/specs/2026-10-17-multibot-design.md). Elle recommande de faire évoluer le moteur Rust existant avant d’extraire ses frontières réseau pour le téléphone. La proposition n’est pas encore approuvée; aucun code produit de ce lot n’a été écrit pendant cette passe. Les changements déjà présents dans l’interface et le fournisseur appartiennent à l’état existant et sont préservés.
