use grokbot_domain::{
    BlobRef, BodyUnavailable, BootId, ContentDigest, ConversationId, EntryBody, ReplicaEpoch,
    ServerGeneration, TranscriptBody, TranscriptEntryId, TranscriptUpdate, UpdateKind,
    UpdateSequence,
};

use super::{Delta, Replica, ReplicaError, Snapshot, REORDER_LIMIT};

fn conversation(name: &str) -> ConversationId {
    ConversationId::new(name).unwrap()
}

fn entry(name: &str) -> TranscriptEntryId {
    TranscriptEntryId::new(name).unwrap()
}

fn epoch(recovery: u32) -> ReplicaEpoch {
    ReplicaEpoch {
        boot_id: BootId::new("boot").unwrap(),
        server_generation: ServerGeneration(1),
        recovery,
    }
}

fn epoch_with(boot_name: &str, generation: u64, recovery: u32) -> ReplicaEpoch {
    ReplicaEpoch {
        boot_id: BootId::new(boot_name).unwrap(),
        server_generation: ServerGeneration(generation),
        recovery,
    }
}

fn text(value: &str) -> TranscriptBody {
    TranscriptBody::Inline {
        content: EntryBody::Text(value.to_owned()),
    }
}

fn pending(byte: u8) -> TranscriptBody {
    TranscriptBody::Pending {
        expected_digest: ContentDigest([byte; 32]),
    }
}

fn blob() -> TranscriptBody {
    TranscriptBody::Blob {
        reference: BlobRef::new("blob-1").unwrap(),
        digest: ContentDigest([9; 32]),
    }
}

fn unavailable() -> TranscriptBody {
    TranscriptBody::Unavailable {
        reason: BodyUnavailable::Purged,
    }
}

fn upsert(sequence: u64, id: &str, body: TranscriptBody) -> TranscriptUpdate {
    TranscriptUpdate {
        sequence: UpdateSequence(sequence),
        entry_id: entry(id),
        kind: UpdateKind::Upsert { body },
    }
}

fn delete(sequence: u64, id: &str) -> TranscriptUpdate {
    TranscriptUpdate {
        sequence: UpdateSequence(sequence),
        entry_id: entry(id),
        kind: UpdateKind::Delete,
    }
}

fn snapshot(
    conversation: &ConversationId,
    recovery: u32,
    updates: Vec<TranscriptUpdate>,
) -> Snapshot {
    Snapshot {
        conversation_id: conversation.clone(),
        epoch: epoch(recovery),
        updates,
    }
}

fn delta(conversation: &ConversationId, recovery: u32, updates: Vec<TranscriptUpdate>) -> Delta {
    Delta {
        conversation_id: conversation.clone(),
        epoch: epoch(recovery),
        updates,
    }
}

fn inline_range(start: u64, count: usize) -> Vec<TranscriptUpdate> {
    (0..count)
        .map(|offset| {
            let sequence = start + offset as u64;
            upsert(sequence, &format!("e{sequence}"), text("ok"))
        })
        .collect()
}

fn install_empty(
    replica: &mut Replica,
    conversation: &ConversationId,
) -> grokbot_domain::ResumeCursor {
    replica
        .apply_snapshot(snapshot(conversation, 0, Vec::new()))
        .unwrap()
}

#[test]
fn snapshot_puis_delta_contigu_couvre_la_sequence() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    assert!(replica.cursor().is_none());

    let cursor = replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![
                upsert(1, "a", text("un")),
                upsert(2, "b", text("deux")),
                upsert(3, "c", text("trois")),
            ],
        ))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(3));
    assert!(cursor.complete);
    assert_eq!(cursor.epoch, epoch(0));

    let cursor = replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(4, "d", text("quatre"))],
        ))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(4));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("d")), Some(&text("quatre")));
}

#[test]
fn cinq_cent_treize_mises_a_jour_contiguës_s_appliquent() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    let cursor = replica
        .apply_delta(delta(&conversation, 0, inline_range(1, 513)))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(513));
    assert!(cursor.complete);
    assert!(replica.entry(&entry("e1")).is_some());
    assert!(replica.entry(&entry("e513")).is_some());
}

#[test]
fn trou_hors_fenetre_laisse_l_etat_inchange() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    let expected = before.covered_through.0 + 1;
    let got = expected + REORDER_LIMIT as u64 + 1;
    let error = replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(got, "z", text("loin"))],
        ))
        .unwrap_err();
    assert_eq!(error, ReplicaError::Gap { expected, got });
    assert_eq!(replica.cursor(), Some(before));
    assert_eq!(replica.entry(&entry("a")), Some(&text("un")));
    assert!(replica.entry(&entry("z")).is_none());
}

#[test]
fn trou_dans_la_fenetre_est_bufferise_puis_draine() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();

    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(3, "c", text("trois"))]))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(1));
    assert!(!cursor.complete);
    assert_eq!(replica.entry(&entry("a")), Some(&text("un")));
    assert!(replica.entry(&entry("c")).is_none());

    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(2, "b", text("deux"))]))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(3));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("b")), Some(&text("deux")));
    assert_eq!(replica.entry(&entry("c")), Some(&text("trois")));
}

#[test]
fn doublon_exact_de_la_sequence_1_ignore() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    assert_eq!(cursor, before);
    assert_eq!(replica.entry(&entry("a")), Some(&text("un")));
}

#[test]
fn prefixe_doublon_puis_partie_nouvelle() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![upsert(1, "a", text("un")), upsert(2, "b", text("deux"))],
        ))
        .unwrap();
    let cursor = replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![
                upsert(1, "a", text("un")),
                upsert(2, "b", text("deux")),
                upsert(3, "c", text("trois")),
            ],
        ))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(3));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("c")), Some(&text("trois")));
}

#[test]
fn contenu_different_sur_une_sequence_connue_est_un_conflit() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    let error = replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(1, "a", text("autre")), upsert(2, "b", text("deux"))],
        ))
        .unwrap_err();
    assert_eq!(error, ReplicaError::Conflict);
    assert_eq!(replica.cursor(), Some(before));
    assert!(replica.entry(&entry("b")).is_none());
}

#[test]
fn corps_pending_bloque_puis_fill_body_avance() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let cursor = replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![
                upsert(1, "a", text("un")),
                upsert(2, "b", pending(1)),
                upsert(3, "c", text("trois")),
            ],
        ))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(1));
    assert!(!cursor.complete);
    assert_eq!(replica.entry(&entry("b")), Some(&pending(1)));
    assert_eq!(replica.entry(&entry("c")), Some(&text("trois")));

    let blocked = replica.cursor();
    let error = replica
        .fill_body(&epoch(0), UpdateSequence(2), &entry("b"), pending(2))
        .unwrap_err();
    assert_eq!(error, ReplicaError::StillPending);
    assert_eq!(replica.cursor(), blocked);
    assert_eq!(replica.entry(&entry("b")), Some(&pending(1)));

    let cursor = replica
        .fill_body(&epoch(0), UpdateSequence(2), &entry("b"), text("deux"))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(3));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("b")), Some(&text("deux")));
}

#[test]
fn fill_body_inconnu_est_not_found_meme_si_le_corps_bloque() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    let error = replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("absent"), pending(1))
        .unwrap_err();
    assert_eq!(error, ReplicaError::NotFound);
}

#[test]
fn blob_et_unavailable_levent_le_blocage() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", pending(9))]))
        .unwrap();
    let cursor = replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), blob())
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(1));
    assert!(cursor.complete);

    replica
        .apply_snapshot(snapshot(&conversation, 1, vec![upsert(1, "a", pending(1))]))
        .unwrap();
    let cursor = replica
        .fill_body(&epoch(1), UpdateSequence(1), &entry("a"), unavailable())
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(1));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("a")), Some(&unavailable()));
}

#[test]
fn un_upsert_pending_anterieur_reste_bloquant_sans_ecraser_le_suivant() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![
                upsert(1, "a", pending(1)),
                upsert(2, "a", text("plus-recent")),
            ],
        ))
        .unwrap();
    assert_eq!(replica.cursor().unwrap().covered_through, UpdateSequence(0));
    assert_eq!(replica.entry(&entry("a")), Some(&text("plus-recent")));

    let cursor = replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("comble"))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(2));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("a")), Some(&text("plus-recent")));
}

#[test]
fn la_pointe_avance_meme_si_la_couverture_est_bloquee() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", pending(1))]))
        .unwrap();
    assert_eq!(replica.cursor().unwrap().covered_through, UpdateSequence(0));
    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(2, "b", text("deux"))]))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(0));
    assert!(!cursor.complete);
    assert_eq!(replica.entry(&entry("b")), Some(&text("deux")));

    let cursor = replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("un"))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(2));
    assert!(cursor.complete);
}

#[test]
fn une_suppression_ne_bloque_pas_la_couverture() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let cursor = replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![delete(1, "a"), upsert(2, "b", text("deux"))],
        ))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(2));
    assert!(cursor.complete);
    assert!(replica.entry(&entry("a")).is_none());
    assert_eq!(replica.entry(&entry("b")), Some(&text("deux")));
}

#[test]
fn epoch_etrangere_refusee_sans_application() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();

    let mut foreign = delta(&conversation, 0, vec![upsert(2, "b", text("deux"))]);
    foreign.epoch = epoch_with("boot", 1, 1);
    assert_eq!(
        replica.apply_delta(foreign).unwrap_err(),
        ReplicaError::ForeignEpoch
    );
    assert_eq!(replica.cursor(), Some(before.clone()));
    assert!(replica.entry(&entry("b")).is_none());

    let mut other_boot = delta(&conversation, 0, vec![upsert(2, "b", text("deux"))]);
    other_boot.epoch = epoch_with("autre-boot", 1, 0);
    assert_eq!(
        replica.apply_delta(other_boot).unwrap_err(),
        ReplicaError::ForeignEpoch
    );

    let mut bare = Replica::new(conversation.clone());
    assert_eq!(
        bare.apply_delta(delta(&conversation, 0, vec![upsert(1, "a", text("un"))]))
            .unwrap_err(),
        ReplicaError::ForeignEpoch
    );
    assert!(bare.cursor().is_none());
}

#[test]
fn mauvaise_conversation() {
    let id = conversation("c");
    let mut replica = Replica::new(id.clone());
    let before = replica
        .apply_snapshot(snapshot(&id, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    let other = conversation("autre");
    assert_eq!(
        replica
            .apply_snapshot(snapshot(&other, 4, vec![upsert(1, "z", text("z"))]))
            .unwrap_err(),
        ReplicaError::WrongConversation
    );
    assert_eq!(
        replica
            .apply_delta(delta(&other, 0, vec![upsert(2, "b", text("deux"))]))
            .unwrap_err(),
        ReplicaError::WrongConversation
    );
    assert_eq!(replica.cursor(), Some(before));
    assert!(replica.entry(&entry("z")).is_none());
}

#[test]
fn depassement_depuis_la_sequence_2_laisse_la_replique_vide() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = install_empty(&mut replica, &conversation);
    assert_eq!(before.covered_through, UpdateSequence(0));
    assert!(before.complete);

    let error = replica
        .apply_delta(delta(&conversation, 0, inline_range(2, 513)))
        .unwrap_err();
    assert_eq!(error, ReplicaError::ReorderOverflow);
    assert!(replica.entry(&entry("e2")).is_none());
    assert!(replica.entry(&entry("e514")).is_none());
    let cursor = replica.cursor().unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(0));
    assert!(cursor.complete);
    assert_eq!(cursor.epoch, epoch(0));
}

#[test]
fn buffer_plein_a_512_puis_depassement_vide_le_buffer() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);

    let cursor = replica
        .apply_delta(delta(&conversation, 0, inline_range(2, REORDER_LIMIT)))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(0));
    assert!(!cursor.complete);
    assert!(replica.entry(&entry("e2")).is_none());

    let error = replica
        .apply_delta(delta(&conversation, 0, inline_range(3, REORDER_LIMIT)))
        .unwrap_err();
    assert_eq!(error, ReplicaError::ReorderOverflow);

    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(1, "e1", text("ok"))]))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(1));
    assert!(cursor.complete);
    assert!(replica.entry(&entry("e2")).is_none());
    assert!(replica.entry(&entry("e3")).is_none());
}

#[test]
fn un_gap_ne_vide_pas_le_buffer() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    replica
        .apply_delta(delta(&conversation, 0, vec![upsert(2, "e2", text("ok"))]))
        .unwrap();
    let got = 1 + REORDER_LIMIT as u64 + 1;
    assert_eq!(
        replica
            .apply_delta(delta(
                &conversation,
                0,
                vec![upsert(got, "loin", text("x"))]
            ))
            .unwrap_err(),
        ReplicaError::Gap { expected: 1, got }
    );
    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(1, "e1", text("ok"))]))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(2));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("e2")), Some(&text("ok")));
    assert!(replica.entry(&entry("loin")).is_none());
}

#[test]
fn la_borne_inclusive_de_la_fenetre_est_bufferisee() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    let first = 1 + REORDER_LIMIT as u64;
    let cursor = replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(first, "bord", text("ok"))],
        ))
        .unwrap();
    assert!(!cursor.complete);
    assert!(replica.entry(&entry("bord")).is_none());
}

#[test]
fn snapshot_avec_recovery_different_remplace_les_entrees_et_vide_le_buffer() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![upsert(1, "ancien", text("avant"))],
        ))
        .unwrap();
    replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(3, "attente", text("x"))],
        ))
        .unwrap();
    assert!(!replica.cursor().unwrap().complete);

    let cursor = replica
        .apply_snapshot(snapshot(
            &conversation,
            7,
            vec![upsert(1, "nouveau", text("apres"))],
        ))
        .unwrap();
    assert_eq!(cursor.epoch.recovery, 7);
    assert_eq!(cursor.covered_through, UpdateSequence(1));
    assert!(cursor.complete);
    assert!(replica.entry(&entry("ancien")).is_none());
    assert!(replica.entry(&entry("attente")).is_none());
    assert_eq!(replica.entry(&entry("nouveau")), Some(&text("apres")));
}

#[test]
fn snapshot_desordonne_sans_trou_est_accepte() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let cursor = replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![
                upsert(2, "a", text("recent")),
                upsert(1, "a", text("ancien")),
            ],
        ))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(2));
    assert_eq!(replica.entry(&entry("a")), Some(&text("recent")));
}

#[test]
fn snapshot_malforme_ne_remplace_pas_l_etat() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    for updates in [
        vec![upsert(2, "b", text("deux"))],
        vec![upsert(1, "a", text("un")), upsert(3, "c", text("trois"))],
        vec![upsert(1, "a", text("un")), upsert(1, "a", text("un"))],
        vec![upsert(0, "z", text("zero"))],
    ] {
        assert_eq!(
            replica
                .apply_snapshot(snapshot(&conversation, 9, updates))
                .unwrap_err(),
            ReplicaError::MalformedSnapshot
        );
        assert_eq!(replica.cursor(), Some(before.clone()));
        assert_eq!(replica.entry(&entry("a")), Some(&text("un")));
    }
}

#[test]
fn snapshot_vide_est_une_resync() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    let cursor = replica
        .apply_snapshot(snapshot(&conversation, 3, Vec::new()))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(0));
    assert!(cursor.complete);
    assert_eq!(cursor.epoch.recovery, 3);
    assert!(replica.entry(&entry("a")).is_none());
}

#[test]
fn curseur_trop_ancien_sans_chevauchement() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, inline_range(1, 513)))
        .unwrap();
    assert_eq!(before.covered_through, UpdateSequence(513));
    let error = replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(1, "e1", text("autre"))],
        ))
        .unwrap_err();
    assert_eq!(error, ReplicaError::CursorTooOld);
    assert_eq!(replica.cursor(), Some(before));
}

#[test]
fn doublon_exact_trop_ancien_est_ignore() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, inline_range(1, 513)))
        .unwrap();
    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(1, "e1", text("ok"))]))
        .unwrap();
    assert_eq!(cursor, before);
}

#[test]
fn doublon_exact_a_la_borne_du_reorder_est_ignore() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, inline_range(1, 512)))
        .unwrap();
    assert_eq!(before.covered_through, UpdateSequence(512));
    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(1, "e1", text("ok"))]))
        .unwrap();
    assert_eq!(cursor, before);
}

#[test]
fn chevauchement_utile_meme_si_l_origine_est_ancienne() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, inline_range(1, 600)))
        .unwrap();
    let mut updates = inline_range(1, 600);
    updates.push(upsert(601, "e601", text("neuf")));
    let cursor = replica
        .apply_delta(delta(&conversation, 0, updates))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(601));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("e601")), Some(&text("neuf")));
}

#[test]
fn delta_non_contigu_est_un_conflit() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = install_empty(&mut replica, &conversation);
    let error = replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(1, "a", text("un")), upsert(3, "c", text("trois"))],
        ))
        .unwrap_err();
    assert_eq!(error, ReplicaError::Conflict);
    assert_eq!(replica.cursor(), Some(before));
    assert!(replica.entry(&entry("a")).is_none());
}

#[test]
fn conflit_dans_le_buffer_n_ecrase_rien() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    replica
        .apply_delta(delta(&conversation, 0, vec![upsert(2, "e2", text("ok"))]))
        .unwrap();
    let error = replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(2, "e2", text("autre")), upsert(3, "e3", text("ok"))],
        ))
        .unwrap_err();
    assert_eq!(error, ReplicaError::Conflict);

    let cursor = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(1, "e1", text("ok"))]))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(2));
    assert!(cursor.complete);
    assert_eq!(replica.entry(&entry("e2")), Some(&text("ok")));
    assert!(replica.entry(&entry("e3")).is_none());
}

#[test]
fn delta_vide_conserve_le_curseur() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let before = replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    let cursor = replica
        .apply_delta(delta(&conversation, 0, Vec::new()))
        .unwrap();
    assert_eq!(cursor, before);
}

#[test]
fn les_erreurs_sont_affichables() {
    let error = ReplicaError::Gap {
        expected: 2,
        got: 9,
    };
    let rendered = error.to_string();
    assert!(rendered.contains('2'));
    assert!(rendered.contains('9'));
    let _dyn_error: &dyn std::error::Error = &error;
}

#[test]
fn export_puis_restauration_conserve_le_curseur_et_le_buffer() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    let buffered = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(3, "c", text("trois"))]))
        .unwrap();
    assert!(!buffered.complete);
    let mut restored = Replica::restore(replica.export()).unwrap();
    assert_eq!(restored.cursor(), replica.cursor());
    assert_eq!(restored.entry(&entry("a")), Some(&text("un")));
    assert!(restored.entry(&entry("c")).is_none());
    let drained = restored
        .apply_delta(delta(&conversation, 0, vec![upsert(2, "b", text("deux"))]))
        .unwrap();
    assert_eq!(drained.covered_through, UpdateSequence(3));
    assert!(drained.complete);
    assert_eq!(restored.entry(&entry("c")), Some(&text("trois")));
}

#[test]
fn hydratation_conserve_le_replay_original_apres_restauration() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    let original = upsert(1, "a", pending(1));
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![original.clone()]))
        .unwrap();
    replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("complet"))
        .unwrap();
    let mut restored = Replica::restore(replica.export()).unwrap();
    let cursor = restored
        .apply_delta(delta(&conversation, 0, vec![original]))
        .expect("le replay original reste idempotent après hydratation");
    assert!(cursor.complete);
    assert_eq!(restored.entry(&entry("a")), Some(&text("complet")));
}

#[test]
fn hydratation_refuse_un_blob_dont_le_digest_differe() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", pending(2))]))
        .unwrap();
    assert!(replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), blob())
        .is_err());
    assert_eq!(replica.entry(&entry("a")), Some(&pending(2)));
    assert!(!replica.cursor().unwrap().complete);
}

#[test]
fn snapshot_perime_de_la_meme_epoque_conserve_log_et_buffer() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", text("un"))]))
        .unwrap();
    replica
        .apply_delta(delta(&conversation, 0, vec![upsert(3, "c", text("trois"))]))
        .unwrap();
    let before = replica.export();
    assert!(replica
        .apply_snapshot(snapshot(&conversation, 0, Vec::new()))
        .is_err());
    assert_eq!(replica.export(), before);
}

#[test]
fn restore_refuse_le_buffer_plus_grand_que_la_limite() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    let mut image = replica.export();
    image.buffer = inline_range(2, REORDER_LIMIT + 1);
    assert!(Replica::restore(image).is_err());
}

#[test]
fn restore_refuse_une_sequence_attendue_dans_le_buffer() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    let mut image = replica.export();
    image.buffer = vec![upsert(1, "a", text("un"))];
    assert!(Replica::restore(image).is_err());
}

#[test]
fn hydratation_ancienne_ne_remplit_pas_la_version_suivante() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![upsert(1, "a", pending(1)), upsert(2, "a", pending(2))],
        ))
        .unwrap();
    let first = replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("ancienne"))
        .unwrap();
    assert_eq!(first.covered_through, UpdateSequence(1));
    assert!(!first.complete);
    assert_eq!(replica.entry(&entry("a")), Some(&pending(2)));

    let complete = replica
        .fill_body(&epoch(0), UpdateSequence(2), &entry("a"), text("nouvelle"))
        .unwrap();
    assert_eq!(complete.covered_through, UpdateSequence(2));
    assert!(complete.complete);
    assert_eq!(replica.entry(&entry("a")), Some(&text("nouvelle")));
    let restored = Replica::restore(replica.export()).unwrap();
    assert_eq!(restored.entry(&entry("a")), Some(&text("nouvelle")));
    assert_eq!(restored.cursor(), Some(complete));
}

#[test]
fn hydratation_tardive_de_l_ancienne_epoque_est_refusee() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", pending(1))]))
        .unwrap();
    replica
        .apply_snapshot(snapshot(&conversation, 1, vec![upsert(1, "a", pending(1))]))
        .unwrap();
    assert_eq!(
        replica.fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("ancienne")),
        Err(ReplicaError::ForeignEpoch)
    );
    assert_eq!(replica.entry(&entry("a")), Some(&pending(1)));
    assert!(!replica.cursor().unwrap().complete);
}

#[test]
fn hydratation_refuse_la_mauvaise_entree_et_les_reponses_contradictoires() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", pending(1))]))
        .unwrap();
    assert_eq!(
        replica.fill_body(&epoch(0), UpdateSequence(1), &entry("b"), text("autre")),
        Err(ReplicaError::NotFound)
    );
    let complete = replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("complet"))
        .unwrap();
    assert_eq!(
        replica.fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("complet")),
        Ok(complete)
    );
    assert_eq!(
        replica.fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("autre")),
        Err(ReplicaError::Conflict)
    );
    assert_eq!(replica.entry(&entry("a")), Some(&text("complet")));
}

#[test]
fn restore_refuse_une_hydratation_incoherente_avec_le_journal() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", pending(1))]))
        .unwrap();
    replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("complet"))
        .unwrap();
    let mut image = replica.export();
    image.hydrations[0].expected_digest = ContentDigest([2; 32]);
    assert!(Replica::restore(image).is_err());

    let mut image = replica.export();
    image.hydrations.push(image.hydrations[0].clone());
    assert!(Replica::restore(image).is_err());
}

#[test]
fn nouveau_snapshot_meme_epoque_conserve_hydratation_et_buffer() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    replica
        .apply_snapshot(snapshot(&conversation, 0, vec![upsert(1, "a", pending(1))]))
        .unwrap();
    replica
        .fill_body(&epoch(0), UpdateSequence(1), &entry("a"), text("complet"))
        .unwrap();
    replica
        .apply_delta(delta(
            &conversation,
            0,
            vec![upsert(4, "d", text("quatre"))],
        ))
        .unwrap();
    let cursor = replica
        .apply_snapshot(snapshot(
            &conversation,
            0,
            vec![upsert(1, "a", pending(1)), upsert(2, "b", text("deux"))],
        ))
        .unwrap();
    assert_eq!(cursor.covered_through, UpdateSequence(2));
    assert!(!cursor.complete);
    assert_eq!(replica.entry(&entry("a")), Some(&text("complet")));
    assert_eq!(replica.export().buffer.len(), 1);
    let drained = replica
        .apply_delta(delta(&conversation, 0, vec![upsert(3, "c", text("trois"))]))
        .unwrap();
    assert_eq!(drained.covered_through, UpdateSequence(4));
    assert!(drained.complete);
}

#[test]
fn delta_qui_depasse_la_fenetre_par_sa_fin_est_refuse() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    let error = replica
        .apply_delta(delta(
            &conversation,
            0,
            inline_range(REORDER_LIMIT as u64, 3),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        ReplicaError::Gap {
            expected: 1,
            got: REORDER_LIMIT as u64 + 2,
        }
    );
    assert!(replica.export().buffer.is_empty());
    let mut image = replica.export();
    image.buffer = inline_range(REORDER_LIMIT as u64 + 2, 1);
    assert!(Replica::restore(image).is_err());
}

#[test]
fn chevauchement_bufferise_compte_seulement_les_sequences_nouvelles() {
    let conversation = conversation("c");
    let mut replica = Replica::new(conversation.clone());
    install_empty(&mut replica, &conversation);
    replica
        .apply_delta(delta(&conversation, 0, inline_range(2, REORDER_LIMIT - 1)))
        .unwrap();
    let cursor = replica
        .apply_delta(delta(
            &conversation,
            0,
            inline_range(REORDER_LIMIT as u64 - 1, 3),
        ))
        .expect("deux doublons et une nouveauté tiennent dans le buffer");
    assert_eq!(cursor.covered_through, UpdateSequence(0));
    assert!(!cursor.complete);
    assert_eq!(replica.export().buffer.len(), REORDER_LIMIT);
    let drained = replica
        .apply_delta(delta(&conversation, 0, inline_range(1, 1)))
        .unwrap();
    assert_eq!(drained.covered_through, UpdateSequence(513));
    assert!(drained.complete);
}
