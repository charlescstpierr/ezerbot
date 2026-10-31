//! The wrapper against real freedesktop services on a throwaway bus and HOME: the `ezerbot://`
//! registration, `xdg-open`, and the notification bridge.
#![cfg(all(target_os = "linux", debug_assertions))]
#[path = "support/launch_identity.rs"]
mod launch_identity;
#[path = "support/linux_host.rs"]
mod linux_host;
use grokbot_linux_desktop::{
    open,
    testing::{recording_handler, tools, wait_for, FakeNotifications, Home, PrivateBus},
};
use grokbot_wrapper::App;
use rusqlite::{params, Connection};
use serde_json::json;
use std::{ffi::OsStr, fs, path::PathBuf, process::Command};

const BODY: &str = "Ouvrez Ezerbot pour consulter cet événement.";

fn register(home: &Home, handler: &std::path::Path) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"));
    home.env(&mut command)
        .arg("--register-url-handler")
        .arg(handler);
    command.output().unwrap()
}

#[test]
fn register_url_handler_makes_xdg_open_start_the_handler_with_the_exact_link() {
    if !tools(&["xdg-open", "xdg-mime"]) {
        return;
    }
    let home = Home::new("ezw");
    let log = home.path().join("received.log");
    let handler = recording_handler(home.path(), &log);
    for _ in 0..2 {
        let output = register(&home, &handler);
        assert!(output.status.success());
        let desktop = home
            .path()
            .join("data/applications/ca.ezerbot.Ezerbot.desktop");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            desktop.to_str().unwrap()
        );
    }
    let link = "ezerbot://app/v1/thread?id=abc";
    let mut command = open::command(OsStr::new(link));
    home.env(&mut command);
    open::run(command).unwrap();
    wait_for("the handler to record the link", || {
        fs::read_to_string(&log).is_ok_and(|text| text == format!("{link}\n"))
    });
}

#[test]
fn register_url_handler_fails_in_french_when_xdg_mime_is_missing() {
    let home = Home::new("ezw");
    let mut command = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"));
    home.env(&mut command)
        .env("PATH", home.path().join("etc"))
        .args(["--register-url-handler", "/opt/ezerbot"]);
    let output = command.output().unwrap();
    assert!(!output.status.success());
    let message = String::from_utf8(output.stderr).unwrap();
    assert!(message.contains("n’a pas pu être enregistré"), "{message}");
    assert!(output.stdout.is_empty());
}

struct Desk {
    home: Home,
    bus: PrivateBus,
    db: PathBuf,
    conversation: String,
}
impl Desk {
    fn new() -> Option<Self> {
        if !tools(&["dbus-daemon", "xdg-open", "xdg-mime"]) {
            return None;
        }
        let home = Home::new("ezw");
        let bus = PrivateBus::new(&home);
        let db = home.path().join("workspace.sqlite");
        let mut app = App::open(&db).unwrap();
        let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_owned();
        Connection::open(&db)
            .unwrap()
            .execute(
                "INSERT INTO wrapper_notifications VALUES ('local-workspace','reply:one','reply',?1,'card-1','Réponse terminée',1,1,1)",
                params![conversation],
            )
            .unwrap();
        Some(Self {
            home,
            bus,
            db,
            conversation,
        })
    }
    fn add_notification(&self, id: &str, conversation: &str) {
        Connection::open(&self.db)
            .unwrap()
            .execute(
                "INSERT INTO wrapper_notifications VALUES ('local-workspace',?1,'reply',?2,'card-2','Réponse terminée',2,1,1)",
                params![id, conversation],
            )
            .unwrap();
    }
    fn host(&self, with_bus: bool) -> linux_host::Host {
        linux_host::launch(
            &self.home,
            &self.db,
            with_bus.then(|| self.bus.address()),
            None,
        )
    }
    fn deliver(&self, host: &linux_host::Host, id: &str) -> serde_json::Value {
        host.post(
            "/api/notifications/native",
            json!({"command":"deliver","event":{"id":id,"kind":"reply","conversation_id":"x","card_id":"y","title":"Titre venu du moteur de rendu"}}),
        )
    }
}

#[test]
fn the_native_route_reports_the_service_and_delivers_the_stored_title_with_a_fixed_body() {
    let Some(desk) = Desk::new() else { return };
    let server = FakeNotifications::new(&desk.bus);
    let host = desk.host(true);
    for command in ["status", "authorize"] {
        let reply = host.post("/api/notifications/native", json!({ "command": command }));
        assert_eq!(reply, json!({"status":"granted"}));
    }
    let presence = host.post(
        "/api/notifications/native",
        json!({"command":"presence","conversation_id":"x","visible":true}),
    );
    assert_eq!(presence, json!({"status":"ok"}));
    assert!(server.calls().is_empty());

    assert_eq!(
        desk.deliver(&host, "reply:one"),
        json!({"status":"delivered"})
    );
    let calls = server.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].summary, "Réponse terminée");
    assert_eq!(calls[0].body, BODY);
    assert_eq!(calls[0].app_name, "Ezerbot");

    let unknown = host
        .try_post(
            "/api/notifications/native",
            json!({"command":"deliver","event":{"id":"reply:absent"}}),
        )
        .unwrap_err();
    assert!(matches!(*unknown, ureq::Error::Status(404, _)));
    let stray = host
        .try_post(
            "/api/notifications/native",
            json!({"command":"status","title":"x"}),
        )
        .unwrap_err();
    assert!(matches!(*stray, ureq::Error::Status(400, _)));
    assert_eq!(server.calls().len(), 1);
}

#[test]
fn without_the_session_override_the_native_route_is_disabled_and_sends_nothing() {
    let Some(desk) = Desk::new() else { return };
    let server = FakeNotifications::new(&desk.bus);
    let host = linux_host::launch(&desk.home, &desk.db, None, Some(desk.bus.address()));
    assert_eq!(
        host.post("/api/notifications/native", json!({"command":"status"})),
        json!({"status":"disabled"})
    );
    assert_eq!(
        desk.deliver(&host, "reply:one"),
        json!({"status":"disabled"})
    );
    assert!(server.calls().is_empty());
}

#[test]
fn a_lost_notification_server_is_an_error_and_the_next_delivery_reconnects() {
    let Some(desk) = Desk::new() else { return };
    let host = desk.host(true);
    assert_eq!(desk.deliver(&host, "reply:one"), json!({"status":"error"}));
    let server = FakeNotifications::new(&desk.bus);
    assert_eq!(
        desk.deliver(&host, "reply:one"),
        json!({"status":"delivered"})
    );
    drop(server);
    assert_eq!(desk.deliver(&host, "reply:one"), json!({"status":"error"}));
    let server = FakeNotifications::new(&desk.bus);
    assert_eq!(
        desk.deliver(&host, "reply:one"),
        json!({"status":"delivered"})
    );
    assert_eq!(server.calls().len(), 1);
}

#[test]
fn clicking_a_notification_opens_its_thread_through_the_registered_link_handler() {
    let Some(desk) = Desk::new() else { return };
    let log = desk.home.path().join("clicked.log");
    let handler = recording_handler(desk.home.path(), &log);
    assert!(register(&desk.home, &handler).status.success());
    let server = FakeNotifications::new(&desk.bus);
    let host = desk.host(true);
    assert_eq!(
        desk.deliver(&host, "reply:one"),
        json!({"status":"delivered"})
    );
    server.invoke(1, "ouvrir-autre");
    server.click(1);
    let link = format!("ezerbot://app/v1/thread?id={}\n", desk.conversation);
    wait_for("the handler to receive the thread link", || {
        fs::read_to_string(&log).is_ok_and(|text| text == link)
    });
}

#[test]
fn after_the_notification_server_restarts_a_reused_id_opens_the_newest_thread() {
    let Some(desk) = Desk::new() else { return };
    let log = desk.home.path().join("clicked.log");
    let handler = recording_handler(desk.home.path(), &log);
    assert!(register(&desk.home, &handler).status.success());
    let mut app = App::open(&desk.db).unwrap();
    let second = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(app);
    desk.add_notification("reply:two", &second);
    let server = FakeNotifications::new(&desk.bus);
    let host = desk.host(true);
    assert_eq!(
        desk.deliver(&host, "reply:one"),
        json!({"status":"delivered"})
    );
    drop(server);
    let restarted = FakeNotifications::new(&desk.bus);
    assert_eq!(
        desk.deliver(&host, "reply:two"),
        json!({"status":"delivered"})
    );
    restarted.click(1);
    let link = format!("ezerbot://app/v1/thread?id={second}\n");
    wait_for("the handler to receive the newest thread", || {
        fs::read_to_string(&log).is_ok_and(|text| text == link)
    });
}
