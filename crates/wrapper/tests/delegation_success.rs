//! Calculateur de réussite par tokens dépensés, sur cinq tâches factices rejouées sur le faux
//! serveur en boucle locale (docs/evaluation.md). Aucun compte, aucun réseau, aucun juge-modèle.
mod support;

use grokbot_delegation::graph::{self, DelegationStatus};
use grokbot_engine::{contracts::Trigger, store};
use grokbot_forms::requests::{self, RequestStatus};
use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    fmt, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

struct Trial {
    task: &'static str,
    completed: bool,
    correct: bool,
    tokens: u64,
}

impl Trial {
    /// Une réponse juste sans `complete_task` n'est pas un succès.
    fn success(&self) -> bool {
        self.completed && self.correct
    }
}

struct Spread {
    n: usize,
    mean: f64,
    /// Écart-type d'échantillon (n - 1) ; 0 quand n < 2.
    std: f64,
    median: f64,
    min: f64,
    max: f64,
}

impl Spread {
    fn of(values: &[f64]) -> Self {
        let n = values.len();
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let (Some(&min), Some(&max)) = (sorted.first(), sorted.last()) else {
            return Self {
                n,
                mean: 0.0,
                std: 0.0,
                median: 0.0,
                min: 0.0,
                max: 0.0,
            };
        };
        let mean = values.iter().sum::<f64>() / n as f64;
        let squares: f64 = values.iter().map(|value| (value - mean).powi(2)).sum();
        Self {
            n,
            mean,
            std: if n < 2 {
                0.0
            } else {
                (squares / (n - 1) as f64).sqrt()
            },
            median: (sorted[(n - 1) / 2] + sorted[n / 2]) / 2.0,
            min,
            max,
        }
    }
}

struct TaskStats {
    task: &'static str,
    /// Sur 0/1 : la moyenne est le taux de réussite de la tâche.
    success: Spread,
    tokens: Spread,
}

struct Report {
    trials: usize,
    successes: usize,
    tokens: u64,
    /// Dans l'ordre de première apparition.
    tasks: Vec<TaskStats>,
}

fn report(trials: &[Trial]) -> Report {
    let mut by_task: Vec<(&'static str, Vec<&Trial>)> = Vec::new();
    for trial in trials {
        match by_task.iter_mut().find(|(task, _)| *task == trial.task) {
            Some((_, group)) => group.push(trial),
            None => by_task.push((trial.task, vec![trial])),
        }
    }
    Report {
        trials: trials.len(),
        successes: trials.iter().filter(|trial| trial.success()).count(),
        tokens: trials.iter().map(|trial| trial.tokens).sum(),
        tasks: by_task
            .into_iter()
            .map(|(task, group)| TaskStats {
                task,
                success: Spread::of(
                    &group
                        .iter()
                        .map(|trial| f64::from(u8::from(trial.success())))
                        .collect::<Vec<_>>(),
                ),
                tokens: Spread::of(
                    &group
                        .iter()
                        .map(|trial| trial.tokens as f64)
                        .collect::<Vec<_>>(),
                ),
            })
            .collect(),
    }
}

impl Report {
    fn success_rate(&self) -> f64 {
        if self.trials == 0 {
            0.0
        } else {
            self.successes as f64 / self.trials as f64
        }
    }

    fn tokens_per_success(&self) -> Option<f64> {
        (self.successes > 0).then(|| self.tokens as f64 / self.successes as f64)
    }

    fn successes_per_million_tokens(&self) -> f64 {
        if self.tokens == 0 {
            0.0
        } else {
            self.successes as f64 * 1e6 / self.tokens as f64
        }
    }
}

/// Virgule décimale et espace pour les milliers, comme le reste de l'interface.
fn french(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    if fraction.is_empty() {
        grouped
    } else {
        format!("{grouped},{fraction}")
    }
}

impl fmt::Display for Report {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            out,
            "essais : {} ; tokens totaux : {}",
            self.trials,
            french(self.tokens as f64, 0)
        )?;
        writeln!(
            out,
            "taux de réussite : {} % ({}/{})",
            french(self.success_rate() * 100.0, 1),
            self.successes,
            self.trials
        )?;
        writeln!(
            out,
            "tokens par succès : {}",
            self.tokens_per_success()
                .map_or("—".to_owned(), |tokens| french(tokens, 0))
        )?;
        writeln!(
            out,
            "succès par million de tokens : {}",
            french(self.successes_per_million_tokens(), 1)
        )?;
        for TaskStats {
            task,
            success,
            tokens,
        } in &self.tasks
        {
            writeln!(
                out,
                "{task} : réussite {} % ± {} ; tokens {} ± {} (médiane {}, min {}, max {}, n {})",
                french(success.mean * 100.0, 1),
                french(success.std * 100.0, 1),
                french(tokens.mean, 0),
                french(tokens.std, 0),
                french(tokens.median, 0),
                french(tokens.min, 0),
                french(tokens.max, 0),
                tokens.n
            )?;
        }
        Ok(())
    }
}

/// Politique du faux modèle (et de la personne qui approuve) pour un essai : c'est le bruit qui
/// fait différer les répétitions. Le correcteur ne la voit jamais.
#[derive(Clone, Copy, Debug)]
enum Behaviour {
    Solve,
    WrongAnswer,
    TextOnly,
    Denied,
}

/// L'état final observable d'un essai : tout ce que voit un correcteur.
struct Observed {
    status: DelegationStatus,
    space: PathBuf,
    /// Faits actifs du bot enfant.
    facts: Vec<String>,
}

struct Task {
    id: &'static str,
    objective: fn(&Path) -> String,
    setup: fn(&Path),
    /// Réponses successives du faux modèle enfant, sans leur événement de fin.
    script: fn(Behaviour, &Path) -> Vec<Vec<Value>>,
    grade: fn(&Observed) -> bool,
    trials: [Behaviour; 3],
}

/// La valeur attendue doit être un mot entier du résumé, sans signe ni décimales collés :
/// `13 fichiers`, `-391` et `391,5` sont faux.
fn summary_has(observed: &Observed, value: &str) -> bool {
    let DelegationStatus::Done { result, .. } = &observed.status else {
        return false;
    };
    result.match_indices(value).any(|(start, _)| {
        let before = result[..start].chars().next_back();
        let mut after = result[start + value.len()..].chars();
        let glued = match after.next() {
            Some('.' | ',') => after.next().is_some_and(char::is_alphanumeric),
            next => next.is_some_and(char::is_alphanumeric),
        };
        !before.is_some_and(|c| c.is_alphanumeric() || "-−+.,".contains(c)) && !glued
    })
}

fn call(name: &str, args: Value) -> Value {
    json!({"type":"response.output_item.done","item":{"id":format!("fc-{name}"),"type":"function_call",
        "call_id":format!("call-{name}"),"name":name,"arguments":args.to_string()}})
}

fn complete(summary: &str) -> Vec<Value> {
    vec![call("complete_task", json!({"summary":summary}))]
}

fn say(text: &str) -> Vec<Value> {
    vec![json!({"type":"response.output_text.delta","delta":text})]
}

const MEETING: &str = "La réunion d’équipe est le mardi.";

const CORPUS: [Task; 5] = [
    Task {
        id: "produit",
        objective: |_| "Calcule 17 × 23 et donne le résultat.".into(),
        setup: |_| {},
        script: |behaviour, _| match behaviour {
            Behaviour::Solve => vec![complete("17 × 23 = 391")],
            Behaviour::WrongAnswer => vec![complete("17 × 23 = 381")],
            Behaviour::TextOnly => vec![say("Terminé : 391.")],
            Behaviour::Denied => unreachable!("aucune approbation dans cette tâche"),
        },
        grade: |observed| summary_has(observed, "391"),
        trials: [Behaviour::Solve, Behaviour::Solve, Behaviour::WrongAnswer],
    },
    Task {
        id: "lecture",
        objective: |space| format!("Lis {}/donnees.txt et donne le code.", space.display()),
        setup: |space| fs::write(space.join("donnees.txt"), "code=7B42").unwrap(),
        script: |behaviour, space| {
            let read = vec![call("read_file", json!({"path":space.join("donnees.txt")}))];
            match behaviour {
                Behaviour::Solve => vec![read, complete("Le code est 7B42.")],
                Behaviour::TextOnly => vec![read, say("Le code est 7B42, c’est terminé.")],
                _ => unreachable!("comportement hors de cette tâche"),
            }
        },
        grade: |observed| summary_has(observed, "7B42"),
        trials: [Behaviour::Solve, Behaviour::TextOnly, Behaviour::Solve],
    },
    Task {
        id: "fichier",
        objective: |space| {
            format!(
                "Crée le fichier {}/rapport.txt contenant exactement OK.",
                space.display()
            )
        },
        setup: |_| {},
        // La personne refuse la carte, mais le faux modèle annonce quand même avoir réussi.
        script: |_, space| {
            vec![
                vec![call(
                    "write_file",
                    json!({"path":space.join("rapport.txt"),"content":"OK","expected_sha256":null}),
                )],
                complete("rapport.txt créé"),
            ]
        },
        grade: |observed| {
            fs::read_to_string(observed.space.join("rapport.txt")).is_ok_and(|text| text == "OK")
        },
        trials: [Behaviour::Solve, Behaviour::Denied, Behaviour::Solve],
    },
    Task {
        id: "memoire",
        objective: |_| "Retiens que la réunion d’équipe est le mardi.".into(),
        setup: |_| {},
        script: |behaviour, _| {
            let text = match behaviour {
                Behaviour::Solve => MEETING,
                Behaviour::WrongAnswer => "La réunion d’équipe est le mercredi.",
                _ => unreachable!("comportement hors de cette tâche"),
            };
            vec![
                vec![call(
                    "remember",
                    json!({"text":text,"sensitive":false,"source":"user_stated"}),
                )],
                complete("Fait mémorisé."),
            ]
        },
        grade: |observed| observed.facts.iter().any(|fact| fact == MEETING),
        trials: [Behaviour::Solve, Behaviour::WrongAnswer, Behaviour::Solve],
    },
    Task {
        id: "dossier",
        objective: |space| {
            format!(
                "Compte les fichiers .txt de {}/inventaire.",
                space.display()
            )
        },
        setup: |space| {
            fs::create_dir(space.join("inventaire")).unwrap();
            for name in ["a.txt", "b.txt", "c.txt"] {
                fs::write(space.join("inventaire").join(name), name).unwrap();
            }
        },
        script: |behaviour, space| {
            let list = vec![call(
                "list_directory",
                json!({"path":space.join("inventaire")}),
            )];
            match behaviour {
                Behaviour::Solve => vec![list, complete("3 fichiers")],
                Behaviour::TextOnly => vec![list, say("Il y a 3 fichiers.")],
                _ => unreachable!("comportement hors de cette tâche"),
            }
        },
        grade: |observed| summary_has(observed, "3 fichiers"),
        trials: [Behaviour::Solve, Behaviour::Solve, Behaviour::TextOnly],
    },
];

/// Présent dans les instructions du seul enfant : le parent, les titres et les appels auxiliaires
/// ne consomment jamais le script de l'enfant.
const CHILD_MARKER: &str = "You are executing a delegated Task";
const LOOPBACK: &str = "http://127.0.0.1:";

struct Outcome {
    trial: Trial,
    observed: Observed,
    /// Garde l'espace de l'essai tant que le test lit son état final.
    _dir: grokbot_test_support::TestDir,
    /// Cartes de lancement en fond levées par le parent.
    launch_cards: usize,
    /// Cartes d'approbation levées par l'enfant.
    child_cards: usize,
}

fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn snapshot(space: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![space.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push((path.clone(), fs::read(path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

/// Un essai complet par le vrai chemin : message, délégation, carte de lancement approuvée, enfant
/// scripté. Chaque essai a son propre dossier, sa base et son faux serveur.
fn run_trial(task: &Task, behaviour: Behaviour, index: usize) -> Outcome {
    let dir = grokbot_test_support::TestDir::new("ezerbot-delegation-success");
    let space = dir.to_path_buf().join("espace");
    fs::create_dir_all(&space).unwrap();
    let space = space.canonicalize().unwrap();
    (task.setup)(&space);
    let db_path = dir.to_path_buf().join("data/app.sqlite");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    fs::write(
        db_path.with_extension("chatgpt-auth.json"),
        json!({"host_id":"test-host","client_id":CLIENT,"access_token":"fake-access",
            "refresh_token":"fake-refresh","expires_at":expires,
            "scopes":"chatgpt.tokens.use.direct"})
        .to_string(),
    )
    .unwrap();

    let child_script: Arc<Mutex<VecDeque<ScriptedResponse>>> = Arc::new(Mutex::new(
        (task.script)(behaviour, &space)
            .into_iter()
            .map(|mut events| {
                events.push(json!({"type":"response.completed","response":{"usage":{
                    "input_tokens":300 + 100 * index,"output_tokens":50,
                    "input_tokens_details":{"cached_tokens":0}}}}));
                ScriptedResponse::events(&events)
            })
            .collect(),
    ));
    let queue = child_script.clone();
    let objective = (task.objective)(&space);
    let seen = Seen {
        reply_for: Some(Arc::new(move |body: &Value| {
            body["instructions"]
                .as_str()
                .is_some_and(|text| text.contains(CHILD_MARKER))
                .then(|| {
                    queue
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or_else(|| ScriptedResponse::json(500, json!({})))
                })
        })),
        responses: VecDeque::from([ScriptedResponse::events(&[
            call(
                "delegate",
                json!({"child_bot":"child","objective":objective,
                    "selected_context":{"texts":[],"entry_ids":[],"fact_ids":[],"skill_ids":[]}}),
            ),
            json!({"type":"response.completed"}),
        ])]),
        ..Seen::default()
    };
    let server = FakeOpenAi::new(Arc::new(Mutex::new(seen)));
    let endpoints = server.endpoints();
    assert!(
        [
            &endpoints.authorize,
            &endpoints.token,
            &endpoints.revoke,
            &endpoints.jwks,
            &endpoints.issuer,
            &endpoints.api
        ]
        .iter()
        .all(|endpoint| endpoint.starts_with(LOOPBACK)),
        "Point d’accès hors boucle locale : essai arrêté."
    );
    let provider = Provider::with_endpoints(&db_path, endpoints, false);
    let mut app = App::open_with(&db_path, provider.clone()).unwrap();
    support::ask_before_acting(&mut app);
    wait("catalogue", || provider.status()["models_ready"] == true);
    post(
        &mut app,
        "/api/bots",
        json!({"id":"child","name":"child","color":"citron","description":"Worker","instructions":""}),
    );
    post(
        &mut app,
        "/api/local/bot",
        json!({"bot_id":"child","files":true}),
    );
    post(&mut app, "/api/local/folders/add", json!({"path":space}));
    let conversation = post(&mut app, "/api/conversations", json!({"title":task.id}))
        ["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"m","body":"Délègue cette tâche."}),
    );

    let db = Connection::open(&db_path).unwrap();
    db.busy_timeout(Duration::from_secs(3)).unwrap();
    let trigger = Trigger::UserMessage {
        message_id: format!("{conversation}/m"),
        attempt: 0,
    };
    let untouched = snapshot(&space);
    let mut handled = HashSet::new();
    let (mut launch_cards, mut child_cards) = (0, 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    let edge = loop {
        app.tick(now_ms());
        if let Some(parent) = store::latest_run_for_trigger(&db, &trigger).unwrap() {
            let edge = graph::descendants(&db, &parent.run_id)
                .unwrap()
                .into_iter()
                .next();
            let child = edge.as_ref().and_then(|edge| edge.child_run_id.clone());
            for (run_id, is_child) in [(Some(parent.run_id.clone()), false), (child, true)] {
                let Some(run_id) = run_id else { continue };
                for request in requests::list_for_run(&db, &run_id).unwrap() {
                    if request.status != RequestStatus::Pending
                        || !handled.insert(request.entry_id.clone())
                    {
                        continue;
                    }
                    let decision = if !is_child {
                        launch_cards += 1;
                        json!("allow_once")
                    } else {
                        child_cards += 1;
                        assert_eq!(
                            snapshot(&space),
                            untouched,
                            "aucun fichier ne change avant la décision"
                        );
                        match behaviour {
                            Behaviour::Denied => json!("deny"),
                            _ => json!("allow_once"),
                        }
                    };
                    post(
                        &mut app,
                        "/api/approvals/resolve",
                        json!({"entry_id":request.entry_id,
                            "submission_id":format!("decision-{}", request.entry_id),
                            "conversation_id":request.route.target_conversation_id,
                            "decision":decision}),
                    );
                }
            }
            if let Some(edge) = edge.filter(|edge| {
                !matches!(
                    edge.status,
                    DelegationStatus::Queued | DelegationStatus::Running
                )
            }) {
                if store::get(&db, &parent.run_id)
                    .unwrap()
                    .unwrap()
                    .status
                    .is_terminal()
                    && provider.status()["titling"] == false
                {
                    break edge;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "délai dépassé : tâche {}",
            task.id
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        child_script.lock().unwrap().is_empty(),
        "toutes les réponses scriptées de l’enfant sont consommées"
    );
    let tokens = store::token_usage(&db, edge.child_run_id.as_deref().unwrap())
        .unwrap()
        .map_or(0, |usage| usage.input_tokens + usage.output_tokens);
    let facts = app.api("GET", "/api/memory?bot_id=child", b"").unwrap()["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| fact["text"].as_str().unwrap().to_owned())
        .collect();
    let observed = Observed {
        status: edge.status,
        space,
        facts,
    };
    Outcome {
        trial: Trial {
            task: task.id,
            completed: matches!(observed.status, DelegationStatus::Done { .. }),
            correct: (task.grade)(&observed),
            tokens,
        },
        observed,
        _dir: dir,
        launch_cards,
        child_cards,
    }
}

fn run_corpus() -> Vec<Outcome> {
    CORPUS
        .iter()
        .flat_map(|task| {
            task.trials
                .into_iter()
                .enumerate()
                .map(|(index, behaviour)| run_trial(task, behaviour, index))
        })
        .collect()
}

fn hand(task: &'static str, completed: bool, correct: bool, tokens: u64) -> Trial {
    Trial {
        task,
        completed,
        correct,
        tokens,
    }
}

#[test]
fn calculator_reports_rate_tokens_per_success_and_noise_on_hand_made_results() {
    let report = report(&[
        hand("tâche", true, true, 1_000),
        hand("tâche", true, false, 1_500),
        hand("tâche", true, true, 2_000),
    ]);
    let printed = report.to_string();
    println!("{printed}");
    assert!((report.success_rate() - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(report.tokens_per_success(), Some(2_250.0));
    assert!(
        printed.contains("taux de réussite : 66,7 % (2/3)"),
        "{printed}"
    );
    assert!(printed.contains("tokens par succès : 2 250"), "{printed}");
    assert!(
        printed.contains("succès par million de tokens : 444,4"),
        "{printed}"
    );
    assert!(
        printed.contains(
            "tâche : réussite 66,7 % ± 57,7 ; tokens 1 500 ± 500 (médiane 1 500, min 1 000, max 2 000, n 3)"
        ),
        "{printed}"
    );
}

#[test]
fn calculator_counts_only_completed_and_correct_and_survives_degenerate_input() {
    for (completed, correct) in [(false, true), (true, false)] {
        let report = report(&[hand("tâche", completed, correct, 1_000)]);
        assert_eq!(
            report.successes, 0,
            "completed={completed} correct={correct}"
        );
        assert!(
            report.to_string().contains("tokens par succès : —"),
            "aucun succès : pas de division par zéro"
        );
        assert_eq!(report.tokens_per_success(), None);
        assert_eq!(report.successes_per_million_tokens(), 0.0);
    }
    let free = report(&[hand("tâche", true, true, 0)]);
    assert_eq!(free.successes_per_million_tokens(), 0.0, "zéro token");
    let empty = report(&[]).to_string();
    assert!(empty.contains("taux de réussite : 0,0 % (0/0)"), "{empty}");
}

#[test]
fn summary_graders_reject_a_neighbouring_value() {
    let observed = |summary: &str| Observed {
        status: DelegationStatus::Done {
            result: summary.into(),
            captures: vec![],
        },
        space: PathBuf::new(),
        facts: vec![],
    };
    let grade = |task: usize, summary: &str| (CORPUS[task].grade)(&observed(summary));
    assert!(grade(0, "17 × 23 = 391."));
    assert!(!grade(0, "17 × 23 = 3910"));
    assert!(!grade(0, "17 × 23 = -391"));
    assert!(!grade(0, "17 × 23 = 391,5"));
    assert!(!grade(0, "17 × 23 = 0.391"));
    assert!(grade(1, "Le code est 7B42."));
    assert!(!grade(1, "Le code est 7B421."));
    assert!(grade(4, "3 fichiers"));
    assert!(!grade(4, "13 fichiers"));
    assert!(!(CORPUS[0].grade)(&Observed {
        status: DelegationStatus::Failed {
            reason: "Terminé : 391.".into()
        },
        space: PathBuf::new(),
        facts: vec![],
    }));
}

#[test]
fn a_task_that_never_calls_complete_task_fails_even_when_its_text_looks_done() {
    let outcome = run_trial(&CORPUS[0], Behaviour::TextOnly, 0);
    let DelegationStatus::Failed { reason } = &outcome.observed.status else {
        panic!(
            "la tâche sans complete_task échoue : {:?}",
            outcome.observed.status
        );
    };
    assert!(
        reason.contains("Terminé : 391."),
        "le texte qui a l’air fini est conservé, mais il ne termine pas la tâche : {reason}"
    );
    assert!(!outcome.trial.completed && !outcome.trial.correct);
    assert_eq!(report(&[outcome.trial]).successes, 0);
}

#[test]
fn a_denied_write_leaves_no_file_and_a_false_claim_of_success_is_graded_incorrect() {
    let outcome = run_trial(&CORPUS[2], Behaviour::Denied, 0);
    assert_eq!(
        outcome.child_cards, 1,
        "la carte d’approbation de l’écriture est levée"
    );
    assert!(
        !outcome.observed.space.join("rapport.txt").exists(),
        "le refus laisse le dossier intact"
    );
    assert!(matches!(
        outcome.observed.status,
        DelegationStatus::Done { .. }
    ));
    assert!(outcome.trial.completed && !outcome.trial.correct);
    assert_eq!(report(&[outcome.trial]).successes, 0);
}

#[test]
fn full_corpus_replays_five_tasks_three_times_with_exact_tokens_and_report() {
    let outcomes = run_corpus();
    for outcome in &outcomes {
        assert_eq!(
            (outcome.launch_cards, outcome.child_cards),
            (1, usize::from(outcome.trial.task == "fichier")),
            "le lancement en fond et l’écriture demandent chacun leur carte : {}",
            outcome.trial.task
        );
    }
    let trials: Vec<Trial> = outcomes.into_iter().map(|outcome| outcome.trial).collect();
    let expected = [
        ("produit", true, true, 350),
        ("produit", true, true, 450),
        ("produit", true, false, 550),
        ("lecture", true, true, 700),
        ("lecture", false, false, 900),
        ("lecture", true, true, 1_100),
        ("fichier", true, true, 700),
        ("fichier", true, false, 900),
        ("fichier", true, true, 1_100),
        ("memoire", true, true, 700),
        ("memoire", true, false, 900),
        ("memoire", true, true, 1_100),
        ("dossier", true, true, 700),
        ("dossier", true, true, 900),
        ("dossier", false, false, 1_100),
    ];
    let actual: Vec<_> = trials
        .iter()
        .map(|trial| (trial.task, trial.completed, trial.correct, trial.tokens))
        .collect();
    assert_eq!(actual, expected);
    let report = report(&trials);
    let printed = report.to_string();
    println!("{printed}");
    assert_eq!((report.successes, report.tokens), (10, 12_150));
    for line in [
        "taux de réussite : 66,7 % (10/15)",
        "tokens par succès : 1 215",
        "succès par million de tokens : 823,0",
        "produit : réussite 66,7 % ± 57,7 ; tokens 450 ± 100 (médiane 450, min 350, max 550, n 3)",
        "lecture : réussite 66,7 % ± 57,7 ; tokens 900 ± 200 (médiane 900, min 700, max 1 100, n 3)",
        "dossier : réussite 66,7 % ± 57,7 ; tokens 900 ± 200 (médiane 900, min 700, max 1 100, n 3)",
    ] {
        assert!(printed.contains(line), "{line}\n{printed}");
    }
}
