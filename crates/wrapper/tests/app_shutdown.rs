//! Détruire une `App` attend ses fils de fond (#463) : ni un tour en vol, ni une tâche shell qu'il
//! lance pendant la destruction, ne doivent rien écrire dans le dossier de l'application une fois
//! l'`App` détruite.
//!
//! Aucun délai ne règle le chevauchement : la réponse du faux serveur est tenue derrière une
//! porte, que le test n'ouvre que lorsque la destruction a commencé (l'attente de l'`App` est
//! comptée par le fournisseur) ou, sans correction, une fois les constats faits.
mod support;

use grokbot_wrapper::{App, Provider};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

struct Fixture {
    app: App,
    provider: Provider,
    seen: Arc<Mutex<Seen>>,
    db: std::path::PathBuf,
    root: std::path::PathBuf,
    chat: String,
    // Après l'App et le faux serveur dans l'ordre de destruction : supprimé en dernier.
    _server: FakeOpenAi,
    _dir: grokbot_test_support::TestDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = grokbot_test_support::TestDir::new("app-shutdown-test");
        let root = dir.to_path_buf();
        // Comme en production : les données de l'application dans leur sous-dossier, les
        // fichiers ordinaires (la commande shell) ailleurs.
        std::fs::create_dir_all(root.join("data")).unwrap();
        let db = root.join("data/workspace.sqlite");
        let seen = Arc::new(Mutex::new(Seen::default()));
        let server = FakeOpenAi::new(seen.clone());
        let provider = Provider::with_endpoints(&db, server.endpoints(), false);
        let mut app = App::open_with(&db, provider.clone()).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(
            db.with_extension("chatgpt-auth.json"),
            json!({
                "host_id": "test-host",
                "client_id": CLIENT,
                "access_token": "access-2",
                "refresh_token": "refresh-2",
                "expires_at": now + 3600,
                "scopes": "chatgpt.tokens.use.direct",
            })
            .to_string(),
        )
        .unwrap();
        provider.refresh_models();
        wait("catalogue", || provider.status()["models_ready"] == true);
        provider.wait_for_background_work().unwrap();
        let chat = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            app,
            provider,
            seen,
            db,
            root,
            chat,
            _server: server,
            _dir: dir,
        }
    }

    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, &serde_json::to_vec(&body).unwrap())
            .unwrap()
    }

    /// Tient la prochaine réponse du modèle derrière une porte fermée, envoie le message et
    /// attend que la requête soit arrivée : le tour est alors en vol, bloqué par la porte.
    fn send_held(&mut self, reply: Option<Value>) -> Arc<AtomicBool> {
        let gate = Arc::new(AtomicBool::new(false));
        let at = {
            let mut seen = self.seen.lock().unwrap();
            let at = seen.bodies.len() + 1;
            seen.response_gate = Some((at, gate.clone()));
            if let Some(call) = reply {
                seen.responses.push_back(ScriptedResponse::events(&[
                    call,
                    json!({"type": "response.completed"}),
                ]));
            }
            at
        };
        let chat = self.chat.clone();
        self.post(
            "/api/send",
            json!({"conversation_id": chat, "message_id": "held", "body": "Bonjour"}),
        );
        wait("le tour est en vol derrière la porte", || {
            self.seen.lock().unwrap().bodies.len() >= at
        });
        gate
    }

    /// Détruit l'App pendant que le tour est tenu. La porte s'ouvre quand l'attente de la
    /// destruction a commencé ; sans cette attente (code sans correction), `drop` rend la main
    /// tout de suite et la porte ne s'ouvre qu'après `observe`, qui lit l'état à cet instant.
    fn destroy_while_held<T>(
        self,
        gate: Arc<AtomicBool>,
        observe: impl FnOnce(&Path) -> T,
    ) -> (T, Vec<(String, u64)>) {
        let Fixture {
            app,
            provider,
            db,
            _server,
            _dir,
            ..
        } = self;
        let waits_before = provider.background_waits_started();
        let returned = Arc::new(AtomicBool::new(false));
        let opener = {
            let (provider, returned) = (provider.clone(), returned.clone());
            std::thread::spawn(move || {
                while provider.background_waits_started() == waits_before
                    && !returned.load(Ordering::SeqCst)
                {
                    std::thread::sleep(Duration::from_millis(1));
                }
                gate.store(true, Ordering::SeqCst);
            })
        };
        drop(app);
        let observed = observe(&db);
        returned.store(true, Ordering::SeqCst);
        opener.join().unwrap();
        let _ = provider.wait_for_background_work();
        (observed, snapshot(db.parent().unwrap()))
    }
}

/// Liste triée (chemin relatif, taille) de tout ce que contient le dossier.
fn snapshot(root: &Path) -> Vec<(String, u64)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                walk(root, &path, out);
            } else {
                let name = path.strip_prefix(root).unwrap().display().to_string();
                out.push((name, meta.len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn query<T: rusqlite::types::FromSql>(db: &Path, sql: &str) -> Option<T> {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.query_row(sql, [], |row| row.get(0)).ok()
}

fn group_alive(pgid: i64) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", "--", &format!("-{pgid}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn dropping_the_app_waits_for_a_turn_held_in_flight() {
    let mut f = Fixture::new();
    let gate = f.send_held(None);
    let ((status, at_drop), finally) = f.destroy_while_held(gate, |db| {
        let status: Option<String> = query(
            db,
            "SELECT status FROM engine_runs ORDER BY rowid DESC LIMIT 1",
        );
        (status, snapshot(db.parent().unwrap()))
    });
    // Le tour était tenu derrière la porte à l'appel de `drop` : il ne peut être fini à son
    // retour que si la destruction l'a attendu.
    assert_eq!(
        status.as_deref(),
        Some("finished"),
        "le tour est fini quand l'App est détruite"
    );
    assert_eq!(finally, at_drop, "rien n'est écrit après la destruction");
}

#[test]
fn a_shell_task_started_by_the_turn_during_the_destruction_is_stopped_and_joined() {
    let mut f = Fixture::new();
    f.post(
        "/api/local/bot",
        json!({"bot_id": "grokbot", "shell": true, "files": false}),
    );
    let arguments = json!({
        "command": "sleep 300",
        "cwd": f.root,
        "timeout_ms": 2000,
        "output_limit": 1024,
        "background": true,
    });
    let call = json!({"type": "response.output_item.done", "item": {
        "id": "fc-held", "type": "function_call", "status": "completed", "call_id": "held",
        "name": "run_shell", "arguments": arguments.to_string()}});
    let gate = f.send_held(Some(call));
    let ((rows, running, pgid), _) = f.destroy_while_held(gate, |db| {
        (
            query::<i64>(db, "SELECT count(*) FROM wrapper_shell_tasks"),
            query::<i64>(
                db,
                "SELECT count(*) FROM wrapper_shell_tasks WHERE status='en cours'",
            ),
            query::<i64>(db, "SELECT pgid FROM wrapper_shell_tasks LIMIT 1"),
        )
    });
    // La commande n'est lancée qu'après le début de la destruction (la porte était fermée).
    assert_eq!(
        rows,
        Some(1),
        "la tâche shell a été lancée pendant la destruction"
    );
    assert_eq!(running, Some(0), "aucune tâche ne reste « en cours »");
    let pgid = pgid.expect("groupe de la tâche");
    wait("le groupe de processus de la tâche est mort", || {
        !group_alive(pgid)
    });
}
