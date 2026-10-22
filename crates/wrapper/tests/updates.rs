//! La route expose la version embarquée et ne lance pas de compilation sans recherche.
use grokbot_wrapper::App;

#[test]
fn la_route_expose_la_version_sans_toucher_aux_sessions() {
    let root = std::env::temp_dir().join(format!("ezerbot-route-updates-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let db = root.join("workspace.sqlite");
    let mut app = App::open(&db).unwrap();
    let body = app.api("GET", "/api/updates", b"").unwrap();
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    let commit = body["installed_commit"].as_str().unwrap();
    assert!(
        commit == "inconnu"
            || commit
                .chars()
                .all(|character| character.is_ascii_hexdigit()),
        "{commit}"
    );
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["updates"]["version"], body["version"]);
    assert_eq!(
        state["updates"]["installed_commit"],
        body["installed_commit"]
    );
    let error = app.api("POST", "/api/updates/apply", b"{}").unwrap_err();
    assert!(error.message.contains("Recherchez"), "{}", error.message);
    assert!(db.is_file());
    let size = std::fs::metadata(&db).unwrap().len();
    assert!(size > 0);
    let _ = std::fs::remove_dir_all(&root);
}
