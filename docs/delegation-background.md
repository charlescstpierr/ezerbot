# Délégation sans attente

`delegate` accepte `mode: "background"` dans un Chat utilisateur direct.
Le bot peut alors terminer son tour par une phrase normale et répondre à un autre message pendant le travail enfant.
Le mode absent ou `"wait"` conserve l'attente du même run parent.
Les groupes, routines et tâches imbriquées conservent ce mode avec attente.
Une demande background dans ces contextes est refusée avant admission.

Le mode ne change ni les approbations, ni les plafonds d'admission, ni les permissions.
L'identité reste liée au run et à l'appel outil. Le digest inclut le mode.
L'enfant reçoit seulement le contexte sélectionné et termine par `complete_task`.
Aucune réponse de l'assistant n'est fabriquée par le wrapper.

## Durée de vie et retour

Le module `features/delegation/background.rs` possède le reçu de retour.
Il crée un groupe durable et ses membres dans la transaction d'admission.
Tous les enfants background du même run initial appartiennent à ce groupe.
Leurs résultats sont exclus du contexte de résultat du tour initial.

Après la fin normale du parent et de tous les enfants, le tick réserve un nouveau Chat du bot parent dans le même fil.
Il attend aussi la fin des autres runs actifs de ce fil avant cette réservation.
Le déclencheur `DelegationReturn` et l'identité du run dépendent du run initial.
La réservation du run, ses réglages de modèle, l'entrée `engine_reply` et le reçu `started` partagent un commit.
L'ordre visible est celui des entrées persistées dans le fil.

Le modèle reçoit les résultats masqués, bornés et marqués comme données non fiables.
Il produit le court résumé par le chemin de réponse habituel.
Une fin enfant sans `complete_task` fournit un échec à résumer.
Une annulation d'un enfant reste une annulation, même si sa réponse réseau arrive ensuite.

Le reçu passe à `delivered` après une fin normale du retour avec du texte assistant durable.
Le tick retrouve les reçus `pending` et `started` même sans arête active.
Une erreur de stockage conserve donc le travail à réconcilier.
Un replay retrouve la même identité et la même entrée de conversation.
L'échec ou l'arrêt du run de retour ferme son reçu sans relance automatique.

## Actions utilisateur

| Action | Effet |
| --- | --- |
| Fin normale du Chat initial | Les enfants background continuent. |
| Arrêt explicite du Chat initial | Le reçu et les descendants sont annulés, même si le Chat était déjà fini. |
| Reset du fil | Les retours en attente sont annulés avant le marqueur. |
| Suppression du fil ou d'un bot concerné | Les gardes de lancement et la réconciliation refusent le retour. |
| Suppression du fil enfant | Le groupe est annulé. |
| Régénération | L'ancienne attempt et son reçu sont invalidés avant la nouvelle. |
| Arrêt d'un enfant | Son état terminal participe au résumé groupé. Aucun résultat tardif ne le remplace. |

## Carte des contrats testés

Les tests de comportement sont dans `crates/wrapper/tests/delegation_background.rs`.
Ils utilisent le vrai moteur et le vrai fournisseur du wrapper avec un serveur modèle local contrôlé.
Une barrière bloque la réponse enfant sans bloquer le serveur.
Le test vérifie une réponse persistée au second message avant de libérer cette barrière.

| Frontière | Test |
| --- | --- |
| Admission, rollback, digest et replay | `admission_rollback_replay_and_argument_digest_keep_one_background_child_and_one_receipt` |
| Erreur d'insertion du reçu ou de ses membres | `receipt_storage_failure_leaves_no_partial_admission_and_can_be_retried` |
| Approbation refusée | `background_permission_denial_creates_no_child_and_no_delivery_receipt` |
| Disponibilité, journal, projection et redémarrage | `background_keeps_the_parent_available_and_delivers_one_later_model_reply_in_the_same_thread` |
| Attente par défaut et explicite | `omitted_mode_and_explicit_wait_preserve_the_same_parent_resume_contract` |
| Groupe d'enfants background | `background_siblings_form_one_return_after_every_member_finishes` |
| Plafond actif et promotion après fin du parent | `queued_background_children_keep_capacity_limits_and_promote_after_the_parent_finishes` |
| Lots mixtes dans les deux ordres | `a_mixed_batch_waits_only_for_wait_children_and_returns_background_results_separately` |
| Fin enfant avant la phrase initiale | `a_child_finishing_before_the_parent_acknowledges_waits_for_the_original_turn_to_finish` |
| Stop, reset, suppressions et régénération | `stop_reset_delete_and_regeneration_prevent_a_late_child_result_from_returning` |
| Arêtes terminales et réservation échouée | `terminal_children_keep_a_pending_receipt_when_return_reservation_storage_fails` |
| Reçu de livraison après fin du modèle | `a_completed_model_return_is_not_reissued_when_its_delivery_marker_needs_reconciliation` |
| Réservation commitée avant le premier appel modèle | `a_reserved_return_survives_reopening_before_its_first_model_call` |
| Invalidation pendant l'appel modèle de retour | `invalidation_during_the_return_model_call_rejects_late_text` |
| Masquage, borne et données non fiables | `return_context_masks_secrets_bounds_child_text_and_marks_it_as_untrusted_data` |
| Suivi reçu, consommé et complétion périmée | `a_followup_rejects_stale_completion_and_is_consumed_once_by_the_same_background_child` |
| Imbrication, mode et contexte refusés | `background_refuses_nested_tasks_unknown_modes_and_inaccessible_selected_context_atomically` |
| Groupe et routine | `groups_and_routine_tasks_refuse_background_and_keep_wait_delegation` |
| Nouveau tour concurrent | `return_admission_waits_for_a_concurrent_user_turn_to_finish` |
| Message reçu après réservation du retour | `a_new_user_message_during_the_return_keeps_both_replies_in_durable_order` |
| Échec enfant | `a_child_that_ends_without_complete_task_returns_a_failure_through_the_parent_model` |

Les tests internes du module vérifient aussi le contrôle d'attempt, les frontières Task Done et fin du run, l'arrêt d'un retour Interrupted et le texte durable requis pour livrer.
Le test `background_probes_use_indexes_with_tenfold_terminal_history` compte les instructions SQLite avec `StatementStatus::VmStep`.
Il multiplie l'historique terminal du même fil par dix et accepte au plus 24 instructions supplémentaires par sonde.
Il contrôle les plans `EXPLAIN QUERY PLAN` des reçus actifs, des membres, des runs actifs et des marqueurs reset.
Ce contrôle porte sur le coût des requêtes, pas sur la vitesse du modèle ou du réseau.

## Notifications et délégations du retour

Le retour possède sa propre entrée `engine_reply` et sa propre notification `reply:<run_id>`.
La projection des réponses terminées accepte son trigger `delegation_return` sur cette entrée persistée.
Elle attend la fin du run de retour.
La projection des soumissions reste limitée au trigger utilisateur, dont la référence désigne un message.
Un succès enfant conserve sa notification distincte et ne remplace pas la notification du résumé.
Un enfant échoué peut donc recevoir un résumé parent terminé et sa notification.
Un résumé échoué ou annulé ne produit aucune notification de réponse terminée.
L'outbox existante conserve les règles de visibilité, de consommation et de replay.

Le Chat de retour peut déléguer avec le mode `wait`, absent ou explicite.
Son contexte réunit les membres background du run initial et ses propres résultats wait.
Chaque arête est validée contre le run propriétaire de sa provenance et son mode.
Les résultats wait du run initial restent exclus du retour.
Le dernier groupe réveillé est conservé en entier, même au-delà des cinq résultats récents.
Les deux provenances partagent le masquage et la borne globale du bloc de résultats.
Les refus restent liés au run courant.

| Site concerné en ronde 2 | Test de comportement |
| --- | --- |
| Trigger persisté, réservation, réponse modèle et entrée `engine_reply` | `background_keeps_the_parent_available_and_delivers_one_later_model_reply_in_the_same_thread` |
| Projection `engine_reply`, bon fil, identité, consommation et réouverture | `a_return_notifies_only_its_persisted_finished_reply_once_after_open_and_restart`, parcours modèle et outbox dans `a_return_model_waits_for_its_own_child_and_resumes_with_both_result_origins` |
| Enfant échoué puis résumé parent terminé | `a_failed_child_still_notifies_the_finished_parent_return_without_a_child_success`, `a_child_that_ends_without_complete_task_returns_a_failure_through_the_parent_model` |
| Signal enfant distinct, retour en cours, échoué ou annulé | `a_child_success_stays_distinct_from_a_running_failed_or_cancelled_parent_return` |
| Projection `submission`, référence du trigger et déduplication des deux ancres utilisateur | `user_message_reply_sources_keep_their_card_and_share_one_durable_identity`, fausse correspondance de soumission dans le test de retour |
| `result_context`, dernier groupe suspendu, résultats récents et arêtes des deux parents | `a_return_model_waits_for_its_own_child_and_resumes_with_both_result_origins` |
| Wait absent et explicite, lot de six, contexte sélectionné, masquage et borne commune | Même test de retour, avec quatre scénarios complets |
| Suivi du wait enfant du retour, `InputReceived`, `InputConsumed` et complétion périmée | Scénario suivi du même test de retour, plus `a_followup_rejects_stale_completion_and_is_consumed_once_by_the_same_background_child` |
| Lots mixtes initiaux, exclusion du wait initial et modes ordinaires | `a_mixed_batch_waits_only_for_wait_children_and_returns_background_results_separately`, `omitted_mode_and_explicit_wait_preserve_the_same_parent_resume_contract` |
| Coût des deux requêtes de résultats indexées | `background_probes_use_indexes_with_tenfold_terminal_history` |
| Coût des notifications avant et après le nouveau filtre | `consumed_user_and_return_history_keeps_linear_source_cost_and_full_message_keys` |

Les autres consommateurs recensés gardent leur contrat.
`projection_hints` joint les soumissions utilisateur, tandis que le retour utilise une entrée `engine_reply`.
La liste des tâches ne crée aucune nouvelle carte pour le Chat de retour.
L'admission background et la régénération restent réservées aux triggers utilisateur.
Le stockage du trigger, le type Chat et l'assemblage du contexte valident déjà le retour.
Les autorités de stop, reset, suppression et attempt restent celles du reçu.

La sonde de notification double l'historique consommé de 1 000 à 2 000 lignes.
Elle mesure la vraie requête de sources avec `StatementStatus::VmStep`, avant et après le filtre.
Son coût attendu reste linéaire, avec un plafond explicite de deux fois la petite mesure plus 128 instructions.
Les plans avant et après doivent conserver les deux recherches par la clé complète compte, fil et message.
Aucune amélioration de vitesse réseau ou modèle n'est déduite de ces mesures.
