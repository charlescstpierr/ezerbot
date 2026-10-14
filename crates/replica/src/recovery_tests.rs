use grokbot_domain::{
    BootId, ContentDigest, ConversationId, EntryBody, ReplicaEpoch, ServerGeneration,
    TranscriptBody, TranscriptEntryId, TranscriptUpdate, UpdateKind, UpdateSequence,
};

use crate::{Delta, RecoveryError, RecoveryGate, Replica, ReplicaError, Snapshot};

fn conversation(name: &str) -> ConversationId {
    ConversationId::new(name).unwrap()
}

fn epoch(recovery: u32) -> ReplicaEpoch {
    ReplicaEpoch {
        boot_id: BootId::new("boot").unwrap(),
        server_generation: ServerGeneration(1),
        recovery,
    }
}

fn text(value: &str) -> TranscriptBody {
    TranscriptBody::Inline {
        content: EntryBody::Text(value.to_owned()),
    }
}

fn update(sequence: u64, value: &str) -> TranscriptUpdate {
    TranscriptUpdate {
        sequence: UpdateSequence(sequence),
        entry_id: TranscriptEntryId::new(format!("e{sequence}")).unwrap(),
        kind: UpdateKind::Upsert { body: text(value) },
    }
}

fn snapshot(conversation: &ConversationId, recovery: u32, count: u64) -> Snapshot {
    Snapshot {
        conversation_id: conversation.clone(),
        epoch: epoch(recovery),
        updates: (1..=count).map(|sequence| update(sequence, "ok")).collect(),
    }
}

fn installed(count: u64) -> (ConversationId, Replica, RecoveryGate) {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, count))
        .unwrap();
    let gate = RecoveryGate::new(conversation.clone());
    (conversation, replica, gate)
}

#[test]
fn requete_d_un_autre_proprietaire_ne_change_rien() {
    let (conversation, mut replica, mut gate) = installed(1);
    let mut other = RecoveryGate::new(conversation.clone());
    let foreign = other.begin(&replica).unwrap();
    let own = gate.begin(&replica).unwrap();
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &foreign,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        ),
        Err(RecoveryError::ForeignRequest)
    );
    assert_eq!(replica.export(), before);
    assert!(gate
        .apply(
            &mut replica,
            &own,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        )
        .is_ok());
}

#[test]
fn nouvelle_tentative_invalide_le_callback_precedent() {
    let (conversation, mut replica, mut gate) = installed(1);
    let old = gate.begin(&replica).unwrap();
    let current = gate.begin(&replica).unwrap();
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &old,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        ),
        Err(RecoveryError::StaleRequest)
    );
    assert_eq!(replica.export(), before);
    assert!(gate
        .apply(
            &mut replica,
            &current,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        )
        .is_ok());
}

#[test]
fn reussite_acquitte_la_tentative_une_seule_fois() {
    let (conversation, mut replica, mut gate) = installed(1);
    let request = gate.begin(&replica).unwrap();
    gate.apply(
        &mut replica,
        &request,
        snapshot(&conversation, 1, 2),
        UpdateSequence(2),
    )
    .unwrap();
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 2, 3),
            UpdateSequence(3)
        ),
        Err(RecoveryError::StaleRequest)
    );
    assert_eq!(replica.export(), before);
    let next = gate.begin(&replica).unwrap();
    assert!(next.attempt() > request.attempt());
}

#[test]
fn revocation_refuse_callbacks_et_nouvelles_tentatives() {
    let (conversation, mut replica, mut gate) = installed(1);
    let request = gate.begin(&replica).unwrap();
    gate.revoke();
    gate.revoke();
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        ),
        Err(RecoveryError::Revoked)
    );
    assert!(matches!(gate.begin(&replica), Err(RecoveryError::Revoked)));
    assert_eq!(replica.export(), before);
}

#[test]
fn couverture_declaree_doit_correspondre_au_nombre_de_mises_a_jour() {
    let (conversation, mut replica, mut gate) = installed(1);
    let request = gate.begin(&replica).unwrap();
    let before = replica.export();
    assert!(matches!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 1, 2),
            UpdateSequence(3)
        ),
        Err(RecoveryError::CoverageMismatch { .. })
    ));
    assert_eq!(replica.export(), before);
    assert!(gate
        .apply(
            &mut replica,
            &request,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        )
        .is_ok());
}

#[test]
fn changement_de_source_invalide_la_reponse() {
    let (conversation, mut replica, mut gate) = installed(1);
    let request = gate.begin(&replica).unwrap();
    replica
        .apply_snapshot(snapshot(&conversation, 1, 1))
        .unwrap();
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 2, 3),
            UpdateSequence(3)
        ),
        Err(RecoveryError::SourceChanged)
    );
    assert_eq!(replica.export(), before);
}

#[test]
fn snapshot_de_meme_epoch_ne_recule_pas_sous_la_pointe_initiale() {
    let (conversation, mut replica, mut gate) = installed(2);
    let request = gate.begin(&replica).unwrap();
    let before = replica.export();
    assert!(matches!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 1),
            UpdateSequence(1)
        ),
        Err(RecoveryError::SnapshotBehind { .. })
    ));
    assert_eq!(replica.export(), before);
}

#[test]
fn snapshot_de_meme_epoch_couvre_les_deltas_arrives_pendant_la_recherche() {
    let (conversation, mut replica, mut gate) = installed(1);
    let request = gate.begin(&replica).unwrap();
    replica
        .apply_delta(Delta {
            conversation_id: conversation.clone(),
            epoch: epoch(0),
            updates: vec![update(2, "ok"), update(3, "ok")],
        })
        .unwrap();
    let before = replica.export();
    assert!(matches!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 2),
            UpdateSequence(2)
        ),
        Err(RecoveryError::SnapshotBehind { .. })
    ));
    assert_eq!(replica.export(), before);
    let cursor = gate
        .apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 4),
            UpdateSequence(4),
        )
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(4));
}

#[test]
fn snapshot_malforme_ne_remplace_ni_epoch_ni_etat_et_garde_la_tentative() {
    let (conversation, mut replica, mut gate) = installed(1);
    let request = gate.begin(&replica).unwrap();
    let mut malformed = snapshot(&conversation, 1, 2);
    malformed.updates[1].sequence = UpdateSequence(1);
    let before = replica.export();
    assert_eq!(
        gate.apply(&mut replica, &request, malformed, UpdateSequence(2)),
        Err(RecoveryError::Replica(ReplicaError::MalformedSnapshot))
    );
    assert_eq!(replica.export(), before);
    assert!(gate
        .apply(
            &mut replica,
            &request,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        )
        .is_ok());
}

#[test]
fn snapshot_identique_preserve_corps_hydrates_et_buffer() {
    let (conversation, mut replica, mut gate) = installed(0);
    let mut initial = snapshot(&conversation, 0, 1);
    initial.updates[0].kind = UpdateKind::Upsert {
        body: TranscriptBody::Pending {
            expected_digest: ContentDigest([9; 32]),
        },
    };
    replica.apply_snapshot(initial.clone()).unwrap();
    replica
        .fill_body(
            &epoch(0),
            UpdateSequence(1),
            &initial.updates[0].entry_id,
            text("retrouve"),
        )
        .unwrap();
    replica
        .apply_delta(Delta {
            conversation_id: conversation.clone(),
            epoch: epoch(0),
            updates: vec![update(3, "ok")],
        })
        .unwrap();
    let request = gate.begin(&replica).unwrap();
    let before = replica.export();
    let cursor = gate
        .apply(&mut replica, &request, initial, UpdateSequence(1))
        .unwrap();
    assert_eq!(replica.export(), before);
    assert_eq!(cursor.covered_through, UpdateSequence(1));
    assert!(!cursor.complete);
    assert_eq!(
        replica.entry(&TranscriptEntryId::new("e1").unwrap()),
        Some(&text("retrouve"))
    );
}

#[test]
fn meme_longueur_ne_suffit_pas_pour_declarer_un_snapshot_identique() {
    let (conversation, mut replica, mut gate) = installed(1);
    let request = gate.begin(&replica).unwrap();
    let mut conflicting = snapshot(&conversation, 0, 1);
    conflicting.updates[0] = update(1, "autre");
    let before = replica.export();
    assert_eq!(
        gate.apply(&mut replica, &request, conflicting, UpdateSequence(1)),
        Err(RecoveryError::Replica(ReplicaError::Conflict))
    );
    assert_eq!(replica.export(), before);
    assert!(gate
        .apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 1),
            UpdateSequence(1)
        )
        .is_ok());
}

#[test]
fn nouvelle_epoch_peut_remplacer_la_source_par_un_snapshot_vide() {
    let (conversation, mut replica, mut gate) = installed(3);
    let request = gate.begin(&replica).unwrap();
    let cursor = gate
        .apply(
            &mut replica,
            &request,
            snapshot(&conversation, 1, 0),
            UpdateSequence(0),
        )
        .unwrap();
    assert_eq!(cursor.epoch, epoch(1));
    assert_eq!(cursor.covered_through, UpdateSequence(0));
    assert!(cursor.complete);
    assert!(replica.export().log.is_empty());
}

#[test]
fn replique_sans_epoch_accepte_un_snapshot_vide() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let mut gate = RecoveryGate::new(conversation.clone());
    let request = gate.begin(&replica).unwrap();
    assert_eq!(request.conversation_id(), &conversation);
    assert_eq!(request.source_epoch(), None);
    assert_eq!(request.minimum_sequence(), UpdateSequence(0));
    let cursor = gate
        .apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 0),
            UpdateSequence(0),
        )
        .unwrap();
    assert_eq!(cursor.epoch, epoch(0));
    assert!(cursor.complete);
}

#[test]
fn conversations_incompatibles_sont_refusees_sans_mutation() {
    let (conversation, mut replica, mut gate) = installed(1);
    let other_conversation = ConversationId::new("autre").unwrap();
    let mut other_replica = Replica::new(other_conversation.clone());
    let request = gate.begin(&replica).unwrap();
    assert!(matches!(
        gate.begin(&other_replica),
        Err(RecoveryError::WrongConversation)
    ));
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&other_conversation, 1, 2),
            UpdateSequence(2)
        ),
        Err(RecoveryError::WrongConversation)
    );
    assert_eq!(replica.export(), before);
    assert_eq!(
        gate.apply(
            &mut other_replica,
            &request,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        ),
        Err(RecoveryError::WrongConversation)
    );
    assert!(gate
        .apply(
            &mut replica,
            &request,
            snapshot(&conversation, 1, 2),
            UpdateSequence(2)
        )
        .is_ok());
}

#[test]
fn pointe_capturee_reste_le_minimum_apres_rechargement_d_une_image_plus_courte() {
    let (conversation, mut replica, mut gate) = installed(3);
    let request = gate.begin(&replica).unwrap();
    let mut shorter = Replica::new(conversation.clone());
    shorter
        .apply_snapshot(snapshot(&conversation, 0, 1))
        .unwrap();
    replica = shorter;
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 2),
            UpdateSequence(2)
        ),
        Err(RecoveryError::SnapshotBehind {
            minimum_sequence: UpdateSequence(3),
            through_sequence: UpdateSequence(2),
        })
    );
    assert_eq!(replica.export(), before);
}

#[test]
fn snapshot_identique_non_ordonne_est_acquitte_sans_mutation() {
    let (conversation, mut replica, mut gate) = installed(2);
    let request = gate.begin(&replica).unwrap();
    let copy = request.clone();
    let mut unordered = snapshot(&conversation, 0, 2);
    unordered.updates.reverse();
    let before = replica.export();
    let cursor = gate
        .apply(&mut replica, &copy, unordered, UpdateSequence(2))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(2));
    assert_eq!(replica.export(), before);
    assert_eq!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 1, 3),
            UpdateSequence(3)
        ),
        Err(RecoveryError::StaleRequest)
    );
}

#[test]
fn snapshot_vide_de_meme_epoch_est_un_noop() {
    let (conversation, mut replica, mut gate) = installed(0);
    let request = gate.begin(&replica).unwrap();
    let before = replica.export();
    let cursor = gate
        .apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 0),
            UpdateSequence(0),
        )
        .unwrap();
    assert_eq!(replica.export(), before);
    assert!(cursor.complete);
}

#[test]
fn prolongement_de_meme_epoch_preserve_hydratations_et_buffer_plus_recent() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let mut initial = snapshot(&conversation, 0, 1);
    initial.updates[0].kind = UpdateKind::Upsert {
        body: TranscriptBody::Pending {
            expected_digest: ContentDigest([7; 32]),
        },
    };
    replica.apply_snapshot(initial.clone()).unwrap();
    replica
        .fill_body(
            &epoch(0),
            UpdateSequence(1),
            &initial.updates[0].entry_id,
            text("hydrate"),
        )
        .unwrap();
    replica
        .apply_delta(Delta {
            conversation_id: conversation.clone(),
            epoch: epoch(0),
            updates: vec![update(5, "cinquieme")],
        })
        .unwrap();
    let mut gate = RecoveryGate::new(conversation.clone());
    let request = gate.begin(&replica).unwrap();
    let mut extended = snapshot(&conversation, 0, 3);
    extended.updates[0] = initial.updates[0].clone();
    let cursor = gate
        .apply(&mut replica, &request, extended, UpdateSequence(3))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(3));
    assert!(!cursor.complete);
    assert_eq!(
        replica.entry(&TranscriptEntryId::new("e1").unwrap()),
        Some(&text("hydrate"))
    );
    assert_eq!(replica.export().buffer, vec![update(5, "cinquieme")]);
    let cursor = replica
        .apply_delta(Delta {
            conversation_id: conversation,
            epoch: epoch(0),
            updates: vec![update(4, "ok")],
        })
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(5));
    assert!(cursor.complete);
}

#[test]
fn conflit_avec_un_delta_bufferise_ne_mutile_pas_la_replique() {
    let (conversation, mut replica, mut gate) = installed(1);
    replica
        .apply_delta(Delta {
            conversation_id: conversation.clone(),
            epoch: epoch(0),
            updates: vec![update(3, "buffer")],
        })
        .unwrap();
    let request = gate.begin(&replica).unwrap();
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 3),
            UpdateSequence(3)
        ),
        Err(RecoveryError::Replica(ReplicaError::Conflict))
    );
    assert_eq!(replica.export(), before);
    let mut coherent = snapshot(&conversation, 0, 3);
    coherent.updates[2] = update(3, "buffer");
    let cursor = gate
        .apply(&mut replica, &request, coherent, UpdateSequence(3))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(3));
    assert!(cursor.complete);
}

#[test]
fn pointe_minimale_inclut_les_mises_a_jour_dont_le_corps_est_en_attente() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let mut initial = snapshot(&conversation, 0, 2);
    initial.updates[0].kind = UpdateKind::Upsert {
        body: TranscriptBody::Pending {
            expected_digest: ContentDigest([1; 32]),
        },
    };
    replica.apply_snapshot(initial).unwrap();
    assert_eq!(replica.cursor().unwrap().covered_through, UpdateSequence(0));
    let mut gate = RecoveryGate::new(conversation.clone());
    let request = gate.begin(&replica).unwrap();
    assert_eq!(request.minimum_sequence(), UpdateSequence(2));
    let before = replica.export();
    assert_eq!(
        gate.apply(
            &mut replica,
            &request,
            snapshot(&conversation, 0, 1),
            UpdateSequence(1)
        ),
        Err(RecoveryError::SnapshotBehind {
            minimum_sequence: UpdateSequence(2),
            through_sequence: UpdateSequence(1),
        })
    );
    assert_eq!(replica.export(), before);
}
