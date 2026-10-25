use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use grokbot_domain::{
    AccountGeneration, AccountId, AuthenticatedActor, ConversationId, DeliveryReceipt, MessageId,
    SubmissionState,
};
use grokbot_session::{
    LookupOutcome, ReconciliationRequest, SendOutcome, SendRequest, SubmissionSession,
    SubmissionTransport,
};
use grokbot_store::SessionStore;

fn actor(generation: u64) -> AuthenticatedActor {
    AuthenticatedActor {
        account_id: AccountId::new("orchestrated-crash-account").unwrap(),
        generation: AccountGeneration(generation),
        team_id: None,
    }
}

fn conversation() -> ConversationId {
    ConversationId::new("crash-conversation").unwrap()
}

fn message() -> MessageId {
    MessageId::new("stable-crash-message").unwrap()
}

fn receipt(request: &SendRequest) -> DeliveryReceipt {
    DeliveryReceipt {
        account_id: request.actor().account_id.clone(),
        conversation_id: request.conversation_id().clone(),
        message_id: request.message_id().clone(),
    }
}

fn persist_remote(path: &Path, request: &SendRequest) {
    let mut file = File::create(path).unwrap();
    let content = serde_json::to_vec(&serde_json::json!({
        "account": request.actor().account_id.as_str(),
        "conversation": request.conversation_id().as_str(),
        "message": request.message_id().as_str(),
        "body": request.body(),
        "send_count": 1,
    }))
    .unwrap();
    file.write_all(&content).unwrap();
    file.sync_all().unwrap();
}

struct CrashTransport {
    database: PathBuf,
    remote: PathBuf,
    point: String,
}

impl SubmissionTransport for CrashTransport {
    fn send(&mut self, request: &SendRequest) -> SendOutcome {
        // The real orchestration must have committed before reaching the adapter.
        let store = SessionStore::open(&self.database, &request.actor().account_id).unwrap();
        let durable = store.load_journal().unwrap();
        assert_eq!(
            durable.get(
                &request.actor().account_id,
                request.conversation_id(),
                request.message_id()
            ),
            Some(SubmissionState::Dispatching {
                attempt: request.attempt().clone()
            })
        );
        if self.point == "before-effect" {
            std::process::exit(92);
        }
        persist_remote(&self.remote, request);
        if self.point == "after-effect" {
            std::process::exit(92);
        }
        SendOutcome::Accepted(receipt(request))
    }

    fn lookup(&mut self, _: &ReconciliationRequest) -> LookupOutcome {
        panic!("le processus enfant doit quitter avant toute recherche");
    }
}

#[test]
fn orchestrated_child() {
    let Ok(directory) = std::env::var("GROKBOT_ORCHESTRATED_CRASH") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let path = directory.join("session.sqlite");
    let mut session = SubmissionSession::open(&path, actor(1)).unwrap();
    session
        .prepare(conversation(), message(), "same body across restart".into())
        .unwrap();
    let mut transport = CrashTransport {
        database: path,
        remote: directory.join("simulated-server.json"),
        point: std::env::var("GROKBOT_ORCHESTRATED_POINT").unwrap(),
    };
    session
        .send_with(&conversation(), &message(), &mut transport)
        .unwrap();
    std::process::exit(92);
}

struct RestartTransport {
    remote: PathBuf,
    sends: usize,
    lookups: usize,
}

impl SubmissionTransport for RestartTransport {
    fn send(&mut self, request: &SendRequest) -> SendOutcome {
        self.sends += 1;
        assert!(
            !self.remote.exists(),
            "le serveur a déjà accepté ce message"
        );
        assert_eq!(request.body(), "same body across restart");
        persist_remote(&self.remote, request);
        SendOutcome::Accepted(receipt(request))
    }

    fn lookup(&mut self, request: &ReconciliationRequest) -> LookupOutcome {
        self.lookups += 1;
        if !self.remote.exists() {
            return LookupOutcome::NotFound;
        }
        let remote: serde_json::Value =
            serde_json::from_slice(&fs::read(&self.remote).unwrap()).unwrap();
        assert_eq!(remote["account"], request.actor().account_id.as_str());
        assert_eq!(remote["conversation"], request.conversation_id().as_str());
        assert_eq!(remote["message"], request.message_id().as_str());
        LookupOutcome::Found(DeliveryReceipt {
            account_id: request.actor().account_id.clone(),
            conversation_id: request.conversation_id().clone(),
            message_id: request.message_id().clone(),
        })
    }
}

struct CrashCase(grokbot_test_support::TestDir);

impl CrashCase {
    fn run(point: &str) -> Self {
        let path = grokbot_test_support::TestDir::new(&format!("grokbot-orchestrated-{point}"));
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "orchestrated_child", "--nocapture"])
            .env("GROKBOT_ORCHESTRATED_CRASH", path.path())
            .env("GROKBOT_ORCHESTRATED_POINT", point)
            .output()
            .unwrap();
        assert_eq!(
            child.status.code(),
            Some(92),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        Self(path)
    }

    fn session(&self, generation: u64) -> SubmissionSession {
        SubmissionSession::open(&self.0.join("session.sqlite"), actor(generation)).unwrap()
    }

    fn transport(&self) -> RestartTransport {
        RestartTransport {
            remote: self.0.join("simulated-server.json"),
            sends: 0,
            lookups: 0,
        }
    }

    fn assert_accepted_once(&self) {
        let remote: serde_json::Value =
            serde_json::from_slice(&fs::read(self.0.join("simulated-server.json")).unwrap())
                .unwrap();
        assert_eq!(remote["send_count"], 1);
        assert_eq!(remote["message"], message().as_str());
        assert!(matches!(
            self.session(3).state(&conversation(), &message()).unwrap(),
            Some(SubmissionState::AcceptedAwaitingEcho { .. })
        ));
    }
}

#[test]
fn crash_apres_effet_recherche_acceptation_sans_renvoyer() {
    let case = CrashCase::run("after-effect");
    let mut session = case.session(2);
    let mut transport = case.transport();
    assert!(matches!(
        session.state(&conversation(), &message()).unwrap(),
        Some(SubmissionState::Dispatching { .. })
    ));
    assert!(session
        .send_with(&conversation(), &message(), &mut transport)
        .is_err());
    assert_eq!(transport.sends, 0);
    session
        .reconcile_with(&conversation(), &message(), &mut transport)
        .unwrap();
    assert_eq!((transport.sends, transport.lookups), (0, 1));
    case.assert_accepted_once();
}

#[test]
fn crash_avant_effet_recherche_absence_puis_renvoie_meme_message() {
    let case = CrashCase::run("before-effect");
    let mut session = case.session(2);
    let mut transport = case.transport();
    assert!(session
        .send_with(&conversation(), &message(), &mut transport)
        .is_err());
    assert_eq!(transport.sends, 0);
    assert_eq!(
        session
            .reconcile_with(&conversation(), &message(), &mut transport)
            .unwrap(),
        SubmissionState::Queued
    );
    session
        .send_with(&conversation(), &message(), &mut transport)
        .unwrap();
    assert_eq!((transport.sends, transport.lookups), (1, 1));
    case.assert_accepted_once();
}

#[test]
fn crash_apres_commit_conserve_recu_sans_renvoi_ni_lookup() {
    let case = CrashCase::run("after-commit");
    let mut session = case.session(2);
    let mut transport = case.transport();
    assert!(matches!(
        session.state(&conversation(), &message()).unwrap(),
        Some(SubmissionState::AcceptedAwaitingEcho { .. })
    ));
    assert!(session
        .send_with(&conversation(), &message(), &mut transport)
        .is_err());
    assert_eq!((transport.sends, transport.lookups), (0, 0));
    case.assert_accepted_once();
}
