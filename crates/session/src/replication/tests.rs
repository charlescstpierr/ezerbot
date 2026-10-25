use super::*;
use grokbot_domain::{
    AccountGeneration, AccountId, BootId, EntryBody, ServerGeneration, TranscriptUpdate, UpdateKind,
};
use grokbot_test_support::TestDir;
use rusqlite::Connection;
use std::path::PathBuf;

struct Database(TestDir);

impl Database {
    fn new() -> Self {
        Self(TestDir::new("grokbot-snapshot-session"))
    }

    fn path(&self) -> PathBuf {
        self.0.join("session.sqlite")
    }

    fn session(&self, account: &str) -> SnapshotSession {
        SnapshotSession::open(&self.path(), actor(account), conversation()).unwrap()
    }
}

fn actor(account: &str) -> AuthenticatedActor {
    AuthenticatedActor {
        account_id: AccountId::new(account).unwrap(),
        team_id: None,
        generation: AccountGeneration(1),
    }
}

fn conversation() -> ConversationId {
    ConversationId::new("same-conversation").unwrap()
}

fn snapshot(generation: u64, count: u64) -> Snapshot {
    Snapshot {
        conversation_id: conversation(),
        epoch: ReplicaEpoch {
            boot_id: BootId::new("boot").unwrap(),
            server_generation: ServerGeneration(generation),
            recovery: 0,
        },
        updates: (1..=count)
            .map(|sequence| TranscriptUpdate {
                sequence: UpdateSequence(sequence),
                entry_id: TranscriptEntryId::new(format!("entry-{sequence}")).unwrap(),
                kind: UpdateKind::Upsert {
                    body: TranscriptBody::Inline {
                        content: EntryBody::Text(format!("text-{sequence}")),
                    },
                },
            })
            .collect(),
    }
}

fn install(session: &mut SnapshotSession, generation: u64, count: u64) {
    let request = session.begin_recovery().unwrap();
    session
        .apply_snapshot(&request, snapshot(generation, count), UpdateSequence(count))
        .unwrap();
}

#[test]
fn snapshot_sauve_et_reouvert_dans_le_bon_compte() {
    let db = Database::new();
    let mut session = db.session("account-a");
    install(&mut session, 1, 2);
    let cursor = session.cursor().unwrap();
    drop(session);
    assert_eq!(db.session("account-a").cursor().unwrap(), cursor);
    assert!(db.session("account-b").cursor().unwrap().is_none());
}

#[test]
fn snapshot_apres_reouverture_ne_reutilise_pas_la_requete_originale() {
    let db = Database::new();
    let mut old = db.session("account-a");
    let request = old.begin_recovery().unwrap();
    let mut current = db.session("account-a");
    assert!(current
        .apply_snapshot(&request, snapshot(1, 1), UpdateSequence(1))
        .is_err());
    assert!(current.cursor().unwrap().is_none());
}

#[test]
fn erreur_commit_recharge_image_et_impose_nouvelle_tentative() {
    let db = Database::new();
    let mut session = db.session("account-a");
    install(&mut session, 1, 1);
    let before = session.cursor().unwrap();
    let request = session.begin_recovery().unwrap();
    let connection = Connection::open(db.path()).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_snapshot BEFORE UPDATE ON replicas BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    assert!(matches!(
        session.apply_snapshot(&request, snapshot(2, 2), UpdateSequence(2)),
        Err(ReplicationSessionError::Store(_))
    ));
    assert_eq!(session.cursor().unwrap(), before);
    connection
        .execute_batch("DROP TRIGGER fail_snapshot")
        .unwrap();
    assert!(session
        .apply_snapshot(&request, snapshot(2, 2), UpdateSequence(2))
        .is_err());
    install(&mut session, 2, 2);
    assert_eq!(
        session.cursor().unwrap().unwrap().covered_through,
        UpdateSequence(2)
    );
}

#[test]
fn concurrence_refuse_ancienne_image_et_recharge_projection_courante() {
    let db = Database::new();
    let mut first = db.session("account-a");
    install(&mut first, 1, 1);
    let mut stale = db.session("account-a");
    let request = stale.begin_recovery().unwrap();
    install(&mut first, 1, 2);
    assert!(matches!(
        stale.apply_snapshot(&request, snapshot(2, 1), UpdateSequence(1)),
        Err(ReplicationSessionError::Store(
            StoreError::StaleReplica { .. }
        ))
    ));
    assert_eq!(stale.cursor().unwrap(), first.cursor().unwrap());
}

#[test]
fn rechargement_apres_conflit_invalide_la_recherche_en_cours() {
    let db = Database::new();
    let mut first = db.session("account-a");
    install(&mut first, 1, 1);
    let mut stale = db.session("account-a");
    let request = stale.begin_recovery().unwrap();
    install(&mut first, 1, 2);
    let current = snapshot(1, 2);
    assert!(stale
        .apply_delta(Delta {
            conversation_id: conversation(),
            epoch: current.epoch,
            updates: vec![current.updates[1].clone()],
        })
        .is_err());
    assert_eq!(stale.cursor().unwrap(), first.cursor().unwrap());
    assert!(stale
        .apply_snapshot(&request, snapshot(2, 2), UpdateSequence(2))
        .is_err());
    assert_eq!(stale.cursor().unwrap(), first.cursor().unwrap());
    install(&mut stale, 2, 2);
}

#[test]
fn une_session_revoquee_refuse_tout_nouveau_snapshot() {
    let db = Database::new();
    let mut session = db.session("account-a");
    let request = session.begin_recovery().unwrap();
    session.revoke();
    assert!(matches!(
        session.begin_recovery(),
        Err(ReplicationSessionError::Inactive)
    ));
    assert!(matches!(
        session.apply_snapshot(&request, snapshot(1, 1), UpdateSequence(1)),
        Err(ReplicationSessionError::Inactive)
    ));
}
