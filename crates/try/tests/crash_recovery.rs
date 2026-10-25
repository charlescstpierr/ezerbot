use std::process::Command;

use grokbot_domain::{
    AccountGeneration, AccountId, AttemptId, AuthenticatedActor, ConversationId, DeliveryReceipt,
    MessageId, SubmissionState,
};
use grokbot_store::SessionStore;
use grokbot_submission_journal::{PreparedMessage, RetryDecision};

fn actor() -> AuthenticatedActor {
    AuthenticatedActor {
        account_id: AccountId::new("crash-account").unwrap(),
        generation: AccountGeneration(1),
        team_id: None,
    }
}

// The child exits without destructors, after a real SQLite commit or before the next save.
#[test]
fn abrupt_child() {
    let Ok(path) = std::env::var("GROKBOT_CRASH_DATABASE") else {
        return;
    };
    let actor = actor();
    let conversation = ConversationId::new("crash-conversation").unwrap();
    let message = MessageId::new("crash-message").unwrap();
    let mut store = SessionStore::open(std::path::Path::new(&path), &actor.account_id).unwrap();
    let mut journal = store.load_journal().unwrap();
    journal
        .prepare(PreparedMessage {
            actor: actor.clone(),
            conversation_id: conversation.clone(),
            message_id: message.clone(),
            body: "message avant coupure".into(),
        })
        .unwrap();
    journal
        .dispatch(
            &actor,
            &conversation,
            &message,
            AttemptId::new("attempt-1").unwrap(),
        )
        .unwrap();
    store.save_journal(&mut journal).unwrap();
    journal
        .observe_accept(
            &actor,
            DeliveryReceipt {
                account_id: actor.account_id.clone(),
                conversation_id: conversation,
                message_id: message,
            },
        )
        .unwrap();
    if std::env::var("GROKBOT_CRASH_AFTER_COMMIT").as_deref() == Ok("1") {
        store.save_journal(&mut journal).unwrap();
    }
    std::process::exit(91);
}

fn after_abrupt_exit(name: &str, accepted: bool) -> (SubmissionState, RetryDecision) {
    let directory = grokbot_test_support::TestDir::new(&format!("grokbot-crash-{name}"));
    let path = directory.join("session.sqlite");
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "abrupt_child", "--nocapture"])
        .env("GROKBOT_CRASH_DATABASE", &path)
        .env(
            "GROKBOT_CRASH_AFTER_COMMIT",
            if accepted { "1" } else { "0" },
        )
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(91),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let actor = actor();
    let store = SessionStore::open(&path, &actor.account_id).unwrap();
    let mut journal = store.load_journal().unwrap();
    let conversation = ConversationId::new("crash-conversation").unwrap();
    let message = MessageId::new("crash-message").unwrap();
    let state = journal
        .get(&actor.account_id, &conversation, &message)
        .unwrap();
    let decision = journal
        .reconcile_before_retry(&actor, &conversation, &message)
        .unwrap();
    drop(store);
    (state, decision)
}

#[test]
fn la_coupure_avant_sauvegarde_exige_la_reconciliation() {
    let (state, decision) = after_abrupt_exit("before", false);
    assert!(matches!(state, SubmissionState::Dispatching { .. }));
    assert!(matches!(
        decision,
        RetryDecision::NeedsReconciliation { .. }
    ));
}

#[test]
fn la_coupure_apres_commit_conserve_l_acceptation() {
    let (state, decision) = after_abrupt_exit("after", true);
    assert!(matches!(
        state,
        SubmissionState::AcceptedAwaitingEcho { .. }
    ));
    assert!(matches!(decision, RetryDecision::AlreadyDurable { .. }));
}
