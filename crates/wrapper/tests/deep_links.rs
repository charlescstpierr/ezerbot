use grokbot_wrapper::{
    deep_links::{parse_link, Destination, SETTINGS},
    App,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ezerbot-links-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> App {
        App::open(&self.0.join("workspace.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn post(app: &mut App, route: &str, body: Value) -> Value {
    app.api("POST", route, body.to_string().as_bytes()).unwrap()
}

#[test]
fn grammar_canonicalizes_without_treating_text_as_a_command() {
    let first =
        parse_link("EZERBOT://APP/v1/open?text=Salut+%C3%A9%21%0A%3Cscript%3E&agent=grokbot")
            .unwrap();
    assert_eq!(
        first.canonical,
        "ezerbot://app/v1/open?agent=grokbot&text=Salut%20%C3%A9%21%0A%3Cscript%3E"
    );
    assert_eq!(first, parse_link(&first.canonical).unwrap());
    assert_eq!(
        first.destination,
        Destination::Compose {
            text: "Salut é!\n<script>".into(),
            agent: Some("grokbot".into())
        }
    );
    let thread = parse_link("ezerbot://app/v1/thread?id=task%3Achild%3Aparent").unwrap();
    assert!(matches!(
        thread.destination,
        Destination::Conversation { .. }
    ));
    for section in SETTINGS {
        assert!(parse_link(&format!("ezerbot://app/v1/settings?id={section}")).is_ok());
    }
    assert_eq!(
        parse_link("ezerbot://app/v1/settings?id=usage")
            .unwrap()
            .destination,
        Destination::Settings { id: "usage".into() }
    );
}

#[test]
fn grammar_denies_ambiguous_or_privileged_links() {
    for raw in [
        "https://cursor.com/grok-bot/link",
        "grokbot://app/v1/agent?id=grokbot",
        "ezerbot://other/v1/agent?id=grokbot",
        "ezerbot://app:80/v1/agent?id=grokbot",
        "ezerbot://user@app/v1/agent?id=grokbot",
        "ezerbot://app/v1/agent?id=grokbot#fragment",
        "ezerbot://app/v1/agent?id=grokbot&id=other",
        "ezerbot://app/v1/agent?%69d=grokbot",
        "ezerbot://app/v1/agent?id=grokbot&send=true",
        "ezerbot://app/v1/agent?id=%GG",
        "ezerbot://app/v1/agent?id=%FF",
        "ezerbot://app/v1/agent?id=%00",
        "ezerbot://app/v1/agent?id=..",
        "ezerbot://app/v1/%61gent?id=grokbot",
        "ezerbot://app/v1/../v1/agent?id=grokbot",
        "ezerbot://app\\v1/agent?id=grokbot",
        "ezerbot://app/v1/settings?id=secrets",
        "ezerbot://app/v1/open?text=",
        "ezerbot://app/v1/open?text=hi%00",
        "ezerbot://app/v1/open?text=hi&approve=true",
        "ezerbot://app/v1/skill?id=one",
        "ezerbot://app/v1/routine?id=one&agent=bot&run=true",
        "ezerbot://app/v1/approve?id=one",
        "ezerbot://app/v1/tool?name=run_command",
        "ezerbot://app/v1/open?text=hello world",
        "ezerbot://app/v1/open?text=hi\n",
    ] {
        assert!(parse_link(raw).is_err(), "accepted {raw:?}");
    }
    assert!(parse_link(&format!("ezerbot://app/v1/open?text={}", "x".repeat(2048))).is_err());
}

#[test]
fn host_deduplication_survives_renderer_reload_expires_and_is_bounded() {
    let fixture = Fixture::new();
    let mut app = fixture.open();
    let raw = "ezerbot://app/v1/open?text=Bonjour+ami";
    assert!(!app.admit_link(raw, 100).unwrap().1);
    // Same App, brand new renderer: canonical spelling still cannot reopen.
    assert!(
        app.admit_link("EZERBOT://APP/v1/open?text=Bonjour%20ami", 101)
            .unwrap()
            .1
    );
    assert!(app.admit_link(raw, 2099).unwrap().1);
    assert!(!app.admit_link(raw, 2100).unwrap().1);
    for index in 0..15 {
        assert!(
            !app.admit_link(&format!("ezerbot://app/v1/open?text={index}"), 2101)
                .unwrap()
                .1
        );
    }
    assert_eq!(
        app.admit_link("ezerbot://app/v1/open?text=overflow", 2101)
            .unwrap_err()
            .status,
        429
    );
    assert!(
        !app.admit_link("ezerbot://app/v1/open?text=overflow", 4101)
            .unwrap()
            .1
    );
    assert_eq!(
        app.admit_link("ezerbot://app/v1/agent?id=missing", 4102)
            .unwrap_err()
            .status,
        404
    );
    assert!(
        app.admit_link("EZERBOT://APP/v1/agent?id=missing", 4103)
            .unwrap()
            .1
    );
    drop(app);
    assert!(
        !fixture.open().admit_link(raw, 4102).unwrap().1,
        "no persisted admission/replay after app restart"
    );
}

#[test]
fn routes_resolve_only_owned_active_targets_and_survive_restart_without_actions() {
    let fixture = Fixture::new();
    let mut app = fixture.open();
    post(
        &mut app,
        "/api/bots",
        json!({"id":"other", "name":"Autre", "color":"cyan"}),
    );
    let conversation = post(&mut app, "/api/conversations", json!({"bot_id":"grokbot"}))
        ["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let skill = post(&mut app, "/api/skills", json!({"bot_id":"grokbot","name":"Résumé","body":"Privé — ne pas retourner dans le lien","description":"Test"}))["skill"]["skill_id"].as_str().unwrap().to_owned();
    let routine = post(
        &mut app,
        "/api/routines",
        json!({"bot_id":"grokbot","name":"Test","prompt":"Privé","schedule":"@every 1h"}),
    )["routine"]["routine_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let links = [
        "ezerbot://app/v1/agent?id=grokbot".into(),
        format!("ezerbot://app/v1/thread?id={conversation}&agent=grokbot"),
        format!("ezerbot://app/v1/skill?id={skill}&agent=grokbot"),
        format!("ezerbot://app/v1/routine?id={routine}&agent=grokbot"),
        "ezerbot://app/v1/open?text=Ex%C3%A9cute%20un%20outil&agent=grokbot".into(),
        "ezerbot://app/v1/settings?id=appearance".into(),
    ];
    for restart in 0..2 {
        if restart == 1 {
            drop(app);
            app = fixture.open();
        }
        let before = app.api("GET", "/api/state", b"").unwrap();
        for link in &links {
            let value = post(&mut app, "/api/deep-links/resolve", json!({"url":link}));
            assert!(value["destination"]["kind"].is_string());
            assert!(!value.to_string().contains("Privé"));
            assert_eq!(value["duplicate"], false);
            let repeated = post(&mut app, "/api/deep-links/resolve", json!({"url":link}));
            assert_eq!(repeated["duplicate"], true);
            assert_eq!(repeated["destination"], value["destination"]);
        }
        let after = app.api("GET", "/api/state", b"").unwrap();
        assert_eq!(
            before["conversations"], after["conversations"],
            "no send/new persisted conversation"
        );
        assert_eq!(
            before["routines"], after["routines"],
            "no routine start or enable"
        );
        assert_eq!(before["forms"], after["forms"], "no approval or disclosure");
    }
    for url in [
        "ezerbot://app/v1/agent?id=missing".into(),
        format!("ezerbot://app/v1/thread?id={conversation}&agent=other"),
        format!("ezerbot://app/v1/skill?id={skill}&agent=other"),
        format!("ezerbot://app/v1/routine?id={routine}&agent=other"),
        "ezerbot://app/v1/open?text=Bonjour&agent=missing".into(),
    ] {
        let error = app.resolve_link(&url).unwrap_err();
        assert_eq!(error.status, 404);
        assert!(error.message.contains("introuvable ou inaccessible"));
    }
    post(
        &mut app,
        "/api/skills/delete",
        json!({"bot_id":"grokbot", "skill_id":skill, "expected_version":1}),
    );
    assert_eq!(
        app.resolve_link(&format!("ezerbot://app/v1/skill?agent=grokbot&id={skill}"))
            .unwrap_err()
            .status,
        404
    );
    post(
        &mut app,
        "/api/routines/delete",
        json!({"bot_id":"grokbot", "routine_id":routine}),
    );
    assert_eq!(
        app.resolve_link(&format!(
            "ezerbot://app/v1/routine?agent=grokbot&id={routine}"
        ))
        .unwrap_err()
        .status,
        404
    );
    post(
        &mut app,
        "/api/conversations/delete",
        json!({"id": conversation}),
    );
    assert_eq!(
        app.resolve_link(&format!("ezerbot://app/v1/thread?id={conversation}"))
            .unwrap_err()
            .status,
        404
    );
    post(&mut app, "/api/bots/delete", json!({"id":"other"}));
    assert_eq!(
        app.resolve_link("ezerbot://app/v1/agent?id=other")
            .unwrap_err()
            .status,
        404
    );
    assert!(app
        .api(
            "POST",
            "/api/deep-links/resolve",
            br#"{"url":"ezerbot://app/v1/agent?id=grokbot","execute":true}"#
        )
        .is_err());
    assert!(app
        .api("POST", "/api/deep-links/resolve", b"not json")
        .is_err());
}
