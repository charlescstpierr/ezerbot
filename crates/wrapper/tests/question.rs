//! `attendre_reponse` through the real model loop: the journal the engine writes is what
//! `GET /api/taches` reads, so the tool, the journal and the route are checked together.
mod support;
use grokbot_wrapper::{App, Provider};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

fn script(f: &FakeOpenAi, events: Vec<Value>) {
    f.seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::events(&events));
}

#[test]
fn a_question_asked_with_the_tool_waits_in_to_decide_until_charles_writes() {
    let guard = grokbot_test_support::TestDir::new("ezerbot-question");
    let root = guard.to_path_buf();
    let path = root.join("workspace.sqlite");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(path.with_extension("chatgpt-auth.json"), json!({"host_id":"test-host","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
    let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
    let provider = Provider::with_endpoints(&path, server.endpoints(), false);
    let mut app = App::open_with(&path, provider.clone()).unwrap();
    wait("models", || provider.status()["models_ready"] == true);
    let mut api =
        |path: &str, body: Value| app.api("POST", path, body.to_string().as_bytes()).unwrap();
    let conversation = api("/api/conversations", json!({"title":"Question"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let call = json!({"type":"response.output_item.done","item":{"id":"fc","type":"function_call","call_id":"ask","name":"attendre_reponse","arguments":json!({"question":"Je prends quelle option ?"}).to_string()}});
    script(&server, vec![call, json!({"type":"response.completed"})]);
    script(
        &server,
        vec![
            json!({"type":"response.output_text.delta","delta":"J'attends."}),
            json!({"type":"response.completed"}),
        ],
    );
    api(
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"m1","body":"Choisis."}),
    );
    let questions = |app: &mut App| -> Vec<Value> {
        let mut items = app.api("GET", "/api/taches", b"").unwrap()["items"]
            .as_array()
            .unwrap()
            .clone();
        items.retain(|i| i["kind"] == "question");
        items
    };
    wait("question line", || !questions(&mut app).is_empty());
    let lines = questions(&mut app);
    assert_eq!(lines[0]["question"], "Je prends quelle option ?");
    assert_eq!(lines[0]["conversation_id"], conversation);
    assert_eq!(lines[0]["group"], "decision");
    script(
        &server,
        vec![
            json!({"type":"response.output_text.delta","delta":"Merci."}),
            json!({"type":"response.completed"}),
        ],
    );
    app.api(
        "POST",
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"m2","body":"Option B."})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert!(questions(&mut app).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
