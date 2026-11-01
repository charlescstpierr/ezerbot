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
        "description": "Attendre la fin de la tâche",
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

#[test]
fn a_pending_connector_sign_in_is_ended_by_the_destruction_and_nothing_answers_afterwards() {
    // Le serveur de jetons n'est jamais contacté : l'adresse n'est qu'un point d'ancrage.
    std::env::set_var("GROKBOT_FAKE_CONNECTORS", "http://127.0.0.1:9");
    let mut f = Fixture::new();
    let run = f.post(
        "/api/connectors/connect",
        json!({"id": "calendar", "client_id": "fixture-oauth-client", "client_secret": "fixture-oauth-secret"}),
    )["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let db = f.db.clone();
    // Le tour de l'action peut attendre une carte : on la décide, comme un utilisateur.
    let authorization = {
        let mut found = None;
        wait(
            "connexion du connecteur en attente dans le navigateur",
            || {
                f.app.tick_suspended(0);
                if let Ok(requests) = grokbot_forms::requests::list_for_run(
                    &rusqlite::Connection::open(&db).unwrap(),
                    &run,
                ) {
                    if let Some(request) = requests.first() {
                        let body = json!({
                            "entry_id": request.entry_id,
                            "conversation_id": request.route.target_conversation_id,
                            "submission_id": "decide-connect",
                            "decision": "allow_once",
                        });
                        let _ = f.app.api(
                            "POST",
                            "/api/approvals/resolve",
                            body.to_string().as_bytes(),
                        );
                    }
                }
                found = f.app.api("GET", "/api/state", b"").unwrap()["mcp"]["catalogue"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["id"] == "calendar" && m["status"] == "authorizing")
                    .and_then(|m| m["authorization_url"].as_str().map(str::to_owned));
                found.is_some()
            },
        );
        found.unwrap()
    };
    let redirect = url::Url::parse(&authorization)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "redirect_uri")
        .map(|(_, value)| url::Url::parse(&value).unwrap())
        .unwrap();
    let port = redirect.port().unwrap();
    let phase_before: Option<String> = query(
        &db,
        "SELECT phase FROM wrapper_connector_catalogue WHERE id='calendar'",
    );
    assert_eq!(phase_before.as_deref(), Some("authorizing"));
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_ok(),
        "l'écouteur de retour OAuth attend le navigateur"
    );

    drop(f.app);

    // Le fil du connecteur est fini et joint : plus personne n'écoute, donc plus de retour
    // tardif possible, et rien n'a écrit la phase (ni retour, ni expiration).
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "l'écouteur de retour OAuth est fermé après la destruction de l'App"
    );
    let phase_after: Option<String> = query(
        &db,
        "SELECT phase FROM wrapper_connector_catalogue WHERE id='calendar'",
    );
    assert_eq!(
        phase_after, phase_before,
        "la destruction n'écrit pas la phase"
    );
}
