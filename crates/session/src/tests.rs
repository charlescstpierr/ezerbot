use std::path::{Path, PathBuf};

use grokbot_domain::{
    AccountGeneration, AccountId, AuthenticatedActor, ConversationId, DeliveryReceipt, MessageId,
    Refusal, SubmissionState, TranscriptEntryId,
};
use grokbot_store::SessionStore;
use grokbot_test_support::TestDir;
use rusqlite::Connection;

use crate::{
    LookupOutcome, ReconciliationRequest, SendOutcome, SendRequest, SessionError,
    SubmissionSession, SubmissionTransport,
};

/// Chemin de la base, puis le dossier unique qui la contient (supprimé en entier à la fin).
struct Database(PathBuf, #[allow(dead_code)] TestDir);
impl Database {
    fn new() -> Self {
        let directory = TestDir::new("grokbot-session");
        Self(directory.join("session.sqlite"), directory)
    }
    fn fail_writes(&self) {
        fail_writes(&self.0);
    }
    fn allow_writes(&self) {
        Connection::open(&self.0)
            .unwrap()
            .execute_batch("DROP TRIGGER fail_submission_save;")
            .unwrap();
    }
}

fn fail_writes(path: &Path) {
    Connection::open(path).unwrap().execute_batch("CREATE TRIGGER fail_submission_save BEFORE INSERT ON submissions BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END;").unwrap();
}
fn actor(account: &str, generation: u64) -> AuthenticatedActor {
    AuthenticatedActor {
        account_id: AccountId::new(account).unwrap(),
        team_id: None,
        generation: AccountGeneration(generation),
    }
}
fn conversation() -> ConversationId {
    ConversationId::new("conversation").unwrap()
}
fn message() -> MessageId {
    MessageId::new("logical-nonce").unwrap()
}
fn receipt(account: &str) -> DeliveryReceipt {
    DeliveryReceipt {
        account_id: AccountId::new(account).unwrap(),
        conversation_id: conversation(),
        message_id: message(),
    }
}
fn prepared(db: &Database) -> SubmissionSession {
    let mut session = SubmissionSession::open(&db.0, actor("alice", 1)).unwrap();
    assert_eq!(
        session
            .prepare(conversation(), message(), "original body".into())
            .unwrap(),
        SubmissionState::Queued
    );
    session
}
fn durable(db: &Database, account: &str) -> SubmissionState {
    SessionStore::open(&db.0, &AccountId::new(account).unwrap())
        .unwrap()
        .load_journal()
        .unwrap()
        .get(
            &AccountId::new(account).unwrap(),
            &conversation(),
            &message(),
        )
        .unwrap()
}

struct Observer {
    path: PathBuf,
    send: SendOutcome,
    lookup: LookupOutcome,
    sends: usize,
    lookups: usize,
    fail_after_send: bool,
    fail_after_lookup: bool,
}
impl Observer {
    fn new(db: &Database, send: SendOutcome, lookup: LookupOutcome) -> Self {
        Self {
            path: db.0.clone(),
            send,
            lookup,
            sends: 0,
            lookups: 0,
            fail_after_send: false,
            fail_after_lookup: false,
        }
    }
}
impl SubmissionTransport for Observer {
    fn send(&mut self, request: &SendRequest) -> SendOutcome {
        self.sends += 1;
        let journal = SessionStore::open(&self.path, &request.actor().account_id)
            .unwrap()
            .load_journal()
            .unwrap();
        assert_eq!(
            journal.get(
                &request.actor().account_id,
                request.conversation_id(),
                request.message_id()
            ),
            Some(SubmissionState::Dispatching {
                attempt: request.attempt().clone()
            })
        );
        let item = journal.export_committed().pop().unwrap();
        assert_eq!(item.body, "original body");
        assert_eq!(request.body(), "original body");
        assert_eq!(request.message_id().as_str(), "logical-nonce");
        if self.fail_after_send {
            fail_writes(&self.path);
        }
        self.send.clone()
    }
    fn lookup(&mut self, request: &ReconciliationRequest) -> LookupOutcome {
        self.lookups += 1;
        let journal = SessionStore::open(&self.path, &request.actor().account_id)
            .unwrap()
            .load_journal()
            .unwrap();
        assert_eq!(
            journal.get(
                &request.actor().account_id,
                request.conversation_id(),
                request.message_id()
            ),
            Some(SubmissionState::OutcomeUnknown {
                reconciliation: request.reconciliation().clone()
            })
        );
        if self.fail_after_lookup {
            fail_writes(&self.path);
        }
        self.lookup.clone()
    }
}

#[test]
fn prepare_commit_failure_leaves_no_visible_message() {
    let db = Database::new();
    let mut session = SubmissionSession::open(&db.0, actor("alice", 1)).unwrap();
    db.fail_writes();
    assert!(matches!(
        session.prepare(conversation(), message(), "original body".into()),
        Err(SessionError::Store(_))
    ));
    assert_eq!(session.state(&conversation(), &message()).unwrap(), None);
    db.allow_writes();
    assert_eq!(
        session
            .prepare(conversation(), message(), "original body".into())
            .unwrap(),
        SubmissionState::Queued
    );
}

#[test]
fn dispatch_commit_failure_prevents_any_transport_call() {
    let db = Database::new();
    let mut session = prepared(&db);
    db.fail_writes();
    let mut transport = Observer::new(
        &db,
        SendOutcome::Accepted(receipt("alice")),
        LookupOutcome::Unknown,
    );
    assert!(matches!(
        session.send_with(&conversation(), &message(), &mut transport),
        Err(SessionError::Store(_))
    ));
    assert_eq!(transport.sends, 0);
    assert_eq!(
        session.state(&conversation(), &message()).unwrap(),
        Some(SubmissionState::Queued)
    );
    assert_eq!(durable(&db, "alice"), SubmissionState::Queued);
}

#[test]
fn transport_observes_dispatch_on_disk_before_sending() {
    let db = Database::new();
    let mut session = prepared(&db);
    let mut transport = Observer::new(
        &db,
        SendOutcome::Accepted(receipt("alice")),
        LookupOutcome::Unknown,
    );
    let state = session
        .send_with(&conversation(), &message(), &mut transport)
        .unwrap();
    assert_eq!(
        state,
        SubmissionState::AcceptedAwaitingEcho {
            receipt: receipt("alice")
        }
    );
    assert_eq!(durable(&db, "alice"), state);
    assert_eq!(transport.sends, 1);
    assert!(session
        .send_with(&conversation(), &message(), &mut transport)
        .is_err());
    assert_eq!(transport.sends, 1);
}

#[test]
fn reply_commit_failure_restores_dispatch_and_requires_lookup() {
    let db = Database::new();
    let mut session = prepared(&db);
    let mut transport = Observer::new(
        &db,
        SendOutcome::Accepted(receipt("alice")),
        LookupOutcome::Found(receipt("alice")),
    );
    transport.fail_after_send = true;
    assert!(matches!(
        session.send_with(&conversation(), &message(), &mut transport),
        Err(SessionError::Store(_))
    ));
    assert_eq!(transport.sends, 1);
    assert_eq!(
        session.state(&conversation(), &message()).unwrap(),
        Some(durable(&db, "alice"))
    );
    assert!(matches!(
        durable(&db, "alice"),
        SubmissionState::Dispatching { .. }
    ));
    assert!(session
        .send_with(&conversation(), &message(), &mut transport)
        .is_err());
    assert_eq!(transport.sends, 1);
    db.allow_writes();
    assert_eq!(
        session
            .reconcile_with(&conversation(), &message(), &mut transport)
            .unwrap(),
        SubmissionState::AcceptedAwaitingEcho {
            receipt: receipt("alice")
        }
    );
    assert_eq!(transport.lookups, 1);
}

#[test]
fn reconciliation_commit_failure_prevents_lookup() {
    let db = Database::new();
    let mut session = prepared(&db);
    session.begin_send(&conversation(), &message()).unwrap();
    db.fail_writes();
    let mut transport = Observer::new(&db, SendOutcome::OutcomeUnknown, LookupOutcome::NotFound);
    assert!(matches!(
        session.reconcile_with(&conversation(), &message(), &mut transport),
        Err(SessionError::Store(_))
    ));
    assert_eq!(transport.lookups, 0);
    assert!(matches!(
        session.state(&conversation(), &message()).unwrap(),
        Some(SubmissionState::Dispatching { .. })
    ));
}

#[test]
fn not_found_reply_commit_failure_keeps_uncertainty_and_original_body() {
    let db = Database::new();
    let mut session = prepared(&db);
    session.begin_send(&conversation(), &message()).unwrap();
    let mut transport = Observer::new(&db, SendOutcome::OutcomeUnknown, LookupOutcome::NotFound);
    transport.fail_after_lookup = true;
    assert!(matches!(
        session.reconcile_with(&conversation(), &message(), &mut transport),
        Err(SessionError::Store(_))
    ));
    assert_eq!(transport.lookups, 1);
    assert!(matches!(
        durable(&db, "alice"),
        SubmissionState::OutcomeUnknown { .. }
    ));
    assert_eq!(
        session.state(&conversation(), &message()).unwrap(),
        Some(durable(&db, "alice"))
    );
    db.allow_writes();
    transport.fail_after_lookup = false;
    assert_eq!(
        session
            .reconcile_with(&conversation(), &message(), &mut transport)
            .unwrap(),
        SubmissionState::Queued
    );
    assert_eq!(transport.sends, 0);
    session
        .send_with(&conversation(), &message(), &mut transport)
        .unwrap();
    assert_eq!(transport.sends, 1);
}

#[test]
fn missing_reply_reopens_as_uncertain_and_never_blindly_resends() {
    let db = Database::new();
    let mut session = prepared(&db);
    let mut transport = Observer::new(
        &db,
        SendOutcome::OutcomeUnknown,
        LookupOutcome::Found(receipt("alice")),
    );
    assert!(matches!(
        session
            .send_with(&conversation(), &message(), &mut transport)
            .unwrap(),
        SubmissionState::OutcomeUnknown { .. }
    ));
    drop(session);
    let mut reopened = SubmissionSession::open(&db.0, actor("alice", 1)).unwrap();
    assert!(reopened
        .send_with(&conversation(), &message(), &mut transport)
        .is_err());
    reopened
        .reconcile_with(&conversation(), &message(), &mut transport)
        .unwrap();
    assert_eq!(transport.sends, 1);
    assert_eq!(transport.lookups, 1);
}

#[test]
fn reopened_unknown_lookup_rotates_its_persisted_token() {
    let db = Database::new();
    let mut session = prepared(&db);
    session.begin_send(&conversation(), &message()).unwrap();
    let prior = session
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    drop(session);
    let mut reopened = SubmissionSession::open(&db.0, actor("alice", 1)).unwrap();
    let current = reopened
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    assert_ne!(prior.reconciliation(), current.reconciliation());
    assert_eq!(
        durable(&db, "alice"),
        SubmissionState::OutcomeUnknown {
            reconciliation: current.reconciliation().clone()
        }
    );
}

#[test]
fn reload_invalidates_old_requests_even_if_another_session_reuses_token_text() {
    let db = Database::new();
    let mut first = prepared(&db);
    first.begin_send(&conversation(), &message()).unwrap();
    let old = first
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    let mut second = SubmissionSession::open(&db.0, actor("alice", 1)).unwrap();
    let second_request = second
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    assert_ne!(old.reconciliation(), second_request.reconciliation());
    let mut third = SubmissionSession::open(&db.0, actor("alice", 1)).unwrap();
    let current = third
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    assert_eq!(old.reconciliation(), current.reconciliation());
    assert!(matches!(
        first.apply_reconciliation(&old, LookupOutcome::NotFound),
        Err(SessionError::Store(_))
    ));
    assert!(matches!(
        first.apply_reconciliation(&old, LookupOutcome::NotFound),
        Err(SessionError::StaleRequest)
    ));
    assert_eq!(
        durable(&db, "alice"),
        SubmissionState::OutcomeUnknown {
            reconciliation: current.reconciliation().clone()
        }
    );
    let fresh = first
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    first
        .apply_reconciliation(&fresh, LookupOutcome::Found(receipt("alice")))
        .unwrap();
}

#[test]
fn old_not_found_cannot_requeue_after_new_lookup_or_new_send() {
    let db = Database::new();
    let mut session = prepared(&db);
    session.begin_send(&conversation(), &message()).unwrap();
    let old = session
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    let current = session
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    assert_ne!(old.reconciliation(), current.reconciliation());
    assert!(matches!(
        session.apply_reconciliation(&old, LookupOutcome::NotFound),
        Err(SessionError::StaleRequest)
    ));
    session
        .apply_reconciliation(&current, LookupOutcome::NotFound)
        .unwrap();
    let send = session.begin_send(&conversation(), &message()).unwrap();
    assert!(matches!(
        session.apply_reconciliation(&current, LookupOutcome::NotFound),
        Err(SessionError::StaleRequest)
    ));
    assert_eq!(
        session.state(&conversation(), &message()).unwrap(),
        Some(SubmissionState::Dispatching {
            attempt: send.attempt().clone()
        })
    );
}

#[test]
fn newer_generation_adopts_only_own_account_and_refuses_old_continuations() {
    let db = Database::new();
    let mut alice = prepared(&db);
    let old = alice.begin_send(&conversation(), &message()).unwrap();
    let mut bob = SubmissionSession::open(&db.0, actor("bob", 8)).unwrap();
    bob.prepare(conversation(), message(), "bob body".into())
        .unwrap();
    let mut new_alice = SubmissionSession::open(&db.0, actor("alice", 2)).unwrap();
    let store = SessionStore::open(&db.0, &AccountId::new("alice").unwrap()).unwrap();
    let item = store
        .load_journal()
        .unwrap()
        .export_committed()
        .pop()
        .unwrap();
    assert_eq!(item.generation, AccountGeneration(2));
    assert_eq!(item.body, "original body");
    assert_eq!(item.message_id, message());
    assert_eq!(
        item.state,
        SubmissionState::Dispatching {
            attempt: old.attempt().clone()
        }
    );
    assert_eq!(durable(&db, "bob"), SubmissionState::Queued);
    let bob_store = SessionStore::open(&db.0, &AccountId::new("bob").unwrap()).unwrap();
    assert_eq!(
        bob_store.list_submissions().unwrap()[0].generation,
        AccountGeneration(8)
    );
    assert!(matches!(
        new_alice.apply_send(&old, SendOutcome::Accepted(receipt("alice"))),
        Err(SessionError::StaleRequest)
    ));
    assert!(alice
        .apply_send(&old, SendOutcome::Accepted(receipt("alice")))
        .is_err());
    assert!(alice
        .begin_reconciliation(&conversation(), &message())
        .is_err());
    assert!(matches!(
        SubmissionSession::open(&db.0, actor("alice", 1)),
        Err(SessionError::Journal(_))
    ));
    let lookup = new_alice
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    new_alice
        .apply_reconciliation(&lookup, LookupOutcome::Found(receipt("alice")))
        .unwrap();
}

#[test]
fn stale_actor_after_reload_cannot_prepare_another_nonce() {
    let db = Database::new();
    let mut stale = prepared(&db);
    let request = stale.begin_send(&conversation(), &message()).unwrap();
    let _newer = SubmissionSession::open(&db.0, actor("alice", 2)).unwrap();
    assert!(matches!(
        stale.apply_send(&request, SendOutcome::Accepted(receipt("alice"))),
        Err(SessionError::Store(_))
    ));
    let next_message = MessageId::new("another-nonce").unwrap();
    assert!(matches!(
        stale.prepare(conversation(), next_message.clone(), "stale body".into()),
        Err(SessionError::Journal(_))
    ));
    let mut transport = Observer::new(&db, SendOutcome::OutcomeUnknown, LookupOutcome::Unknown);
    assert!(stale
        .send_with(&conversation(), &next_message, &mut transport)
        .is_err());
    assert_eq!(transport.sends, 0);
    let store = SessionStore::open(&db.0, &AccountId::new("alice").unwrap()).unwrap();
    assert_eq!(store.list_submissions().unwrap().len(), 1);
    assert_eq!(
        store.list_submissions().unwrap()[0].generation,
        AccountGeneration(2)
    );
}

#[test]
fn request_owner_is_unique_even_when_reopened_with_same_generation() {
    let db = Database::new();
    let mut first = prepared(&db);
    let request = first.begin_send(&conversation(), &message()).unwrap();
    let mut second = SubmissionSession::open(&db.0, actor("alice", 1)).unwrap();
    assert!(matches!(
        second.apply_send(&request, SendOutcome::Accepted(receipt("alice"))),
        Err(SessionError::StaleRequest)
    ));
    let lookup = second
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    assert!(matches!(
        first.apply_reconciliation(&lookup, LookupOutcome::NotFound),
        Err(SessionError::StaleRequest)
    ));
}

#[test]
fn revoked_session_rejects_late_replies_and_new_effects() {
    let db = Database::new();
    let mut session = prepared(&db);
    let request = session.begin_send(&conversation(), &message()).unwrap();
    session.revoke();
    assert!(matches!(
        session.apply_send(&request, SendOutcome::Accepted(receipt("alice"))),
        Err(SessionError::Revoked)
    ));
    let mut transport = Observer::new(&db, SendOutcome::OutcomeUnknown, LookupOutcome::Unknown);
    assert!(matches!(
        session.reconcile_with(&conversation(), &message(), &mut transport),
        Err(SessionError::Revoked)
    ));
    assert_eq!(transport.lookups, 0);
    assert!(matches!(
        durable(&db, "alice"),
        SubmissionState::Dispatching { .. }
    ));
}

#[test]
fn refusal_is_durable_and_terminal_for_send_and_lookup() {
    for from_lookup in [false, true] {
        let db = Database::new();
        let mut session = prepared(&db);
        let mut transport = Observer::new(
            &db,
            SendOutcome::Refused(Refusal::Policy),
            LookupOutcome::Refused(Refusal::UnknownConversation),
        );
        let state = if from_lookup {
            session.begin_send(&conversation(), &message()).unwrap();
            session
                .reconcile_with(&conversation(), &message(), &mut transport)
                .unwrap()
        } else {
            session
                .send_with(&conversation(), &message(), &mut transport)
                .unwrap()
        };
        assert_eq!(
            state,
            SubmissionState::Refused {
                reason: if from_lookup {
                    Refusal::UnknownConversation
                } else {
                    Refusal::Policy
                }
            }
        );
        assert_eq!(durable(&db, "alice"), state);
        assert!(session
            .send_with(&conversation(), &message(), &mut transport)
            .is_err());
        assert!(session
            .reconcile_with(&conversation(), &message(), &mut transport)
            .is_err());
        assert_eq!(transport.sends, usize::from(!from_lookup));
        assert_eq!(transport.lookups, usize::from(from_lookup));
    }
}

#[test]
fn receipt_cannot_redirect_a_reply_to_another_key() {
    let db = Database::new();
    let mut session = prepared(&db);
    let request = session.begin_send(&conversation(), &message()).unwrap();
    let mut wrong = receipt("bob");
    for field in 0..3 {
        if field == 1 {
            wrong = receipt("alice");
            wrong.conversation_id = ConversationId::new("wrong-conversation").unwrap();
        }
        if field == 2 {
            wrong = receipt("alice");
            wrong.message_id = MessageId::new("wrong-message").unwrap();
        }
        assert!(matches!(
            session.apply_send(&request, SendOutcome::Accepted(wrong.clone())),
            Err(SessionError::InvalidReceipt)
        ));
    }
    let lookup = session
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    assert!(matches!(
        session.apply_reconciliation(&lookup, LookupOutcome::Found(wrong)),
        Err(SessionError::InvalidReceipt)
    ));
    assert!(matches!(
        durable(&db, "alice"),
        SubmissionState::OutcomeUnknown { .. }
    ));
}

#[test]
fn pending_and_unknown_lookup_never_resend_or_requeue() {
    let db = Database::new();
    let mut session = prepared(&db);
    session.begin_send(&conversation(), &message()).unwrap();
    let mut transport = Observer::new(&db, SendOutcome::OutcomeUnknown, LookupOutcome::Pending);
    for outcome in [LookupOutcome::Pending, LookupOutcome::Unknown] {
        transport.lookup = outcome;
        assert!(matches!(
            session
                .reconcile_with(&conversation(), &message(), &mut transport)
                .unwrap(),
            SubmissionState::OutcomeUnknown { .. }
        ));
    }
    assert_eq!(transport.lookups, 2);
    assert_eq!(transport.sends, 0);
}

#[test]
fn echoed_state_is_committed_before_return_and_rejects_old_lookup() {
    let db = Database::new();
    let mut session = prepared(&db);
    session.begin_send(&conversation(), &message()).unwrap();
    let request = session
        .begin_reconciliation(&conversation(), &message())
        .unwrap();
    let entry = TranscriptEntryId::new("echo-entry").unwrap();
    assert_eq!(
        session
            .observe_echo(&conversation(), &message(), entry.clone())
            .unwrap(),
        SubmissionState::Echoed {
            entry: entry.clone()
        }
    );
    assert_eq!(durable(&db, "alice"), SubmissionState::Echoed { entry });
    assert!(matches!(
        session.apply_reconciliation(&request, LookupOutcome::NotFound),
        Err(SessionError::StaleRequest)
    ));
}

fn other_conversation() -> ConversationId {
    ConversationId::new("other").unwrap()
}

fn deliver(
    session: &mut SubmissionSession,
    db: &Database,
    conversation: &ConversationId,
    entry: &str,
) {
    let mut transport = Observer::new(
        db,
        SendOutcome::Accepted(DeliveryReceipt {
            account_id: AccountId::new("alice").unwrap(),
            conversation_id: conversation.clone(),
            message_id: message(),
        }),
        LookupOutcome::Unknown,
    );
    session
        .prepare(conversation.clone(), message(), "original body".into())
        .unwrap();
    session
        .send_with(conversation, &message(), &mut transport)
        .unwrap();
    session
        .observe_echo(
            conversation,
            &message(),
            TranscriptEntryId::new(entry).unwrap(),
        )
        .unwrap();
}

#[test]
fn sending_in_one_conversation_never_rewrites_another_conversations_rows() {
    let db = Database::new();
    let mut session = SubmissionSession::open(&db.0, actor("alice", 1)).unwrap();
    deliver(&mut session, &db, &other_conversation(), "other-echo");
    let before = SessionStore::open(&db.0, &AccountId::new("alice").unwrap())
        .unwrap()
        .list_conversation_submissions(&other_conversation())
        .unwrap();
    assert_eq!(before.len(), 1);

    Connection::open(&db.0)
        .unwrap()
        .execute_batch(
            "CREATE TABLE audit (event TEXT NOT NULL);
             CREATE TRIGGER audit_insert AFTER INSERT ON submissions
             WHEN NEW.conversation_id = 'other'
             BEGIN INSERT INTO audit VALUES ('insert'); END;
             CREATE TRIGGER audit_update AFTER UPDATE ON submissions
             WHEN NEW.conversation_id = 'other' OR OLD.conversation_id = 'other'
             BEGIN INSERT INTO audit VALUES ('update'); END;
             CREATE TRIGGER audit_delete AFTER DELETE ON submissions
             WHEN OLD.conversation_id = 'other'
             BEGIN INSERT INTO audit VALUES ('delete'); END;",
        )
        .unwrap();

    deliver(&mut session, &db, &conversation(), "echo");

    let audit: Vec<String> = {
        let connection = Connection::open(&db.0).unwrap();
        let mut statement = connection.prepare("SELECT event FROM audit").unwrap();
        let rows = statement.query_map([], |row| row.get(0)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    assert_eq!(audit, Vec::<String>::new());
    let store = SessionStore::open(&db.0, &AccountId::new("alice").unwrap()).unwrap();
    assert_eq!(
        store
            .list_conversation_submissions(&other_conversation())
            .unwrap(),
        before
    );
    assert_eq!(
        store
            .list_conversation_submissions(&conversation())
            .unwrap()
            .len(),
        1
    );
}
