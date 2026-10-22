#![cfg(debug_assertions)]
//! Routine webhook, observed from outside: the real binary, its loopback webhook
//! listener, the routine history and the routine thread. Tailscale is a fake CLI
//! in a throwaway HOME; no test reaches a real tailnet or the Internet.
mod support;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use support::wait;

const DNS: &str = "mac.qa.ts.net";

struct Host {
    child: Option<Child>,
    root: PathBuf,
    url: String,
    hook: String,
}
impl Host {
    fn new() -> Self {
        // The clock alone can repeat across parallel tests: add a counter.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ezerbot-webhook-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        // Behaviour is switched by files, so a test can stop Tailscale between calls.
        let fake = root.join("tailscale-fake");
        let script = format!(
            r#"#!/bin/sh
ROOT='{root}'
printf '%s\n' "$*" >> "$ROOT/commands"
mode=running; [ -f "$ROOT/ts-mode" ] && mode=$(cat "$ROOT/ts-mode")
case "$1" in
  status)
    if [ "$mode" = stopped ]; then
      printf '%s\n' '{{"BackendState":"Stopped","Self":{{"DNSName":"{dns}."}}}}'
    else
      printf '%s\n' '{{"BackendState":"Running","Self":{{"DNSName":"{dns}."}},"CurrentTailnet":{{"Name":"qa"}}}}'
    fi ;;
  funnel)
    [ "$mode" = refused ] && exit 1
    case "$*" in
      *" off") rm -f "$ROOT/funnel-on" ;;
      *) [ "$mode" = unconfirmed ] || touch "$ROOT/funnel-on" ;;
    esac ;;
  serve)
    if [ "$2" = status ] && [ -f "$ROOT/funnel-on" ]; then
      printf '%s\n' '{{"TCP":{{"8443":{{"HTTPS":true}}}},"Web":{{"{dns}:8443":{{"Handlers":{{"/hooks":{{"Proxy":"http://127.0.0.1:'"$(cat "$ROOT/hook-port")"'"}}}}}}}},"AllowFunnel":{{"{dns}:8443":true}}}}'
    else
      printf '%s\n' '{{}}'
    fi ;;
esac
"#,
            root = root.display(),
            dns = DNS
        );
        fs::write(&fake, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
        let mut host = Self {
            child: None,
            root,
            url: String::new(),
            hook: String::new(),
        };
        host.open();
        host
    }
    fn open(&mut self) {
        let hook_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        fs::write(self.root.join("hook-port"), hook_port.to_string()).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
            .args(["--port", "0", "--db"])
            .arg(self.root.join("workspace.sqlite"))
            .current_dir(&self.root)
            .env("HOME", &self.root)
            .env("CFFIXED_USER_HOME", &self.root)
            .env("GROKBOT_FAKE_TAILSCALE", self.root.join("tailscale-fake"))
            .env("GROKBOT_WEBHOOK_PORT", hook_port.to_string())
            .env_remove("GROKBOT_FAKE_OPENAI")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        self.url = serde_json::from_str::<Value>(&line).unwrap()["url"]
            .as_str()
            .unwrap()
            .into();
        self.hook = format!("http://127.0.0.1:{hook_port}");
        self.child = Some(child);
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    fn state(&self) -> Value {
        ureq::get(&format!("{}/api/state", self.url))
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn get(&self, path: &str) -> Value {
        ureq::get(&format!("{}{path}", self.url))
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn post(&self, path: &str, body: Value) -> Value {
        ureq::post(&format!("{}{path}", self.url))
            .set("X-Grokbot-Request", "local")
            .send_json(body)
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn webhook_routine(&self, name: &str) -> Hook {
        let created = self.post(
            "/api/routines",
            json!({"bot_id":"grokbot","name":name,"prompt":"Traite l’appel reçu.","trigger":{"kind":"webhook"},"enabled":true}),
        );
        Hook {
            routine_id: created["routine"]["routine_id"].as_str().unwrap().into(),
            conversation_id: created["routine"]["conversation_id"]
                .as_str()
                .unwrap()
                .into(),
            trigger_id: created["webhook"]["trigger_id"].as_str().unwrap().into(),
            token: created["webhook"]["token"].as_str().unwrap().into(),
        }
    }
    fn history(&self, hook: &Hook) -> Value {
        self.get(&format!(
            "/api/routines/history?bot_id=grokbot&routine_id={}",
            hook.routine_id
        ))
    }
    fn runs(&self, hook: &Hook) -> usize {
        self.history(hook)["occurrences"].as_array().unwrap().len()
    }
    fn thread(&self, hook: &Hook) -> String {
        let state = self.state();
        let conversation = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == hook.conversation_id)
            .unwrap();
        conversation["messages"].to_string()
    }
    fn call(
        &self,
        hook: &Hook,
        token: Option<&str>,
        key: Option<&str>,
        body: &[u8],
    ) -> (u16, Value) {
        let mut request = ureq::post(&format!(
            "{}/hooks/{}/{}",
            self.hook, hook.routine_id, hook.trigger_id
        ))
        .set("Content-Type", "application/json");
        if let Some(token) = token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        if let Some(key) = key {
            request = request.set("Idempotency-Key", key);
        }
        match request.send_bytes(body) {
            Ok(r) => (r.status(), r.into_json().unwrap_or(Value::Null)),
            Err(ureq::Error::Status(code, r)) => (code, r.into_json().unwrap_or(Value::Null)),
            Err(error) => panic!("{error}"),
        }
    }
    fn wait_listener(&self) {
        wait("webhook listener", || {
            std::net::TcpStream::connect(self.hook.trim_start_matches("http://")).is_ok()
                && self.state()["routine_webhooks"]["listener"]["listening"] == true
        });
    }
    fn commands(&self) -> String {
        fs::read_to_string(self.root.join("commands")).unwrap_or_default()
    }
    fn tailscale(&self, mode: &str) {
        fs::write(self.root.join("ts-mode"), mode).unwrap();
    }
    fn approve_funnel(&self, enabled: bool) {
        let queued = self.post("/api/routines/funnel", json!({"enabled":enabled}));
        let mut request = Value::Null;
        wait("funnel approval card", || {
            request = self.state()["forms"]["requests"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["run_id"] == queued["run_id"])
                .cloned()
                .unwrap_or(Value::Null);
            !request.is_null()
        });
        self.post(
            "/api/approvals/resolve",
            json!({"entry_id":request["entry_id"],"submission_id":format!("decision-{}", now()),"conversation_id":request["route"]["target_conversation_id"],"decision":"allow_once"}),
        );
        wait("funnel decision settled", || {
            self.state()["forms"]["requests"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["run_id"] == queued["run_id"] && !r["configuration_result"].is_null())
        });
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct Hook {
    routine_id: String,
    conversation_id: String,
    trigger_id: String,
    token: String,
}
fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

#[test]
fn an_authenticated_post_starts_one_webhook_run_with_its_json_in_the_thread() {
    let host = Host::new();
    let hook = host.webhook_routine("Déploiement");
    host.wait_listener();
    let (code, _) = host.call(
        &hook,
        Some(&hook.token),
        None,
        br#"{"event":"deploy","build":4242}"#,
    );
    assert_eq!(code, 200);
    let history = host.history(&hook);
    let runs = history["occurrences"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["trigger"]["kind"], "webhook");
    assert_eq!(runs[0]["trigger"]["label"], "Webhook");
    assert!(runs[0]["trigger"]["received_at"].as_i64().unwrap() > 0);
    wait("the JSON reaches the routine thread", || {
        host.thread(&hook).contains("4242")
    });
    assert!(host.thread(&hook).contains("deploy"));
    // Each authenticated call without a key is its own run.
    assert_eq!(host.call(&hook, Some(&hook.token), None, b"{}").0, 200);
    assert_eq!(host.runs(&hook), 2);
}

#[test]
fn a_missing_or_wrong_token_leaves_the_history_unchanged() {
    let host = Host::new();
    let hook = host.webhook_routine("Protégée");
    host.wait_listener();
    assert_eq!(host.call(&hook, None, None, b"{}").0, 401);
    assert_eq!(
        host.call(&hook, Some("pas-le-bon-jeton"), None, b"{}").0,
        401
    );
    assert_eq!(host.runs(&hook), 0);
    assert!(!host.thread(&hook).contains("pas-le-bon-jeton"));
}

#[test]
fn unknown_routines_and_routines_without_webhook_answer_404() {
    let host = Host::new();
    let hook = host.webhook_routine("Connue");
    let hourly = host.post(
        "/api/routines",
        json!({"bot_id":"grokbot","name":"Horaire","prompt":"Bilan","schedule":"@every 1h"}),
    );
    host.wait_listener();
    let unknown = Hook {
        routine_id: "routine-inconnue".into(),
        conversation_id: String::new(),
        trigger_id: hook.trigger_id.clone(),
        token: hook.token.clone(),
    };
    assert_eq!(host.call(&unknown, Some(&hook.token), None, b"{}").0, 404);
    let without_webhook = Hook {
        routine_id: hourly["routine"]["routine_id"].as_str().unwrap().into(),
        conversation_id: String::new(),
        trigger_id: hook.trigger_id.clone(),
        token: hook.token.clone(),
    };
    assert_eq!(
        host.call(&without_webhook, Some(&hook.token), None, b"{}")
            .0,
        404
    );
    assert_eq!(host.runs(&hook), 0);
}

#[test]
fn a_paused_routine_answers_409_and_an_oversized_body_413() {
    let host = Host::new();
    let hook = host.webhook_routine("Bornée");
    host.wait_listener();
    let big = vec![b' '; 256_001];
    assert_eq!(host.call(&hook, Some(&hook.token), None, &big).0, 413);
    assert_eq!(host.runs(&hook), 0);
    host.post(
        "/api/routines/pause",
        json!({"bot_id":"grokbot","routine_id":hook.routine_id}),
    );
    assert_eq!(host.call(&hook, Some(&hook.token), None, b"{}").0, 409);
    assert_eq!(host.runs(&hook), 0);
}

#[test]
fn the_same_idempotency_key_leaves_a_single_run() {
    let host = Host::new();
    let hook = host.webhook_routine("Idempotente");
    host.wait_listener();
    assert_eq!(
        host.call(&hook, Some(&hook.token), Some("retry-1"), b"{\"n\":1}")
            .0,
        200
    );
    assert_eq!(
        host.call(&hook, Some(&hook.token), Some("retry-1"), b"{\"n\":1}")
            .0,
        409
    );
    assert_eq!(host.runs(&hook), 1);
    assert_eq!(
        host.call(&hook, Some(&hook.token), Some("retry-2"), b"{\"n\":2}")
            .0,
        200
    );
    assert_eq!(host.runs(&hook), 2);
}

#[test]
fn regenerating_the_token_kills_the_old_one_and_never_reaches_the_thread() {
    let host = Host::new();
    let hook = host.webhook_routine("Régénérée");
    host.wait_listener();
    assert_eq!(host.call(&hook, Some(&hook.token), None, b"{}").0, 200);
    let fresh = host.post(
        "/api/routines/webhook/regenerate",
        json!({"bot_id":"grokbot","routine_id":hook.routine_id}),
    )["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(fresh, hook.token);
    assert_eq!(host.call(&hook, Some(&hook.token), None, b"{}").0, 401);
    assert_eq!(host.call(&hook, Some(&fresh), None, b"{}").0, 200);
    assert_eq!(host.runs(&hook), 2);
    let everything = host.state().to_string() + &host.history(&hook).to_string();
    assert!(!everything.contains(&hook.token));
    assert!(!everything.contains(&fresh));
}

#[test]
fn funnel_publishes_only_after_approval_and_only_on_its_own_port() {
    let host = Host::new();
    let hook = host.webhook_routine("Publique");
    host.wait_listener();
    let before = host.get("/api/routines/funnel");
    assert_eq!(before["state"], "off");
    assert!(before["public_base"].is_null());
    assert!(!host.commands().contains("funnel"));
    host.approve_funnel(true);
    let funnel = host.get("/api/routines/funnel");
    assert_eq!(funnel["state"], "published");
    assert_eq!(funnel["public_base"], format!("https://{DNS}:8443/hooks"));
    let state = host.state();
    let item = &state["routine_webhooks"]["items"][&hook.routine_id];
    assert_eq!(
        item["url"],
        format!(
            "https://{DNS}:8443/hooks/{}/{}",
            hook.routine_id, hook.trigger_id
        )
    );
    let commands = host.commands();
    assert!(commands.contains("funnel"));
    assert!(commands.contains("--https=8443"));
    assert!(!commands.contains("--https=443"));
    assert!(!commands.contains("serve --bg"));
    // The iPhone keeps its own private address, distinct from the public webhook URL.
    let private = host.get("/api/iphone/tailscale");
    assert_eq!(private["url"], format!("https://{DNS}"));
    assert_ne!(private["url"], funnel["public_base"]);
    // Tailscale stopping later withdraws the public URL from the panel.
    host.tailscale("stopped");
    assert_eq!(
        host.get("/api/routines/funnel")["state"],
        "tailscale_stopped"
    );
    assert!(host.state()["routine_webhooks"]["items"][&hook.routine_id]["url"].is_null());
    // A refused withdrawal never records "off": the endpoint may still be public.
    host.tailscale("running");
    host.get("/api/routines/funnel");
    host.tailscale("refused");
    host.approve_funnel(false);
    let stuck = host.get("/api/routines/funnel");
    assert_eq!(stuck["state"], "withdraw_failed");
    assert_eq!(stuck["enabled"], true);
    assert!(stuck["public_base"].is_null());
    // Withdrawing goes through the same approval card and the same dedicated port.
    host.tailscale("running");
    host.approve_funnel(false);
    assert_eq!(host.get("/api/routines/funnel")["state"], "off");
    assert!(host
        .commands()
        .contains("--https=8443 --set-path=/hooks off"));
    assert!(!host.commands().contains("--https=443"));
}

#[test]
fn a_body_that_is_not_json_is_refused_without_a_run() {
    let host = Host::new();
    let hook = host.webhook_routine("Stricte");
    host.wait_listener();
    assert_eq!(
        host.call(&hook, Some(&hook.token), None, b"pas du json").0,
        400
    );
    assert_eq!(host.runs(&hook), 0);
    assert_eq!(host.call(&hook, Some(&hook.token), None, b"").0, 200);
    assert_eq!(host.runs(&hook), 1);
}

#[test]
fn the_webhook_listener_is_loopback_only_and_closed_without_any_webhook() {
    let host = Host::new();
    let port = host.hook.rsplit(':').next().unwrap().to_owned();
    std::thread::sleep(std::time::Duration::from_millis(700));
    assert!(std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_err());
    host.webhook_routine("Écoute");
    host.wait_listener();
    assert_eq!(
        host.state()["routine_webhooks"]["listener"]["listening"],
        true
    );
}

#[test]
fn without_a_confirmed_funnel_the_panel_gives_the_reason_and_no_public_url() {
    let host = Host::new();
    let hook = host.webhook_routine("Muette");
    host.wait_listener();
    host.tailscale("stopped");
    let funnel = host.get("/api/routines/funnel");
    assert_eq!(funnel["state"], "tailscale_stopped");
    assert!(funnel["public_base"].is_null());
    assert!(funnel["reason"].as_str().unwrap().contains("Tailscale"));
    host.tailscale("refused");
    host.approve_funnel(true);
    let refused = host.get("/api/routines/funnel");
    assert_eq!(refused["state"], "refused");
    assert!(refused["public_base"].is_null());
    host.tailscale("unconfirmed");
    host.approve_funnel(true);
    let unconfirmed = host.get("/api/routines/funnel");
    assert_eq!(unconfirmed["state"], "unconfirmed");
    assert!(unconfirmed["public_base"].is_null());
    let state = host.state();
    assert!(state["routine_webhooks"]["items"][&hook.routine_id]["url"].is_null());
}

#[test]
fn an_hourly_routine_still_exists_lists_and_is_not_a_webhook() {
    let host = Host::new();
    let created = host.post(
        "/api/routines",
        json!({"bot_id":"grokbot","name":"Horaire","prompt":"Bilan","schedule":"@every 1h"}),
    );
    assert!(created["webhook"].is_null());
    assert_eq!(created["routine"]["trigger"]["kind"], "every");
    let listed = host.get("/api/routines?bot_id=grokbot");
    assert_eq!(listed["routines"][0]["name"], "Horaire");
    assert_eq!(
        listed["routines"][0]["schedule_description"],
        "Chaque heure, à partir de l’activation"
    );
    assert!(host.state()["routine_webhooks"]["items"]
        .as_object()
        .unwrap()
        .is_empty());
}

#[test]
fn a_stalled_authenticated_upload_does_not_freeze_the_host() {
    use std::io::Write;
    let host = Host::new();
    let hook = host.webhook_routine("Lente");
    host.wait_listener();
    let mut stalled =
        std::net::TcpStream::connect(host.hook.trim_start_matches("http://")).unwrap();
    write!(
        stalled,
        "POST /hooks/{}/{} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{{\"par",
        hook.routine_id, hook.trigger_id, hook.token
    )
    .unwrap();
    stalled.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    // The local API and a second webhook call keep answering while the upload hangs.
    let started = std::time::Instant::now();
    assert!(host.state()["routine_webhooks"]["listener"]["listening"] == true);
    assert_eq!(host.call(&hook, Some(&hook.token), None, b"{}").0, 200);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(host.runs(&hook), 1);
    drop(stalled);
}
