mod support;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use grokbot_engine::{
    contracts::{RunEventKind, ToolOutcome},
    model::RunStatus,
    store,
};
use grokbot_forms::requests;
use grokbot_wrapper::{App, Provider};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};
use tiny_http::{Header, Response, Server};

#[derive(Default)]
struct SeenOAuth {
    requests: Vec<(String, String)>,
    challenge: String,
    token_error: bool,
    tools_error: bool,
    revoke_error: bool,
    sse: bool,
}
struct OAuth {
    base: String,
    seen: Arc<Mutex<SeenOAuth>>,
    stop: Arc<AtomicBool>,
}
impl OAuth {
    fn new() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr());
        let seen = Arc::new(Mutex::new(SeenOAuth::default()));
        let captured = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let Some(mut req) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                    continue;
                };
                let url = req.url().to_owned();
                let mut body = String::new();
                req.as_reader().read_to_string(&mut body).unwrap();
                let mut seen = captured.lock().unwrap();
                seen.requests.push((url.clone(), body.clone()));
                let fields: std::collections::HashMap<_, _> =
                    url::form_urlencoded::parse(body.as_bytes())
                        .into_owned()
                        .collect();
                let (status, data) = if url.ends_with("/register") {
                    (200, json!({"client_id":"fake-notion-client"}))
                } else if url.ends_with("/token") {
                    if seen.token_error {
                        (
                            400,
                            json!({"error":"invalid_grant","error_description":"secret-must-not-leak"}),
                        )
                    } else {
                        if fields
                            .get("grant_type")
                            .is_some_and(|s| s == "authorization_code")
                        {
                            let verifier = fields.get("code_verifier").expect("PKCE verifier");
                            assert_eq!(
                                URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                                seen.challenge
                            );
                        }
                        (
                            200,
                            json!({"access_token":"access-secret-never-visible-123456789","refresh_token":"refresh-secret-never-visible-987654321","token_type":"Bearer","expires_in":3600}),
                        )
                    }
                } else if url.ends_with("/userinfo") {
                    (200, json!({"email":"personne@example.test"}))
                } else if url.ends_with("/revoke") {
                    if seen.revoke_error {
                        (500, json!({"error":"revocation-failed-secret"}))
                    } else {
                        (200, json!({"ok":true,"revoked":true}))
                    }
                } else if url.ends_with("/mcp") {
                    let frame: Value = serde_json::from_str(&body).unwrap();
                    if seen.tools_error {
                        (401, json!({"error":"private-server-error"}))
                    } else if frame.get("id").is_none() {
                        (202, Value::Null)
                    } else {
                        let result = match frame["method"].as_str().unwrap() {
                            "initialize" => {
                                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"factice","version":"1"}})
                            }
                            "tools/list" => {
                                json!({"tools":[{"name":"echo","description":"Outil factice ; access-secret-never-visible-123456789","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]})
                            }
                            "tools/call" => {
                                json!({"content":[{"type":"text","text":format!("{} · access-secret-never-visible-123456789",frame["params"]["arguments"]["text"])}]})
                            }
                            _ => panic!("unexpected method"),
                        };
                        (
                            200,
                            json!({"jsonrpc":"2.0","id":frame["id"],"result":result}),
                        )
                    }
                } else {
                    (404, json!({}))
                };
                let sse = seen.sse && url.ends_with("/mcp") && status == 200;
                drop(seen);
                let text = if sse {
                    format!("event: message\ndata: {}\n\n", data)
                } else {
                    data.to_string()
                };
                let _ = req.respond(
                    Response::from_string(text)
                        .with_status_code(status)
                        .with_header(
                            Header::from_bytes(
                                "Content-Type",
                                if sse {
                                    "text/event-stream"
                                } else {
                                    "application/json"
                                },
                            )
                            .unwrap(),
                        )
                        .with_header(Header::from_bytes("Mcp-Session-Id", "fake-session").unwrap()),
                );
            }
        });
        Self { base, seen, stop }
    }
    fn count(&self, suffix: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(p, _)| p.ends_with(suffix))
            .count()
    }
}
impl Drop for OAuth {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
struct Fixture {
    app: App,
    path: PathBuf,
    provider: Provider,
    ai: FakeOpenAi,
    oauth: OAuth,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir()
            .join(format!("ezerbot-connectors-{}", std::process::id()))
            .join("workspace.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let ai = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, ai.endpoints(), false);
        let oauth = OAuth::new();
        std::env::set_var("GROKBOT_FAKE_CONNECTORS", &oauth.base);
        let app = App::open_with(&path, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        Self {
            app,
            path,
            provider,
            ai,
            oauth,
        }
    }
    fn db(&self) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(4)).unwrap();
        c
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn item(&mut self, id: &str) -> Value {
        self.state()["mcp"]["catalogue"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()
            .clone()
    }
    fn connect(&mut self, id: &str) -> String {
        let body = if id != "notion" && self.item(id)["has_client"] != true {
            json!({"id":id,"client_id":"fixture-oauth-client","client_secret":"fixture-oauth-secret"})
        } else {
            json!({"id":id})
        };
        self.post("/api/connectors/connect", body)["run_id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn finish(&self, run: &str) {
        wait("terminal host action", || {
            store::required(&self.db(), run)
                .unwrap()
                .status
                .is_terminal()
        });
    }
    fn decide(&mut self, run: &str, decision: Value) {
        wait("approval", || {
            if !requests::list_for_run(&self.db(), run).unwrap().is_empty() {
                return true;
            }
            let status = store::required(&self.db(), run).unwrap().status;
            assert!(
                !status.is_terminal(),
                "run {run} ended before approval: {:?}",
                store::events(&self.db(), run, 0).unwrap()
            );
            false
        });
        let r = requests::list_for_run(&self.db(), run).unwrap().remove(0);
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"conversation_id":r.route.target_conversation_id,"submission_id":format!("decide-{run}"),"decision":decision}));
        self.app.tick(0);
        self.finish(run);
    }
    fn callback(&mut self, id: &str, deny: bool, wrong_state: bool) -> u16 {
        let auth = self.item(id)["authorization_url"]
            .as_str()
            .unwrap()
            .to_owned();
        let auth = url::Url::parse(&auth).unwrap();
        let fields: std::collections::HashMap<_, _> = auth.query_pairs().into_owned().collect();
        assert_eq!(fields["code_challenge_method"], "S256");
        self.oauth.seen.lock().unwrap().challenge = fields["code_challenge"].clone();
        let mut callback = url::Url::parse(&fields["redirect_uri"]).unwrap();
        callback
            .query_pairs_mut()
            .append_pair(
                "state",
                if wrong_state {
                    "wrong-state"
                } else {
                    &fields["state"]
                },
            )
            .append_pair(
                if deny { "error" } else { "code" },
                if deny { "access_denied" } else { "fake-code" },
            );
        match ureq::get(callback.as_str()).call() {
            Ok(r) => r.status(),
            Err(ureq::Error::Status(code, _)) => code,
            Err(e) => panic!("callback transport {e}"),
        }
    }
    fn outcome(&self, run: &str) -> ToolOutcome {
        let event = store::events(&self.db(), run, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult)
            .unwrap();
        serde_json::from_value(event.payload["outcome"].clone()).unwrap()
    }
    fn script(&self, events: Vec<Value>) {
        self.ai
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn tool_run(&mut self, server: &Value, id: &str) -> String {
        let conversation = self.post("/api/conversations", json!({"title":"Connecteur factice"}))
            ["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let name = server["tools"][0]["namespace"].as_str().unwrap();
        self.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":name,"arguments":json!({"text":"bonjour"}).to_string()}}),json!({"type":"response.completed"})]);
        self.post(
            "/api/send",
            json!({"conversation_id":conversation,"message_id":id,"body":"Utilise le service"}),
        );
        self.db().query_row("SELECT r.run_id FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')=?1",[id],|r|r.get::<_,String>(0)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::env::remove_var("GROKBOT_FAKE_CONNECTORS");
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}

#[test]
fn google_server_adapter_keeps_only_an_access_snapshot_in_memory() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new("python3")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("ACCESS_TOKEN", "fake-google-access-token")
        .env("GOOGLE_EMAIL", "factice@example.test")
        .env("GOOGLE_SCOPES", "openid email gmail.modify")
        .arg(root.join("tests/fixtures/google_bridge_check.py"))
        .arg(root.join("src/features/mcp/google_bridge.py"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "memory-only credential adapter: ok"
    );
}

#[test]
fn catalogue_oauth_approvals_refresh_revocation_and_restart_are_end_to_end() {
    let mut f = Fixture::new();
    assert_eq!(f.state()["mcp"]["catalogue"].as_array().unwrap().len(), 6);
    assert!(f
        .app
        .api("POST", "/api/connectors/connect", br#"{"id":"unknown"}"#)
        .is_err());
    let denied = f.connect("gmail");
    assert_eq!(f.oauth.count("/token"), 0);
    assert_eq!(f.item("gmail")["authorization_url"], Value::Null);
    f.decide(&denied, json!("deny"));
    assert_eq!(f.item("gmail")["has_client"], true);
    assert_eq!(f.oauth.count("/token"), 0);
    assert_eq!(f.item("gmail")["status"], "disconnected");
    let run = f.connect("gmail");
    f.decide(&run, json!("allow_once"));
    let state = f.state();
    let request = state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == run)
        .unwrap();
    assert_eq!(request["connector"]["tool"], "Connecter Gmail");
    assert_eq!(request["connector"]["catalogue_id"], "gmail");
    assert!(request["configuration_result"]["output"]["message"]
        .as_str()
        .unwrap()
        .contains("navigateur"));
    assert_eq!(f.callback("gmail", false, true), 400);
    assert_eq!(f.oauth.count("/token"), 0);
    assert_eq!(f.callback("gmail", true, false), 400);
    assert_eq!(f.item("gmail")["status"], "reconnect");
    for id in ["gmail", "calendar", "drive", "slack", "notion", "github"] {
        let run = f.connect(id);
        f.decide(&run, json!("allow_once"));
        assert_eq!(f.item(id)["status"], "authorizing");
        assert_eq!(f.callback(id, false, false), 200);
        let item = f.item(id);
        assert_eq!(item["status"], "connected");
        assert!(item["server"]["enabled_bots"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(item["server"]["tools"].as_array().unwrap().len(), 1);
        assert!(!item.to_string().contains("access-secret"));
    }
    let server = f.item("gmail")["server"].clone();
    f.post(
        "/api/mcp/select",
        json!({"id":server["id"],"bot_id":"grokbot","enabled":true}),
    );
    let calls = f.oauth.count("/mcp");
    let run = f.tool_run(&server, "tool-refused");
    f.decide(&run, json!("deny"));
    assert_eq!(calls, f.oauth.count("/mcp"));
    let run = f.tool_run(&server, "tool-allowed");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run, json!({"always_allow":{"scope":"global"}}));
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert!(!format!("{:?}", f.outcome(&run)).contains("access-secret"));
    // A standing permission still uses the normal gate, and renewal happens only
    // within that admitted call, never during state polling or app startup.
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET expires_at=1 WHERE id='gmail'",
            [],
        )
        .unwrap();
    let before = f.oauth.count("/token");
    assert_eq!(f.item("gmail")["status"], "connected");
    assert_eq!(f.item("gmail")["can_refresh"], true);
    assert_eq!(f.item("gmail")["account"], "personne@example.test");
    assert!(f.item("gmail")["message"]
        .as_str()
        .unwrap()
        .contains("prochain appel autorisé"));
    assert_eq!(
        f.item("gmail")["server"]["enabled_bots"],
        json!(["grokbot"])
    );
    // Expiry without a refresh token, unlike normal expiry, requires a sign-in.
    let key = format!("MCP_{}_ENV", server["id"].as_str().unwrap()).to_uppercase();
    let vault = grokbot_wrapper::features::forms::vault(&f.app.feature_context()).unwrap();
    let original = vault.get(&key).unwrap().unwrap();
    let mut without_refresh: Value = serde_json::from_str(&original).unwrap();
    without_refresh["variables"]
        .as_object_mut()
        .unwrap()
        .remove("REFRESH_TOKEN");
    vault
        .set(
            &key,
            grokbot_forms::vault::MCP_ENV_PURPOSE,
            &without_refresh.to_string(),
        )
        .unwrap();
    assert_eq!(f.item("gmail")["status"], "reconnect");
    vault
        .set(&key, grokbot_forms::vault::MCP_ENV_PURPOSE, &original)
        .unwrap();
    assert_eq!(f.oauth.count("/token"), before);
    let run = f.tool_run(&server, "tool-standing-refresh");
    f.script(vec![json!({"type":"response.completed"})]);
    f.finish(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert_eq!(f.oauth.count("/token"), before + 1);
    f.oauth.seen.lock().unwrap().tools_error = true;
    let run = f.tool_run(&server, "tool-expired-remote");
    f.script(vec![json!({"type":"response.completed"})]);
    f.finish(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Failed { .. }));
    assert_eq!(f.item("gmail")["status"], "reconnect");
    f.oauth.seen.lock().unwrap().tools_error = false;
    let before = f.oauth.count("/token");
    let recovery = f.connect("gmail");
    f.decide(&recovery, json!("allow_once"));
    assert_eq!(f.item("gmail")["status"], "connected");
    assert_eq!(
        f.oauth.count("/token"),
        before + 1,
        "a rejected unexpired token must renew, not loop on the same token"
    );
    // State/configuration/catalogues persist, but no provider is contacted on restart.
    let pending = f.connect("notion");
    f.decide(&pending, json!("allow_once")); // valid token: starts without browser
    let provider = Provider::with_endpoints(&f.path, f.ai.endpoints(), false);
    let before = f.oauth.count("/mcp");
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    f.provider = provider;
    assert_eq!(f.item("drive")["status"], "saved");
    assert_eq!(f.oauth.count("/mcp"), before);
    f.oauth.seen.lock().unwrap().sse = true;
    let run = f.connect("drive");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("drive")["status"], "connected");
    // Revocation requires its own explicit decision; denial keeps credentials.
    let before = f.oauth.count("/revoke");
    let run = f.post("/api/connectors/disconnect", json!({"id":"drive"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("deny"));
    assert_eq!(before, f.oauth.count("/revoke"));
    assert_eq!(f.item("drive")["status"], "connected");
    let run = f.post("/api/connectors/disconnect", json!({"id":"drive"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("drive")["status"], "disconnected");
    assert!(f.item("drive")["server"].is_null());
    let removed_id: String = f
        .db()
        .query_row(
            "SELECT server_id FROM wrapper_connector_catalogue WHERE id='drive'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let remaining: Value = serde_json::from_str(
        &grokbot_wrapper::features::forms::vault(&f.app.feature_context())
            .unwrap()
            .get(&format!("MCP_{removed_id}_ENV").to_uppercase())
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(remaining["variables"].get("ACCESS_TOKEN").is_none());
    assert!(remaining["variables"].get("REFRESH_TOKEN").is_none());
    assert_eq!(f.item("drive")["has_client"], true);
    assert_eq!(f.item("drive")["account"], Value::Null);
    assert_eq!(before + 1, f.oauth.count("/revoke"));
    f.oauth.seen.lock().unwrap().revoke_error = true;
    let run = f.post("/api/connectors/disconnect", json!({"id":"slack"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("slack")["warning"], true);
    assert!(f.item("slack")["message"]
        .as_str()
        .unwrap()
        .contains("non confirmée"));
    // Bad token response is not logged, reflected or persisted.
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET expires_at=1 WHERE id='github'",
            [],
        )
        .unwrap();
    f.oauth.seen.lock().unwrap().token_error = true;
    let run = f.connect("github");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.callback("github", false, false), 400);
    assert_eq!(f.item("github")["status"], "reconnect");
    // A restart invalidates an unfinished, one-time callback without contacting
    // the service or reviving an old browser session.
    let before = f.oauth.count("/token");
    let pending = f.connect("github");
    f.decide(&pending, json!("allow_once"));
    assert_eq!(f.item("github")["status"], "authorizing");
    let provider = Provider::with_endpoints(&f.path, f.ai.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    f.provider = provider;
    assert_eq!(f.item("github")["status"], "reconnect");
    assert!(f.item("github")["authorization_url"].is_null());
    assert_eq!(f.oauth.count("/token"), before + 1); // only the failed refresh above
                                                     // Disconnect also cancels pending authorization and removes its vault value.
    let pending = f.connect("github");
    f.decide(&pending, json!("allow_once"));
    let removed = f.post("/api/connectors/disconnect", json!({"id":"github"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&removed, json!("allow_once"));
    assert_eq!(f.item("github")["status"], "disconnected");
    assert!(f.item("github")["authorization_url"].is_null());
    let public = f.state().to_string();
    for secret in [
        "access-secret",
        "refresh-secret",
        "secret-must-not-leak",
        "private-server-error",
    ] {
        assert!(!public.contains(secret));
        assert!(!std::fs::read(&f.path)
            .unwrap()
            .windows(secret.len())
            .any(|s| s == secret.as_bytes()));
    }
    let inputs = f.ai.seen.lock().unwrap().bodies.clone();
    assert!(!format!("{inputs:?}").contains("access-secret"));
    assert_eq!(
        store::required(&f.db(), &run).unwrap().status,
        RunStatus::Finished
    );
}
