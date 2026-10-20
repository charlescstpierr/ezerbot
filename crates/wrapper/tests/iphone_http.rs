#![cfg(debug_assertions)]
//! Actual dual-listener, HttpOnly cookie and PWA path. All hosts are loopback;
//! the fake CLI lives in a throwaway HOME and never calls real Tailscale.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    time::Duration,
};
struct Host {
    child: std::process::Child,
    root: std::path::PathBuf,
    url: String,
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
#[test]
fn actual_http_pairing_cookie_assets_disable_and_fake_tailscale() {
    let root = std::env::temp_dir().join(format!("iphone-http-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let fake = root.join("tailscale-fake");
    std::fs::write(&fake,format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}/commands'\nprintf '%s\\n' '{{\"BackendState\":\"Running\",\"Self\":{{\"DNSName\":\"mac.qa.ts.net.\"}},\"CurrentTailnet\":{{\"Name\":\"qa-tailnet\"}}}}'\n",root.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
        .args(["--port", "0", "--db"])
        .arg(root.join("workspace.sqlite"))
        .env("HOME", &root)
        .env("CFFIXED_USER_HOME", &root)
        .env("GROKBOT_FAKE_TAILSCALE", fake)
        .env_remove("GROKBOT_FAKE_OPENAI")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut host = Host {
        child,
        root,
        url: String::new(),
    };
    let mut line = String::new();
    BufReader::new(host.child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    host.url = serde_json::from_str::<Value>(&line).unwrap()["url"]
        .as_str()
        .unwrap()
        .into();
    let remote = format!(
        "http://127.0.0.1:{}",
        grokbot_wrapper::features::iphone::PORT
    );
    assert!(ureq::get(&format!("{remote}/api/state")).call().is_err());
    post(&host.url, "/api/iphone/enabled", json!({"enabled":true}));
    for _ in 0..60 {
        if ureq::get(&format!("{remote}/")).call().is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(
        response(ureq::get(&format!("{remote}/api/state")).call()).status(),
        401
    );
    for path in [
        "/manifest.webmanifest",
        "/sw.js",
        "/icon-192.png",
        "/icon-512.png",
        "/?notification=reply:test",
    ] {
        let r = response(ureq::get(&format!("{remote}{path}")).call());
        assert_eq!(r.status(), 200, "{path}");
        assert_eq!(r.header("X-Frame-Options"), Some("DENY"));
    }
    // A fresh browser can load these together over persistent HTTP/1.1 sockets.
    // Exercise the actual host, framing and connection reuse, not source strings.
    let readers: Vec<_> = (0..16)
        .map(|_| {
            let remote = remote.clone();
            std::thread::spawn(move || {
                use std::io::Read;
                let agent = ureq::AgentBuilder::new()
                    .timeout(Duration::from_secs(3))
                    .build();
                for index in 0..8 {
                    let (path, mime) = if index % 2 == 0 {
                        ("/iphone.css", "text/css; charset=utf-8")
                    } else {
                        ("/iphone.js", "text/javascript; charset=utf-8")
                    };
                    let reply = agent.get(&format!("{remote}{path}")).call().unwrap();
                    assert_eq!(reply.status(), 200);
                    assert_eq!(reply.header("Content-Type"), Some(mime));
                    let length: usize = reply.header("Content-Length").unwrap().parse().unwrap();
                    let mut bytes = Vec::new();
                    reply.into_reader().read_to_end(&mut bytes).unwrap();
                    assert_eq!(bytes.len(), length);
                    assert!(!bytes.is_empty());
                }
            })
        })
        .collect();
    for reader in readers {
        reader.join().unwrap();
    }
    // Continuous private-port traffic cannot monopolize the single App thread:
    // the Mac must retain a bounded path to its revoke/disable controls.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flood: Vec<_> = (0..16)
        .map(|_| {
            let remote = remote.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                use std::io::Read;
                let agent = ureq::AgentBuilder::new()
                    .timeout(Duration::from_secs(2))
                    .build();
                let deadline = std::time::Instant::now() + Duration::from_secs(4);
                while !stop.load(std::sync::atomic::Ordering::SeqCst)
                    && std::time::Instant::now() < deadline
                {
                    let mut bytes = Vec::new();
                    agent
                        .get(&format!("{remote}/iphone.js"))
                        .call()
                        .unwrap()
                        .into_reader()
                        .read_to_end(&mut bytes)
                        .unwrap();
                }
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(80));
    let local = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(1))
        .build()
        .get(&format!("{}/api/ping", host.url))
        .call();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    for reader in flood {
        reader.join().unwrap();
    }
    assert_eq!(local.unwrap().status(), 200);
    let ts: Value = response(ureq::get(&format!("{}/api/iphone/tailscale", host.url)).call())
        .into_json()
        .unwrap();
    assert_eq!(ts["url"], "https://mac.qa.ts.net");
    assert_eq!(
        post(&host.url, "/api/iphone/serve", json!({"start":true})).status(),
        200
    );
    assert_eq!(
        post(&host.url, "/api/iphone/serve", json!({"start":false})).status(),
        200
    );
    let code: Value = post(&host.url, "/api/iphone/code", json!({}))
        .into_json()
        .unwrap();
    let claim: Value = post(
        &remote,
        "/api/iphone/claim",
        json!({"code":code["code"],"name":"iPhone HTTP"}),
    )
    .into_json()
    .unwrap();
    let status: Value = response(ureq::get(&format!("{}/api/iphone/status", host.url)).call())
        .into_json()
        .unwrap();
    post(
        &host.url,
        "/api/iphone/confirm",
        json!({"id":status["devices"][0]["id"],"allow":true}),
    );
    let exchange = post(
        &remote,
        "/api/iphone/exchange",
        json!({"receipt":claim["receipt"]}),
    );
    assert_eq!(exchange.status(), 200);
    let cookie = exchange.header("Set-Cookie").unwrap().to_owned();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    let value: Value = exchange.into_json().unwrap();
    assert_eq!(value, json!({"paired":true}));
    assert!(value.get("credential").is_none());
    let cookie = cookie.split(';').next().unwrap();
    assert_eq!(
        response(
            ureq::get(&format!("{remote}/api/state"))
                .set("Cookie", cookie)
                .call()
        )
        .status(),
        200
    );
    assert_eq!(
        response(
            ureq::post(&format!("{remote}/api/iphone/revoke"))
                .set("Cookie", cookie)
                .set("Origin", &remote)
                .set("X-Grokbot-Request", "local")
                .send_json(json!({"id":status["devices"][0]["id"]}))
        )
        .status(),
        403
    );
    assert_eq!(
        response(
            ureq::post(&format!("{remote}/api/preferences"))
                .set("Cookie", cookie)
                .set("Origin", "https://evil.test")
                .set("X-Grokbot-Request", "local")
                .send_json(json!({"theme":"light"}))
        )
        .status(),
        403
    );
    // Desktop protection has NOT been relaxed for a tailnet Host.
    assert_eq!(
        response(
            ureq::get(&format!("{}/api/state", host.url))
                .set("Host", "mac.qa.ts.net")
                .call()
        )
        .status(),
        403
    );
    post(&host.url, "/api/iphone/enabled", json!({"enabled":false}));
    std::thread::sleep(Duration::from_millis(120));
    let disabled = ureq::get(&format!("{remote}/api/state"))
        .set("Cookie", cookie)
        .call();
    assert!(disabled.is_err());
    let commands = std::fs::read_to_string(host.root.join("commands")).unwrap();
    assert!(commands.contains("serve --bg http://127.0.0.1:47863"));
    assert!(commands.contains("serve --https=443 off"));
}
