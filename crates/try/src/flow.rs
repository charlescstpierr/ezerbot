//! Parcours local de l'orchestrateur, avec un serveur simulé et une vraie base SQLite.

use std::collections::HashMap;
use std::path::Path;

use grokbot_domain::{
    AccountId, ConversationId, DeliveryReceipt, MessageId, SubmissionState, TranscriptEntryId,
    UpdateSequence,
};
use grokbot_replica::Snapshot;
use grokbot_session::{
    LookupOutcome, ReconciliationRequest, SendOutcome, SendRequest, SnapshotSession,
    SubmissionSession, SubmissionTransport,
};

use super::{conversation, create_database, demo_actor, must_id, text, update};

#[derive(Default)]
struct LocalServer {
    accepted: HashMap<(AccountId, ConversationId, MessageId), DeliveryReceipt>,
    lose_next_reply: bool,
    sends: usize,
    lookups: usize,
}

impl SubmissionTransport for LocalServer {
    fn send(&mut self, request: &SendRequest) -> SendOutcome {
        self.sends += 1;
        let receipt = DeliveryReceipt {
            account_id: request.actor().account_id.clone(),
            conversation_id: request.conversation_id().clone(),
            message_id: request.message_id().clone(),
        };
        self.accepted
            .entry((
                receipt.account_id.clone(),
                receipt.conversation_id.clone(),
                receipt.message_id.clone(),
            ))
            .or_insert_with(|| receipt.clone());
        if self.lose_next_reply {
            self.lose_next_reply = false;
            SendOutcome::OutcomeUnknown
        } else {
            SendOutcome::Accepted(receipt)
        }
    }

    fn lookup(&mut self, request: &ReconciliationRequest) -> LookupOutcome {
        self.lookups += 1;
        match self.accepted.get(&(
            request.actor().account_id.clone(),
            request.conversation_id().clone(),
            request.message_id().clone(),
        )) {
            Some(receipt) => LookupOutcome::Found(receipt.clone()),
            None => LookupOutcome::NotFound,
        }
    }
}

pub(super) fn run(path: &Path) -> Result<(), String> {
    create_database(path)?;
    println!("Orchestration locale avec transport simulé");
    println!("Base : {}", path.display());
    let conversation = conversation();
    let lost = must_id(MessageId::new("msg-orchestration-loss"));
    let absent = must_id(MessageId::new("msg-orchestration-not-found"));
    let mut server = LocalServer {
        lose_next_reply: true,
        ..LocalServer::default()
    };

    let mut session = SubmissionSession::open(path, demo_actor(1)).map_err(show)?;
    session
        .prepare(conversation.clone(), lost.clone(), "réponse perdue".into())
        .map_err(show)?;
    let uncertain = session
        .send_with(&conversation, &lost, &mut server)
        .map_err(show)?;
    if !matches!(uncertain, SubmissionState::OutcomeUnknown { .. }) {
        return Err("la perte de réponse doit rester incertaine".into());
    }
    drop(session);

    let mut session = SubmissionSession::open(path, demo_actor(2)).map_err(show)?;
    let accepted = session
        .reconcile_with(&conversation, &lost, &mut server)
        .map_err(show)?;
    if !matches!(accepted, SubmissionState::AcceptedAwaitingEcho { .. }) {
        return Err("la recherche doit retrouver l'acceptation".into());
    }
    if session.send_with(&conversation, &lost, &mut server).is_ok() {
        return Err("une acceptation connue ne doit pas déclencher un second envoi".into());
    }
    println!("1. Réponse perdue : acceptation retrouvée après réouverture, sans renvoi.");

    session
        .prepare(
            conversation.clone(),
            absent.clone(),
            "coupure avant transport".into(),
        )
        .map_err(show)?;
    let _unsent_request = session.begin_send(&conversation, &absent).map_err(show)?;
    drop(session);

    let mut session = SubmissionSession::open(path, demo_actor(3)).map_err(show)?;
    let not_found = session
        .reconcile_with(&conversation, &absent, &mut server)
        .map_err(show)?;
    if !matches!(not_found, SubmissionState::Queued) {
        return Err("seul le statut absent doit remettre le même message en file".into());
    }
    session
        .send_with(&conversation, &absent, &mut server)
        .map_err(show)?;
    println!("2. Coupure avant transport : statut absent, puis envoi du même identifiant.");

    let mut replica =
        SnapshotSession::open(path, demo_actor(3), conversation.clone()).map_err(show)?;
    let stale = replica.begin_recovery().map_err(show)?;
    let current = replica.begin_recovery().map_err(show)?;
    let snapshot = Snapshot {
        conversation_id: conversation.clone(),
        epoch: grokbot_domain::ReplicaEpoch {
            boot_id: must_id(grokbot_domain::BootId::new("flow-boot")),
            server_generation: grokbot_domain::ServerGeneration(1),
            recovery: 0,
        },
        updates: vec![
            update(1, "flow-entry-1", text("réponse perdue")),
            update(2, "flow-entry-2", text("coupure avant transport")),
        ],
    };
    if replica
        .apply_snapshot(&stale, snapshot.clone(), UpdateSequence(2))
        .is_ok()
        || replica.cursor().map_err(show)?.is_some()
    {
        return Err("un ancien snapshot ne doit pas remplacer la projection".into());
    }
    replica
        .apply_snapshot(&current, snapshot, UpdateSequence(2))
        .map_err(show)?;
    session
        .observe_echo(
            &conversation,
            &lost,
            must_id(TranscriptEntryId::new("flow-entry-1")),
        )
        .map_err(show)?;
    session
        .observe_echo(
            &conversation,
            &absent,
            must_id(TranscriptEntryId::new("flow-entry-2")),
        )
        .map_err(show)?;
    println!("3. Snapshot périmé refusé. Snapshot courant et deux échos sauvegardés.");
    if server.sends != 2 || server.lookups != 2 || server.accepted.len() != 2 {
        return Err("le transport simulé a produit un nombre d'effets inattendu".into());
    }
    println!(
        "{} envois, {} recherches, deux messages logiques.",
        server.sends, server.lookups
    );
    println!("Scénario réussi. La commande status relit le journal et la couverture 2.");
    Ok(())
}

fn show(error: impl std::fmt::Display) -> String {
    error.to_string()
}
