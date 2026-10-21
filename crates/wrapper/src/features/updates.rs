//! Réglages « Mises à jour » : recherche de `main` et remplacement de l’application.
use super::Context;
use crate::updates::{
    self, ApplyInput, Availability, CommitRef, Fault, Phase, SystemRunner, UpdateError,
};
use crate::{ApiError, ApiResponse, App};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once};
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
}

static JOB: Mutex<Job> = Mutex::new(Job {
    phase: "idle",
    available: false,
    remote_sha: String::new(),
    remote_unix: None,
    message: String::new(),
    log: String::new(),
    running: false,
});
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

pub fn tick(_: &Context<'_>) {
    if !quiet_enabled() {
        return;
    }
    QUIET.call_once(|| {
        let _ = begin_check(true);
    });
}

fn quiet_enabled() -> bool {
    std::env::var_os("EZERBOT_QUIET_UPDATE_CHECK").is_some() || updates::running_bundle().is_some()
}

pub fn routes(
    _app: &mut App,
    method: &str,
    path: &str,
    _body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !path.starts_with("/api/updates") {
        return None;
    }
    Some(match (method, path) {
        ("GET", "/api/updates") => Ok(ApiResponse::Json(snapshot())),
        ("POST", "/api/updates/check") => begin_check(false).map(ApiResponse::Json),
        ("POST", "/api/updates/apply") => begin_apply().map(ApiResponse::Json),
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
                });
            }
            Err(error) => publish(|job| {
                job.running = false;
                job.available = false;
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
    });
    thread::spawn(move || {
        let mut runner = SystemRunner;
        let mut on_progress = |phase: Phase| {
            publish(|job| {
                job.phase = phase.id();
                job.message = phase.label().into();
            });
        };
        match updates::apply(
            &ApplyInput {
                paths: &paths,
                remote: &remote,
                fault: Fault::None,
            },
            &mut runner,
            &mut on_progress,
        ) {
            Ok(()) => finish_success(),
            Err(error) => publish(|job| {
                job.running = false;
                job.available = true;
                job.phase = "erreur";
                job.message = error.message;
                job.log = error.log;
            }),
        }
    });
    Ok(snapshot())
}

fn finish_success() {
    publish(|job| {
        job.running = true;
        job.phase = Phase::Redemarrage.id();
        job.message = Phase::Redemarrage.label().into();
    });
    if cfg!(test) {
        return;
    }
    thread::sleep(Duration::from_millis(400));
    std::process::exit(updates::RELAUNCH_EXIT_CODE);
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
    })
}

fn optional_sha(sha: &str) -> Value {
    if sha.is_empty() {
        Value::Null
    } else {
        Value::String(sha.to_string())
    }
}
