#![cfg(debug_assertions)]
//! Actual dual-listener, HttpOnly cookie and PWA path. All hosts are loopback;
//! the fake CLI lives in a throwaway HOME and never calls real Tailscale.
#[path = "support/launch_identity.rs"]
mod launch_identity;
use launch_identity::LaunchIdentity;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    time::Duration,
};
struct Host {
    child: std::process::Child,
    identity: LaunchIdentity,
    root: std::path::PathBuf,
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
fn raw_get(port: u16, path: &str, accept_encoding: &str) -> (String, Vec<u8>) {
    use std::io::{Read, Write};
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(socket, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept-Encoding: {accept_encoding}\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = Vec::new();
    socket.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (
        String::from_utf8(raw[..split].to_vec()).unwrap(),
        raw[split + 4..].to_vec(),
    )
}
#[test]
fn actual_http_pairing_cookie_assets_disable_and_fake_tailscale() {
    let guard = grokbot_test_support::TestDir::new("iphone-http");
    let root = guard.to_path_buf();
    let fake = root.join("tailscale-fake");
    std::fs::write(&fake,format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}/commands'\nprintf '%s\\n' '{{\"BackendState\":\"Running\",\"Self\":{{\"DNSName\":\"mac.qa.ts.net.\"}},\"CurrentTailnet\":{{\"Name\":\"qa-tailnet\"}}}}'\n",root.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    // A free private port: an installed app may already hold 47863 (403 "désactivé").
    let iphone_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
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
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()),
    );
    let mut host = Host {
        child,
        identity,
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
    let remote = format!("http://127.0.0.1:{iphone_port}");
    assert!(ureq::get(&format!("{remote}/api/state")).call().is_err());
    host.post("/api/iphone/enabled", json!({"enabled":true}));
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
        assert_eq!(r.header("Connection"), Some("close"));
        assert_eq!(r.header("Cache-Control"), Some("no-store"));
    }
    // The private listener compresses too. Read the wire bytes, ureq would decode them.
    for path in ["/iphone.js", "/sw.js"] {
        let (head, packed) = raw_get(iphone_port, path, "gzip");
        for line in [
            "Content-Encoding: gzip",
            "Vary: Accept-Encoding",
            "Connection: close",
            "Cache-Control: no-store",
        ] {
            assert!(head.contains(line), "{path}: {line}\n{head}");
        }
        assert!(head.contains(&format!("Content-Length: {}", packed.len())));
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(packed.as_slice())
            .read_to_end(&mut decoded)
            .unwrap();
        let (head, plain) = raw_get(iphone_port, path, "identity");
        assert!(!head.contains("Content-Encoding"), "{path}\n{head}");
        assert!(head.contains("Vary: Accept-Encoding"), "{path}\n{head}");
        assert!(packed.len() < plain.len(), "{path}");
        assert_eq!(decoded, plain, "{path}");
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
                    let reply = agent
                        .get(&format!("{remote}{path}"))
                        .set("Accept-Encoding", "identity")
                        .call()
                        .unwrap();
                    assert_eq!(reply.status(), 200);
                    assert_eq!(reply.header("Content-Encoding"), None);
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
        .set("X-Ezerbot-Launch", &host.identity.value())
        .call();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    for reader in flood {
        reader.join().unwrap();
    }
    assert_eq!(local.unwrap().status(), 200);
    let ts: Value = host.get("/api/iphone/tailscale").into_json().unwrap();
    assert_eq!(ts["url"], "https://mac.qa.ts.net");
    assert_eq!(
        host.post("/api/iphone/serve", json!({"start":true}))
            .status(),
        200
    );
    assert_eq!(
        host.post("/api/iphone/serve", json!({"start":false}))
            .status(),
        200
    );
    let code: Value = host
        .post("/api/iphone/code", json!({}))
        .into_json()
        .unwrap();
    let claim: Value = post(
        &remote,
        "/api/iphone/claim",
        json!({"code":code["code"],"name":"iPhone HTTP"}),
    )
    .into_json()
    .unwrap();
    let status: Value = host.get("/api/iphone/status").into_json().unwrap();
    host.post(
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
                .set("X-Ezerbot-Launch", &host.identity.value())
                .call()
        )
        .status(),
        403
    );
    // Forced-close private responses retain binary data and WebKit byte ranges.
    use base64::{engine::general_purpose::STANDARD, Engine};
    let conversation: Value = host
        .post("/api/conversations", json!({}))
        .into_json()
        .unwrap();
    let wav = include_bytes!("fixtures/voice.wav");
    let memo:Value=host.post("/api/voice/memos",json!({"conversation_id":conversation["conversation"]["id"],"upload_id":"phone-range","data":STANDARD.encode(wav),"transcript":"Lecture de test.","language":"fr"})).into_json().unwrap();
    let ranged = ureq::get(&format!(
        "{remote}/api/voice/audio/{}",
        memo["id"].as_str().unwrap()
    ))
    .set("Cookie", cookie)
    .set("Range", "bytes=0-43")
    .call()
    .unwrap();
    assert_eq!(ranged.status(), 206);
    assert_eq!(ranged.header("Connection"), Some("close"));
    assert_eq!(
        ranged.header("Content-Range"),
        Some(format!("bytes 0-43/{}", wav.len()).as_str())
    );
    let mut received = Vec::new();
    ranged.into_reader().read_to_end(&mut received).unwrap();
    assert_eq!(received, &wav[..44]);
    // Reuse the SAME accepted socket over disable/re-enable. A newly bound
    // listener cannot read an old tiny_http queue: this reproduced the blank PWA.
    use std::io::{Read, Write};
    fn exchange_socket(
        reader: &mut BufReader<std::net::TcpStream>,
        port: u16,
    ) -> std::io::Result<(u16, Vec<String>)> {
        reader.get_mut().write_all(
            format!("GET /iphone.css HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: keep-alive\r\n\r\n")
                .as_bytes(),
        )?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.is_empty() {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let status = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut headers = Vec::new();
        let mut length = 0;
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            headers.push(line.trim().to_ascii_lowercase());
        }
        reader.read_exact(&mut vec![0; length])?;
        Ok((status, headers))
    }
    let socket = std::net::TcpStream::connect(("127.0.0.1", iphone_port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(1500)))
        .unwrap();
    let mut socket = BufReader::new(socket);
    assert_eq!(exchange_socket(&mut socket, iphone_port).unwrap().0, 200);
    for _ in 0..4 {
        host.post("/api/iphone/enabled", json!({"enabled":false}));
        // Access is denied on both old and fresh connections, never left hanging.
        let denial = response(ureq::get(&format!("{remote}/iphone.js")).call());
        assert_eq!(denial.status(), 403);
        assert_eq!(denial.header("Connection"), Some("close"));
        assert_eq!(denial.header("Cache-Control"), Some("no-store"));
        host.post("/api/iphone/enabled", json!({"enabled":true}));
        let start = std::time::Instant::now();
        assert_eq!(exchange_socket(&mut socket, iphone_port).unwrap().0, 200);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    host.post("/api/iphone/enabled", json!({"enabled":false}));
    let (status, headers) = exchange_socket(&mut socket, iphone_port).unwrap();
    assert_eq!(status, 403);
    assert!(headers.iter().any(|h| h == "connection: close"));
    std::thread::sleep(Duration::from_millis(120));
    let disabled = ureq::get(&format!("{remote}/api/state"))
        .set("Cookie", cookie)
        .call();
    assert!(disabled.is_err());
    let commands = std::fs::read_to_string(host.root.join("commands")).unwrap();
    assert!(commands.contains(&format!("serve --bg http://127.0.0.1:{iphone_port}")));
    assert!(commands.contains("serve --https=443 off"));
}
