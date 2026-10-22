use grokbot_domain::{
    AccountId, AttemptId, BlobRef, BodyUnavailable, BootId, ContentDigest, ConversationId,
    DeliveryReceipt, EntryBody, MessageId, ReconciliationId, Refusal, ReplicaEpoch, ResumeCursor,
    ServerGeneration, SubmissionState, TranscriptBody, TranscriptEntryId, TranscriptUpdate,
    UpdateKind, UpdateSequence,
};

use crate::{
    cursor_from_dto, cursor_to_dto, submission_from_dto, submission_to_dto, update_from_dto,
    update_to_dto, ResumeCursorDto, SubmissionStateDto, TranscriptUpdateDto, WireError,
    SCHEMA_VERSION,
};

fn digest_pattern() -> ContentDigest {
    let mut bytes = [0u8; 32];
    bytes[0] = 0x01;
    bytes[1] = 0xab;
    bytes[31] = 0x0f;
    ContentDigest(bytes)
}

fn receipt() -> DeliveryReceipt {
    DeliveryReceipt {
        account_id: AccountId::new("acct-1").unwrap(),
        conversation_id: ConversationId::new("conv-1").unwrap(),
        message_id: MessageId::new("msg-1").unwrap(),
    }
}

fn entry(id: &str) -> TranscriptEntryId {
    TranscriptEntryId::new(id).unwrap()
}

fn update(sequence: u64, kind: UpdateKind) -> TranscriptUpdate {
    TranscriptUpdate {
        sequence: UpdateSequence(sequence),
        entry_id: entry("entry-1"),
        kind,
    }
}

fn assert_submission_round_trip(state: SubmissionState) {
    let dto = submission_to_dto(&state);
    assert_eq!(dto.schema_version, SCHEMA_VERSION);
    let json = serde_json::to_string(&dto).unwrap();
    let parsed: SubmissionStateDto = serde_json::from_str(&json).unwrap();
    let restored = submission_from_dto(parsed).unwrap();
    assert_eq!(restored, state);
}

fn assert_update_round_trip(value: TranscriptUpdate) {
    let dto = update_to_dto(&value);
    assert_eq!(dto.schema_version, SCHEMA_VERSION);
    let json = serde_json::to_string(&dto).unwrap();
    let parsed: TranscriptUpdateDto = serde_json::from_str(&json).unwrap();
    let restored = update_from_dto(parsed).unwrap();
    assert_eq!(restored, value);
}

#[test]
fn round_trip_json_de_chaque_submission() {
    let states = [
        SubmissionState::Prepared,
        SubmissionState::Queued,
        SubmissionState::Dispatching {
            attempt: AttemptId::new("attempt-1").unwrap(),
        },
        SubmissionState::AcceptedAwaitingEcho { receipt: receipt() },
        SubmissionState::Echoed {
            entry: entry("entry-9"),
        },
        SubmissionState::Refused {
            reason: Refusal::Conflict,
        },
        SubmissionState::Refused {
            reason: Refusal::UnknownConversation,
        },
        SubmissionState::Refused {
            reason: Refusal::StaleAccountGeneration,
        },
        SubmissionState::Refused {
            reason: Refusal::Policy,
        },
        SubmissionState::Refused {
            reason: Refusal::Other("quota dépassé".to_owned()),
        },
        SubmissionState::OutcomeUnknown {
            reconciliation: ReconciliationId::new("recon-1").unwrap(),
        },
    ];
    for state in states {
        assert_submission_round_trip(state);
    }

    let prepared = serde_json::to_value(submission_to_dto(&SubmissionState::Prepared)).unwrap();
    assert_eq!(
        prepared,
        serde_json::json!({"schema_version": 1, "type": "prepared"})
    );

    let refused = submission_to_dto(&SubmissionState::Refused {
        reason: Refusal::Other("quota dépassé".to_owned()),
    });
    let refused_json = serde_json::to_value(&refused).unwrap();
    assert_eq!(refused_json["type"], "refused");
    assert_eq!(
        refused_json["reason"],
        serde_json::json!({"type": "other", "message": "quota dépassé"})
    );
}

#[test]
fn round_trip_upsert_pending_et_delete() {
    let pending = update(
        4,
        UpdateKind::Upsert {
            body: TranscriptBody::Pending {
                expected_digest: digest_pattern(),
            },
        },
    );
    assert_update_round_trip(pending.clone());
    let pending_json = serde_json::to_value(update_to_dto(&pending)).unwrap();
    assert_eq!(pending_json["schema_version"], 1);
    assert_eq!(pending_json["sequence"], 4);
    assert_eq!(pending_json["entry_id"], "entry-1");
    assert_eq!(pending_json["type"], "upsert");
    assert_eq!(pending_json["body"]["type"], "pending");
    assert_eq!(
        pending_json["body"]["expected_digest"],
        "01ab00000000000000000000000000000000000000000000000000000000000f"
    );

    let deleted = update(5, UpdateKind::Delete);
    assert_update_round_trip(deleted.clone());
    let deleted_json = serde_json::to_value(update_to_dto(&deleted)).unwrap();
    assert_eq!(deleted_json["type"], "delete");
    assert!(deleted_json.get("body").is_none());

    assert_update_round_trip(update(
        6,
        UpdateKind::Upsert {
            body: TranscriptBody::Inline {
                content: EntryBody::Text("bonjour".to_owned()),
            },
        },
    ));
    assert_update_round_trip(update(
        7,
        UpdateKind::Upsert {
            body: TranscriptBody::Inline {
                content: EntryBody::Unknown {
                    kind: "carte".to_owned(),
                },
            },
        },
    ));
    assert_update_round_trip(update(
        8,
        UpdateKind::Upsert {
            body: TranscriptBody::Blob {
                reference: BlobRef::new("blob-1").unwrap(),
                digest: digest_pattern(),
            },
        },
    ));
    assert_update_round_trip(update(
        9,
        UpdateKind::Upsert {
            body: TranscriptBody::Unavailable {
                reason: BodyUnavailable::Expired,
            },
        },
    ));
}

#[test]
fn contenu_mystery_devient_entree_inconnue() {
    let inline = r#"{
        "schema_version": 1,
        "sequence": 3,
        "entry_id": "entry-3",
        "type": "upsert",
        "body": {
            "type": "inline",
            "content": {"type": "mystery", "title": "carte"}
        }
    }"#;
    let dto: TranscriptUpdateDto = serde_json::from_str(inline).unwrap();
    let restored = update_from_dto(dto).unwrap();
    assert_eq!(
        restored.kind,
        UpdateKind::Upsert {
            body: TranscriptBody::Inline {
                content: EntryBody::Unknown {
                    kind: "mystery".to_owned(),
                },
            },
        }
    );

    let bare = r#"{
        "schema_version": 1,
        "sequence": 3,
        "entry_id": "entry-3",
        "type": "upsert",
        "body": {"type": "mystery", "title": "carte"}
    }"#;
    let dto: TranscriptUpdateDto = serde_json::from_str(bare).unwrap();
    let restored = update_from_dto(dto).unwrap();
    assert_eq!(
        restored.kind,
        UpdateKind::Upsert {
            body: TranscriptBody::Inline {
                content: EntryBody::Unknown {
                    kind: "mystery".to_owned(),
                },
            },
        }
    );
}

#[test]
fn contenu_inconnu_ignore_les_champs_homonymes_des_variantes_connues() {
    for body in [
        serde_json::json!({"type": "mystery", "content": {"title": "future card"}}),
        serde_json::json!({"type": "inline", "content": {
            "type": "mystery", "text": {"value": "future text"}, "kind": 42
        }}),
    ] {
        let dto: TranscriptUpdateDto = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "sequence": 1, "entry_id": "future-entry",
            "type": "upsert", "body": body
        }))
        .expect("un type inconnu ne doit pas décoder les champs d'un type connu");
        assert_eq!(
            update_from_dto(dto).unwrap().kind,
            UpdateKind::Upsert {
                body: TranscriptBody::Inline {
                    content: EntryBody::Unknown {
                        kind: "mystery".into()
                    }
                }
            }
        );
    }
}

#[test]
fn contenu_connu_refuse_un_champ_de_type_incorrect() {
    for body in [
        serde_json::json!({"type": "inline", "content": {"type": "text", "text": {"value": "bad"}}}),
        serde_json::json!({"type": "blob", "reference": {"value": "bad"}, "digest": "00"}),
    ] {
        assert!(
            serde_json::from_value::<TranscriptUpdateDto>(serde_json::json!({
                "schema_version": 1, "sequence": 1, "entry_id": "known-entry",
                "type": "upsert", "body": body
            }))
            .is_err()
        );
    }
}

#[test]
fn submission_type_inconnu_est_une_erreur() {
    let dto: SubmissionStateDto =
        serde_json::from_str(r#"{"schema_version":1,"type":"nope"}"#).unwrap();
    assert_eq!(
        submission_from_dto(dto).unwrap_err(),
        WireError::UnknownSubmissionState
    );
}

#[test]
fn schema_version_2_est_une_erreur() {
    let mut submission =
        serde_json::to_value(submission_to_dto(&SubmissionState::Prepared)).unwrap();
    submission["schema_version"] = serde_json::json!(2);
    let dto: SubmissionStateDto = serde_json::from_value(submission).unwrap();
    assert_eq!(
        submission_from_dto(dto).unwrap_err(),
        WireError::UnsupportedVersion { version: 2 }
    );

    let mut update = serde_json::to_value(update_to_dto(&update(1, UpdateKind::Delete))).unwrap();
    update["schema_version"] = serde_json::json!(2);
    let dto: TranscriptUpdateDto = serde_json::from_value(update).unwrap();
    assert_eq!(
        update_from_dto(dto).unwrap_err(),
        WireError::UnsupportedVersion { version: 2 }
    );

    let cursor = ResumeCursor {
        epoch: ReplicaEpoch {
            boot_id: BootId::new("boot-1").unwrap(),
            server_generation: ServerGeneration(7),
            recovery: 2,
        },
        covered_through: UpdateSequence(11),
        complete: true,
    };
    assert_eq!(cursor_from_dto(cursor_from_json(&cursor)).unwrap(), cursor);

    let mut cursor_json = serde_json::to_value(cursor_to_dto(&cursor)).unwrap();
    cursor_json["schema_version"] = serde_json::json!(2);
    let dto: ResumeCursorDto = serde_json::from_value(cursor_json).unwrap();
    assert_eq!(
        cursor_from_dto(dto).unwrap_err(),
        WireError::UnsupportedVersion { version: 2 }
    );
}

fn cursor_from_json(cursor: &ResumeCursor) -> ResumeCursorDto {
    let dto = cursor_to_dto(cursor);
    let json = serde_json::to_string(&dto).unwrap();
    serde_json::from_str(&json).unwrap()
}

#[test]
fn curseur_avec_boot_id_vide_est_une_erreur() {
    let dto = ResumeCursorDto {
        schema_version: 1,
        epoch: crate::ReplicaEpochDto {
            boot_id: String::new(),
            server_generation: 7,
            recovery: 2,
        },
        covered_through: 11,
        complete: true,
    };

    assert_eq!(cursor_from_dto(dto), Err(WireError::EmptyId));
}

#[test]
fn messages_des_erreurs_wire_en_francais() {
    let cases = [
        (
            WireError::UnsupportedVersion { version: 2 },
            "version de schéma 2 non prise en charge",
        ),
        (
            WireError::UnknownSubmissionState,
            "état de soumission inconnu",
        ),
        (
            WireError::InvalidDigest,
            "empreinte hexadécimale invalide : 64 caractères minuscules attendus",
        ),
        (WireError::EmptyId, "identifiant vide"),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn digest_hex_invalide_est_une_erreur() {
    let cases = [
        "zzzz",
        "01ab",
        "01AB00000000000000000000000000000000000000000000000000000000000f",
        "",
    ];
    for digest in cases {
        let raw = format!(
            r#"{{
                "schema_version": 1,
                "sequence": 1,
                "entry_id": "entry-1",
                "type": "upsert",
                "body": {{"type": "pending", "expected_digest": "{digest}"}}
            }}"#
        );
        let dto: TranscriptUpdateDto = serde_json::from_str(&raw).unwrap();
        assert_eq!(update_from_dto(dto).unwrap_err(), WireError::InvalidDigest);
    }
}
