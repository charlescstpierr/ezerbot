use std::collections::{BTreeSet, HashSet};
use std::error::Error;

use super::*;

fn attempt(id: &str) -> AttemptId {
    AttemptId::new(id).unwrap()
}

fn entry(id: &str) -> TranscriptEntryId {
    TranscriptEntryId::new(id).unwrap()
}

fn reconciliation(id: &str) -> ReconciliationId {
    ReconciliationId::new(id).unwrap()
}

fn receipt(message_id: &str) -> DeliveryReceipt {
    DeliveryReceipt {
        account_id: AccountId::new("compte").unwrap(),
        conversation_id: ConversationId::new("conversation").unwrap(),
        message_id: MessageId::new(message_id).unwrap(),
    }
}

#[test]
fn identifiant_vide_refuse() {
    macro_rules! check_id {
        ($ty:ty) => {{
            assert_eq!(<$ty>::new(""), Err(EmptyId));
            assert_eq!(<$ty>::new(String::new()), Err(EmptyId));
            assert_eq!(<$ty>::new("abc").unwrap().as_str(), "abc");
        }};
    }

    check_id!(AccountId);
    check_id!(TeamId);
    check_id!(BotId);
    check_id!(ConversationId);
    check_id!(SessionId);
    check_id!(RunId);
    check_id!(MessageId);
    check_id!(ToolCallId);
    check_id!(ApprovalId);
    check_id!(ComputerId);
    check_id!(ScreenLeaseId);
    check_id!(BootId);
    check_id!(AttemptId);
    check_id!(TranscriptEntryId);
    check_id!(ReconciliationId);
    check_id!(CheckpointId);
    check_id!(HumanInputId);
    check_id!(ActionGrantId);

    assert_eq!(
        AccountId::new("pas-un-uuid").unwrap().as_str(),
        "pas-un-uuid"
    );
    assert_eq!(AccountId::new(" ").unwrap().as_str(), " ");

    let err = AccountId::new("").unwrap_err();
    let as_error: &dyn Error = &err;
    assert_eq!(as_error.to_string(), "identifiant vide");

    let mut hashed = HashSet::new();
    hashed.insert(AccountId::new("a").unwrap());
    assert!(hashed.contains(&AccountId::new("a").unwrap()));

    let mut ordered = BTreeSet::new();
    ordered.insert(AccountId::new("b").unwrap());
    ordered.insert(AccountId::new("a").unwrap());
    let labels: Vec<_> = ordered.iter().map(|id| id.as_str().to_owned()).collect();
    assert_eq!(labels, vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn accept_idempotent_meme_receipt_receipt_different_refuse() {
    let same = receipt("msg-1");
    let accepted = SubmissionState::Dispatching {
        attempt: attempt("tentative-1"),
    }
    .accept(same.clone())
    .unwrap();

    let again = accepted.clone().accept(same).unwrap();
    assert_eq!(again, accepted);

    let err = again.accept(receipt("msg-2")).unwrap_err();
    let as_error: &dyn Error = &err;
    assert_eq!(
        as_error.to_string(),
        "transition « accept » refusée depuis l'état SubmissionState::AcceptedAwaitingEcho"
    );
}

#[test]
fn echoed_est_terminal() {
    let echoed = SubmissionState::Prepared
        .queue()
        .unwrap()
        .begin_dispatch(attempt("tentative-1"))
        .unwrap()
        .accept(receipt("msg-1"))
        .unwrap()
        .echo(entry("entree-1"))
        .unwrap();
    assert_eq!(
        echoed,
        SubmissionState::Echoed {
            entry: entry("entree-1"),
        }
    );

    let echoed = SubmissionState::Echoed {
        entry: entry("entree-1"),
    };
    let err = echoed.clone().queue().unwrap_err();
    let as_error: &dyn Error = &err;
    assert_eq!(
        as_error.to_string(),
        "transition « queue » refusée depuis l'état SubmissionState::Echoed"
    );
    assert!(echoed.clone().begin_dispatch(attempt("autre")).is_err());
    assert!(echoed.clone().accept(receipt("msg-1")).is_err());
    assert!(echoed.clone().echo(entry("entree-2")).is_err());
    assert!(echoed.clone().refuse(Refusal::Conflict).is_err());
    assert!(echoed
        .clone()
        .mark_unknown(reconciliation("recon-1"))
        .is_err());
    assert!(echoed
        .reconcile_found(&reconciliation("r-1"), receipt("msg-1"))
        .is_err());
}

#[test]
fn accepts_continuation_faux_si_generation_differente() {
    let actor = AuthenticatedActor {
        account_id: AccountId::new("compte").unwrap(),
        team_id: Some(TeamId::new("equipe").unwrap()),
        generation: AccountGeneration(4),
    };
    assert!(actor.accepts_continuation(AccountGeneration(4)));
    assert!(!actor.accepts_continuation(AccountGeneration(5)));
    assert!(!actor.accepts_continuation(AccountGeneration(0)));
}

#[test]
fn dispose_tool_result_ignore_cancelled_et_cancelling() {
    let checkpoint = CheckpointId::new("cp").unwrap();
    assert_eq!(
        RunState::Queued.dispose_tool_result(),
        EffectDisposition::Applied
    );
    assert_eq!(
        RunState::Running {
            checkpoint: checkpoint.clone(),
        }
        .dispose_tool_result(),
        EffectDisposition::Applied
    );
    assert_eq!(
        RunState::AwaitingApproval {
            approval: ApprovalId::new("approbation").unwrap(),
        }
        .dispose_tool_result(),
        EffectDisposition::Applied
    );
    assert_eq!(
        RunState::AwaitingHumanInput {
            request: HumanInputId::new("saisie").unwrap(),
        }
        .dispose_tool_result(),
        EffectDisposition::Applied
    );

    let cancelling = RunState::Cancelling {
        reason: InterruptionReason::User,
    };
    let cancelling_before = cancelling.clone();
    assert_eq!(
        cancelling.dispose_tool_result(),
        EffectDisposition::IgnoredAfterInterruption
    );
    assert_eq!(cancelling, cancelling_before);

    let cancelled = RunState::Cancelled {
        checkpoint: checkpoint.clone(),
    };
    let cancelled_before = cancelled.clone();
    assert_eq!(
        cancelled.dispose_tool_result(),
        EffectDisposition::IgnoredAfterInterruption
    );
    assert_eq!(cancelled, cancelled_before);

    assert_eq!(
        RunState::Succeeded.dispose_tool_result(),
        EffectDisposition::IgnoredAfterInterruption
    );
    assert_eq!(
        RunState::Failed {
            error: RunFailure::Tool,
        }
        .dispose_tool_result(),
        EffectDisposition::IgnoredAfterInterruption
    );
}

#[test]
fn blocks_coverage_vrai_seulement_pour_pending() {
    let digest = ContentDigest([7; 32]);
    assert!(TranscriptBody::Pending {
        expected_digest: digest,
    }
    .blocks_coverage());
    assert!(!TranscriptBody::Inline {
        content: EntryBody::Text("bonjour".into()),
    }
    .blocks_coverage());
    assert!(!TranscriptBody::Inline {
        content: EntryBody::Unknown {
            kind: "image".into(),
        },
    }
    .blocks_coverage());
    assert!(!TranscriptBody::Blob {
        reference: BlobRef::new("blob-1").unwrap(),
        digest,
    }
    .blocks_coverage());
    assert!(!TranscriptBody::Unavailable {
        reason: BodyUnavailable::Expired,
    }
    .blocks_coverage());
    assert!(!TranscriptBody::Unavailable {
        reason: BodyUnavailable::Purged,
    }
    .blocks_coverage());
    assert!(!TranscriptBody::Unavailable {
        reason: BodyUnavailable::Acl,
    }
    .blocks_coverage());
    assert_eq!(BlobRef::new(""), Err(EmptyId));
    assert_eq!(BlobRef::new("blob-1").unwrap().as_str(), "blob-1");
}

#[test]
fn soumission_suit_les_transitions_autorisees() {
    let receipt_a = receipt("m-1");
    let dispatching = SubmissionState::Prepared
        .queue()
        .unwrap()
        .begin_dispatch(attempt("a-1"))
        .unwrap();

    let accepted = dispatching.clone().accept(receipt_a.clone()).unwrap();
    let unknown = dispatching
        .clone()
        .mark_unknown(reconciliation("r-1"))
        .unwrap();
    assert!(accepted
        .clone()
        .mark_unknown(reconciliation("r-2"))
        .is_err());
    assert_eq!(
        unknown
            .clone()
            .reconcile_found(&reconciliation("r-1"), receipt_a)
            .unwrap(),
        accepted
    );
    assert_eq!(
        unknown.echo(entry("e-1")).unwrap(),
        SubmissionState::Echoed {
            entry: entry("e-1"),
        }
    );

    assert_eq!(
        dispatching.refuse(Refusal::StaleAccountGeneration).unwrap(),
        SubmissionState::Refused {
            reason: Refusal::StaleAccountGeneration,
        }
    );

    assert!(SubmissionState::Prepared
        .begin_dispatch(attempt("a"))
        .is_err());
    assert!(SubmissionState::Queued.accept(receipt("m")).is_err());
    assert!(SubmissionState::Queued.queue().is_err());
    assert!(SubmissionState::Dispatching {
        attempt: attempt("a"),
    }
    .echo(entry("e"))
    .is_err());
    assert!(SubmissionState::AcceptedAwaitingEcho {
        receipt: receipt("m")
    }
    .refuse(Refusal::Conflict)
    .is_err());
    assert!(SubmissionState::OutcomeUnknown {
        reconciliation: reconciliation("r"),
    }
    .refuse(Refusal::UnknownConversation)
    .is_ok());
    assert!(SubmissionState::OutcomeUnknown {
        reconciliation: reconciliation("r"),
    }
    .accept(receipt("m"))
    .is_err());
    assert_eq!(
        SubmissionState::OutcomeUnknown {
            reconciliation: reconciliation("r"),
        }
        .mark_unknown(reconciliation("r2"))
        .unwrap(),
        SubmissionState::OutcomeUnknown {
            reconciliation: reconciliation("r2"),
        }
    );
    assert!(SubmissionState::Refused {
        reason: Refusal::Policy,
    }
    .echo(entry("e"))
    .is_err());
    assert!(SubmissionState::Refused {
        reason: Refusal::Other("ailleurs".into()),
    }
    .queue()
    .is_err());
}

#[test]
fn reconciliation_exige_le_token_courant_et_seul_not_found_reouvre_la_file() {
    let current = reconciliation("r-courant");
    let stale = reconciliation("r-perime");
    let unknown = SubmissionState::OutcomeUnknown {
        reconciliation: current.clone(),
    };
    assert!(unknown
        .clone()
        .reconcile_found(&stale, receipt("msg"))
        .is_err());
    assert!(unknown.clone().reconcile_not_found(&stale).is_err());
    assert_eq!(
        unknown.reconcile_not_found(&current).unwrap(),
        SubmissionState::Queued
    );
    assert!(SubmissionState::Refused {
        reason: Refusal::Policy,
    }
    .reconcile_not_found(&current)
    .is_err());
    assert!(SubmissionState::AcceptedAwaitingEcho {
        receipt: receipt("msg"),
    }
    .reconcile_not_found(&current)
    .is_err());
}

#[test]
fn annulation_de_run() {
    let checkpoint = CheckpointId::new("cp").unwrap();
    let cancelling = RunState::Running {
        checkpoint: checkpoint.clone(),
    }
    .begin_cancel(InterruptionReason::User)
    .unwrap();
    assert_eq!(
        cancelling
            .clone()
            .begin_cancel(InterruptionReason::User)
            .unwrap(),
        cancelling
    );
    let err = cancelling
        .clone()
        .begin_cancel(InterruptionReason::Superseded)
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "transition « begin_cancel » refusée depuis l'état RunState::Cancelling"
    );

    let cancelled = cancelling.finish_cancel(checkpoint.clone()).unwrap();
    assert_eq!(
        cancelled,
        RunState::Cancelled {
            checkpoint: checkpoint.clone(),
        }
    );
    assert!(cancelled.finish_cancel(checkpoint).is_err());

    assert!(RunState::AwaitingApproval {
        approval: ApprovalId::new("ap").unwrap(),
    }
    .begin_cancel(InterruptionReason::AccountRevoked)
    .is_ok());
    assert!(RunState::AwaitingHumanInput {
        request: HumanInputId::new("saisie").unwrap(),
    }
    .begin_cancel(InterruptionReason::User)
    .is_ok());
    assert!(RunState::Queued
        .begin_cancel(InterruptionReason::User)
        .is_ok());
    assert!(RunState::Succeeded
        .begin_cancel(InterruptionReason::User)
        .is_err());
    assert!(RunState::Failed {
        error: RunFailure::Model,
    }
    .begin_cancel(InterruptionReason::User)
    .is_err());
    assert!(RunState::Failed {
        error: RunFailure::Policy,
    }
    .finish_cancel(CheckpointId::new("cp2").unwrap())
    .is_err());
    assert!(RunState::Running {
        checkpoint: CheckpointId::new("cp3").unwrap(),
    }
    .finish_cancel(CheckpointId::new("cp4").unwrap())
    .is_err());
}

#[test]
fn queued_run_can_be_cancelled() {
    let cancelled = RunState::Queued
        .begin_cancel(InterruptionReason::User)
        .unwrap()
        .finish_cancel(CheckpointId::new("before-start").unwrap())
        .unwrap();
    assert_eq!(
        cancelled.dispose_tool_result(),
        EffectDisposition::IgnoredAfterInterruption
    );
}

#[test]
fn awaiting_children_accepts_results_until_cancelled() {
    let waiting = RunState::AwaitingChildren {
        children: vec![RunId::new("child-1").unwrap()],
    };
    assert_eq!(waiting.dispose_tool_result(), EffectDisposition::Applied);
    let cancelling = waiting.begin_cancel(InterruptionReason::User).unwrap();
    assert_eq!(
        cancelling.dispose_tool_result(),
        EffectDisposition::IgnoredAfterInterruption
    );
    assert!(cancelling
        .finish_cancel(CheckpointId::new("last-parent-event").unwrap())
        .is_ok());
}

#[test]
fn compteurs_epoque_et_transcription() {
    assert!(ServerGeneration(1) < ServerGeneration(2));
    assert_eq!(EntrySequence(3), EntrySequence(3));
    assert!(UpdateSequence(1) < UpdateSequence(9));
    assert!(ReplicaSequence(0) < ReplicaSequence(1));

    let epoch = ReplicaEpoch {
        boot_id: BootId::new("boot-1").unwrap(),
        server_generation: ServerGeneration(2),
        recovery: 1,
    };
    let cursor = ResumeCursor {
        epoch: epoch.clone(),
        covered_through: UpdateSequence(8),
        complete: false,
    };
    assert_eq!(cursor.epoch, epoch);
    assert!(!cursor.complete);

    let update = TranscriptUpdate {
        sequence: UpdateSequence(8),
        entry_id: entry("e"),
        kind: UpdateKind::Upsert {
            body: TranscriptBody::Pending {
                expected_digest: ContentDigest([1; 32]),
            },
        },
    };
    match update.kind {
        UpdateKind::Upsert { body } => assert!(body.blocks_coverage()),
        UpdateKind::Delete => panic!("la mise à jour doit être un upsert"),
    }

    let deleted = TranscriptUpdate {
        sequence: UpdateSequence(9),
        entry_id: entry("e"),
        kind: UpdateKind::Delete,
    };
    assert!(matches!(deleted.kind, UpdateKind::Delete));

    assert!(matches!(
        ApprovalDecision::Allow {
            grant: ActionGrantId::new("g").unwrap(),
        },
        ApprovalDecision::Allow { .. }
    ));
    assert!(matches!(
        ApprovalDecision::Ask {
            approval: ApprovalId::new("ap").unwrap(),
        },
        ApprovalDecision::Ask { .. }
    ));
    assert_eq!(
        ApprovalDecision::Deny {
            reason: "non".into(),
        },
        ApprovalDecision::Deny {
            reason: "non".into(),
        }
    );
}
