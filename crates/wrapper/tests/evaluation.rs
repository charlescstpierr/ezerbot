//! Fake-only evaluation launcher (hermetic, loopback) and subprocess safety regressions.
//! See docs/evaluation.md. No production entry point or account is used.
mod support;

use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use support::{FakeOpenAi, Seen, CLIENT};

const MARKER: &str = ".ezerbot-evaluation";
const MARKER_CONTENT: &str = "ezerbot-evaluation-v1\n";
const FAKE_ACCESS: &str = "fake-eval-access-DO-NOT-EXPORT";
const FAKE_REFRESH: &str = "fake-eval-refresh-DO-NOT-EXPORT";
type EvalResult<T> = Result<T, &'static str>;

// An explicit numeric allowlist: never serialize scenario/provider/database objects.
#[derive(Serialize)]
struct Scores {
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    model_calls: u64,
    elapsed_ms: u64,
    max_calls: u64,
    /// 1 = plafond atteint (scenario truncated), 0 = complete.
    limit_reached: u64,
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn private_dir(path: &Path) -> EvalResult<()> {
    fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|_| "Impossible de créer un dossier neuf d’évaluation.")
}

fn claim_space(path: &Path) -> EvalResult<PathBuf> {
    let refused = "Dossier refusé : espace d’évaluation marqué et neuf requis.";
    if !path.is_absolute()
        || !fs::symlink_metadata(path)
            .map_err(|_| refused)?
            .file_type()
            .is_dir()
    {
        return Err(refused);
    }
    let path = path.canonicalize().map_err(|_| refused)?;
    let marker = path.join(MARKER);
    if !fs::symlink_metadata(&marker)
        .map_err(|_| refused)?
        .file_type()
        .is_file()
        || fs::read(&marker).map_err(|_| refused)? != MARKER_CONTENT.as_bytes()
        || fs::read_dir(&path)
            .map_err(|_| refused)?
            .any(|entry| entry.map_or(true, |entry| entry.file_name() != MARKER))
    {
        return Err(refused);
    }
    // Atomic one-shot claim: never open/reuse an existing database or auth file.
    let run = path.join("run");
    private_dir(&run)?;
    private_dir(&run.join("home"))?;
    Ok(run)
}

fn scenario(path: &Path) -> EvalResult<Vec<String>> {
    let invalid = "Scénario invalide : liste JSON non vide de messages attendue (1 Mio maximum).";
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| invalid)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid);
    }
    let messages: Vec<String> = serde_json::from_slice(&bytes).map_err(|_| invalid)?;
    if messages.is_empty() || messages.iter().any(|message| message.trim().is_empty()) {
        return Err(invalid);
    }
    Ok(messages)
}

fn post(app: &mut App, path: &str, value: Value) -> EvalResult<Value> {
    app.api("POST", path, value.to_string().as_bytes())
        .map_err(|_| "Échec de l’API locale d’évaluation.")
}

fn wait_for(mut condition: impl FnMut() -> EvalResult<bool>) -> EvalResult<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition()? {
        if Instant::now() >= deadline {
            return Err("Délai d’évaluation dépassé.");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Entry of the subprocess regressions only: inputs come from the environment their launcher sets.
/// The sentinel is set by `launch` alone; the hermetic run below never reads the environment.
fn replay_from_environment() -> EvalResult<(PathBuf, Scores)> {
    // Validate the mandatory budget before touching a space or starting a server.
    let max_calls: usize = std::env::var("EZERBOT_EVAL_MAX_CALLS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .ok_or("EZERBOT_EVAL_MAX_CALLS obligatoire : entier strictement positif.")?;
    let space = std::env::var_os("EZERBOT_EVAL_SPACE")
        .ok_or("EZERBOT_EVAL_SPACE obligatoire : espace d’évaluation marqué requis.")?;
    let source =
        std::env::var_os("EZERBOT_EVAL_SCENARIO").ok_or("EZERBOT_EVAL_SCENARIO obligatoire.")?;
    // This entry runs alone in a subprocess, before any app/provider threads exist.
    replay(Path::new(&space), Path::new(&source), max_calls, true)
}

fn replay(
    space: &Path,
    source: &Path,
    max_calls: usize,
    isolate_home: bool,
) -> EvalResult<(PathBuf, Scores)> {
    let messages = scenario(source)?;
    let run = claim_space(space)?;
    if isolate_home {
        std::env::set_var("HOME", run.join("home"));
        std::env::set_var("CFFIXED_USER_HOME", run.join("home"));
    }
    let db = run.join("app.sqlite");
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Horloge indisponible.")?
        .as_secs()
        + 3600;
    fs::write(
        db.with_extension("chatgpt-auth.json"),
        json!({"host_id":"eval", "client_id":CLIENT, "access_token":FAKE_ACCESS,
            "refresh_token":FAKE_REFRESH, "expires_at":expires,
            "scopes":"chatgpt.tokens.use.direct"})
        .to_string(),
    )
    .map_err(|_| "Impossible d’écrire la session factice.")?;
    let seen = Arc::new(Mutex::new(Seen {
        usage: Some(json!({"input_tokens":100,"output_tokens":20,
            "input_tokens_details":{"cached_tokens":40}})),
        response_limit: Some(max_calls),
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    // Endpoints come ONLY from this in-process loopback fixture, never the environment.
    let endpoints = server.endpoints();
    let loopback = "http://127.0.0.1:";
    if ![
        &endpoints.authorize,
        &endpoints.token,
        &endpoints.revoke,
        &endpoints.jwks,
        &endpoints.issuer,
        &endpoints.api,
    ]
    .iter()
    .all(|endpoint| endpoint.starts_with(loopback))
    {
        return Err("Point d’accès hors boucle locale : évaluation arrêtée.");
    }
    let provider = Provider::with_endpoints(&db, endpoints, false);
    let mut app = App::open_with(&db, provider.clone())
        .map_err(|_| "Impossible d’ouvrir la base dédiée d’évaluation.")?;
    // status() would start an auxiliary connection probe; load only the catalogue.
    provider.refresh_models();
    wait_for(|| Ok(!provider.models().is_empty()))?;
    // Exclude auxiliary model calls, which engine_response_usage does not measure.
    post(
        &mut app,
        "/api/memory/automatic",
        json!({"bot_id":"grokbot","enabled":false}),
    )?;
    // Keep this bounded synthetic scenario below the automatic compaction threshold.
    post(
        &mut app,
        "/api/engine/config",
        json!({"estimated_tokens":2_000_000}),
    )?;
    let conversation = post(
        &mut app,
        "/api/conversations",
        json!({"title":"Évaluation factice"}),
    )?["conversation"]["id"]
        .as_str()
        .ok_or("Conversation d’évaluation indisponible.")?
        .to_owned();
    let conn = Connection::open(&db).map_err(|_| "Base de mesures indisponible.")?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|_| "Base de mesures indisponible.")?;
    for (index, message) in messages.iter().take(max_calls).enumerate() {
        post(
            &mut app,
            "/api/send",
            json!({"conversation_id":conversation,
                "message_id":format!("eval-{index}"),"body":message}),
        )?;
        wait_for(|| {
            let (finished, failed): (usize, usize) = conn
                .query_row(
                    "SELECT COUNT(*) FILTER (WHERE status='finished'),
                        COUNT(*) FILTER (WHERE status='failed') FROM engine_runs",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| "Mesures d’exécution indisponibles.")?;
            if failed != 0 {
                return Err("Réponse factice en échec : évaluation arrêtée.");
            }
            Ok(finished == index + 1)
        })?;
    }
    let mut scores = conn
        .query_row(
            "SELECT COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(cached_input_tokens),0) FROM engine_response_usage",
            [],
            |row| {
                Ok(Scores {
                    input_tokens: row.get(0)?,
                    output_tokens: row.get(1)?,
                    cached_input_tokens: row.get(2)?,
                    model_calls: 0,
                    elapsed_ms: 0,
                    max_calls: max_calls as u64,
                    limit_reached: u64::from(messages.len() > max_calls),
                })
            },
        )
        .map_err(|_| "Mesures de tokens indisponibles.")?;
    (scores.model_calls, scores.elapsed_ms) = conn
        .query_row(
            "SELECT COALESCE(SUM(json_extract(usage,'$.model_calls')),0),
                COALESCE(SUM(json_extract(usage,'$.elapsed_ms')),0) FROM engine_runs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "Mesures d’exécution indisponibles.")?;
    let observed = seen.lock().map_err(|_| "Compteur factice indisponible.")?;
    if observed.rejected_response_calls != 0
        || observed.response_calls != scores.model_calls as usize
        || observed.response_calls != messages.len().min(max_calls)
    {
        return Err("Appels inattendus : évaluation arrêtée.");
    }
    drop(observed);
    let output_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/eval");
    fs::create_dir_all(&output_dir).map_err(|_| "Dossier de résultats indisponible.")?;
    let output = output_dir.join(format!("fake-{}-{}.json", timestamp(), std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .map_err(|_| "Impossible de créer les résultats.")?;
    file.write_all(&serde_json::to_vec_pretty(&scores).map_err(|_| "Résultats invalides.")?)
        .map_err(|_| "Impossible d’écrire les résultats.")?;
    Ok((output, scores))
}

/// Set by `launch` only, to run the subprocess regressions through this same test.
const SUBPROCESS: &str = "EZERBOT_EVAL_SUBPROCESS";

/// Hermetic by default: a fresh marked space, the repository scenario and a fixed call cap are
/// prepared here, against the in-process loopback fake server. Under the subprocess sentinel the
/// same test runs the launcher on the inputs its parent regression prepared.
#[test]
fn replay_fake_scenario() {
    let subprocess = std::env::var_os(SUBPROCESS).is_some();
    let space = (!subprocess).then(|| test_space(true));
    let outcome = match &space {
        Some(space) => replay(space, &fixture(), HERMETIC_MAX_CALLS, false),
        None => replay_from_environment(),
    };
    match outcome {
        Ok((path, scores)) => {
            println!("EVAL_RESULT={}", path.display());
            // The hermetic run owns its numeric result file; a subprocess one is read by its parent.
            let _result = space.as_ref().map(|_| TestFile(path));
            assert_eq!(
                scores.limit_reached, 0,
                "Plafond atteint : évaluation arrêtée."
            );
        }
        Err(message) => panic!("{message}"), // Static messages only; no raw errors or input.
    }
}

/// Call cap fixed by the hermetic run: exactly the three messages of the repository scenario.
const HERMETIC_MAX_CALLS: usize = 3;

/// A result file removed on drop, even when an assertion fails first.
struct TestFile(PathBuf);

impl Drop for TestFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A synthetic space owned by one regression; removed with its sibling fixtures.
struct TestSpace(PathBuf);

impl std::ops::Deref for TestSpace {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestSpace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
        for sidecar in ["link", "marker", "json"] {
            let _ = fs::remove_file(self.0.with_extension(sidecar));
        }
    }
}

fn test_space(marked: bool) -> TestSpace {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "ezerbot-eval-test-{}-{}-{}",
        std::process::id(),
        timestamp(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    private_dir(&path).unwrap();
    if marked {
        fs::write(path.join(MARKER), MARKER_CONTENT).unwrap();
    }
    TestSpace(path)
}

fn launch(space: &Path, cap: Option<&str>, source: &Path) -> Output {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "replay_fake_scenario", "--nocapture"])
        .env(SUBPROCESS, "1")
        .env("EZERBOT_EVAL_SPACE", space)
        .env("EZERBOT_EVAL_SCENARIO", source)
        // A runtime fake-server override cannot redirect this launcher.
        .env("GROKBOT_FAKE_OPENAI", "http://127.0.0.1:1")
        .env_remove("EZERBOT_EVAL_MAX_CALLS");
    if let Some(cap) = cap {
        command.env("EZERBOT_EVAL_MAX_CALLS", cap);
    }
    command.output().unwrap()
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval-three-messages.json")
}

fn report(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let path = stdout
        .lines()
        .find_map(|line| line.strip_prefix("EVAL_RESULT="))
        .expect("numeric results written");
    let bytes = fs::read(path).unwrap();
    fs::remove_file(path).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value.as_object().unwrap().values().all(Value::is_number));
    for trap in [
        "eval-private-message-ONE",
        "eval-private-message-TWO",
        "eval-private-message-THREE",
        FAKE_ACCESS,
        FAKE_REFRESH,
    ] {
        assert!(!String::from_utf8_lossy(&bytes).contains(trap));
        assert!(!stdout.contains(trap));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(trap));
    }
    value
}

#[test]
fn three_messages_produce_numeric_private_database_scores() {
    let space = test_space(true);
    let output = launch(&space, Some("3"), &fixture());
    assert!(output.status.success(), "evaluation failed");
    let value = report(&output);
    assert_eq!(value["input_tokens"], 300);
    assert_eq!(value["output_tokens"], 60);
    assert_eq!(value["cached_input_tokens"], 120);
    assert_eq!(value["model_calls"], 3);
    assert_eq!(value["max_calls"], 3);
    assert_eq!(value["limit_reached"], 0);
    assert!(value["elapsed_ms"].is_u64());
    // A marked space cannot be reused, even with a valid previous run.
    assert!(!launch(&space, Some("3"), &fixture()).status.success());
}

#[test]
fn unmarked_or_nonfresh_spaces_are_refused_before_database_creation() {
    for marked in [false, true] {
        let space = test_space(marked);
        if marked {
            fs::write(space.join("app.sqlite"), b"existing-data-canary").unwrap();
        }
        let output = launch(&space, Some("3"), &fixture());
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("espace d’évaluation"));
        assert!(!space.join("run").exists());
        if marked {
            assert_eq!(
                fs::read(space.join("app.sqlite")).unwrap(),
                b"existing-data-canary"
            );
        }
    }
}

#[test]
fn symlinked_space_or_marker_is_refused() {
    use std::os::unix::fs::symlink;
    let space = test_space(true);
    let alias = space.with_extension("link");
    symlink(&*space, &alias).unwrap();
    let output = launch(&alias, Some("3"), &fixture());
    assert!(!output.status.success());
    assert!(!space.join("run").exists());

    let space = test_space(false);
    let marker = space.with_extension("marker");
    fs::write(&marker, MARKER_CONTENT).unwrap();
    symlink(&marker, space.join(MARKER)).unwrap();
    let output = launch(&space, Some("3"), &fixture());
    assert!(!output.status.success());
    assert!(!space.join("run").exists());
}

#[test]
fn missing_or_invalid_cap_refuses_without_claiming_space() {
    for cap in [None, Some("0"), Some("-1"), Some("not-a-number")] {
        let space = test_space(true);
        let output = launch(&space, cap, &fixture());
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("EZERBOT_EVAL_MAX_CALLS"));
        assert!(!space.join("run").exists());
    }
}

#[test]
fn cap_two_stops_before_the_third_message_and_writes_numeric_limit_flag() {
    let space = test_space(true);
    let output = launch(&space, Some("2"), &fixture());
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Plafond atteint"));
    let value = report(&output);
    assert_eq!(value["model_calls"], 2);
    assert_eq!(value["input_tokens"], 200);
    assert_eq!(value["cached_input_tokens"], 80);
    assert_eq!(value["limit_reached"], 1);
    let conn = Connection::open(space.join("run/app.sqlite")).unwrap();
    let count: u64 = conn
        .query_row("SELECT COUNT(*) FROM engine_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn malformed_scenario_errors_do_not_echo_input() {
    let space = test_space(true);
    let source = space.with_extension("json");
    fs::write(
        &source,
        br#"["eval-private-message-ONE", {"invalid":"eval-private-message-TWO"}]"#,
    )
    .unwrap();
    let output = launch(&space, Some("3"), &source);
    assert!(!output.status.success());
    assert!(!space.join("run").exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("eval-private-message"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("eval-private-message"));
}

#[test]
fn fake_server_enforces_budget_even_for_auxiliary_requests() {
    let seen = Arc::new(Mutex::new(Seen {
        response_limit: Some(2),
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    for _ in 0..2 {
        ureq::post(&format!("{}/v1/responses", server.base))
            .send_json(json!({"instructions":"titre"}))
            .unwrap();
    }
    let result = ureq::post(&format!("{}/v1/responses", server.base))
        .send_json(json!({"instructions":"Test de connexion."}));
    assert!(matches!(result, Err(ureq::Error::Status(429, _))));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.response_calls, 2);
    assert_eq!(seen.rejected_response_calls, 1);
    assert!(seen.probe_bodies.is_empty());
}
