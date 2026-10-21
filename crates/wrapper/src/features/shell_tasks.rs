//! Background `run_shell` tasks. The process group lives outside the model turn.
//! Quitting or killing the host does not relaunch a task: startup only marks
//! whatever was still "en cours" as Arrêtée.
use super::write_transaction;
use crate::ApiError;
use grokbot_forms::{redact, vault::Vault};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

#[derive(Default)]
pub(crate) struct Piece {
    pub text: String,
    pub truncated: bool,
    pub incomplete: bool,
}

pub(crate) struct Snap {
    pub status: &'static str,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub incomplete: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

pub(crate) enum Publish {
    Pending,
    Updated,
    Closed,
    /// The vault replaced the live outcome. The worker must kill the group.
    Terminal,
}

pub(crate) struct TaskControl {
    pub stop: Arc<AtomicBool>,
    pub done: Arc<AtomicBool>,
}

pub(crate) struct DoneFlag(pub Arc<AtomicBool>);
impl Drop for DoneFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Live {
    db: PathBuf,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    pgid: u32,
}

fn registry() -> &'static Mutex<HashMap<String, Live>> {
    static LIVE: OnceLock<Mutex<HashMap<String, Live>>> = OnceLock::new();
    LIVE.get_or_init(Mutex::default)
}

fn key(db: &Path, run_id: &str, call_id: &str) -> String {
    format!("{}\n{run_id}\n{call_id}", db.display())
}

pub(crate) fn insert(
    conn: &Connection,
    run_id: &str,
    call_id: &str,
    pgid: u32,
) -> Result<(), ApiError> {
    conn.execute(
        "INSERT INTO wrapper_shell_tasks(run_id, call_id, status, exit_code, pgid) VALUES (?1, ?2, 'en cours', NULL, ?3)",
        params![run_id, call_id, i64::from(pgid)],
    )?;
    Ok(())
}

pub(crate) fn register(db: &Path, run_id: &str, call_id: &str, pgid: u32) -> TaskControl {
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let mut tasks = registry().lock().unwrap_or_else(|error| error.into_inner());
    tasks.insert(
        key(db, run_id, call_id),
        Live {
            db: db.to_path_buf(),
            stop: stop.clone(),
            done: done.clone(),
            pgid,
        },
    );
    TaskControl { stop, done }
}

pub(crate) fn finish(db: &Path, run_id: &str, call_id: &str) {
    let mut tasks = registry().lock().unwrap_or_else(|error| error.into_inner());
    tasks.remove(&key(db, run_id, call_id));
}

pub(crate) fn kill_group(pgid: u32) {
    if pgid == 0 {
        return;
    }
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn redact_outcome(db: &Path, value: Value) -> Value {
    match Vault::open_app(&db.with_extension("secrets.json"))
        .and_then(|vault| redact::redact_value(&vault, &value))
    {
        Ok(value) => value,
        Err(_) => json!({"status":"failed","reason":"Sortie locale indisponible."}),
    }
}

pub(crate) fn publish(
    db: &Path,
    run_id: &str,
    call_id: &str,
    snap: &Snap,
) -> Result<Publish, ApiError> {
    let conn = Connection::open(db)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let tx = write_transaction(&conn)?;
    // A missing row and a SQL NULL are the same window: the admission exists,
    // but the initial result is not durable yet.
    let raw: Option<Option<String>> = tx
        .query_row(
            "SELECT outcome FROM engine_external_tools WHERE run_id=?1 AND call_id=?2",
            params![run_id, call_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(raw) = raw.flatten() else {
        tx.commit()?;
        return Ok(Publish::Pending);
    };
    let mut value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    if value["output"]["status"] == "arrêtée" && snap.status == "arrêtée" {
        value["output"]["stdout"] = json!(snap.stdout);
        value["output"]["stderr"] = json!(snap.stderr);
        value["output"]["truncated"] = json!(snap.truncated);
        value["output"]["output_incomplete"] = json!(snap.incomplete);
        value["output"]["exit_code"] = Value::Null;
        value["output"]["signal"] = Value::Null;
        value["output"]["background"] = json!(true);
        let redacted = redact_outcome(db, value);
        if redacted["status"] != "failed" {
            let updated = tx.execute(
                "UPDATE engine_external_tools SET outcome=?3 WHERE run_id=?1 AND call_id=?2 AND json_extract(outcome,'$.output.status')='arrêtée'",
                params![run_id, call_id, redacted.to_string()],
            )?;
            if updated == 1 {
                tx.execute(
                    "UPDATE wrapper_shell_tasks SET status='arrêtée', exit_code=NULL WHERE run_id=?1 AND call_id=?2",
                    params![run_id, call_id],
                )?;
                tx.commit()?;
                return Ok(Publish::Updated);
            }
        }
        tx.commit()?;
        return Ok(Publish::Closed);
    }
    if value["output"]["status"] != "en cours" {
        let status = value["output"]["status"].as_str().unwrap_or("arrêtée");
        tx.execute(
            "UPDATE wrapper_shell_tasks SET status=?3 WHERE run_id=?1 AND call_id=?2 AND status='en cours'",
            params![run_id, call_id, status],
        )?;
        tx.commit()?;
        return Ok(Publish::Closed);
    }
    value["output"]["status"] = json!(snap.status);
    value["output"]["stdout"] = json!(snap.stdout);
    value["output"]["stderr"] = json!(snap.stderr);
    value["output"]["truncated"] = json!(snap.truncated);
    value["output"]["output_incomplete"] = json!(snap.incomplete);
    value["output"]["exit_code"] = snap.exit_code.map_or(Value::Null, |code| json!(code));
    value["output"]["signal"] = snap.signal.map_or(Value::Null, |signal| json!(signal));
    value["output"]["background"] = json!(true);
    let redacted = redact_outcome(db, value);
    let terminal = redacted["status"] == "failed";
    let task_status = if terminal { "arrêtée" } else { snap.status };
    let exit_code = if terminal { None } else { snap.exit_code };
    let updated = tx.execute(
        "UPDATE engine_external_tools SET outcome=?3 WHERE run_id=?1 AND call_id=?2 AND json_extract(outcome,'$.output.status')='en cours'",
        params![run_id, call_id, redacted.to_string()],
    )?;
    if updated == 1 {
        tx.execute(
            "UPDATE wrapper_shell_tasks SET status=?3, exit_code=?4 WHERE run_id=?1 AND call_id=?2 AND status='en cours'",
            params![run_id, call_id, task_status, exit_code],
        )?;
    }
    tx.commit()?;
    if terminal && updated == 1 {
        return Ok(Publish::Terminal);
    }
    Ok(if updated == 1 {
        Publish::Updated
    } else {
        Publish::Closed
    })
}

fn interrupted_outcome() -> Value {
    json!({
        "status": "done",
        "output": {
            "status": "arrêtée",
            "exit_code": Value::Null,
            "signal": Value::Null,
            "stdout": "",
            "stderr": "",
            "truncated": false,
            "output_incomplete": false,
            "background": true
        }
    })
}

fn mark_stopped(conn: &Connection, only: Option<(&str, &str)>) -> Result<(), ApiError> {
    let tx = write_transaction(conn)?;
    let mut statement =
        tx.prepare("SELECT run_id, call_id FROM wrapper_shell_tasks WHERE status='en cours'")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (run_id, call_id) in rows {
        if only.is_some_and(|(run, call)| run != run_id || call != call_id) {
            continue;
        }
        let raw: Option<Option<String>> = tx
            .query_row(
                "SELECT outcome FROM engine_external_tools WHERE run_id=?1 AND call_id=?2",
                params![run_id, call_id],
                |row| row.get(0),
            )
            .optional()?;
        let mut stored = "arrêtée".to_string();
        let mut clear_code = true;
        match raw {
            Some(None) => {
                tx.execute(
                    "UPDATE engine_external_tools SET outcome=?3 WHERE run_id=?1 AND call_id=?2 AND outcome IS NULL",
                    params![run_id, call_id, interrupted_outcome().to_string()],
                )?;
            }
            Some(Some(raw)) => {
                if let Ok(mut value) = serde_json::from_str::<Value>(&raw) {
                    if value["output"]["status"] == "en cours" {
                        value["output"]["status"] = json!("arrêtée");
                        value["output"]["exit_code"] = Value::Null;
                        value["output"]["signal"] = Value::Null;
                        tx.execute(
                            "UPDATE engine_external_tools SET outcome=?3 WHERE run_id=?1 AND call_id=?2 AND json_extract(outcome,'$.output.status')='en cours'",
                            params![run_id, call_id, value.to_string()],
                        )?;
                    } else if let Some(status) = value["output"]["status"].as_str() {
                        stored = match status {
                            "terminé" | "échec" | "arrêtée" => status.to_string(),
                            _ => "arrêtée".to_string(),
                        };
                        clear_code = stored == "arrêtée";
                    }
                }
            }
            None => {}
        }
        if clear_code {
            tx.execute(
                "UPDATE wrapper_shell_tasks SET status=?3, exit_code=NULL WHERE run_id=?1 AND call_id=?2 AND status='en cours'",
                params![run_id, call_id, stored],
            )?;
        } else {
            tx.execute(
                "UPDATE wrapper_shell_tasks SET status=?3 WHERE run_id=?1 AND call_id=?2 AND status='en cours'",
                params![run_id, call_id, stored],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn startup(conn: &Connection) -> Result<(), ApiError> {
    // The in-memory registry died with the host. Kill groups that were still
    // starting, then mark those tasks interrupted. A NULL journal outcome is
    // that crash window, not a running card.
    let mut statement = conn.prepare(
        "SELECT pgid FROM wrapper_shell_tasks WHERE status='en cours' AND pgid IS NOT NULL AND pgid > 0",
    )?;
    let pgids = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for pgid in pgids {
        if let Ok(pgid) = u32::try_from(pgid) {
            kill_group(pgid);
        }
    }
    mark_stopped(conn, None)
}

pub(crate) fn shutdown(conn: &Connection, db: &Path) {
    let mine = {
        let mut tasks = registry().lock().unwrap_or_else(|error| error.into_inner());
        let keys: Vec<_> = tasks
            .iter()
            .filter(|(_, task)| task.db == db)
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter()
            .filter_map(|key| tasks.remove(&key))
            .collect::<Vec<_>>()
    };
    for task in &mine {
        task.stop.store(true, Ordering::SeqCst);
        kill_group(task.pgid);
    }
    let start = std::time::Instant::now();
    while mine.iter().any(|task| !task.done.load(Ordering::SeqCst))
        && start.elapsed() < Duration::from_secs(2)
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    if let Err(error) = mark_stopped(conn, None) {
        eprintln!("Arrêt des tâches de fond incomplet : {}", error.message);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopTask {
    run_id: String,
    call_id: String,
}

pub(crate) fn stop(conn: &Connection, db: &Path, body: &[u8]) -> Result<Value, ApiError> {
    let input: StopTask = crate::parse(body)?;
    crate::validate_id(&input.run_id)?;
    crate::validate_id(&input.call_id)?;
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM wrapper_shell_tasks WHERE run_id=?1 AND call_id=?2",
            params![input.run_id, input.call_id],
            |row| row.get(0),
        )
        .optional()?;
    if status.as_deref() != Some("en cours") {
        return Err(ApiError::new(409, "Cette tâche n’est plus en cours."));
    }
    let live = registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key(db, &input.run_id, &input.call_id))
        .map(|task| (task.stop.clone(), task.pgid));
    if let Some((stop, pgid)) = live {
        stop.store(true, Ordering::SeqCst);
        kill_group(pgid);
    }
    mark_stopped(conn, Some((&input.run_id, &input.call_id)))?;
    Ok(json!({"status":"arrêtée"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokbot_forms::vault::Vault;
    use std::path::PathBuf;

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn db() -> (Temp, Connection) {
        let path = std::env::temp_dir().join(format!(
            "ezerbot-shell-task-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let file = path.join("workspace.sqlite");
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch(
            "CREATE TABLE engine_external_tools(run_id TEXT NOT NULL, call_id TEXT NOT NULL, outcome TEXT, PRIMARY KEY(run_id, call_id));
             CREATE TABLE wrapper_shell_tasks(run_id TEXT NOT NULL, call_id TEXT NOT NULL, status TEXT NOT NULL, exit_code INTEGER, pgid INTEGER, PRIMARY KEY(run_id, call_id));",
        )
        .unwrap();
        Vault::open_app(&file.with_extension("secrets.json")).unwrap();
        (Temp(path), conn)
    }

    #[test]
    fn stopped_publish_keeps_the_drained_output_and_a_null_exit_code() {
        let (temp, conn) = db();
        let file = temp.0.join("workspace.sqlite");
        let initial = json!({
            "status": "done",
            "output": {
                "status": "arrêtée",
                "stdout": "ancien",
                "stderr": "",
                "truncated": false,
                "output_incomplete": false,
                "exit_code": null,
                "signal": null,
                "background": true
            }
        });
        conn.execute(
            "INSERT INTO engine_external_tools(run_id,call_id,outcome) VALUES ('run','call',?1)",
            [initial.to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO wrapper_shell_tasks(run_id,call_id,status,exit_code) VALUES ('run','call','arrêtée',NULL)",
            [],
        )
        .unwrap();
        let result = publish(
            &file,
            "run",
            "call",
            &Snap {
                status: "arrêtée",
                stdout: "queue complète".into(),
                stderr: "err".into(),
                truncated: true,
                incomplete: false,
                exit_code: Some(9),
                signal: Some(15),
            },
        )
        .unwrap();
        assert!(matches!(result, Publish::Updated));
        let raw: String = conn
            .query_row(
                "SELECT outcome FROM engine_external_tools WHERE call_id='call'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["output"]["status"], "arrêtée");
        assert_eq!(value["output"]["stdout"], "queue complète");
        assert_eq!(value["output"]["stderr"], "err");
        assert_eq!(value["output"]["truncated"], true);
        assert!(value["output"]["exit_code"].is_null());
        assert!(value["output"]["signal"].is_null());
        let exit_code: Option<i64> = conn
            .query_row(
                "SELECT exit_code FROM wrapper_shell_tasks WHERE call_id='call'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exit_code.is_none());
        let late = publish(
            &file,
            "run",
            "call",
            &Snap {
                status: "en cours",
                stdout: "trop tard".into(),
                stderr: String::new(),
                truncated: false,
                incomplete: false,
                exit_code: None,
                signal: None,
            },
        )
        .unwrap();
        assert!(matches!(late, Publish::Closed));
        let raw: String = conn
            .query_row(
                "SELECT outcome FROM engine_external_tools WHERE call_id='call'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["output"]["stdout"], "queue complète");
        assert_eq!(value["output"]["status"], "arrêtée");
    }
}
