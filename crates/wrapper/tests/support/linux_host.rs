//! A real `grokbot-wrapper` process in a throwaway HOME, for the Linux desktop tests.
#![allow(dead_code)]
use super::launch_identity::LaunchIdentity;
use grokbot_linux_desktop::testing::Home;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Child, Command},
    time::{Duration, Instant},
};

pub struct Host {
    child: Child,
    identity: LaunchIdentity,
    url: String,
    log: std::path::PathBuf,
    errors: std::path::PathBuf,
}

/// Starts the host on `db` with the isolated environment of `home`. `session_bus` is the
/// debug override and `ambient_bus` what a stray `DBUS_SESSION_BUS_ADDRESS` would point to.
pub fn launch(
    home: &Home,
    db: &Path,
    session_bus: Option<&str>,
    ambient_bus: Option<&str>,
) -> Host {
    let log = home.path().join("host.log");
    let errors = home.path().join("host.err");
    let mut command = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"));
    command
        .args(["--port", "0", "--db"])
        .arg(db)
        .env_remove("GROKBOT_FAKE_OPENAI")
        .stdout(fs::File::create(&log).unwrap())
        .stderr(fs::File::create(&errors).unwrap());
    home.env(&mut command);
    if let Some(address) = session_bus {
        command.env("GROKBOT_TEST_SESSION_BUS", address);
    }
    if let Some(address) = ambient_bus {
        command.env("DBUS_SESSION_BUS_ADDRESS", address);
    }
    let identity = LaunchIdentity::new();
    let mut child = identity.spawn(&mut command);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = fs::read_to_string(&log).unwrap();
        if let Some(url) = text.lines().find_map(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v["url"].as_str().map(str::to_owned))
        }) {
            return Host {
                child,
                identity,
                url,
                log,
                errors,
            };
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "host exited before serving"
        );
        assert!(Instant::now() < deadline, "host startup timed out");
        std::thread::sleep(Duration::from_millis(25));
    }
}

impl Host {
    pub fn state(&self) -> Value {
        ureq::get(&format!("{}/api/state", self.url))
            .set("X-Ezerbot-Launch", &self.identity.value())
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }
    pub fn try_post(&self, path: &str, body: Value) -> Result<ureq::Response, Box<ureq::Error>> {
        ureq::post(&format!("{}{path}", self.url))
            .set("Origin", &self.url)
            .set("X-Grokbot-Request", "local")
            .set("X-Ezerbot-Launch", &self.identity.value())
            .send_json(body)
            .map_err(Box::new)
    }
    pub fn post(&self, path: &str, body: Value) -> Value {
        self.try_post(path, body).unwrap().into_json().unwrap()
    }
    pub fn wait_state(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = self.state();
            if predicate(&state) {
                return state;
            }
            assert!(Instant::now() < deadline, "state transition timed out");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    pub fn delete(&self, name: &str) -> ureq::Response {
        self.try_post("/api/secrets/delete", serde_json::json!({"name":name}))
            .unwrap()
    }
    pub fn output(&self) -> String {
        fs::read_to_string(&self.log).unwrap() + &fs::read_to_string(&self.errors).unwrap()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
