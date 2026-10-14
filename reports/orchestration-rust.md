# Orchestration durable du socle local

La crate `grokbot-session` transforme les opérations du journal et de la réplique en commandes sauvegardées avant leur résultat. Le transport de cette phase est simulé. La démo `flow` et les tests de processus exercent une vraie base SQLite, sans appeler Grok Bot ni un fournisseur de modèle.

## Envoi et réconciliation

`SubmissionSession` possède un acteur fixe, sa génération, le store scoped par compte et l'image versionnée du journal. Les requêtes ont des champs privés. Leurs getters permettent à un adaptateur de lire la clé, le corps et la tentative sans les modifier.

| Commande | Écriture avant l'effet | Résultat sauvegardé |
|---|---|---|
| `prepare` | Nonce logique et corps dans `Queued` | État en file |
| `begin_send` puis `send_with` | `Dispatching` avec tentative courante | Acceptation, refus ou incertitude |
| `begin_reconciliation` puis `reconcile_with` | `OutcomeUnknown` avec nouveau token | Acceptation retrouvée, refus, maintien de l'incertitude ou `Queued` après `NOT_FOUND` |
| `observe_echo` | Aucun nouvel envoi | Confirmation de l'entrée de transcript |

Le transport réalise un seul appel par commande. `Dispatching` restauré interdit l'envoi. Un statut trouvé conserve le reçu et l'attente d'écho. Un statut absent remet en file le même identifiant et le même corps. Les statuts en attente ou inconnus conservent l'incertitude. Aucun refus ne déclenche de renvoi automatique.

La recherche de statut avant un retry est une observation statique du client livré : [W8e](../artifacts/formatted/dist/electron-main/main-app.cjs:111093). Une acceptation connue reste acquise même si l'écho tarde : [branche du renderer](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:71536). Les preuves réutilisées ont pour parent REA `ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d`. Ces observations ne démontrent pas la transaction d'acceptation du serveur fermé.

## Propriété des continuations

Une requête porte un propriétaire opaque propre à l'instance de session, sa génération et son token. Les réponses vérifient cette propriété, la clé et l'état courant avant toute mutation. Une autre session ou une réouverture ne peut pas appliquer cette requête.

La réouverture avec une génération plus récente adopte les messages du même compte sans changer les corps, les nonces ni les états. Une génération inférieure aux records durables est refusée. Si un ancien acteur découvre une génération plus récente après un conflit, toutes ses mutations sont refusées, y compris la préparation d'un nonce neuf.

Tout abandon puis rechargement d'image change le propriétaire des requêtes. Cette règle ferme un cas reproduit pendant la revue : trois ouvertures réutilisaient successivement les textes `lookup-1`, `lookup-2`, puis `lookup-1`. Après un échec CAS, une ancienne réponse `NOT_FOUND` pouvait redevenir compatible avec le token courant. Elle retourne maintenant `StaleRequest` et laisse l'état incertain intact. Une nouvelle recherche est nécessaire.

Une erreur de sauvegarde recharge la dernière image durable. Si cette lecture échoue aussi, aucune commande ne dispose d'une image utilisable jusqu'à son rechargement. Les anciennes requêtes restent invalides. Un résultat accepté par l'adaptateur dont le commit a échoué n'est pas annoncé comme une acceptation durable.

## Récupération des snapshots

`RecoveryGate` capture la conversation, l'epoch source et la pointe connue lors de `begin`. Une nouvelle recherche remplace la précédente. `apply` exige la tentative active, la source toujours courante, une couverture déclarée cohérente et une pointe suffisante pour un snapshot de la même epoch.

Le lot est validé dans une réplique candidate avant de remplacer la projection. Un snapshot identique à la pointe courante réussit sans supprimer les hydratations ni les événements bufferisés. Un changement d'epoch passe exclusivement par une réponse de la tentative active. Cette garde suit les obligations observées dans [ingestSnapshot](../artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:1588).

`SnapshotSession` ajoute le compte et le commit SQLite au contrôleur. Elle sauvegarde snapshots, deltas et hydratations avant de rendre leur curseur. Un conflit d'écriture recharge la projection courante et invalide les recherches précédentes. Un double échec de sauvegarde et de lecture désactive cette instance.

## Essais et preuve reproductible

[Les tests de processus](../crates/try/tests/orchestrated_crash.rs) quittent brutalement un enfant dans le vrai adaptateur `SubmissionTransport::send`. Ils vérifient d'abord que `Dispatching` est visible sur une autre connexion SQLite. Le serveur simulé conserve ensuite son effet dans un fichier distinct. Le parent rouvre la session avec une nouvelle génération et exerce la réconciliation.

| Point de coupure | Reprise attendue |
|---|---|
| Avant l'effet simulé | Recherche absente, remise en file, un envoi du même message |
| Après l'effet et avant le reçu local | Acceptation retrouvée, aucun nouvel envoi |
| Après le commit du reçu | Acceptation conservée, aucun envoi ni recherche |

La commande `cargo run --locked -p grokbot-try -- flow --db try-data/flow.sqlite` montre les branches trouvé et absent, une nouvelle génération de compte, un snapshot périmé refusé et deux échos conservés. Elle exige une base neuve. `status` relit ensuite les données depuis un autre processus.

[verify-rust.py](../tools/verify-rust.py) vérifie tout le workspace, le formatage, Clippy, les preuves historiques et les deux parcours CLI. [rust-verification.json](../evidence/rust-verification.json) conserve les commandes, sorties et empreintes des sources. La revue indépendante a reproduit puis vérifié les défauts d'ancien acteur et de réutilisation de token.

Le contrôle complet du 2026-10-14T00:35:53.331986+00:00 passe : 163 tests, formatage, Clippy strict, preuves historiques, démo initiale et parcours orchestré. Les empreintes du code n'ont pas changé pendant cette vérification.

## Limites et prochaine étape

Le transport reste synchrone et simulé. Aucun endpoint réel, modèle, client graphique ni runtime authentifié de Grok Bot n'est testé. Les effets du serveur de simulation sont une preuve de nos parcours, pas une promesse d'idempotence du backend fermé.

L'acteur est fourni par l'appelant. Le futur hôte doit révoquer l'ancienne session lors d'un changement de compte ou de droits. Il n'existe pas encore de registre durable de génération pour un compte sans messages. Les compteurs locaux et les propriétaires opaques ne sont pas un mécanisme d'authentification réseau.

Les API basses de `Journal` et `Replica` restent accessibles pour les règles pures et leurs tests. Les futurs adaptateurs du produit doivent passer par `SubmissionSession` et `SnapshotSession` pour obtenir les garanties de persistance et de continuation. Le transport asynchrone, les délais, le backoff et la supervision restent à ajouter.

Le hash des octets récupérés, le contenu complet des cartes inconnues, les alias de renvoi manuel et les pièces jointes staged restent hors de cette phase. La prochaine étape est un control-plane local avec une API indépendante, une acceptation idempotente, une outbox durable et un worker simulé. Elle permettra de remplacer le transport en mémoire par un vrai parcours client-serveur avant les intégrations de modèle et de cloud.
