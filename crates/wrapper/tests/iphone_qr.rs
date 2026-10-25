#![cfg(debug_assertions)]
//! QR pairing over the real listeners: the scanned link carries the Tailscale
//! address and a working code, the Mac still confirms, and no log keeps the code.
#[path = "support/launch_identity.rs"]
mod launch_identity;
use base64::Engine;
use launch_identity::LaunchIdentity;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
struct Host {
    child: std::process::Child,
    identity: LaunchIdentity,
    root: PathBuf,
    url: String,
}
impl Host {
    fn post(&self, path: &str, body: Value) -> ureq::Response {
        response(
            ureq::post(&format!("{}{path}", self.url))
                .set("Origin", &self.url)
                .set("X-Grokbot-Request", "local")
                .set("X-Ezerbot-Launch", &self.identity.value())
                .send_json(body),
        )
    }
    fn get(&self, path: &str) -> ureq::Response {
        response(
            ureq::get(&format!("{}{path}", self.url))
                .set("X-Ezerbot-Launch", &self.identity.value())
                .call(),
        )
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn response(result: Result<ureq::Response, ureq::Error>) -> ureq::Response {
    match result {
        Ok(r) | Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("{e}"),
    }
}
fn post(base: &str, path: &str, body: Value) -> ureq::Response {
    response(
        ureq::post(&format!("{base}{path}"))
            .set("Origin", base)
            .set("X-Grokbot-Request", "local")
            .send_json(body),
    )
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
/// Reads the served SVG back into pixels and decodes it like a camera would.
fn scan(image: &str) -> String {
    let svg = base64::engine::general_purpose::STANDARD
        .decode(image.strip_prefix("data:image/svg+xml;base64,").unwrap())
        .unwrap();
    let svg = String::from_utf8(svg).unwrap();
    let side: usize = svg
        .split("viewBox=\"0 0 ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let path = svg
        .split(" d=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let mut dark = vec![false; side * side];
    for module in path.split('M').filter(|m| !m.is_empty()) {
        let (x, rest) = module.split_once(',').unwrap();
        let y = rest.split('h').next().unwrap();
        dark[y.parse::<usize>().unwrap() * side + x.parse::<usize>().unwrap()] = true;
    }
    let scale = 6;
    let mut image =
        rqrr::PreparedImage::prepare_from_greyscale(side * scale, side * scale, |x, y| {
            if dark[(y / scale) * side + x / scale] {
                0
            } else {
                255
            }
        });
    let grids = image.detect_grids();
    assert_eq!(grids.len(), 1);
    grids[0].decode().unwrap().1
}
fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn qr_link_prefills_a_single_use_code_that_still_needs_the_mac_and_never_reaches_logs() {
    let guard = grokbot_test_support::TestDir::new("iphone-qr");
    let root = guard.to_path_buf();
    let fake = root.join("tailscale-fake");
    std::fs::write(&fake,"#!/bin/sh\nprintf '%s\\n' '{\"BackendState\":\"Running\",\"Self\":{\"DNSName\":\"mac.qa.ts.net.\"},\"CurrentTailnet\":{\"Name\":\"qa-tailnet\"}}'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let iphone_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    // Every byte the wrapper prints lands in these files, scanned at the end.
    let stdout = root.join("stdout.log");
    let identity = LaunchIdentity::new();
    let child = identity.spawn(
        Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
            .env("GROKBOT_IPHONE_PORT", iphone_port.to_string())
            .args(["--port", "0", "--db"])
            .arg(root.join("workspace.sqlite"))
            .env("HOME", &root)
            .env("CFFIXED_USER_HOME", &root)
            .env("GROKBOT_FAKE_TAILSCALE", fake)
            .env_remove("GROKBOT_FAKE_OPENAI")
            .stdout(Stdio::from(std::fs::File::create(&stdout).unwrap()))
            .stderr(Stdio::from(
                std::fs::File::create(root.join("stderr.log")).unwrap(),
            )),
    );
    let mut host = Host {
        child,
        identity,
        root: root.clone(),
        url: String::new(),
    };
    let mut line = String::new();
    for _ in 0..300 {
        line = std::fs::read_to_string(&stdout).unwrap();
        if line.contains('\n') {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    host.url = serde_json::from_str::<Value>(line.lines().next().unwrap()).unwrap()["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let remote = format!("http://127.0.0.1:{iphone_port}");
    host.post("/api/iphone/enabled", json!({"enabled":true}));
    for _ in 0..60 {
        if ureq::get(&format!("{remote}/")).call().is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let before = now();
    let created: Value = host
        .post("/api/iphone/code", json!({}))
        .into_json()
        .unwrap();
    let code = created["code"].as_str().unwrap().to_owned();
    // The camera reads the Tailscale address with the code in the fragment.
    let link = scan(created["qr"].as_str().unwrap());
    assert_eq!(link, format!("https://mac.qa.ts.net/#jumelage={code}"));
    let scanned = link.split_once("#jumelage=").unwrap().1;
    let expires = created["expires"].as_u64().unwrap();
    assert!((before + 120_000..=now() + 120_000).contains(&expires));

    let db = rusqlite::Connection::open(root.join("workspace.sqlite")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    let claim = |code: &str| {
        post(
            &remote,
            "/api/iphone/claim",
            json!({"code":code,"name":"iPhone QR"}),
        )
    };
    for _ in 0..5 {
        assert_eq!(claim("0000000000").status(), 403);
    }
    // The attempt limit holds even for the scanned, valid code.
    assert_eq!(claim(scanned).status(), 429);
    db.execute("UPDATE iphone_settings SET attempt_window=0", [])
        .unwrap();
    let receipt: Value = claim(scanned).into_json().unwrap();
    assert_eq!(claim(scanned).status(), 403, "a QR code serves only once");

    let exchange = || {
        post(
            &remote,
            "/api/iphone/exchange",
            json!({"receipt":receipt["receipt"]}),
        )
    };
    let waiting = exchange();
    assert!(waiting.header("Set-Cookie").is_none());
    assert_eq!(
        waiting.into_json::<Value>().unwrap(),
        json!({"pending":true})
    );
    let status: Value = host.get("/api/iphone/status").into_json().unwrap();
    assert_eq!(status["devices"][0]["status"], "pending");
    host.post(
        "/api/iphone/confirm",
        json!({"id":status["devices"][0]["id"],"allow":true}),
    );
    let paired = exchange();
    assert!(paired.header("Set-Cookie").unwrap().contains("HttpOnly"));
    assert_eq!(paired.into_json::<Value>().unwrap(), json!({"paired":true}));

    // Two minutes later the next QR no longer pairs.
    let late: Value = host
        .post("/api/iphone/code", json!({}))
        .into_json()
        .unwrap();
    let late = late["code"].as_str().unwrap().to_owned();
    db.execute("UPDATE iphone_settings SET expires=expires-120001", [])
        .unwrap();
    assert_eq!(claim(&late).status(), 403);

    let _ = host.child.kill();
    let _ = host.child.wait();
    drop(db);
    for file in files(&host.root) {
        let bytes = std::fs::read(&file).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for secret in [&code, &late] {
            assert!(
                !text.contains(secret.as_str()),
                "{} contient un code",
                file.display()
            );
        }
    }
    assert!(host.root.join("stderr.log").is_file());
}
