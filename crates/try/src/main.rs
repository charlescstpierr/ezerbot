//! Scénario local du journal d'envoi et de la réplique.
//!
//! La base SQLite est fermée puis rouverte pendant le scénario.

#![forbid(unsafe_code)]

mod flow;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use grokbot_domain::{
    AccountGeneration, AccountId, AttemptId, AuthenticatedActor, BootId, CheckpointId,
    ContentDigest, ConversationId, DeliveryReceipt, EntryBody, MessageId, ReconciliationId,
    ReplicaEpoch, RunState, ServerGeneration, TranscriptBody, TranscriptEntryId, TranscriptUpdate,
    UpdateKind, UpdateSequence,
};
use grokbot_replica::{Delta, ReplicaError, Snapshot, REORDER_LIMIT};
use grokbot_store::{SessionStore, StoredJournal, StoredReplica};
use grokbot_submission_journal::{JournalError, PreparedMessage, RetryDecision};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("erreur : {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut command = None;
    let mut database = PathBuf::from("try-data/session.sqlite");
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--db" => {
                let path = args
                    .next()
                    .ok_or_else(|| "il manque le chemin après --db".to_owned())?;
                database = PathBuf::from(path);
            }
            "demo" | "flow" | "status" | "help" | "-h" | "--help" => {
                if command.is_some() {
                    return Err("une seule commande à la fois".into());
                }
                command = Some(argument);
            }
            other => {
                return Err(format!("argument inconnu : {other}\n\n{}", help_text()));
            }
        }
    }
    match command.as_deref() {
        Some("status") => status(&database),
        Some("flow") => flow::run(&database),
        Some("demo") | None => demo(&database),
        Some("help") | Some("-h") | Some("--help") => {
            print_help();
            Ok(())
        }
        Some(_) => unreachable!("les commandes sont filtrées plus haut"),
    }
}

fn print_help() {
    println!("{}", help_text());
}

fn help_text() -> &'static str {
    "Usage :\n  \
     cargo run -p grokbot-try -- demo\n  \
     cargo run -p grokbot-try -- status\n  \
     cargo run -p grokbot-try -- flow --db try-data/flow.sqlite\n  \
     cargo run -p grokbot-try -- demo --db try-data/session.sqlite\n\n\
     Le scénario écrit une base SQLite, la ferme et la rouvre.\n\
     Il exerce la répétition d'un message, une coupure, un effet incertain,\n\
     un changement de compte, un trou de réplique et un corps manquant."
}

fn create_database(path: &Path) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "la démo exige une nouvelle base à {} : {error}",
                path.display()
            )
        })?;
    Ok(())
}

fn demo(path: &Path) -> Result<(), String> {
    create_database(path)?;
    println!("Essai du journal et de la réplique");
    println!("Base : {}", path.display());
    println!();

    let mut failures = Vec::new();
    let actor = demo_actor(1);
    let next_account = demo_actor(2);
    let conversation = conversation();

    repetition(path, &actor, &conversation, &mut failures)?;
    acceptance(path, &actor, &conversation, &mut failures)?;
    uncertain(path, &actor, &conversation, &mut failures)?;
    account_switch(path, &actor, &next_account, &conversation, &mut failures)?;
    replica(path, &conversation, &mut failures)?;
    late_tool_result(&mut failures);

    println!();
    if failures.is_empty() {
        println!("Scénario réussi. Le journal et la réplique se relisent depuis SQLite.");
        println!();
        println!("Pour relire la base sans rejouer le scénario :");
        println!(
            "  cargo run -p grokbot-try -- status --db {}",
            path.display()
        );
        Ok(())
    } else {
        Err(format!(
            "{} écart(s) : {}",
            failures.len(),
            failures.join(", ")
        ))
    }
}

fn repetition(
    path: &Path,
    actor: &AuthenticatedActor,
    conversation: &ConversationId,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    println!("1. Répétition du même message");
    let message = must_id(MessageId::new("msg-repetition"));
    let mut journal = load_journal(path)?;
    let draft = PreparedMessage {
        actor: actor.clone(),
        conversation_id: conversation.clone(),
        message_id: message.clone(),
        body: "bonjour".into(),
    };
    journal
        .prepare(draft.clone())
        .map_err(|error| error.to_string())?;
    journal.prepare(draft).map_err(|error| error.to_string())?;
    let count = journal
        .export_committed()
        .into_iter()
        .filter(|item| item.message_id == message)
        .count();
    reopen_journal(path, &mut journal)?;
    let state = journal
        .get(&actor.account_id, conversation, &message)
        .ok_or_else(|| "le message répété a disparu après réouverture".to_owned())?;
    check(
        failures,
        "répétition",
        count == 1 && matches!(state, grokbot_domain::SubmissionState::Queued),
        format!("un seul enregistrement, état {}", format_state(&state)),
    );
    Ok(())
}

fn acceptance(
    path: &Path,
    actor: &AuthenticatedActor,
    conversation: &ConversationId,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    println!("2. Coupure avant la validation de l'acceptation");
    let message = must_id(MessageId::new("msg-acceptation"));
    let mut journal = load_journal(path)?;
    journal
        .prepare(PreparedMessage {
            actor: actor.clone(),
            conversation_id: conversation.clone(),
            message_id: message.clone(),
            body: "envoie ceci".into(),
        })
        .map_err(|error| error.to_string())?;
    journal
        .dispatch(
            actor,
            conversation,
            &message,
            must_id(AttemptId::new("tentative-1")),
        )
        .map_err(|error| error.to_string())?;
    let after_crash = journal
        .stage_accept_then_crash(actor, receipt(actor, conversation, &message))
        .map_err(|error| error.to_string())?;
    reopen_journal(path, &mut journal)?;
    let visible = journal
        .get(&actor.account_id, conversation, &message)
        .ok_or_else(|| "l'envoi a disparu".to_owned())?;
    let lost = visible == after_crash
        && matches!(visible, grokbot_domain::SubmissionState::Dispatching { .. });

    journal
        .observe_accept(actor, receipt(actor, conversation, &message))
        .map_err(|error| error.to_string())?;
    reopen_journal(path, &mut journal)?;
    let accepted = journal
        .get(&actor.account_id, conversation, &message)
        .ok_or_else(|| "l'acceptation validée a disparu".to_owned())?;
    let decision = journal
        .reconcile_before_retry(actor, conversation, &message)
        .map_err(|error| error.to_string())?;
    let durable = matches!(decision, RetryDecision::AlreadyDurable { .. })
        && matches!(
            accepted,
            grokbot_domain::SubmissionState::AcceptedAwaitingEcho { .. }
        );
    check(
        failures,
        "acceptation",
        lost && durable,
        format!(
            "la coupure laisse {}, la base rouverte garde {}",
            format_state(&after_crash),
            format_state(&accepted)
        ),
    );
    Ok(())
}

fn uncertain(
    path: &Path,
    actor: &AuthenticatedActor,
    conversation: &ConversationId,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    println!("3. Effet incertain");
    let message = must_id(MessageId::new("msg-incertitude"));
    let mut journal = load_journal(path)?;
    journal
        .prepare(PreparedMessage {
            actor: actor.clone(),
            conversation_id: conversation.clone(),
            message_id: message.clone(),
            body: "peut-être parti".into(),
        })
        .map_err(|error| error.to_string())?;
    journal
        .dispatch(
            actor,
            conversation,
            &message,
            must_id(AttemptId::new("tentative-2")),
        )
        .map_err(|error| error.to_string())?;
    journal
        .mark_outcome_unknown(
            actor,
            conversation,
            &message,
            must_id(ReconciliationId::new("reconcile-1")),
        )
        .map_err(|error| error.to_string())?;
    reopen_journal(path, &mut journal)?;
    let decision = journal
        .reconcile_before_retry(actor, conversation, &message)
        .map_err(|error| error.to_string())?;
    check(
        failures,
        "incertitude",
        matches!(decision, RetryDecision::NeedsReconciliation { .. }),
        "après réouverture, le renvoi aveugle est refusé",
    );
    Ok(())
}

fn account_switch(
    path: &Path,
    owner: &AuthenticatedActor,
    next_account: &AuthenticatedActor,
    conversation: &ConversationId,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    println!("4. Changement de génération du compte");
    let message = must_id(MessageId::new("msg-compte"));
    let mut journal = load_journal(path)?;
    journal
        .prepare(PreparedMessage {
            actor: owner.clone(),
            conversation_id: conversation.clone(),
            message_id: message.clone(),
            body: "pour le premier compte".into(),
        })
        .map_err(|error| error.to_string())?;
    let stale = journal.observe_accept(next_account, receipt(owner, conversation, &message));
    let unchanged = journal
        .get(&owner.account_id, conversation, &message)
        .ok_or_else(|| "le message du premier compte a disparu".to_owned())?;
    reopen_journal(path, &mut journal)?;
    let reloaded = journal
        .get(&owner.account_id, conversation, &message)
        .ok_or_else(|| "le message du premier compte a disparu au rechargement".to_owned())?;
    check(
        failures,
        "compte",
        matches!(stale, Err(JournalError::StaleAccount))
            && matches!(unchanged, grokbot_domain::SubmissionState::Queued)
            && reloaded == unchanged,
        "la réponse de génération 2 est ignorée, la file originale reste en place",
    );
    Ok(())
}

fn replica(
    path: &Path,
    conversation: &ConversationId,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    println!("5. Trou, corps manquant, puis réouverture");
    let epoch = ReplicaEpoch {
        boot_id: must_id(BootId::new("boot-demo")),
        server_generation: ServerGeneration(1),
        recovery: 0,
    };
    let mut replica = SessionStore::open(path, &demo_actor(1).account_id)
        .map_err(|error| error.to_string())?
        .load_replica(conversation)
        .map_err(|error| error.to_string())?;
    replica
        .apply_snapshot(Snapshot {
            conversation_id: conversation.clone(),
            epoch: epoch.clone(),
            updates: vec![
                update(1, "entree-1", text("premier")),
                update(
                    2,
                    "entree-2",
                    TranscriptBody::Pending {
                        expected_digest: ContentDigest([9; 32]),
                    },
                ),
            ],
        })
        .map_err(|error| error.to_string())?;
    let too_far = 3 + REORDER_LIMIT as u64 + 1;
    let far = replica.apply_delta(Delta {
        conversation_id: conversation.clone(),
        epoch: epoch.clone(),
        updates: vec![update(too_far, "entree-loin", text("trop loin"))],
    });
    let blocked = replica
        .cursor()
        .ok_or_else(|| "curseur absent".to_owned())?;
    reopen_replica(path, conversation, &mut replica)?;
    let still_blocked = replica.cursor().ok_or_else(|| "curseur perdu".to_owned())?;
    replica
        .fill_body(
            &epoch,
            UpdateSequence(2),
            &must_id(TranscriptEntryId::new("entree-2")),
            text("corps arrivé"),
        )
        .map_err(|error| error.to_string())?;
    reopen_replica(path, conversation, &mut replica)?;
    let filled = replica
        .cursor()
        .ok_or_else(|| "curseur perdu après le corps".to_owned())?;
    let gap = matches!(
        far,
        Err(ReplicaError::Gap { expected: 3, got }) if got == too_far
    );
    check(
        failures,
        "réplique",
        gap
            && !blocked.complete
            && blocked.covered_through == UpdateSequence(1)
            && still_blocked == blocked
            && filled.complete
            && filled.covered_through == UpdateSequence(2)
            && replica.entry(&must_id(TranscriptEntryId::new("entree-loin"))).is_none(),
        format!(
            "le trou lointain est refusé, le corps manquant bloque la séquence 2, puis la couverture atteint {}",
            filled.covered_through.0
        ),
    );
    Ok(())
}

fn late_tool_result(failures: &mut Vec<String>) {
    println!("6. Réponse tardive après annulation");
    let state = RunState::Cancelled {
        checkpoint: must_id(CheckpointId::new("checkpoint-1")),
    };
    let ignored = matches!(
        state.dispose_tool_result(),
        grokbot_domain::EffectDisposition::IgnoredAfterInterruption
    );
    check(
        failures,
        "réponse tardive",
        ignored,
        "le résultat d'outil n'achève pas le run annulé",
    );
}

fn status(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!(
            "aucune base à {}. Lance d'abord : cargo run -p grokbot-try -- demo",
            path.display()
        ));
    }
    let store =
        SessionStore::open(path, &demo_actor(1).account_id).map_err(|error| error.to_string())?;
    println!("Base : {}", path.display());
    println!();
    println!("Journal");
    let submissions = store
        .list_submissions()
        .map_err(|error| error.to_string())?;
    if submissions.is_empty() {
        println!("  (vide)");
    }
    for item in submissions {
        println!(
            "  {} / {} / {}  génération {}  {}",
            item.account_id.as_str(),
            item.conversation_id.as_str(),
            item.message_id.as_str(),
            item.generation.0,
            format_state(&item.state)
        );
        println!("    corps : {}", item.body);
    }
    println!();
    println!("Réplique");
    let conversation = conversation();
    let replica = store
        .load_replica(&conversation)
        .map_err(|error| error.to_string())?;
    match replica.cursor() {
        Some(cursor) => {
            println!(
                "  {}  couverture {}  {}",
                conversation.as_str(),
                cursor.covered_through.0,
                if cursor.complete {
                    "complète"
                } else {
                    "incomplète"
                }
            );
            println!(
                "  journal {}, buffer {}",
                replica.export().log.len(),
                replica.export().buffer.len()
            );
        }
        None => println!("  {} sans epoch", conversation.as_str()),
    }
    Ok(())
}

fn reopen_journal(path: &Path, journal: &mut StoredJournal) -> Result<(), String> {
    let mut store =
        SessionStore::open(path, &demo_actor(1).account_id).map_err(|error| error.to_string())?;
    store
        .save_journal(journal)
        .map_err(|error| error.to_string())?;
    drop(store);
    *journal = load_journal(path)?;
    Ok(())
}

fn load_journal(path: &Path) -> Result<StoredJournal, String> {
    let store =
        SessionStore::open(path, &demo_actor(1).account_id).map_err(|error| error.to_string())?;
    store.load_journal().map_err(|error| error.to_string())
}

fn reopen_replica(
    path: &Path,
    conversation: &ConversationId,
    replica: &mut StoredReplica,
) -> Result<(), String> {
    let mut store =
        SessionStore::open(path, &demo_actor(1).account_id).map_err(|error| error.to_string())?;
    store
        .save_replica(replica)
        .map_err(|error| error.to_string())?;
    drop(store);
    let store =
        SessionStore::open(path, &demo_actor(1).account_id).map_err(|error| error.to_string())?;
    *replica = store
        .load_replica(conversation)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn check(failures: &mut Vec<String>, name: &str, held: bool, detail: impl std::fmt::Display) {
    if held {
        println!("  tenu — {detail}");
    } else {
        println!("  écart — {detail}");
        failures.push(name.to_owned());
    }
}

fn demo_actor(generation: u64) -> AuthenticatedActor {
    AuthenticatedActor {
        account_id: must_id(AccountId::new("compte-demo")),
        team_id: None,
        generation: AccountGeneration(generation),
    }
}

fn conversation() -> ConversationId {
    must_id(ConversationId::new("conversation-demo"))
}

fn receipt(
    actor: &AuthenticatedActor,
    conversation: &ConversationId,
    message: &MessageId,
) -> DeliveryReceipt {
    DeliveryReceipt {
        account_id: actor.account_id.clone(),
        conversation_id: conversation.clone(),
        message_id: message.clone(),
    }
}

fn text(value: &str) -> TranscriptBody {
    TranscriptBody::Inline {
        content: EntryBody::Text(value.to_owned()),
    }
}

fn update(sequence: u64, id: &str, body: TranscriptBody) -> TranscriptUpdate {
    TranscriptUpdate {
        sequence: UpdateSequence(sequence),
        entry_id: must_id(TranscriptEntryId::new(id)),
        kind: UpdateKind::Upsert { body },
    }
}

fn format_state(state: &grokbot_domain::SubmissionState) -> String {
    match state {
        grokbot_domain::SubmissionState::Prepared => "préparé".into(),
        grokbot_domain::SubmissionState::Queued => "en file".into(),
        grokbot_domain::SubmissionState::Dispatching { attempt } => {
            format!("envoi en cours ({})", attempt.as_str())
        }
        grokbot_domain::SubmissionState::AcceptedAwaitingEcho { .. } => {
            "accepté, en attente de l'écho".into()
        }
        grokbot_domain::SubmissionState::Echoed { entry } => {
            format!("écho reçu ({})", entry.as_str())
        }
        grokbot_domain::SubmissionState::Refused { .. } => "refusé".into(),
        grokbot_domain::SubmissionState::OutcomeUnknown { .. } => "issue inconnue".into(),
    }
}

fn must_id<T>(result: Result<T, grokbot_domain::EmptyId>) -> T {
    result.expect("identifiant de démonstration")
}
