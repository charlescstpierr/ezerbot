//! Réglages « Mises à jour » : recherche de `main` et remplacement de l’application.
use super::Context;
use crate::updates::{
    self, ApplyInput, Availability, CommitRef, Fault, Phase, Refresh, SystemRunner, UpdateError,
};
use crate::{ApiError, ApiResponse, App};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use std::thread;
use std::time::Duration;

empty_hooks!(
    migrate,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

struct Job {
    phase: &'static str,
    available: bool,
    remote_sha: String,
    remote_unix: Option<i64>,
    message: String,
    log: String,
    running: bool,
    relaunch_ready: bool,
    last_attempt: &'static str,
    last_error: String,
}

static JOB: Mutex<Job> = Mutex::new(Job {
    phase: "idle",
    available: false,
    remote_sha: String::new(),
    remote_unix: None,
    message: String::new(),
    log: String::new(),
    running: false,
    relaunch_ready: false,
    last_attempt: "aucune",
    last_error: String::new(),
});
static DATABASE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
static HOLD: AtomicBool = AtomicBool::new(false);
static RESTART: Mutex<Restart> = Mutex::new(Restart {
    pending: false,
    clear: 0,
    armed: false,
});

struct Restart {
    pending: bool,
    clear: u8,
    armed: bool,
}

pub fn remember_database(path: &Path) {
    *database_lock() = Some(path.to_path_buf());
}

fn database_lock() -> MutexGuard<'static, Option<std::path::PathBuf>> {
    DATABASE.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn restart_lock() -> MutexGuard<'static, Restart> {
    RESTART.lock().unwrap_or_else(|poison| poison.into_inner())
}
static REVISION: AtomicU64 = AtomicU64::new(0);
static QUIET: Once = Once::new();

fn lock() -> std::sync::MutexGuard<'static, Job> {
    JOB.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn publish(update: impl FnOnce(&mut Job)) {
    let mut job = lock();
    update(&mut job);
    REVISION.fetch_add(1, Ordering::Relaxed);
}

pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

pub fn state(_: &Context<'_>) -> Value {
    snapshot()
}

pub fn tick(ctx: &Context<'_>) {
    if quiet_enabled() {
        QUIET.call_once(|| {
            let _ = begin_check(true);
        });
    }
    let blocked = capture_blocked(ctx.connection);
    let Ok(mut restart) = RESTART.try_lock() else {
        return;
    };
    if !restart.pending {
        return;
    }
    let exit_now = observe(&mut restart, blocked);
    drop(restart);
    if exit_now {
        relaunch_process();
    }
}

fn quiet_enabled() -> bool {
    std::env::var_os("EZERBOT_QUIET_UPDATE_CHECK").is_some() || updates::running_bundle().is_some()
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !path.starts_with("/api/updates") {
        return None;
    }
    Some(match (method, path) {
        ("GET", "/api/updates") => Ok(ApiResponse::Json(snapshot())),
        ("POST", "/api/updates/check") => begin_check(false).map(ApiResponse::Json),
        ("POST", "/api/updates/apply") => begin_apply().map(ApiResponse::Json),
        ("POST", "/api/updates/hold") => note_hold(app, body).map(ApiResponse::Json),
        _ => Err(ApiError::new(
            404,
            "Cette action de mise à jour n’existe pas.",
        )),
    })
}

fn begin_check(quiet: bool) -> Result<Value, ApiError> {
    {
        let job = lock();
        if job.running {
            return Err(ApiError::new(409, "Une mise à jour est déjà en cours."));
        }
    }
    publish(|job| {
        job.running = true;
        job.phase = "recherche";
        job.log.clear();
        job.message = "Recherche des mises à jour…".into();
        job.relaunch_ready = false;
    });
    thread::spawn(move || {
        let installed = updates::installed_commit();
        match updates::fetch_remote(&mut SystemRunner).and_then(|remote| {
            updates::availability(&installed, &remote).map(|availability| (availability, remote))
        }) {
            Ok((Availability::UpToDate, remote)) => publish(|job| {
                job.running = false;
                job.available = false;
                job.phase = "a_jour";
                job.remote_sha = remote.sha;
                job.remote_unix = remote.unix;
                job.message = "Ezerbot est à jour.".into();
                job.log.clear();
                job.relaunch_ready = false;
            }),
            Ok((Availability::Available(remote), _)) => {
                let short = updates::short_commit(&remote.sha);
                publish(|job| {
                    job.running = false;
                    job.available = true;
                    job.phase = "disponible";
                    job.remote_sha = remote.sha;
                    job.remote_unix = remote.unix;
                    job.message = format!("Une mise à jour est disponible ({short}).");
                    job.log.clear();
                    job.relaunch_ready = false;
                });
            }
            Err(error) => publish(|job| {
                job.running = false;
                job.available = false;
                job.relaunch_ready = false;
                if quiet {
                    job.phase = "idle";
                    job.message.clear();
                    job.log.clear();
                } else {
                    job.phase = "erreur";
                    job.message = error.message;
                    job.log = error.log;
                }
            }),
        }
    });
    Ok(snapshot())
}

fn begin_apply() -> Result<Value, ApiError> {
    let remote = {
        let job = lock();
        if job.running {
            return Err(ApiError::new(409, "Une mise à jour est déjà en cours."));
        }
        if !job.available || job.remote_sha.is_empty() {
            return Err(ApiError::new(409, "Recherchez d’abord les mises à jour."));
        }
        CommitRef {
            sha: job.remote_sha.clone(),
            unix: job.remote_unix,
        }
    };
    let paths = updates::production_paths().map_err(into_api)?;
    updates::validate_paths(&paths).map_err(into_api)?;
    publish(|job| {
        job.running = true;
        job.phase = Phase::Telechargement.id();
        job.message = Phase::Telechargement.label().into();
        job.log.clear();
        job.last_attempt = "en_cours";
        job.last_error.clear();
    });
    thread::spawn(move || {
        let mut runner = SystemRunner;
        let installed = updates::installed_commit();
        let remote = match updates::refresh_target(&installed, &mut runner) {
            Ok(Refresh::Current) => {
                publish(|job| {
                    job.running = false;
                    job.available = false;
                    job.phase = "a_jour";
                    job.message = "Ezerbot est à jour.".into();
                    job.log.clear();
                    job.relaunch_ready = false;
                    job.last_attempt = "a_jour";
                    job.last_error.clear();
                });
                return;
            }
            Ok(Refresh::Ready(commit)) => commit,
            Err(_) => remote,
        };
        let mut on_progress = |phase: Phase| {
            publish(|job| {
                job.phase = phase.id();
                job.message = phase.label().into();
                job.relaunch_ready = false;
            });
        };
        match updates::apply(
            &ApplyInput {
                paths: &paths,
                remote: &remote,
                installed: Some(&installed),
                repo: None,
                fault: Fault::None,
            },
            &mut runner,
            &mut on_progress,
        ) {
            Ok(()) => finish_success(),
            Err(error) => note_failure(error),
        }
    });
    Ok(snapshot())
}

fn note_failure(error: UpdateError) {
    publish(|job| {
        job.running = false;
        job.available = true;
        job.phase = "erreur";
        job.message = error.message.clone();
        job.log = error.log;
        job.relaunch_ready = false;
        job.last_attempt = "echec";
        job.last_error = error.message;
    });
}

fn finish_success() {
    let blocked = capture_blocked_saved();
    let mut restart = restart_lock();
    let exit_now = observe(&mut restart, blocked);
    drop(restart);
    if exit_now {
        relaunch_process();
    }
}

/// Deux observations sans capture avant de quitter : une réunion démarrée
/// pendant ce délai annule le redémarrage.
fn observe(restart: &mut Restart, blocked: bool) -> bool {
    if blocked {
        restart.clear = 0;
        restart.pending = true;
        restart.armed = false;
        publish(|job| {
            job.running = true;
            job.available = true;
            job.phase = "attente";
            job.message = "Le redémarrage suivra la fin de l’enregistrement.".into();
            job.relaunch_ready = false;
            job.last_attempt = "attente";
            job.last_error.clear();
        });
        return false;
    }
    restart.clear = restart.clear.saturating_add(1);
    restart.pending = true;
    if restart.clear < 2 {
        publish(|job| {
            job.running = true;
            job.phase = Phase::Redemarrage.id();
            job.message = Phase::Redemarrage.label().into();
            job.relaunch_ready = false;
            job.last_attempt = "reussie";
            job.last_error.clear();
        });
        return false;
    }
    restart.pending = false;
    restart.armed = true;
    publish(|job| {
        job.running = true;
        job.phase = Phase::Redemarrage.id();
        job.message = Phase::Redemarrage.label().into();
        job.relaunch_ready = true;
        job.last_attempt = "reussie";
        job.last_error.clear();
    });
    true
}

fn relaunch_process() {
    if cfg!(test) || std::env::var(updates::UPDATE_RELAUNCH_ENV).ok().as_deref() == Some("hold") {
        return;
    }
    thread::sleep(Duration::from_millis(200));
    if capture_blocked_saved() {
        let mut restart = restart_lock();
        restart.armed = false;
        restart.pending = true;
        restart.clear = 0;
        publish(|job| {
            job.running = true;
            job.available = true;
            job.phase = "attente";
            job.message = "Le redémarrage suivra la fin de l’enregistrement.".into();
            job.relaunch_ready = false;
            job.last_attempt = "attente";
            job.last_error.clear();
        });
        return;
    }
    std::process::exit(updates::RELAUNCH_EXIT_CODE);
}

fn note_hold(app: &App, body: &[u8]) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Hold {
        active: bool,
    }
    let input: Hold = crate::parse(body)?;
    HOLD.store(input.active, Ordering::SeqCst);
    let blocked = input.active || unsaved_meeting(&app.connection);
    let exit_now = {
        let mut restart = restart_lock();
        if !restart.pending {
            false
        } else {
            observe(&mut restart, blocked)
        }
    };
    if exit_now {
        relaunch_process();
    }
    Ok(snapshot())
}

pub fn unsaved_meeting(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_meetings WHERE status='recording')",
        [],
        |row| row.get(0),
    )
    .unwrap_or(true)
}

fn capture_blocked(conn: &Connection) -> bool {
    HOLD.load(Ordering::SeqCst) || unsaved_meeting(conn)
}

fn capture_blocked_saved() -> bool {
    if HOLD.load(Ordering::SeqCst) {
        return true;
    }
    let Some(path) = database_lock().clone() else {
        return false;
    };
    match Connection::open(path) {
        Ok(conn) => {
            let _ = conn.busy_timeout(Duration::from_secs(3));
            unsaved_meeting(&conn)
        }
        Err(_) => true,
    }
}

fn into_api(error: UpdateError) -> ApiError {
    ApiError::new(400, error.message)
}

fn snapshot() -> Value {
    let job = lock();
    let installed = updates::installed_commit();
    json!({
        "version": updates::installed_version(),
        "installed_commit": installed.sha,
        "installed_short": updates::short_commit(&installed.sha),
        "phase": job.phase,
        "available": job.available,
        "remote_commit": optional_sha(&job.remote_sha),
        "remote_short": if job.remote_sha.is_empty() {
            Value::Null
        } else {
            Value::String(updates::short_commit(&job.remote_sha))
        },
        "message": job.message,
        "log": job.log,
        "running": job.running,
        "last_attempt": job.last_attempt,
        "last_error": job.last_error,
        "relaunch": if job.phase == "attente" {
            json!("attente")
        } else if job.relaunch_ready {
            json!("pret")
        } else {
            Value::Null
        },
    })
}

fn optional_sha(sha: &str) -> Value {
    if sha.is_empty() {
        Value::Null
    } else {
        Value::String(sha.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reset(restart: &mut Restart) {
        restart.pending = false;
        restart.clear = 0;
        restart.armed = false;
        HOLD.store(false, Ordering::SeqCst);
        publish(|job| {
            job.phase = "idle";
            job.available = false;
            job.running = false;
            job.message.clear();
            job.log.clear();
            job.relaunch_ready = false;
            job.last_attempt = "aucune";
            job.last_error.clear();
        });
    }

    #[test]
    fn le_redemarrage_attend_la_fin_de_la_reunion() {
        let root = std::env::temp_dir().join(format!("ezerbot-maj-reunion-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut app = crate::App::open(&root.join("workspace.sqlite")).unwrap();
        {
            let mut restart = restart_lock();
            reset(&mut restart);
        }
        note_failure(UpdateError::message(
            "La compilation a échoué. L’application actuelle est restée en place.",
        ));
        let failed = snapshot();
        assert_eq!(failed["phase"], "erreur");
        assert_eq!(failed["last_attempt"], "echec");
        assert_eq!(
            failed["last_error"],
            "La compilation a échoué. L’application actuelle est restée en place."
        );
        assert_eq!(failed["available"], true);
        {
            let mut restart = restart_lock();
            reset(&mut restart);
        }
        let held = app
            .api("POST", "/api/updates/hold", br#"{"active":true}"#)
            .unwrap();
        assert_eq!(held["phase"], "idle");
        assert!(HOLD.load(Ordering::SeqCst));
        app.api("POST", "/api/updates/hold", br#"{"active":false}"#)
            .unwrap();
        assert!(!HOLD.load(Ordering::SeqCst));

        let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_string();
        let id = app
            .api(
                "POST",
                "/api/meetings/start",
                &serde_json::to_vec(&json!({
                    "conversation_id": conversation,
                    "title": "Point équipe",
                    "consent": true
                }))
                .unwrap(),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();

        let mut restart = restart_lock();
        assert!(unsaved_meeting(app.feature_context().connection));
        assert!(!observe(&mut restart, true));
        assert!(!restart.armed);
        let waiting = snapshot();
        assert_eq!(waiting["phase"], "attente");
        assert_eq!(waiting["last_attempt"], "attente");
        assert_eq!(waiting["last_error"], "");
        assert_eq!(waiting["relaunch"], "attente");
        assert_eq!(
            waiting["message"],
            "Le redémarrage suivra la fin de l’enregistrement."
        );
        assert!(!observe(
            &mut restart,
            unsaved_meeting(app.feature_context().connection)
        ));
        assert!(!restart.armed);

        app.api(
            "POST",
            "/api/meetings/discard",
            &serde_json::to_vec(&json!({"id": id})).unwrap(),
        )
        .unwrap();
        assert!(!unsaved_meeting(app.feature_context().connection));
        assert!(!observe(&mut restart, false));
        assert!(!restart.armed);
        assert_eq!(snapshot()["phase"], "redemarrage");
        assert!(observe(&mut restart, false));
        assert!(restart.armed);
        assert_eq!(snapshot()["relaunch"], "pret");
        assert_eq!(snapshot()["last_attempt"], "reussie");

        // Une capture qui commence dans l’intervalle annule le départ.
        reset(&mut restart);
        let id = app
            .api(
                "POST",
                "/api/meetings/start",
                &serde_json::to_vec(&json!({
                    "conversation_id": conversation,
                    "title": "Suite",
                    "consent": true
                }))
                .unwrap(),
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!observe(&mut restart, false));
        assert!(!observe(
            &mut restart,
            unsaved_meeting(app.feature_context().connection)
        ));
        assert!(!restart.armed);
        assert_eq!(snapshot()["phase"], "attente");
        app.feature_context()
            .connection
            .execute(
                "UPDATE wrapper_meetings SET status='interrupted' WHERE id=?1",
                [&id],
            )
            .unwrap();
        assert!(!unsaved_meeting(app.feature_context().connection));
        assert!(!observe(&mut restart, false));
        assert!(observe(&mut restart, false));
        assert!(restart.armed);

        reset(&mut restart);
        HOLD.store(true, Ordering::SeqCst);
        assert!(!observe(&mut restart, true));
        assert!(!restart.armed);
        assert_eq!(snapshot()["relaunch"], "attente");
        reset(&mut restart);
        let _ = std::fs::remove_dir_all(&root);
    }
}
