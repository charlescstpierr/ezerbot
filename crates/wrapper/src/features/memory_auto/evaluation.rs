//! Offline evaluation of frozen model-format outputs through the real extractor.
//! No HTTP server, account, app launch or live model call is needed.
use super::*;
use grokbot_engine::{contracts::RunKind, model};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    message: String,
    expected: Vec<String>,
    #[serde(default)]
    known: Vec<String>,
    #[serde(default)]
    secret_traps: Vec<String>,
    #[serde(default)]
    vault_secrets: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recording {
    id: String,
    called: bool,
    // Preserve raw output: the evaluator must not construct proposals from labels.
    output: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    correct: usize,
    retained: usize,
    expected: usize,
    correct_empty: usize,
}

impl Counts {
    fn add(&mut self, expected: &[String], retained: &[String]) {
        // One-to-one matching: a repeated prediction must not earn a second TP.
        let mut unmatched: Vec<_> = expected.iter().map(|s| normalize(s)).collect();
        for fact in retained {
            if let Some(index) = unmatched.iter().position(|s| *s == normalize(fact)) {
                self.correct += 1;
                unmatched.remove(index);
            }
        }
        self.retained += retained.len();
        self.expected += expected.len();
        self.correct_empty += usize::from(expected.is_empty() && retained.is_empty());
    }

    fn scores(&self) -> (f64, f64, f64) {
        // Empty/empty is correct, not 0/0. Empty messages do not manufacture
        // true-positive facts in a corpus-level micro-average.
        if self.retained == 0 && self.expected == 0 {
            return (1.0, 1.0, 1.0);
        }
        let precision = self.correct as f64 / self.retained.max(1) as f64;
        let recall = self.correct as f64 / self.expected.max(1) as f64;
        let f1 = 2.0 * self.correct as f64 / (self.retained + self.expected) as f64;
        (precision, recall, f1)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Guardrails {
    inexact_quotes: usize,
    secrets: usize,
    duplicates: usize,
}

impl Guardrails {
    fn observe(&mut self, case: &Case, written: &[String]) {
        let mut seen: BTreeSet<_> = case.known.iter().map(|s| normalize(s)).collect();
        for text in written {
            self.inexact_quotes += usize::from(!case.message.contains(text.as_str()));
            // Independent labels, not unsafe_text/redact: a regression in either
            // production filter must not make the evaluator bless a trap.
            self.secrets += usize::from(
                case.secret_traps
                    .iter()
                    .any(|trap| normalize(text).contains(&normalize(trap))),
            );
            self.duplicates += usize::from(!seen.insert(normalize(text)));
        }
    }

    fn check(&self) -> Result<(), String> {
        if *self == Self::default() {
            Ok(())
        } else {
            Err(format!(
                "zero-tolerance memory guardrails violated: {self:?}"
            ))
        }
    }
}

struct Fixture {
    app: App,
    path: PathBuf,
    _dir: grokbot_test_support::TestDir,
}

impl Fixture {
    fn new(case: &Case) -> Self {
        let guard = grokbot_test_support::TestDir::new("memory-auto-evaluation");
        let directory = guard.to_path_buf();
        // A new, explicit space only; never read the user's database or auth.
        let path = directory.join("workspace.sqlite");
        let app = App::open(&path).unwrap();
        let conn = &app.connection;
        conn.execute(
            "INSERT INTO wrapper_conversations(account_id,id,title,created_at,updated_at) VALUES (?1,'eval','Évaluation',1,1)",
            [ACCOUNT],
        ).unwrap();
        for text in &case.known {
            remember(conn, text, Source::UserStated);
        }
        let vault = Vault::open(&path.with_extension("secrets.json")).unwrap();
        for (index, secret) in case.vault_secrets.iter().enumerate() {
            vault
                .set(
                    &format!("trap_{index}"),
                    "Piège synthétique d’évaluation",
                    secret,
                )
                .unwrap();
        }
        let tx = conn.unchecked_transaction().unwrap();
        let mut run = model::start(
            &tx,
            &model::Run {
                run_id: case.id.clone(),
                bot_id: "grokbot".into(),
                conversation_id: "eval".into(),
                trigger: Trigger::UserMessage {
                    message_id: case.id.clone(),
                    attempt: 0,
                },
                kind: RunKind::Chat,
                status: RunStatus::Running,
                task_status: None,
                turn_end: None,
                budgets: Default::default(),
                started_at: 1,
            },
        )
        .unwrap();
        run.status = RunStatus::Finished;
        run.turn_end = Some(grokbot_engine::contracts::TurnEnd::Finished);
        store::save(&tx, &run).unwrap();
        let input = HistoryItem {
            entry_id: case.id.clone(),
            role: grokbot_engine::contracts::Role::User,
            text: case.message.clone(),
            images: vec![],
            tool_items: vec![],
        };
        tx.execute(
            "INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) VALUES (?1,?2,1,'offline-recording')",
            params![case.id, serde_json::to_string(&input).unwrap()],
        ).unwrap();
        tx.commit().unwrap();
        Self {
            app,
            path,
            _dir: guard,
        }
    }

    fn written(&self) -> Vec<String> {
        // Inspect ALL persisted automatic facts, not only active ones: a later
        // correction must not hide an unsafe write from the safety counters.
        let mut stmt = self
            .app
            .connection
            .prepare("SELECT text,provenance FROM memory_facts ORDER BY fact_id")
            .unwrap();
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|row| {
            let (text, provenance) = row.unwrap();
            (
                text,
                serde_json::from_str::<Provenance>(&provenance).unwrap(),
            )
        })
        .filter(|(_, provenance)| provenance.source == Source::Automatic)
        .map(|(text, _)| text)
        .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Only the new directory this fixture created, never an existing space.
    }
}

fn remember(conn: &Connection, text: &str, source: Source) {
    facts::remember(
        conn,
        &Fact {
            fact_id: String::new(),
            bot_id: "grokbot".into(),
            scope: Scope::Private,
            text: text.into(),
            provenance: Provenance {
                source,
                conversation_id: None,
                entry_id: None,
            },
            learned_at: 1,
            status: FactStatus::Active,
            sensitive: false,
        },
    )
    .unwrap();
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).into()).collect()
}

#[test]
fn calculator_exact_normalized_quotes() {
    let mut counts = Counts::default();
    counts.add(
        &strings(&["Je préfère le thé."]),
        &strings(&[" JE PRÉFÈRE  LE THÉ. "]),
    );
    assert_eq!(counts.scores(), (1.0, 1.0, 1.0));
}

#[test]
fn calculator_partial_recall_and_duplicate_prediction() {
    let mut counts = Counts::default();
    counts.add(
        &strings(&["thé vert", "café noir", "eau fraîche"]),
        &strings(&["thé vert", "THÉ VERT"]),
    );
    assert_eq!(counts.scores(), (0.5, 1.0 / 3.0, 0.4));
}

#[test]
fn calculator_empty_message_is_correct_without_inflating_fact_counts() {
    let mut counts = Counts::default();
    counts.add(&[], &[]);
    assert_eq!(counts.scores(), (1.0, 1.0, 1.0));
    assert_eq!(counts.correct_empty, 1);
    assert_eq!(counts.correct, 0);
    counts.add(&[], &strings(&["fait indésirable"]));
    assert_eq!(counts.scores(), (0.0, 0.0, 0.0));
}

#[test]
fn recorded_outputs_replay_through_extract_with_zero_tolerance_guardrails() {
    let corpus: Vec<Case> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/memory_auto/corpus.json"
    ))
    .unwrap();
    let recordings: Vec<Recording> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/memory_auto/recorded_outputs.json"
    ))
    .unwrap();
    assert!(corpus.len() >= 60);
    let outputs: BTreeMap<_, _> = recordings.iter().map(|r| (r.id.as_str(), r)).collect();
    assert_eq!(outputs.len(), recordings.len(), "duplicate recording IDs");
    let ids: BTreeSet<_> = corpus.iter().map(|case| case.id.as_str()).collect();
    assert_eq!(ids.len(), corpus.len(), "duplicate corpus IDs");
    assert_eq!(
        ids,
        outputs.keys().copied().collect(),
        "recordings must match corpus IDs"
    );
    let mut counts = Counts::default();
    let mut guardrails = Guardrails::default();
    let mut calls = 0;
    for case in &corpus {
        let expected: BTreeSet<_> = case.expected.iter().map(|s| normalize(s)).collect();
        assert_eq!(
            expected.len(),
            case.expected.len(),
            "duplicate labels: {}",
            case.id
        );
        for label in &case.expected {
            assert!(
                normalize(&case.message).contains(&normalize(label)),
                "ungrounded label: {}",
                case.id
            );
            assert!(
                !case
                    .known
                    .iter()
                    .any(|known| normalize(known) == normalize(label)),
                "known fact labeled as new: {}",
                case.id
            );
        }
        for trap in &case.secret_traps {
            assert!(!trap.trim().is_empty(), "empty secret label: {}", case.id);
        }
        let fixture = Fixture::new(case);
        let recording = outputs[case.id.as_str()];
        let mut called = false;
        extract(
            &fixture.app.connection,
            &fixture.path,
            &case.id,
            |_, input| {
                called = true;
                let input: Value = serde_json::from_str(input).unwrap();
                assert_eq!(input["message"], case.message);
                Ok(recording.output.clone())
            },
        )
        .unwrap_or_else(|_| panic!("recorded extraction failed: {}", case.id));
        assert_eq!(called, recording.called, "provider boundary: {}", case.id);
        calls += usize::from(called);
        let written = fixture.written();
        guardrails.observe(case, &written);
        guardrails
            .check()
            .unwrap_or_else(|error| panic!("{}: {error}", case.id));
        counts.add(&case.expected, &written);
    }
    let (precision, recall, f1) = counts.scores();
    println!("memory_auto recorded baseline: messages={} calls={calls} correct={} retained={} expected={} correct_empty={} P={precision:.9} R={recall:.9} F1={f1:.9} guardrails={guardrails:?}", corpus.len(), counts.correct, counts.retained, counts.expected, counts.correct_empty);
    // A reproducibility baseline, not a minimum score for any live model.
    assert_eq!(
        counts,
        Counts {
            correct: 30,
            retained: 33,
            expected: 42,
            correct_empty: 26
        }
    );
    assert_eq!(calls, 47);
    assert_eq!(guardrails, Guardrails::default());
}

#[test]
fn guardrail_gate_fails_if_a_secret_trap_is_actually_written() {
    let corpus: Vec<Case> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/memory_auto/corpus.json"
    ))
    .unwrap();
    let case = corpus.iter().find(|case| case.id == "fr51").unwrap();
    let fixture = Fixture::new(case);
    // Bypass extraction deliberately, simulating a broken production filter.
    remember(&fixture.app.connection, &case.message, Source::Automatic);
    let mut guardrails = Guardrails::default();
    guardrails.observe(case, &fixture.written());
    assert_eq!(guardrails.secrets, 1);
    assert_eq!(guardrails.inexact_quotes, 0);
    assert_eq!(guardrails.duplicates, 0);
    assert!(
        guardrails.check().is_err(),
        "a perfect quote/score cannot excuse a secret write"
    );
}

#[test]
fn guardrail_gate_detects_inexact_quotes_and_normalized_known_or_new_duplicates() {
    let case = Case {
        id: "guardrail-check".into(),
        message: "Je préfère le thé.".into(),
        expected: vec![],
        known: strings(&["Je préfère le thé."]),
        secret_traps: vec![],
        vault_secrets: vec![],
    };
    let mut guardrails = Guardrails::default();
    guardrails.observe(
        &case,
        &strings(&[
            "JE PRÉFÈRE LE THÉ.",
            "Je préfère le thé.",
            "citation inventée",
        ]),
    );
    assert_eq!(guardrails.inexact_quotes, 2);
    assert_eq!(guardrails.duplicates, 2);
    assert!(guardrails.check().is_err());
    let case = Case {
        known: vec![],
        ..case
    };
    let mut guardrails = Guardrails::default();
    guardrails.observe(
        &case,
        &strings(&["Je préfère le thé.", "Je préfère le thé."]),
    );
    assert_eq!(guardrails.duplicates, 1);
    assert_eq!(guardrails.inexact_quotes, 0);
    assert!(guardrails.check().is_err());
}
