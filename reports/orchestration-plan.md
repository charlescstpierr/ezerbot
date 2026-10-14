# Orchestration durable Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Prolonger le socle local par des commandes qui sauvegardent leur état avant un transport et refusent les continuations périmées.

**Architecture:** Une crate `grokbot-session` possède un acteur, un store et son image de journal. Des requêtes opaques portent un jeton propre à cette session et à leur tentative. Un contrôleur de récupération dans `replica` lie les snapshots à la tentative active. Les adaptateurs de transport restent synchrones et simulés pour cette phase.

**Tech Stack:** Rust existant, SQLite/rusqlite existant, bibliothèque standard. Aucune nouvelle dépendance externe, aucune clé ni requête réseau réelle.

**Spec:** [Architecture](architecture-rust.md), [Consolidation](consolidation-rust.md), demande de poursuite après livraison du socle.

## Global Constraints

- Ne modifier ni `inputs`, ni `artifacts`, ni les preuves historiques REA.
- Garder le nonce logique `MessageId` et le corps d'origine lors d'un renvoi après `NOT_FOUND`.
- Sauvegarder `Queued`, puis `Dispatching`, avant de rendre une requête d'envoi utilisable.
- Sauvegarder un jeton de réconciliation avant la recherche du statut.
- Les résultats sont annoncés seulement après leur commit SQLite.
- Une erreur de stockage recharge la dernière image durable avant toute autre opération.
- Les jetons sont opaques, liés à l'instance de session et à sa génération, et jamais réutilisés dans cette instance.
- Aucune nouvelle epoch de snapshot n'est installée sans la tentative active de récupération.
- La concurrence des écritures passe par les wrappers versionnés existants.
- Pas de Git propre au projet. Ne pas créer de commit dans le dépôt parent du Bureau.

## Review Focus

1. Erreur SQLite avant l'envoi : aucun appel au transport.
2. Réponse perdue ou crash après envoi : recherche du statut, aucun renvoi aveugle.
3. Ancienne recherche `NOT_FOUND` après une nouvelle tentative : aucune remise en file.
4. Réouverture avec une nouvelle génération légitime : reprise des messages, anciennes continuations ignorées.
5. Snapshot tardif, mauvaise couverture ou changement de source pendant la recherche : aucune mutation.

## Task 1: Commandes durables et journal

**Files:** créer `crates/session/{Cargo.toml,src/lib.rs,src/error.rs,src/submission.rs,src/tests.rs}`. Modifier les méthodes nécessaires dans `crates/submission-journal` et le domaine de soumission si nécessaire. Le responsable modifie le workspace et les callers.

**Interfaces:** `SubmissionSession::open(path, actor)`, `prepare(conversation, message, body)`, `begin_send`, `apply_send`, `begin_reconciliation`, `apply_reconciliation`, `send_with`, `reconcile_with`, `observe_echo`, `revoke`, `state`. `SendRequest` et `ReconciliationRequest` ont des champs privés et des getters pour les adaptateurs. `SubmissionTransport` expose `send` et `lookup`. Résultats distincts : acceptation, refus, résultat incertain; statut trouvé, absent, refusé, en attente ou inconnu.

- [x] Écrire des tests rouges sur les cinq frontières du journal et du transport.
- [x] Ajouter le refus journalisé, la rotation du token de réconciliation et l'adoption explicite d'une génération de compte strictement plus récente, sans effacer les états.
- [x] Implémenter la session. Les tentatives locales ont des compteurs distincts et un propriétaire `Arc` opaque. Un résultat exige propriétaire actif, génération, clé et tentative courante.
- [x] Implémenter le transport synchrone à un appel par commande, sans boucle automatique.
- [x] Vérifier les erreurs de commit avant et après le transport, les réponses tardives, la réouverture et l'isolation entre comptes et instances.

## Task 2: Tentatives de récupération de snapshot

**Files:** créer `crates/replica/src/recovery.rs` et tests associés. Modifier les exports de `crates/replica/src/lib.rs` uniquement pour ce module.

**Interfaces:** `RecoveryGate::new(conversation)`, `begin(&Replica) -> RecoveryRequest`, `apply(&mut Replica, &RecoveryRequest, Snapshot, UpdateSequence) -> Result<ResumeCursor, RecoveryError>`, `revoke`. La requête porte un propriétaire opaque, un numéro de tentative, la source initiale et la pointe minimale observée.

- [x] Écrire des tests rouges pour une ancienne tentative, une réponse après révocation, une couverture incohérente, une source modifiée et un snapshot en retard sur les deltas appliqués.
- [x] Valider le lot sans mutation avant de changer l'epoch. Un snapshot identique à la même pointe est une réussite sans effacer les hydratations ni le buffer.
- [x] N'acquitter que la tentative active. Une nouvelle recherche invalide la précédente.
- [x] Vérifier les cas d'epoch nouvelle, de réplique vide et de récupération après un échec.

## Task 3: Intégration et preuves

**Files:** workspace, CLI `try`, tests de processus, `tools/verify-rust.py`, rapports et état de reprise. Le responsable réalise cette tâche.

- [x] Ajouter une démonstration d'orchestration sur SQLite avec perte de réponse, lookup trouvé et lookup absent.
- [x] Ajouter un arrêt brutal d'enfant à l'intérieur du vrai adapter `SubmissionTransport::send`, puis reprendre via `SubmissionSession` depuis un autre processus.
- [x] Relier la garde de snapshot à une image SQLite versionnée et recharger après échec de sauvegarde.
- [x] Demander une revue indépendante du résultat complet, corriger les findings, puis lancer `python3 tools/verify-rust.py`.
- [x] Mettre à jour les limites et la prochaine étape sans revendiquer de runtime authentifié ni de parité globale.
