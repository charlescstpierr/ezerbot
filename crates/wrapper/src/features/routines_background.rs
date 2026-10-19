//! Local reconstruction: CR20 establishes no shipped OS scheduler. launchd wakes
//! a cheap read-only probe each minute; the engine is opened ONLY for due work.
//! No resident daemon. The binary holds Owner across startup, workers and exit,
//! including the foreground app (startup recovery must never interrupt its peer).
use super::Context;
use crate::{ApiError, ApiResponse, App};
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};
use grokbot_scheduler::{cron, routine};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

empty_hooks!(tools, context, entry, on_run_event, link_attachments, tick);
const LABEL: &str = "local.ezerbot.routines";
pub(crate) const ACTION: &str = "routines_background_configure";

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_routines_background (
        id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL DEFAULT 0,
        installed INTEGER NOT NULL DEFAULT 0, last_run INTEGER, error TEXT,
        outcome TEXT, database_path TEXT);
        INSERT OR IGNORE INTO wrapper_routines_background(id) VALUES (1);",
    )?;
    Ok(())
}
fn fail(message: impl Into<String>) -> ApiError {
    ApiError::new(500, message)
}
fn io(error: std::io::Error) -> ApiError {
    fail(format!("Routines en arrière-plan : {error}"))
}

/// OS lock, not a lease: process death releases it without a stale-PID race.
/// Never unlink the lock file (an old inode would permit two owners).
pub struct Owner {
    _file: File,
}
fn canonical(db: &Path) -> Result<PathBuf, ApiError> {
    Ok(if db.exists() {
        fs::canonicalize(db).map_err(io)?
    } else {
        let parent = db
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(io)?;
        fs::canonicalize(parent).map_err(io)?.join(
            db.file_name()
                .ok_or_else(|| fail("Chemin SQLite invalide."))?,
        )
    })
}
fn try_lock(db: &Path, extension: &str) -> Result<Option<File>, ApiError> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(canonical(db)?.with_extension(extension))
        .map_err(io)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(io(error)),
    }
}
/// A second OS lock held by a foreground opener while it waits for the owner.
/// A headless pass that sees it hands the engine over instead of finishing.
fn foreground_waiting(db: &Path) -> bool {
    matches!(try_lock(db, "engine-wanted"), Ok(None))
}
impl Owner {
    pub fn try_acquire(db: &Path) -> Result<Option<Self>, ApiError> {
        Ok(try_lock(db, "engine-owner")?.map(|file| Self { _file: file }))
    }
    pub fn foreground(db: &Path) -> Result<Self, ApiError> {
        let start = Instant::now();
        let mut wanted = None;
        loop {
            if wanted.is_none() {
                wanted = try_lock(db, "engine-wanted")?;
            }
            if let Some(owner) = Self::try_acquire(db)? {
                return Ok(owner);
            }
            if start.elapsed() > Duration::from_secs(210) {
                return Err(fail(
                    "Le moteur est déjà utilisé. Réessayez après la fin de la routine.",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

struct Agent {
    home: PathBuf,
    plist: PathBuf,
    recorder: Option<PathBuf>,
}
impl Agent {
    fn environment() -> Result<Self, ApiError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| fail("Dossier personnel introuvable."))?;
        #[cfg(debug_assertions)]
        {
            // Debug is physically incapable of using launchctl. The override
            // must name a real throwaway HOME below the OS temporary directory.
            let root = std::env::var_os("GROKBOT_LAUNCH_AGENT_TEST_ROOT").map(PathBuf::from)
                .ok_or_else(|| fail("Version de développement : installation système désactivée. Utilisez un HOME de test isolé."))?;
            let root = fs::canonicalize(root).map_err(io)?;
            let actual = fs::canonicalize(&home).map_err(io)?;
            let fixed = std::env::var_os("CFFIXED_USER_HOME")
                .map(PathBuf::from)
                .and_then(|p| fs::canonicalize(p).ok());
            let temporary = fs::canonicalize(std::env::temp_dir()).map_err(io)?;
            let private_tmp = fs::canonicalize("/tmp").unwrap_or(temporary.clone());
            if root != actual
                || fixed.as_ref() != Some(&root)
                || !(root.starts_with(&temporary) || root.starts_with(&private_tmp))
                || root == temporary
                || root == private_tmp
            {
                return Err(fail(
                    "L’agent de test exige un HOME temporaire isolé identique à CFFIXED_USER_HOME.",
                ));
            }
            Ok(Self::at(
                root.clone(),
                Some(root.join("launchctl-recording.jsonl")),
            ))
        }
        #[cfg(not(debug_assertions))]
        {
            if !cfg!(target_os = "macos") {
                return Err(fail("Les routines en arrière-plan nécessitent macOS."));
            }
            Ok(Self::at(home, None))
        }
    }
    fn at(home: PathBuf, recorder: Option<PathBuf>) -> Self {
        let plist = home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        Self {
            home,
            plist,
            recorder,
        }
    }
    fn owns(&self, db: &Path, previous: Option<&str>) -> Result<bool, ApiError> {
        let output = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(&self.plist)
            .output()
            .map_err(io)?;
        if !output.status.success() {
            return Err(fail(
                "Le fichier de l’agent macOS est illisible. Aucun fichier n’a été modifié.",
            ));
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| fail("Le fichier de l’agent macOS est invalide."))?;
        let arguments = &value["ProgramArguments"];
        let same_path = |value: &Value, path: &Path| {
            value.as_str().is_some_and(|s| {
                Path::new(s) == path || fs::canonicalize(s).is_ok_and(|p| p == path)
            })
        };
        Ok(value["Label"] == LABEL
            && arguments[1] == "--routine-wake"
            && arguments[2] == "--db"
            && (same_path(&arguments[3], db)
                || previous.is_some_and(|p| arguments[3].as_str() == Some(p)))
            && same_path(&value["EnvironmentVariables"]["HOME"], &self.home)
            && same_path(
                &value["EnvironmentVariables"]["CFFIXED_USER_HOME"],
                &self.home,
            ))
    }
    fn domain(&self) -> Result<String, ApiError> {
        if self.recorder.is_some() {
            return Ok("gui/test".into());
        }
        #[cfg(not(debug_assertions))]
        {
            let output = Command::new("/usr/bin/id").arg("-u").output().map_err(io)?;
            let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !output.status.success()
                || uid.is_empty()
                || !uid.bytes().all(|c| c.is_ascii_digit())
            {
                return Err(fail("Session macOS introuvable."));
            }
            Ok(format!("gui/{uid}"))
        }
        #[cfg(debug_assertions)]
        Err(fail("launchctl est désactivé en développement."))
    }
    fn control(&self, args: &[String], allow_absent: bool) -> Result<(), ApiError> {
        if let Some(path) = &self.recorder {
            use std::io::Write;
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(io)?;
            writeln!(file, "{}", json!({"command":"launchctl","arguments":args})).map_err(io)?;
            if self.home.join("launchctl-fail").exists() {
                return Err(fail("Échec simulé de launchctl. Réessayez."));
            }
            return Ok(());
        }
        #[cfg(not(debug_assertions))]
        {
            let output = Command::new("/bin/launchctl")
                .args(args)
                .output()
                .map_err(io)?;
            // launchctl returns 3/113 for a job which is no longer registered.
            if output.status.success()
                || (allow_absent && matches!(output.status.code(), Some(3 | 113)))
            {
                return Ok(());
            }
            Err(fail("macOS n’a pas pu configurer l’agent. Vérifiez Réglages Système › Général › Ouverture et extensions, puis réessayez."))
        }
        #[cfg(debug_assertions)]
        {
            let _ = allow_absent;
            Err(fail("launchctl est désactivé en développement."))
        }
    }
}
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn plist(binary: &Path, db: &Path, home: &Path) -> String {
    let strings = [
        binary.to_string_lossy().into_owned(),
        "--routine-wake".into(),
        "--db".into(),
        db.to_string_lossy().into_owned(),
    ]
    .iter()
    .map(|s| format!("<string>{}</string>", xml(s)))
    .collect::<String>();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array>{strings}</array><key>StartInterval</key><integer>60</integer><key>RunAtLoad</key><true/><key>ProcessType</key><string>Background</string><key>EnvironmentVariables</key><dict><key>HOME</key><string>{}</string><key>CFFIXED_USER_HOME</key><string>{}</string></dict></dict></plist>\n", xml(&home.to_string_lossy()), xml(&home.to_string_lossy()))
}
fn atomic(path: &Path, body: &[u8]) -> Result<(), ApiError> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| fail("Chemin de l’agent invalide."))?;
    fs::create_dir_all(parent).map_err(io)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let result = (|| {
        let mut file = options.open(&temp).map_err(io)?;
        file.write_all(body).map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&temp, path).map_err(io)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Setting {
    enabled: bool,
}
pub fn host_tools() -> Vec<ToolSpec> {
    vec![ToolSpec { name: ACTION.into(), description:"Configurer l’exécution des routines quand Ezerbot est fermé. Installe ou retire un agent macOS pour cet utilisateur uniquement.".into(), schema:json!({"type":"object","properties":{"enabled":{"type":"boolean"}},"required":["enabled"],"additionalProperties":false}), requires_approval:true }]
}
pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    bytes: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if path != "/api/routines/background" {
        return None;
    }
    Some((|| {
        if method == "GET" {
            return Ok(ApiResponse::Json(state(&app.feature_context())));
        }
        if method != "POST" {
            return Err(ApiError::new(405, "Méthode non permise."));
        }
        let input: Setting = crate::parse(bytes)?;
        // Check the debug safety boundary before even creating a request.
        Agent::environment()?;
        Ok(ApiResponse::Json(super::engine::queue_approved_action(
            &app.feature_context(),
            ACTION,
            json!({"enabled":input.enabled}),
        )?))
    })())
}
pub fn call_tool(ctx: &Context<'_>, call: &ToolCall) -> Option<ToolOutcome> {
    if call.name != ACTION {
        return None;
    }
    Some(
        match (|| -> Result<Value, ApiError> {
            super::forms::verify_tool_admitted(ctx, call)?;
            let input: Setting = crate::parse(call.arguments.to_string().as_bytes())?;
            let result = configure(ctx, input.enabled);
            if let Err(error) = &result {
                ctx.connection.execute(
                    "UPDATE wrapper_routines_background SET error=?1 WHERE id=1",
                    [&error.message],
                )?;
            }
            result?;
            Ok(
                json!({"message":if input.enabled {"Agent de routines installé."} else {"Agent de routines désinstallé."}}),
            )
        })() {
            Ok(output) => ToolOutcome::Done { output },
            Err(error) => ToolOutcome::Failed {
                reason: error.message,
            },
        },
    )
}
fn configure(ctx: &Context<'_>, enabled: bool) -> Result<(), ApiError> {
    let agent = Agent::environment()?;
    let domain = agent.domain()?;
    let job = format!("{domain}/{LABEL}");
    let db = fs::canonicalize(ctx.db_path).map_err(io)?;
    let binary = std::env::current_exe().map_err(io)?;
    let previous: Option<String> = ctx.connection.query_row(
        "SELECT database_path FROM wrapper_routines_background WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    // Binary paths change when an app is moved or upgraded. Ownership is the
    // label, HOME and database, never byte equality with today's executable.
    // The persisted old database path also permits withdrawing a moved workspace.
    if agent.plist.exists() && !agent.owns(&db, previous.as_deref())? {
        return Err(fail(
            "Cet agent appartient à un autre espace local. Aucun fichier n’a été modifié.",
        ));
    }
    if enabled {
        if agent.plist.exists() {
            agent.control(&["bootout".into(), job.clone()], true)?;
        }
        atomic(&agent.plist, plist(&binary, &db, &agent.home).as_bytes())?;
        let lint = Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(&agent.plist)
            .output()
            .map_err(io)?;
        if !lint.status.success() {
            let _ = fs::remove_file(&agent.plist);
            return Err(fail("Le fichier de l’agent macOS est invalide."));
        }
        if let Err(error) = agent.control(
            &[
                "bootstrap".into(),
                domain,
                agent.plist.to_string_lossy().into_owned(),
            ],
            false,
        ) {
            let _ = fs::remove_file(&agent.plist);
            ctx.connection.execute(
                "UPDATE wrapper_routines_background SET installed=0 WHERE id=1",
                [],
            )?;
            return Err(error);
        }
    } else {
        let present = agent.plist.exists();
        // If the plist was removed externally, unregister our persisted job.
        if present || previous.is_some() {
            agent.control(&["bootout".into(), job], true)?;
        }
        if present {
            fs::remove_file(&agent.plist).map_err(io)?;
        }
    }
    ctx.connection.execute("UPDATE wrapper_routines_background SET enabled=?1,installed=?1,error=NULL,database_path=?2 WHERE id=1", params![enabled,db.to_string_lossy()])?;
    Ok(())
}
#[cfg(debug_assertions)]
pub(crate) fn test_root() -> Result<PathBuf, ApiError> {
    Ok(Agent::environment()?.home)
}
pub fn state(ctx: &Context<'_>) -> Value {
    let load = || -> Result<Value, ApiError> {
        let (enabled, installed, last, error, outcome): (bool,bool,Option<i64>,Option<String>,Option<String>) = ctx.connection.query_row("SELECT enabled,installed,last_run,error,outcome FROM wrapper_routines_background WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        let available = Agent::environment();
        let present = available.as_ref().is_ok_and(|a| a.plist.is_file());
        Ok(
            json!({"enabled":enabled,"installed":installed && present,"last_run":last,"error":error,"outcome":outcome,"available":available.is_ok(),"unavailable_reason":available.err().map(|e|e.message),"test_mode":cfg!(debug_assertions)}),
        )
    };
    load().unwrap_or_else(|_| json!({"error":"État de l’agent indisponible."}))
}

/// No credentials, provider, migrations or model thread is touched by idle wakes.
/// The minute cadence gives <=60s latency; catch-up/DST/pauses use the exact same
/// scheduler cursor and cron implementation as the foreground app.
pub fn due(db: &Path, now: i64) -> Result<bool, ApiError> {
    if !db.exists() {
        return Ok(false);
    }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let enabled: bool = conn
        .query_row(
            "SELECT enabled FROM wrapper_routines_background WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false);
    if !enabled {
        return Ok(false);
    }
    let recovery: bool = conn.query_row("WITH RECURSIVE threads(id) AS (SELECT s.conversation_id FROM scheduler_occurrences o JOIN scheduler_routines s ON s.routine_id=o.routine_id WHERE o.status='running' UNION SELECT c.id FROM wrapper_conversations c JOIN threads t ON c.parent_id=t.id) SELECT EXISTS(SELECT 1 FROM engine_runs e JOIN threads t ON e.conversation_id=t.id WHERE e.status IN ('queued','running','interrupted'))", [], |r|r.get(0))?;
    if recovery {
        return Ok(true);
    }
    let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM scheduler_occurrences o JOIN scheduler_routines r ON r.routine_id=o.routine_id WHERE o.status='pending' AND r.deleted=0 AND (r.enabled=1 OR o.manual_nonce IS NOT NULL))", [], |r|r.get(0))?;
    if pending {
        return Ok(true);
    }
    let mut stmt = conn.prepare("SELECT routine_id,materialized_after FROM scheduler_routines WHERE enabled=1 AND deleted=0")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, cursor) in rows {
        // Corruption must wake the real scheduler so it can quarantine and
        // publish a failed occurrence instead of silently hiding the error.
        let r = match routine::get(&conn, &id) {
            Ok(Some(r)) => r,
            _ => return Ok(true),
        };
        match cron::next_after(&r.trigger, &r.time_zone, cursor) {
            Ok(Some(at)) if at <= now => return Ok(true),
            Err(_) => return Ok(true),
            _ => {}
        }
    }
    Ok(false)
}

pub fn run(app: &mut App) -> Result<(), ApiError> {
    let started = crate::now();
    let ctx = app.feature_context();
    ctx.connection.execute("UPDATE wrapper_routines_background SET last_run=?1,error=NULL,outcome='Exécution en cours' WHERE id=1", [started])?;
    let failed_before: i64 = ctx.connection.query_row(
        "SELECT count(*) FROM scheduler_occurrences WHERE status='failed'",
        [],
        |r| r.get(0),
    )?;
    let result = drive(app);
    // Even errors must drain workers before main exits and releases its lock.
    app.provider.interrupt_runs();
    while app.provider.has_run_workers() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let ctx = app.feature_context();
    let failed_after: i64 = ctx.connection.query_row(
        "SELECT count(*) FROM scheduler_occurrences WHERE status='failed'",
        [],
        |r| r.get(0),
    )?;
    let error = result
        .as_ref()
        .err()
        .map(|e| e.message.clone())
        .or_else(|| {
            (failed_after > failed_before).then(|| {
                "Une routine a échoué. Consultez son historique dans Routines ou Notifications."
                    .into()
            })
        });
    let outcome = if error.is_some() {
        "Passage avec erreur"
    } else {
        "Passage terminé"
    };
    ctx.connection.execute(
        "UPDATE wrapper_routines_background SET error=?1,outcome=?2 WHERE id=1",
        params![error, outcome],
    )?;
    // Projection uses the existing deduplicated outbox, including failures and
    // pending human-input notifications. No custom notification history.
    if let Err(error) = super::notifications::background(app) {
        app.connection.execute(
            "UPDATE wrapper_routines_background SET error=?1 WHERE id=1",
            [&error.message],
        )?;
        return Err(error);
    }
    result
}
fn drive(app: &mut App) -> Result<(), ApiError> {
    let start = Instant::now();
    // Never report simulated successes while unattended and disconnected.
    if !app.provider.connected() {
        return Err(fail(
            "Connectez ChatGPT dans Ezerbot pour exécuter les routines en arrière-plan.",
        ));
    }
    loop {
        if app.provider.status()["models_ready"] == true {
            break;
        }
        if start.elapsed() > Duration::from_secs(15) {
            return Err(fail(
                "Les modèles ChatGPT sont indisponibles. Un prochain réveil réessaiera.",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Same per-run budget as the app, plus a margin to let the engine settle.
    // Debug tests may shorten the pass deadline, never the run's own budget.
    let deadline = pass_deadline();
    // Materialize once: don't stay alive forever for short-interval schedules.
    super::routines::tick(&app.feature_context());
    let start = Instant::now();
    loop {
        if foreground_waiting(app.feature_context().db_path) {
            return Ok(()); // run() drains/checkpoints workers before releasing Owner
        }
        if start.elapsed() > deadline {
            return Err(fail("Le passage a atteint sa limite. Les routines reprendront avec leur budget conservé au prochain réveil ou à l’ouverture d’Ezerbot."));
        }
        let ctx = app.feature_context();
        super::engine::tick_routines(&ctx);
        let active: bool = ctx.connection.query_row("WITH RECURSIVE threads(id) AS (SELECT conversation_id FROM scheduler_routines UNION SELECT c.id FROM wrapper_conversations c JOIN threads t ON c.parent_id=t.id) SELECT EXISTS(SELECT 1 FROM engine_runs e JOIN threads t ON e.conversation_id=t.id WHERE e.status IN ('queued','running','interrupted'))", [], |r|r.get(0))?;
        if !active && !app.provider.has_run_workers() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn pass_deadline() -> Duration {
    #[cfg(debug_assertions)]
    if Agent::environment().is_ok() {
        if let Ok(ms) = std::env::var("GROKBOT_ROUTINE_PASS_MS") {
            if let Ok(ms) = ms.parse::<u64>() {
                return Duration::from_millis(ms.max(100));
            }
        }
    }
    Duration::from_millis(grokbot_engine::contracts::RunBudgets::default().duration_ms)
        + Duration::from_secs(30)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_is_exclusive_and_recovers_without_unlinking() {
        let root = std::env::temp_dir().join(format!(
            "ezerbot-owner-{}-{}",
            std::process::id(),
            crate::now()
        ));
        let db = root.join("workspace.sqlite");
        let one = Owner::try_acquire(&db).unwrap().unwrap();
        assert!(Owner::try_acquire(&db).unwrap().is_none());
        #[cfg(unix)]
        {
            fs::write(&db, b"").unwrap();
            let alias = root.join("alias.sqlite");
            std::os::unix::fs::symlink(&db, &alias).unwrap();
            assert!(Owner::try_acquire(&alias).unwrap().is_none());
        }
        drop(one);
        assert!(Owner::try_acquire(&db).unwrap().is_some());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn probe_recovers_a_child_but_does_not_spin_on_human_waits() {
        use grokbot_engine::{
            contracts::{RunBudgets, RunKind, TaskStatus, Trigger},
            model::{Run, RunStatus},
        };
        use grokbot_scheduler::occurrences;
        let root = std::env::temp_dir().join(format!(
            "ezerbot-probe-{}-{}",
            std::process::id(),
            crate::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let db = root.join("workspace.sqlite");
        {
            let mut app = App::open(&db).unwrap();
            let future = crate::now() as i64 + 86_400_000;
            let r=app.api("POST","/api/routines",json!({"bot_id":"grokbot","name":"Parent","prompt":"Bilan","trigger":{"kind":"once","at":future},"time_zone":"UTC"}).to_string().as_bytes()).unwrap()["routine"].clone();
            let conn = &app.connection;
            conn.execute(
                "UPDATE wrapper_routines_background SET enabled=1 WHERE id=1",
                [],
            )
            .unwrap();
            let tx = crate::features::write_transaction(conn).unwrap();
            let occurrence =
                occurrences::run_now(&tx, r["routine_id"].as_str().unwrap(), "manual", 0).unwrap();
            let run = Run {
                run_id: "parent".into(),
                bot_id: "grokbot".into(),
                conversation_id: r["conversation_id"].as_str().unwrap().into(),
                trigger: Trigger::Occurrence {
                    occurrence_id: occurrence.occurrence_id.clone(),
                },
                kind: RunKind::Task,
                status: RunStatus::Queued,
                task_status: Some(TaskStatus::Working),
                turn_end: None,
                budgets: RunBudgets::default(),
                started_at: 0,
            };
            grokbot_engine::model::start(&tx, &run).unwrap();
            occurrences::attach_run(&tx, &occurrence.occurrence_id, &run.run_id).unwrap();
            tx.execute(
                "UPDATE engine_runs SET status='awaiting_children' WHERE run_id='parent'",
                [],
            )
            .unwrap();
            tx.execute("INSERT INTO wrapper_conversations(account_id,id,title,bot_id,kind,parent_id,created_at,updated_at) VALUES (?1,'child-thread','Enfant','grokbot','task',?2,0,0)",params![crate::ACCOUNT,run.conversation_id]).unwrap();
            let child = Run {
                run_id: "child".into(),
                conversation_id: "child-thread".into(),
                trigger: Trigger::Delegation {
                    delegation_id: "child".into(),
                },
                ..run
            };
            grokbot_engine::model::start(&tx, &child).unwrap();
            tx.commit().unwrap();
            assert!(due(&db, crate::now() as i64).unwrap());
            conn.execute(
                "UPDATE engine_runs SET status='awaiting_human_input' WHERE run_id='child'",
                [],
            )
            .unwrap();
            assert!(!due(&db, crate::now() as i64).unwrap());
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn plist_escapes_paths_and_only_invokes_readonly_wake() {
        let body = plist(
            Path::new("/a & b/wrapper"),
            Path::new("/tmp/<home>/workspace.sqlite"),
            Path::new("/tmp/<home>"),
        );
        assert!(body.contains("/a &amp; b/wrapper"));
        assert!(body.contains("/tmp/&lt;home&gt;/workspace.sqlite"));
        assert!(body.contains("--routine-wake"));
        assert!(!body.contains("KeepAlive"));
    }
}
