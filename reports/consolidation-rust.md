# Consolidation du socle Rust

La reprise du 13 octobre 2026 consolide les six crates déjà présentes : `domain`, `wire`, `submission-journal`, `replica`, `store` et `try`. L’utilisateur a choisi les reprises après panne et demandé des sous-agents. Le socle est un système indépendant, limité aux états, aux contrats, aux projections et à SQLite.

La suite de cette phase est livrée dans [orchestration-rust.md](orchestration-rust.md), avec la septième crate `session`.

## Ce que les corrections garantissent

| Problème reproduit | Comportement retenu | Preuve du socle |
|---|---|---|
| Quatre erreurs de compilation du CLI | La démo et sa lecture depuis un autre processus s’exécutent | [Tests CLI](../crates/try/tests/cli.rs) |
| La démo effaçait une base existante | Le fichier est créé exclusivement. Un second lancement échoue sans le remplacer | [Tests CLI](../crates/try/tests/cli.rs) |
| Un `Dispatching` restauré permettait un renvoi aveugle | Le journal exige une recherche du statut. Seul un `NOT_FOUND` lié à la réconciliation courante remet le même message en file | [Journal](../crates/submission-journal/src/lib.rs) |
| L’incertitude pouvait effacer une acceptation connue | Le reçu durable reste acquis. L’absence d’écho ne permet pas de renvoyer | [Domaine](../crates/domain/src/submission.rs) |
| Une réponse tardive pouvait résoudre la mauvaise recherche | Les résultats vérifient compte, génération et identifiant de réconciliation | [Journal](../crates/submission-journal/src/lib.rs) |
| L’hydratation modifiait l’événement original, puis son replay devenait un conflit | Les événements restent immuables. Les corps retrouvés sont stockés séparément par epoch, séquence et entrée | [Réplique](../crates/replica/src/lib.rs) |
| Plusieurs versions en attente recevaient le même corps | Une résolution vise une seule version. Un digest annoncé par un Blob différent est refusé | [Tests de réplique](../crates/replica/src/tests.rs) |
| Un snapshot ancien pouvait faire reculer une réplique | Un snapshot de la même epoch doit progresser et conserver un préfixe cohérent | [Tests de réplique](../crates/replica/src/tests.rs) |
| Le buffer pouvait dépasser ses bornes au chargement ou être vidé par un chevauchement valide | Le chargement valide la fenêtre. Le quota compte les nouvelles séquences distinctes | [Tests de réplique](../crates/replica/src/tests.rs) |
| SQLite partageait les répliques entre comptes et remplaçait tous les journaux | Le compte est obligatoire à l’ouverture et fait partie des clés. Un journal mêlant plusieurs comptes est refusé | [Stockage](../crates/store/src/lib.rs) |
| Un ancien objet pouvait écraser une version récente malgré une nouvelle connexion | `StoredJournal` et `StoredReplica` portent leur propre jeton de révision, leur compte et leur base. La comparaison et l’écriture sont transactionnelles | [Stockage](../crates/store/src/lib.rs) |
| Un schéma futur ou une ancienne réplique sans propriétaire était accepté | Les migrations sont versionnées. Un schéma futur ou une propriété non attribuable produit un refus sans réécrire les données | [Stockage](../crates/store/src/lib.rs) |
| Une acceptation SQL pouvait nommer une autre clé | Le reçu embarqué doit correspondre au compte, à la conversation et au message de la ligne | [Stockage](../crates/store/src/lib.rs) |
| Un type futur réutilisant un champ connu faisait échouer le décodage | Le discriminant sélectionne les champs à décoder. Les types inconnus deviennent une entrée `Unknown` et les types connus restent validés | [Contrats](../crates/wire/src/transcript.rs) |

## Ce qui vient de l’original

Les preuves sont réutilisées depuis le bundle conservé. Le parent REA est `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`. Le résumé est `ev_a285ee38d595a45bafa84e314c107cd5641bbc45115f33ae6e32ec752dccbe11`. L’ASAR SHA-256 est `9fe94a66942f8511281f6d7c80a42432d749284c46e1e26f5b6d2f99b87652b0`.

Le client appelle la recherche du statut après une erreur de transport et ne répète l’envoi que sur `not-found`. C’est une observation statique du chemin [W8e](../artifacts/formatted/dist/electron-main/main-app.cjs:111093). Le renderer conserve l’attente d’écho pour un message dont l’acceptation était déjà acquise, même si une recherche retourne `not-found` : [branche du journal](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:71536).

Le coordinateur garde une frontière serveur avant les corps encore en attente : [curseur de couverture](../artifacts/formatted/dist/node-agent-coordinator/main.cjs:36435). Le renderer vérifie aussi la couverture et la tentative courante avant d’installer un snapshot : [ingestSnapshot](../artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:1588). Le socle reproduit les invariants locaux de séquence et de corps. La garde des tentatives asynchrones reste à ajouter au futur orchestrateur.

Ces lectures prouvent des branches du client livré. Elles n’observent aucune exécution authentifiée et n’établissent pas les garanties du serveur fermé. Le graphe REA reste partiel et les six contradictions natives restent au statut `observed-untrusted`. La session MCP actuelle ne contenait aucun record retenu correspondant. Le bundle portable et les fichiers déjà vérifiés ont suffi, sans répéter l’analyse globale.

## Ce que les essais vérifient

[verify-rust.py](../tools/verify-rust.py) exécute le formatage, les tests de tout le workspace, Clippy strict, la vérification des preuves conservées et la vérification du dossier statique. Il lance ensuite la vraie démo compilée, relit SQLite depuis un processus séparé et vérifie le refus d’un second lancement. Le rapport [rust-verification.json](../evidence/rust-verification.json) conserve les sorties, les commandes et les empreintes des sources.

[Les tests de coupure](../crates/try/tests/crash_recovery.rs) lancent un processus enfant qui quitte sans exécuter les destructeurs. Une sortie avant la sauvegarde du reçu laisse `Dispatching` et exige la réconciliation. Une sortie après le commit conserve `AcceptedAwaitingEcho`. Ces essais portent sur des messages fictifs et une vraie base SQLite locale.

Les tests SQLite exercent aussi les écritures concurrentes, les comptes partageant les mêmes identifiants, le rollback d’une écriture partielle, les migrations refusées et la persistance des corps retrouvés. Les tests de domaine et de réplique restent déterministes.

## Limites et suite du travail

Le journal reste une machine d’état en mémoire. `StoredJournal` ajoute la durabilité lorsque l’appelant sauvegarde son image. La crate `session` orchestre désormais cette sauvegarde avant son transport simulé, puis sauvegarde chaque résultat validé avant de l’annoncer.

`apply_snapshot` accepte une nouvelle epoch comme resynchronisation explicite. `RecoveryGate` et `SnapshotSession` lient désormais ce chemin à la tentative active et à la couverture demandée. Les API basses de `Replica` restent les règles pures, sans effet de stockage.

Le contrôle du digest d’un Blob compare l’empreinte annoncée. Le socle ne récupère pas ses octets et ne calcule pas le hash d’un corps en ligne. La future frontière de récupération devra valider les octets avant `fill_body`. Les cartes de type inconnu ne conservent encore que leur nom, sans leur contenu complet.

Le domaine ne contient pas encore les alias de nonces d’un renvoi manuel, les pièces jointes staged, les budgets ni les opérations réseau. Le client natif, l’authentification, le moteur de modèle, PostgreSQL, les workers et les intégrations restent à construire. Aucune CI distante, aucun déploiement ni parité complète de Grok Bot n’est établi.

Le manifeste de sauvegarde reste historique. `verify-handoff.py --evidence-only` vérifie ses artefacts et preuves immuables. Son mode strict compare encore tout le dossier et échoue sur les ajouts autorisés et le lien local `Forge compagnon.app`, qui n’appartient pas à la cible Grok Bot.
