//! Debug builds must refuse a background-agent install unless the process
//! names an explicit throwaway root. This guard does not call plutil, so it
//! runs on Linux as well as macOS. The rest of the launch-agent suite stays
//! in routines_background.rs behind the macOS cfg.
use serde_json::json;
use std::{
    fs,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
};

struct Stopped(Child);
impl Drop for Stopped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn debug_build_cannot_install_without_explicit_throwaway_override() {
    let guard = grokbot_test_support::TestDir::new("ezerbot-routines-guard");
    let root = guard.to_path_buf();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("unsafe-override")).unwrap();
    let plist = root.join("Library/LaunchAgents/local.ezerbot.routines.plist");
    let mut child = Stopped(
        Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
            .args(["--port", "0", "--db"])
            .arg(root.join("unsafe-override/workspace.sqlite"))
            .current_dir(&root)
            .env("HOME", &root)
            .env("CFFIXED_USER_HOME", &root)
            .env_remove("GROKBOT_LAUNCH_AGENT_TEST_ROOT")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let url = serde_json::from_str::<serde_json::Value>(&line).unwrap()["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let response = ureq::post(&format!("{url}/api/routines/background"))
        .set("X-Grokbot-Request", "local")
        .send_json(json!({"enabled": true}));
    assert!(response.is_err());
    assert!(!plist.exists());
}
