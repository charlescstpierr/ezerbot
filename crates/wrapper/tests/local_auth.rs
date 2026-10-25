#[path = "support/launch_identity.rs"]
mod launch_identity;
use launch_identity::LaunchIdentity;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};

struct Host {
    child: Child,
    url: String,
    identity: LaunchIdentity,
    dir: grokbot_test_support::TestDir,
}
impl Host {
    fn new() -> Self {
        let dir = grokbot_test_support::TestDir::new("launch-auth");
        let identity = LaunchIdentity::new();
        let mut child = identity.spawn(
            Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
                .args(["--port", "0", "--db"])
                .arg(dir.join("workspace.sqlite"))
                .env("HOME", dir.path())
                .env("CFFIXED_USER_HOME", dir.path())
                .env_remove("GROKBOT_FAKE_OPENAI")
                .env_remove("GROKBOT_TEST_KEYCHAIN")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        );
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let value: Value = serde_json::from_str(&line).expect("loopback announcement");
        assert!(value
            .as_object()
            .is_some_and(|o| o.len() == 1 && o.contains_key("url")));
        Self {
            url: value["url"].as_str().unwrap().into(),
            child,
            identity,
            dir,
        }
    }
    fn status(&self, method: &str, path: &str, identity: Option<&LaunchIdentity>) -> u16 {
        let mut request = ureq::request(method, &format!("{}{path}", self.url))
            .set("Origin", &self.url)
            .set("X-Grokbot-Request", "local")
            .set("Content-Type", "application/json");
        if let Some(identity) = identity {
            request = request.set("X-Ezerbot-Launch", &identity.value());
        }
        match request.call() {
            Ok(r) | Err(ureq::Error::Status(_, r)) => r.status(),
            Err(_) => panic!("local authentication transport failed"),
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn missing_wrong_current_and_previous_launch_identity() {
    let first = Host::new();
    let second = Host::new();
    for method in ["GET", "HEAD", "POST"] {
        assert_eq!(second.status(method, "/api/state", None), 403);
        assert_eq!(
            second.status(method, "/api/state", Some(&first.identity)),
            403
        );
    }
    assert_eq!(
        first.status("GET", "/api/state", Some(&first.identity)),
        200
    );
    assert_eq!(
        second.status("GET", "/api/state", Some(&second.identity)),
        200
    );
    for path in [
        "/",
        "/floating",
        "/app.js",
        "/chat.js",
        "/sw.js",
        "/?notification=event",
    ] {
        assert_eq!(
            second.status("GET", path, None),
            200,
            "public static asset {path}"
        );
    }
    assert_eq!(second.status("GET", "/unlisted", None), 403);
    assert_eq!(
        second.status("GET", "/unlisted", Some(&second.identity)),
        404
    );
    assert_eq!(second.status("GET", "/api/iphone/status", None), 403);
    assert_eq!(
        second.status("GET", "/api/iphone/status", Some(&second.identity)),
        200
    );
}

#[test]
fn all_local_routes_require_launch_identity() {
    let host = Host::new();
    let wrong = LaunchIdentity::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![root.join("lib.rs")];
    files.extend(
        std::fs::read_dir(root.join("features"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "rs")),
    );
    let mut routes = std::collections::BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(file).unwrap();
        for literal in source.split('"') {
            if literal.starts_with("/api/") && !literal.contains(['\n', '\r', ' ']) {
                routes.insert(literal.to_string());
            }
        }
    }
    assert!(
        routes.len() > 100,
        "production route inventory must not be empty"
    );
    for route in routes {
        for method in ["GET", "HEAD", "POST"] {
            assert_eq!(
                host.status(method, &route, None),
                403,
                "anonymous {method} {route}"
            );
            assert_eq!(
                host.status(method, &route, Some(&wrong)),
                403,
                "wrong identity {method} {route}"
            );
        }
    }
}

#[test]
fn no_valid_launch_channel_fails_before_database_and_announcement() {
    for length in [0, 1, 31, 33, 64] {
        let dir = grokbot_test_support::TestDir::new("invalid-launch");
        let db = dir.join("workspace.sqlite");
        let mut bytes = vec![0; length];
        getrandom::getrandom(&mut bytes).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
            .args(["--port", "0", "--db"])
            .arg(&db)
            .env("HOME", dir.path())
            .env("CFFIXED_USER_HOME", dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success(), "invalid launch must fail");
        assert!(
            output.stdout.is_empty(),
            "invalid launch cannot announce a listener"
        );
        assert!(!db.exists(), "invalid launch cannot open the database");
        assert!(String::from_utf8_lossy(&output.stderr).contains("canal de lancement"));
    }
}

#[test]
fn launch_identity_is_not_echoed_or_persisted() {
    let mut host = Host::new();
    let value = host.identity.value();
    let response = ureq::get(&format!("{}/api/state", host.url))
        .set("X-Ezerbot-Launch", &value)
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(
        !response.contains(&value),
        "state cannot expose launch identity"
    );
    host.child.kill().unwrap();
    host.child.wait().unwrap();
    let mut error = String::new();
    host.child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut error)
        .unwrap();
    assert!(
        !error.contains(&value),
        "stderr cannot expose launch identity"
    );
    fn scan(dir: &std::path::Path, value: &[u8]) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path, value);
            } else {
                let data = std::fs::read(path).unwrap();
                assert!(
                    !data.windows(value.len()).any(|part| part == value),
                    "launch identity cannot persist"
                );
            }
        }
    }
    scan(host.dir.path(), value.as_bytes());
}

#[test]
fn duplicate_launch_headers_are_rejected() {
    use std::io::Write;
    let host = Host::new();
    let address = host.url.strip_prefix("http://").unwrap();
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let value = host.identity.value();
    write!(stream, "GET /api/state HTTP/1.1\r\nHost: {address}\r\nX-Ezerbot-Launch: {value}\r\nx-ezerbot-launch: {value}\r\nConnection: close\r\n\r\n").unwrap();
    let mut status = String::new();
    BufReader::new(stream).read_line(&mut status).unwrap();
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "duplicate identity headers must be refused"
    );
}

#[test]
fn anonymous_unix_socket_pair_is_a_private_launch_channel() {
    use std::io::Write;
    let dir = grokbot_test_support::TestDir::new("unix-launch");
    let (mut writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
    let input: std::os::fd::OwnedFd = reader.into();
    let mut bytes = [0; 32];
    getrandom::getrandom(&mut bytes).unwrap();
    let identity = grokbot_wrapper::LaunchSecret::read(bytes.as_slice()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
        .args(["--port", "0", "--db"])
        .arg(dir.join("workspace.sqlite"))
        .env("HOME", dir.path())
        .env("CFFIXED_USER_HOME", dir.path())
        .env_remove("GROKBOT_FAKE_OPENAI")
        .env_remove("GROKBOT_TEST_KEYCHAIN")
        .stdin(Stdio::from(input))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writer.write_all(&bytes).unwrap();
    drop(writer);
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let result: Value = serde_json::from_str(&line).expect("private Unix channel announcement");
    let status = ureq::get(&format!("{}/api/state", result["url"].as_str().unwrap()))
        .set("X-Ezerbot-Launch", identity.header_value())
        .call()
        .map(|r| r.status());
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        matches!(status, Ok(200)),
        "private Unix channel identity must authenticate"
    );
}

#[test]
fn network_socket_is_not_a_private_launch_channel() {
    let dir = grokbot_test_support::TestDir::new("network-launch");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let socket = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (_peer, _) = listener.accept().unwrap();
    let input: std::os::fd::OwnedFd = socket.into();
    let db = dir.join("workspace.sqlite");
    let output = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
        .args(["--port", "0", "--db"])
        .arg(&db)
        .env("HOME", dir.path())
        .env("CFFIXED_USER_HOME", dir.path())
        .stdin(Stdio::from(input))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!db.exists());
}
