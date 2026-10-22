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

pub(crate) fn leader_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("le système fournit de l'aléa");
    let mut token = String::from("ezerbot-shell-");
    for byte in bytes {
        token.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        token.push(char::from(b"0123456789abcdef"[(byte & 0xf) as usize]));
    }
    token
}

fn valid_leader(token: &str) -> bool {
    let Some(hex) = token.strip_prefix("ezerbot-shell-") else {
        return false;
    };
    hex.len() == 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn insert(
    conn: &Connection,
    run_id: &str,
    call_id: &str,
    pgid: u32,
    leader: &str,
    started: &str,
) -> Result<(), ApiError> {
    conn.execute(
        "INSERT INTO wrapper_shell_tasks(run_id, call_id, status, exit_code, pgid, leader, started) VALUES (?1, ?2, 'en cours', NULL, ?3, ?4, ?5)",
        params![run_id, call_id, i64::from(pgid), leader, started],
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

fn redact_outcome(
    db: &Path,
    redactor: &mut redact::SnapshotRedactor,
    value: Value,
    live: bool,
) -> Value {
    match Vault::open_app(&db.with_extension("secrets.json"))
        .and_then(|vault| redactor.redact(&vault, &value, live))
    {
        Ok(value) => value,
        Err(_) => json!({"status":"failed","reason":"Sortie locale indisponible."}),
    }
}

// Replay uses the ToolResult event's entry ID but reads this mutable journal.
// Invalidate its covering summaries in the same transaction as each change;
// the revision also rejects an in-flight summary made from the old result.
fn invalidate_result(conn: &Connection, run_id: &str, call_id: &str) -> Result<(), ApiError> {
    let kind = serde_json::to_value(grokbot_engine::contracts::RunEventKind::ToolResult)
        .expect("event kind serializes");
    let mut statement = conn.prepare(
        "SELECT r.conversation_id, e.seq FROM engine_run_events e
         JOIN engine_runs r USING(run_id)
         WHERE e.run_id=?1 AND json_extract(e.payload,'$.call_id')=?2 AND e.kind=?3",
    )?;
    let entries = statement
        .query_map(params![run_id, call_id, kind.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (conversation, seq) in entries {
        grokbot_engine::compaction::invalidate_covering(
            conn,
            &conversation,
            &format!("{run_id}:{seq}"),
        )?;
    }
    Ok(())
}

pub(crate) fn publish(
    db: &Path,
    run_id: &str,
    call_id: &str,
    snap: &Snap,
    redactor: &mut redact::SnapshotRedactor,
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
        let redacted = redact_outcome(db, redactor, value, snap.status == "en cours");
        let serialized = redacted.to_string();
        if redacted["status"] != "failed" {
            let updated = tx.execute(
                "UPDATE engine_external_tools SET outcome=?3 WHERE run_id=?1 AND call_id=?2 AND json_extract(outcome,'$.output.status')='arrêtée'",
                params![run_id, call_id, serialized],
            )?;
            if updated == 1 {
                if raw != serialized {
                    invalidate_result(&tx, run_id, call_id)?;
                }
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
    let redacted = redact_outcome(db, redactor, value, snap.status == "en cours");
    let serialized = redacted.to_string();
    let terminal = redacted["status"] == "failed";
    let task_status = if terminal { "arrêtée" } else { snap.status };
    let exit_code = if terminal { None } else { snap.exit_code };
    let updated = tx.execute(
        "UPDATE engine_external_tools SET outcome=?3 WHERE run_id=?1 AND call_id=?2 AND json_extract(outcome,'$.output.status')='en cours'",
        params![run_id, call_id, serialized],
    )?;
    if updated == 1 {
        if raw != serialized {
            invalidate_result(&tx, run_id, call_id)?;
        }
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
                        invalidate_result(&tx, &run_id, &call_id)?;
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

pub(crate) fn leader_started(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let fields = proc_stat_fields(pid)?;
        // Field 22 of /proc/pid/stat: start ticks since boot. A reused pid
        // gets a new value. Index 19 is that field once comm is removed.
        fields.get(19).cloned()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "lstart="])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?;
        let text = text.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }
}

#[cfg(target_os = "linux")]
fn proc_stat_fields(pid: u32) -> Option<Vec<String>> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let end = stat.rfind(')')?;
    Some(
        stat[end + 1..]
            .split_whitespace()
            .map(str::to_string)
            .collect(),
    )
}

fn process_group_id(pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        // After comm: state, ppid, pgrp.
        proc_stat_fields(pid)?.get(2)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "pgid="])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout)
            .ok()?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }
}

fn arguments(pid: u32) -> Option<Vec<Vec<u8>>> {
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let args: Vec<Vec<u8>> = raw
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(<[u8]>::to_vec)
            .collect();
        if args.is_empty() {
            None
        } else {
            Some(args)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let output = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-ww", "-o", "args="])
            .env("COLUMNS", "10000")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?;
        let args: Vec<Vec<u8>> = text
            .split_whitespace()
            .map(|arg| arg.as_bytes().to_vec())
            .collect();
        if args.is_empty() {
            None
        } else {
            Some(args)
        }
    }
}

/// The stored pgid is only ours while that pid still leads the group, still
/// carries the watchdog token, and still has the start time we recorded.
fn proven_leader(pgid: u32, leader: &str, started: &str) -> bool {
    if started.is_empty() || !valid_leader(leader) {
        return false;
    }
    if process_group_id(pgid) != Some(pgid) {
        return false;
    }
    if leader_started(pgid).as_deref() != Some(started) {
        return false;
    }
    arguments(pgid).is_some_and(|args| args.iter().any(|arg| arg == leader.as_bytes()))
}

pub(crate) fn startup(conn: &Connection) -> Result<(), ApiError> {
    // The in-memory registry died with the host. A stored pgid may already
    // belong to someone else: kill only a leader we can still prove is ours,
    // then mark every interrupted task Arrêtée.
    let mut statement = conn.prepare(
        "SELECT pgid, leader, started FROM wrapper_shell_tasks WHERE status='en cours' AND pgid IS NOT NULL AND pgid > 0",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (pgid, leader, started) in rows {
        let Some(leader) = leader.filter(|value| !value.is_empty()) else {
            continue;
        };
        let Some(started) = started.filter(|value| !value.is_empty()) else {
            continue;
        };
        if let Ok(pgid) = u32::try_from(pgid) {
            if proven_leader(pgid, &leader, &started) {
                kill_group(pgid);
            }
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
        grokbot_engine::store::migrate(&conn).unwrap();
        grokbot_engine::compaction::migrate(&conn).unwrap();
        Vault::open_app(&file.with_extension("secrets.json")).unwrap();
        (Temp(path), conn)
    }

    #[test]
    fn snapshots_keep_masking_secrets_after_vault_changes() {
        for final_status in ["en cours", "terminé", "arrêtée"] {
            let (temp, conn) = db();
            let file = temp.0.join("workspace.sqlite");
            let vault = Vault::open_app(&file.with_extension("secrets.json")).unwrap();
            vault.set("TOKEN", "", "old-private-token").unwrap();
            let mut redactor = redact::SnapshotRedactor::default();
            conn.execute(
                "INSERT INTO engine_external_tools VALUES ('run','call',?1)",
                [json!({"status":"done","output":{"status":"en cours"}}).to_string()],
            )
            .unwrap();
            conn.execute("INSERT INTO wrapper_shell_tasks(run_id,call_id,status) VALUES ('run','call','en cours')", []).unwrap();
            let mut snap = Snap {
                status: "en cours",
                stdout: format!("old-private-token {}", "ordinary output ".repeat(8)),
                stderr: format!("old-private-token {}", "ordinary error ".repeat(8)),
                truncated: false,
                incomplete: false,
                exit_code: None,
                signal: None,
            };
            for change in 0..3 {
                if change == 1 {
                    vault.set("TOKEN", "", "new-private-token").unwrap();
                    snap.stdout.push_str(" new-private-token");
                    snap.stderr.push_str(" new-private-token");
                } else if change == 2 {
                    vault.remove("TOKEN").unwrap();
                    snap.status = final_status;
                    if final_status == "arrêtée" {
                        mark_stopped(&conn, Some(("run", "call"))).unwrap();
                    }
                }
                assert!(matches!(
                    publish(&file, "run", "call", &snap, &mut redactor).unwrap(),
                    Publish::Updated
                ));
                let raw: String = conn
                    .query_row("SELECT outcome FROM engine_external_tools", [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                assert!(!raw.contains("old-private-token"));
                assert!(!raw.contains("new-private-token"));
                assert!(raw.contains("[...]"));
            }
        }
    }

    #[test]
    fn live_snapshots_withhold_secret_suffixes_and_final_snapshots_flush_safely() {
        let (temp, conn) = db();
        let file = temp.0.join("workspace.sqlite");
        let vault = Vault::open_app(&file.with_extension("secrets.json")).unwrap();
        vault.set("TOKEN", "", "private-token-é🔒123").unwrap();
        let initial = json!({"status":"done","output":{"status":"en cours"}});
        conn.execute(
            "INSERT INTO engine_external_tools VALUES ('run','call',?1)",
            [initial.to_string()],
        )
        .unwrap();
        conn.execute("INSERT INTO wrapper_shell_tasks(run_id,call_id,status) VALUES ('run','call','en cours')", []).unwrap();
        let mut snap = Snap {
            status: "en cours",
            stdout: "ordinary output before private-token-é🔒12".into(),
            stderr: "ordinary error before private-token-é🔒12".into(),
            truncated: false,
            incomplete: false,
            exit_code: None,
            signal: None,
        };
        for finished in [false, true] {
            if finished {
                snap.status = "terminé";
                snap.stdout.push_str("3 fin");
                snap.stderr.push_str("3 fin");
                snap.exit_code = Some(0);
            }
            assert!(matches!(
                publish(
                    &file,
                    "run",
                    "call",
                    &snap,
                    &mut redact::SnapshotRedactor::default()
                )
                .unwrap(),
                Publish::Updated
            ));
            let raw: String = conn
                .query_row("SELECT outcome FROM engine_external_tools", [], |r| {
                    r.get(0)
                })
                .unwrap();
            let value: Value = serde_json::from_str(&raw).unwrap();
            for field in ["stdout", "stderr"] {
                let text = value["output"][field].as_str().unwrap();
                assert!(!text.contains("private-token"), "{field}: {text}");
                assert!(text.starts_with("ordinary"));
                if finished {
                    assert!(text.ends_with("[...] fin"));
                }
            }
        }
    }

    #[test]
    fn changed_task_results_invalidate_covering_summaries_and_their_descendants() {
        for (stored, status) in [
            ("en cours", "terminé"),
            ("en cours", "stop"),
            ("arrêtée", "arrêtée"),
        ] {
            let (temp, conn) = db();
            let file = temp.0.join("workspace.sqlite");
            conn.execute("INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record) VALUES ('run','bot','chat','user','ref',0,'completed','{}')", []).unwrap();
            conn.execute(
                "INSERT INTO engine_run_events VALUES ('run',2,?1,?2)",
                params![
                    serde_json::to_value(grokbot_engine::contracts::RunEventKind::ToolResult)
                        .unwrap()
                        .as_str(),
                    json!({"call_id":"call"}).to_string()
                ],
            )
            .unwrap();
            let initial = json!({"status":"done","output":{"status":stored}});
            conn.execute(
                "INSERT INTO engine_external_tools VALUES ('run','call',?1)",
                [initial.to_string()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO wrapper_shell_tasks(run_id,call_id,status) VALUES ('run','call',?1)",
                [stored],
            )
            .unwrap();
            for (id, parent) in [
                ("covering", None),
                ("child", Some("covering")),
                ("unrelated", None),
            ] {
                let summary = grokbot_engine::contracts::Summary {
                    summary_id: id.into(),
                    conversation_id: "chat".into(),
                    text: "still running".into(),
                    truncation_last_entry_inclusive: "run:2".into(),
                    resume_from_entry_inclusive: "later".into(),
                    previous_summary_id: parent.map(str::to_owned),
                    includes_tool_results: true,
                    strategy: "test".into(),
                };
                conn.execute("INSERT INTO engine_summaries(summary_id,conversation_id,previous_summary_id,record,segment_start_entry_id,coverage_complete) VALUES (?1,'chat',?2,?3,?1,1)", params![id,parent,serde_json::to_string(&summary).unwrap()]).unwrap();
            }
            conn.execute(
                "INSERT INTO engine_summary_entries VALUES ('covering','run:2')",
                [],
            )
            .unwrap();
            let snap = Snap {
                status,
                stdout: "final output".into(),
                stderr: String::new(),
                truncated: false,
                incomplete: false,
                exit_code: Some(7),
                signal: None,
            };
            if status == "stop" {
                mark_stopped(&conn, Some(("run", "call"))).unwrap();
            } else {
                assert!(matches!(
                    publish(
                        &file,
                        "run",
                        "call",
                        &snap,
                        &mut redact::SnapshotRedactor::default()
                    )
                    .unwrap(),
                    Publish::Updated
                ));
            }

            assert!(
                grokbot_engine::compaction::latest_in_segment(&conn, "chat", "covering")
                    .unwrap()
                    .is_none()
            );
            assert!(
                grokbot_engine::compaction::latest_in_segment(&conn, "chat", "child")
                    .unwrap()
                    .is_none()
            );
            assert!(
                grokbot_engine::compaction::latest_in_segment(&conn, "chat", "unrelated")
                    .unwrap()
                    .is_some()
            );
            assert!(grokbot_engine::compaction::revision(&conn, "chat").unwrap() > 0);
        }
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
            &mut redact::SnapshotRedactor::default(),
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
            &mut redact::SnapshotRedactor::default(),
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
