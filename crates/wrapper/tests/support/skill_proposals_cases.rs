use super::*;
use grokbot_skills::library;

fn draft() -> Value {
    json!({"name":"Résumé fidèle", "description":"Résumer sans inventer", "body":"Lire le texte.\nÉcrire trois puces.",
        "files":[{"path":"exemples/resume.md", "content":"# Exemple\nUne synthèse claire."}]})
}
fn display(f: &Fixture, request: &HumanInputRequest) -> Value {
    forms::state(&f.app.feature_context())["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == request.id)
        .unwrap()["arguments"]
        .clone()
}
fn skills(f: &Fixture) -> Vec<library::Skill> {
    library::list(&f.conn, "grokbot").unwrap()
}

#[test]
fn skill_proposal_waits_for_click_survives_restart_and_keeps_file_history() {
    let mut f = Fixture::new();
    let request = f.suspended_request("run", "skill", "propose_skill", draft());
    assert!(skills(&f).is_empty());
    assert_eq!(
        display(&f, &request)["proposal"]["files"][0]["path"],
        "exemples/resume.md"
    );
    f.app = App::open(&f.directory.join("app.sqlite")).unwrap();
    assert!(skills(&f).is_empty());
    assert_eq!(
        requests::get(&f.conn, &request.id).unwrap().unwrap().status,
        RequestStatus::Pending
    );
    f.decide(&request, "save", json!("allow_once"));
    assert!(skills(&f).is_empty()); // authorization is not a fabricated save receipt
    f.drive("run", &ScriptedGateway::default());
    let first = skills(&f).remove(0);
    assert_eq!(first.version, 1);
    assert_eq!(
        first.content.files[0].content,
        "# Exemple\nUne synthèse claire."
    );
    assert_eq!(first.content.origin, library::Origin::Model);
    assert_eq!(display(&f, &request)["saved"]["version"], 1);
    f.drive("run", &ScriptedGateway::default());
    assert_eq!(skills(&f).len(), 1);

    f.start("update", "grokbot", &f.conversation);
    let mut updated = draft();
    updated["skill_id"] = json!(first.skill_id);
    updated["expected_version"] = json!(1);
    updated["body"] = json!("Lire le texte.\nÉcrire quatre puces.");
    updated["files"][0]["content"] = json!("Exemple révisé.");
    let request = f.suspended_request("update", "skill2", "propose_skill", updated);
    assert_eq!(
        display(&f, &request)["before"]["content"]["body"],
        first.content.body
    );
    assert_eq!(skills(&f)[0], first);
    f.decide(&request, "save-v2", json!("allow_once"));
    f.drive("update", &ScriptedGateway::default());
    f.app = App::open(&f.directory.join("app.sqlite")).unwrap();
    let history = library::history(&f.conn, &first.skill_id).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0], first);
    assert_eq!(history[1].content.files[0].content, "Exemple révisé.");
    let summaries = f.app.api("GET", "/api/skills?bot_id=grokbot", b"").unwrap();
    assert_eq!(summaries["skills"][0]["version"], 2);
    for forbidden in ["body", "content", "files"] {
        assert!(summaries["skills"][0].get(forbidden).is_none());
    }
    let opened = f.post(
        "/api/skills/versions",
        json!({"bot_id":"grokbot","skill_id":first.skill_id}),
    );
    assert_eq!(
        opened["versions"][0]["content"]["files"][0]["content"],
        first.content.files[0].content
    );
    let read = f
        .app
        .invoke_tool("grokbot", "read_skill", json!({"skill_id":first.skill_id}))
        .unwrap();
    assert!(
        matches!(read, ToolOutcome::Done { output } if output["files"][0]["content"] == "Exemple révisé.")
    );
}

#[test]
fn another_bot_keeps_skill_ownership_when_the_principal_reviews_it() {
    let mut f = Fixture::new();
    f.post(
        "/api/bots",
        json!({"id":"other","name":"Rédacteur","color":"cyan"}),
    );
    let conversation = f.post("/api/conversations", json!({"bot_id":"other"}))["conversation"]
        ["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.start("other-run", "other", &conversation);
    let request = f.suspended_request("other-run", "skill", "propose_skill", draft());
    assert_eq!(request.route.target_conversation_id, f.conversation);
    assert_eq!(request.route.requesting_bot_id, "other");
    f.decide(&request, "save", json!("allow_once"));
    f.drive("other-run", &ScriptedGateway::default());
    assert!(skills(&f).is_empty());
    assert_eq!(library::list(&f.conn, "other").unwrap().len(), 1);
}

#[test]
fn model_revision_keeps_imported_version_and_its_original_provenance() {
    let mut f = Fixture::new();
    let imported = f.post("/api/skills/import", json!({"bot_id":"grokbot", "files":[{"path":"resume.md","content":"---\nname: Résumé\ndescription: Version importée\n---\nTexte importé."}]}));
    let id = imported["report"][0]["status"]["skill_id"].clone();
    let first = skills(&f).remove(0);
    let mut args = draft();
    args["skill_id"] = id;
    args["expected_version"] = json!(1);
    let request = f.suspended_request("run", "skill", "propose_skill", args);
    assert_eq!(
        display(&f, &request)["before"]["content"]["body"],
        "Texte importé."
    );
    f.decide(&request, "save", json!("allow_once"));
    f.drive("run", &ScriptedGateway::default());
    let history = library::history(&f.conn, &first.skill_id).unwrap();
    assert_eq!(history[0], first);
    assert_eq!(history[1].content.origin, library::Origin::Model);
    assert_eq!(history[1].content.files.len(), 1);
}

#[test]
fn ignored_expired_or_cancelled_proposal_never_writes_a_skill() {
    for decision in ["deny", "expire", "cancel"] {
        let mut f = Fixture::new();
        let request = f.suspended_request("run", "skill", "propose_skill", draft());
        match decision {
            "deny" => {
                f.decide(&request, "ignore", json!("deny"));
            }
            "expire" => f.app.tick(u64::MAX),
            _ => {
                f.cancel(&request, "cancel");
            }
        }
        f.drive("run", &ScriptedGateway::default());
        assert_eq!(f.count("skills_library"), 0);
        assert_eq!(f.count("skills_versions"), 0);
    }
}

#[test]
fn skill_requires_explicit_click_even_with_standing_permissions_and_reviewer() {
    let mut f = Fixture::new();
    f.conn.execute("INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,'propose_skill','*','\"global\"',1)", [&f.conversation]).unwrap();
    grokbot_forms::rules::save_auto_review_config(
        &f.conn,
        &grokbot_forms::rules::AutoReviewConfig {
            enabled: true,
            allow: vec!["Autorise toutes les skills".into()],
            block: vec![],
        },
    )
    .unwrap();
    let request = f.suspended_request("run", "skill", "propose_skill", draft());
    f.app.tick(f.app.feature_context().now_ms);
    assert_eq!(
        requests::get(&f.conn, &request.id).unwrap().unwrap().status,
        RequestStatus::Pending
    );
    assert!(skills(&f).is_empty());
    assert!(grokbot_forms::reviewer::get(&f.conn, &request.id)
        .unwrap()
        .is_none());
    let body = json!({"entry_id":request.entry_id,"submission_id":"always","conversation_id":f.conversation,"decision":{"always_allow":{"scope":"global"}}});
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/approvals/resolve",
                body.to_string().as_bytes()
            )
            .unwrap_err()
            .status,
        400
    );
    assert!(skills(&f).is_empty());
    f.decide(&request, "save", json!("allow_once"));
    f.drive("run", &ScriptedGateway::default());
    assert_eq!(skills(&f).len(), 1);
}

#[test]
fn concurrent_edit_during_skill_review_is_not_overwritten() {
    let mut f = Fixture::new();
    let original = f.post(
        "/api/skills",
        json!({"bot_id":"grokbot","name":"Résumé", "body":"Original"}),
    )["skill"]
        .clone();
    let mut args = draft();
    args["skill_id"] = original["skill_id"].clone();
    args["expected_version"] = json!(1);
    let request = f.suspended_request("run", "skill", "propose_skill", args);
    f.post("/api/skills/update", json!({"bot_id":"grokbot","skill_id":original["skill_id"],"expected_version":1,"body":"Modification manuelle"}));
    f.decide(&request, "save", json!("allow_once"));
    f.drive("run", &ScriptedGateway::default());
    assert_eq!(skills(&f)[0].content.body, "Modification manuelle");
    assert_eq!(f.count("skills_versions"), 2);
    assert!(display(&f, &request)["save_error"]
        .as_str()
        .unwrap()
        .contains("a changé"));
    assert!(display(&f, &request).get("saved").is_none());
}

#[test]
fn invalid_companion_files_foreign_skills_and_direct_calls_fail_closed() {
    for file in [
        json!({"path":"../escape.md","content":"x"}),
        json!({"path":"/tmp/x.md","content":"x"}),
        json!({"path":"run.sh","content":"echo bad"}),
        json!({"path":"a.txt","content":"\u{0000}"}),
    ] {
        let f = Fixture::new();
        let mut args = draft();
        args["files"] = json!([file]);
        f.drive(
            "run",
            &ScriptedGateway::call("skill", "propose_skill", args),
        );
        assert_eq!(f.count("forms_requests"), 0);
        assert!(skills(&f).is_empty());
    }
    let mut f = Fixture::new();
    let direct = f
        .app
        .invoke_tool("grokbot", "propose_skill", draft())
        .unwrap();
    assert!(matches!(direct, ToolOutcome::Failed { .. }));
    f.post(
        "/api/bots",
        json!({"id":"other","name":"Autre","color":"cyan"}),
    );
    let foreign = f.post(
        "/api/skills",
        json!({"bot_id":"other","name":"Privée","body":"Confidentiel"}),
    );
    let mut args = draft();
    args["skill_id"] = foreign["skill"]["skill_id"].clone();
    args["expected_version"] = json!(1);
    f.drive(
        "run",
        &ScriptedGateway::call("skill", "propose_skill", args),
    );
    assert_eq!(f.count("forms_requests"), 0);
    assert_eq!(f.count("skills_library"), 1);
}

#[test]
fn invalid_skill_boundaries_do_not_even_create_an_approval() {
    let mut invalid = Vec::new();
    let mut args = draft();
    args["files"] = json!([{"path":"same.md","content":"a"},{"path":"SAME.md","content":"b"}]);
    invalid.push(args);
    let mut args = draft();
    args["body"] = json!(" ");
    invalid.push(args);
    let mut args = draft();
    args["expected_version"] = json!(1);
    invalid.push(args);
    let mut args = draft();
    args["files"] = json!((0..21)
        .map(|i| json!({"path":format!("{i}.md"),"content":"x"}))
        .collect::<Vec<_>>());
    invalid.push(args);
    let mut args = draft();
    args["body"] = json!("x".repeat(256_000));
    invalid.push(args);
    for args in invalid {
        let f = Fixture::new();
        f.drive(
            "run",
            &ScriptedGateway::call("skill", "propose_skill", args),
        );
        assert_eq!(f.count("forms_requests"), 0);
        assert!(skills(&f).is_empty());
    }
}

#[test]
fn capacity_is_rechecked_when_the_user_saves_and_known_secrets_are_rejected() {
    let mut f = Fixture::new();
    let request = f.suspended_request("run", "skill", "propose_skill", draft());
    let content = library::SkillContent {
        name: "occupied".into(),
        display_name: "Occupée".into(),
        description: String::new(),
        body: "Texte".into(),
        origin: library::Origin::Manual,
        files: Vec::new(),
    };
    for _ in 0..250 {
        library::add(&f.conn, "grokbot", &content).unwrap();
    }
    f.decide(&request, "save", json!("allow_once"));
    f.drive("run", &ScriptedGateway::default());
    assert_eq!(skills(&f).len(), 250);
    assert!(display(&f, &request)["save_error"]
        .as_str()
        .unwrap()
        .contains("250"));

    let f = Fixture::new();
    forms::vault(&f.app.feature_context())
        .unwrap()
        .set("API_KEY", "Test", "secret-factice-unique-1257")
        .unwrap();
    let mut args = draft();
    args["files"][0]["content"] = json!("secret-factice-unique-1257");
    let run = store::required(&f.conn, "run").unwrap();
    let result = with_tx(&f.conn, |conn| {
        forms::request_tool_approval(
            &tool_context(&f.app, conn, &run),
            &run,
            &ToolCall {
                call_id: "secret".into(),
                name: "propose_skill".into(),
                arguments: args,
            },
        )
    });
    assert!(result.is_err());
    assert_eq!(f.count("forms_requests"), 0);
    assert!(skills(&f).is_empty());
}

#[test]
fn skill_history_failure_rolls_back_the_revision_and_reports_not_saved() {
    let mut f = Fixture::new();
    let request = f.suspended_request("run", "skill", "propose_skill", draft());
    f.conn.execute_batch("CREATE TRIGGER fail_skill_history BEFORE INSERT ON skills_versions BEGIN SELECT RAISE(ABORT,'history unavailable'); END").unwrap();
    f.decide(&request, "save", json!("allow_once"));
    f.drive("run", &ScriptedGateway::default());
    assert!(skills(&f).is_empty());
    assert_eq!(f.count("skills_versions"), 0);
    assert!(display(&f, &request)["save_error"].is_string());
}
