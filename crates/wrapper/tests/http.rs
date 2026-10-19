//! HTTP host extension contracts, exercised against the actual wrapper process.
//! Never launch a release host here: its OS vault deliberately ignores test overrides.
#![cfg(debug_assertions)]
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use grokbot_wrapper::{
    body_limit, header_safe, too_large, validate_headers_for_path, ApiResponse, BODY_LIMIT,
};
use serde_json::{json, Value};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn first_line_announces_loopback_and_http_enforces_origin_and_size() {
    let directory =
        std::env::temp_dir().join(format!("grokbot-wrapper-http-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let db = directory.join("app.sqlite");
    let child = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
        .args(["--port", "0", "--db"])
        .arg(&db)
        .env("HOME", &directory)
        .env("CFFIXED_USER_HOME", &directory)
        .env_remove("GROKBOT_TEST_KEYCHAIN")
        .env_remove("GROKBOT_TEST_SIGNATURE_REQUIREMENT")
        .env_remove("GROKBOT_FAKE_OPENAI")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut running = Running(child);
    let mut first = String::new();
    BufReader::new(running.0.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    assert!(
        first.contains("http://127.0.0.1:"),
        "first stdout line must announce the actual loopback URL: {first:?}"
    );
    let value: serde_json::Value = serde_json::from_str(&first).unwrap();
    let address = value["url"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap();
    let request = |host: &str, method: &str, path: &str, extra: &str, body: &[u8]| {
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(socket, "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n{extra}\r\n", body.len()).unwrap();
        socket.write_all(body).unwrap();
        let mut result = String::new();
        socket.read_to_string(&mut result).unwrap();
        result
    };
    let state = request(address, "GET", "/api/state", "", b"");
    assert!(state.starts_with("HTTP/1.1 200"), "{state}");
    assert!(state.contains("Cache-Control: no-store"), "{state}");
    assert!(state.contains("\"simulated\""), "{state}");
    assert!(request("evil.test", "GET", "/api/state", "", b"").starts_with("HTTP/1.1 403"));
    assert!(request(address, "GET", "/../../Cargo.toml", "", b"").starts_with("HTTP/1.1 404"));
    let headers = "Content-Type: application/json\r\nX-Grokbot-Request: local\r\n";
    assert!(request(
        address,
        "POST",
        "/api/conversations",
        &format!("{headers}Origin: https://example.com\r\n"),
        b"{}"
    )
    .starts_with("HTTP/1.1 403"));
    assert!(request(
        address,
        "POST",
        "/api/conversations",
        headers,
        &vec![b'x'; 65_537]
    )
    .starts_with("HTTP/1.1 413"));
    assert!(
        request(address, "POST", "/api/conversations", headers, b"{}").starts_with("HTTP/1.1 200")
    );
    drop(running);
    let _ = std::fs::remove_dir_all(directory);
}

struct Host {
    child: Child,
    directory: PathBuf,
    url: String,
}

impl Host {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "grokbot-http-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
            .args(["--port", "0", "--db"])
            .arg(directory.join("app.sqlite"))
            .env("HOME", &directory)
            .env("CFFIXED_USER_HOME", &directory)
            .env_remove("GROKBOT_TEST_KEYCHAIN")
            .env_remove("GROKBOT_TEST_SIGNATURE_REQUIREMENT")
            .env_remove("GROKBOT_FAKE_OPENAI")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut host = Self {
            child,
            directory,
            url: String::new(),
        };
        let mut line = String::new();
        BufReader::new(host.child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let announcement: Value = serde_json::from_str(&line).expect("host URL announcement");
        host.url = announcement["url"].as_str().unwrap().to_owned();
        host
    }

    fn post(&self, path: &str, content_type: &str, body: &[u8]) -> u16 {
        status(
            ureq::post(&format!("{}{path}", self.url))
                .set("Origin", &self.url)
                .set("X-Grokbot-Request", "local")
                .set("Content-Type", content_type)
                .send_bytes(body),
        )
    }

    fn raw_post(&self, path: &str, content_type: &str, framing: &str, body: &[u8]) -> u16 {
        let address = self.url.strip_prefix("http://").unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream, "POST {path} HTTP/1.1\r\nHost: {address}\r\nOrigin: {}\r\nX-Grokbot-Request: local\r\nContent-Type: {content_type}\r\nConnection: close\r\n{framing}\r\n\r\n", self.url).unwrap();
        stream.write_all(body).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        response
            .split_whitespace()
            .nth(1)
            .expect("HTTP status")
            .parse()
            .unwrap()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn status(result: Result<ureq::Response, ureq::Error>) -> u16 {
    match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response.status(),
        Err(error) => panic!("HTTP transport: {error}"),
    }
}

fn accepted_by_host(status: u16) -> bool {
    !matches!(status, 403 | 413)
}

#[test]
fn voice_audio_supports_webkit_byte_ranges() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let host = Host::new();
    let post = |path: &str, body: Value| -> Value {
        ureq::post(&format!("{}{path}", host.url))
            .set("X-Grokbot-Request", "local")
            .send_json(body)
            .unwrap()
            .into_json()
            .unwrap()
    };
    let conversation = post("/api/conversations", json!({}));
    let wav = include_bytes!("fixtures/voice.wav");
    let memo = post(
        "/api/voice/memos",
        json!({"conversation_id":conversation["conversation"]["id"],"upload_id":"range-test","data":STANDARD.encode(wav),"transcript":"Test de lecture locale.","language":"fr"}),
    );
    let url = format!(
        "{}/api/voice/audio/{}",
        host.url,
        memo["id"].as_str().unwrap()
    );
    let full = ureq::get(&url).call().unwrap();
    assert_eq!(full.status(), 200);
    assert_eq!(full.header("Accept-Ranges"), Some("bytes"));
    assert!(full.header("Content-Range").is_none());
    let mut bytes = Vec::new();
    full.into_reader().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, wav);
    for (range, start, end) in [
        ("bytes=0-1", 0, 1),
        ("bytes=100-199", 100, 199),
        ("bytes=32000-", 32000, wav.len() - 1),
        ("bytes=-12", wav.len() - 12, wav.len() - 1),
        ("bytes=32000-999999", 32000, wav.len() - 1),
        ("bytes=-999999", 0, wav.len() - 1),
    ] {
        let response = ureq::get(&url).set("Range", range).call().unwrap();
        assert_eq!(response.status(), 206, "{range}");
        assert_eq!(response.header("Accept-Ranges"), Some("bytes"));
        assert_eq!(
            response.header("Content-Range"),
            Some(format!("bytes {start}-{end}/{}", wav.len()).as_str())
        );
        assert_eq!(
            response
                .header("Content-Length")
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            end - start + 1
        );
        let mut bytes = Vec::new();
        response.into_reader().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, wav[start..=end]);
    }
    for range in [
        "bytes=999999-",
        "bytes=20-10",
        "bytes=-0",
        "bytes=0-1,3-4",
        "bytes=no",
        "bytes=-",
        "bytes=+1-2",
    ] {
        let response = match ureq::get(&url).set("Range", range).call() {
            Err(ureq::Error::Status(416, response)) => response,
            other => panic!("{range}: {other:?}"),
        };
        assert_eq!(
            response.header("Content-Range"),
            Some(format!("bytes */{}", wav.len()).as_str())
        );
    }
    // A Range header does not change JSON or error responses.
    assert_eq!(
        ureq::get(&format!("{}/api/state", host.url))
            .set("Range", "bytes=0-1")
            .call()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        status(
            ureq::get(&format!("{}/api/voice/audio/missing", host.url))
                .set("Range", "bytes=0-1")
                .call()
        ),
        404
    );
}

#[test]
fn binary_post_only_allowed_on_attachment_route() {
    let host = Host::new();
    let image = b"\x89PNG\r\n\x1a\n\0\xff";
    // Assert only host policy; future feature routes may reject these incomplete image bytes.
    for mime in ["image/png", "IMAGE/JPEG; charset=binary"] {
        assert!(
            accepted_by_host(host.post("/api/attachments", mime, image)),
            "{mime}"
        );
    }
    for path in [
        "/api/send",
        "/api/skills/import",
        "/api/attachments/other",
        "/api/attachments/",
    ] {
        assert_eq!(host.post(path, "image/png", image), 403, "{path}");
    }
    for mime in [
        "application/octet-stream",
        "text/plain",
        "image/",
        "image/*",
        "image/png/extra",
    ] {
        assert_eq!(host.post("/api/attachments", mime, image), 403, "{mime}");
    }
    assert_eq!(
        host.post("/api/conversations", "application/json", b"{}"),
        200
    );
    let port: u16 = host.url.rsplit(':').next().unwrap().parse().unwrap();
    let headers = vec![
        ("Host".into(), format!("127.0.0.1:{port}")),
        ("X-Grokbot-Request".into(), "local".into()),
        ("Content-Type".into(), "image/png".into()),
        ("Origin".into(), "https://foreign.example".into()),
    ];
    assert_eq!(
        validate_headers_for_path(port, "POST", "/api/attachments", &headers)
            .unwrap_err()
            .status,
        403
    );
    assert_eq!(
        validate_headers_for_path(port, "POST", "/api/attachments", &headers[..1])
            .unwrap_err()
            .status,
        403
    );
}

#[test]
fn body_limit_by_route() {
    let host = Host::new();
    assert_eq!(body_limit("/api/attachments"), 20 * 1024 * 1024);
    assert_eq!(body_limit("/api/skills/import"), 16 * 1024 * 1024);
    assert_eq!(body_limit("/api/approvals/reviewer"), 256 * 1024);
    for path in [
        "/api/send",
        "/api/attachments/one",
        "/api/skills/import/",
        "/api/attachments?x=1",
        "/api/approvals/reviewer/",
    ] {
        assert_eq!(body_limit(path), 64 * 1024);
    }
    for (path, size) in [
        ("/api/attachments", "20 Mio"),
        ("/api/skills/import", "16 Mio"),
        ("/api/send", "64 Ko"),
        ("/api/approvals/reviewer", "256 Ko"),
    ] {
        let error = too_large(path);
        assert_eq!(error.status, 413);
        assert_eq!(
            error.message,
            format!("Le fichier ou la requête dépasse la taille permise ({size}).")
        );
    }
    let body = vec![b' '; BODY_LIMIT + 1];
    for (path, mime) in [
        ("/api/attachments", "image/png"),
        ("/api/skills/import", "application/json"),
        ("/api/approvals/reviewer", "application/json"),
    ] {
        assert!(accepted_by_host(host.post(path, mime, &body)), "{path}");
    }
    assert_eq!(
        host.post("/api/conversations", "application/json", &body),
        413
    );
    for (path, mime) in [
        ("/api/conversations", "application/json"),
        ("/api/skills/import", "application/json"),
        ("/api/attachments", "image/png"),
    ] {
        assert_eq!(
            host.raw_post(
                path,
                mime,
                &format!("Content-Length: {}", body_limit(path) + 1),
                b""
            ),
            413
        );
    }
    // No Content-Length: the bounded reader must also reject oversized chunked bodies.
    let mut chunked = format!("{:X}\r\n", body.len()).into_bytes();
    chunked.extend(&body);
    chunked.extend(b"\r\n0\r\n\r\n");
    assert_eq!(
        host.raw_post(
            "/api/conversations",
            "application/json",
            "Transfer-Encoding: chunked",
            &chunked
        ),
        413
    );
}

#[test]
fn non_read_methods_require_local_marker_and_origin() {
    let host = Host::new();
    for method in ["DELETE", "PUT", "PATCH", "OPTIONS"] {
        let request = || {
            ureq::request(method, &format!("{}/api/conversations", host.url))
                .set("Content-Type", "application/json")
        };
        assert_eq!(status(request().send_string("{}")), 403, "{method}");
        assert_eq!(
            status(
                request()
                    .set("X-Grokbot-Request", "local")
                    .set("Origin", "https://foreign.example")
                    .send_string("{}")
            ),
            403,
            "{method}"
        );
        assert!(
            accepted_by_host(status(
                request()
                    .set("X-Grokbot-Request", "local")
                    .set("Origin", &host.url)
                    .send_string("{}")
            )),
            "{method}"
        );
    }
}

#[test]
fn unsafe_content_type_is_rejected() {
    for value in [
        "",
        "image/pngé",
        "image/png\r\nSet-Cookie: x=1",
        "image/png\0",
        "image/png\t",
    ] {
        assert!(!header_safe(value), "{value:?}");
    }
    for value in ["image/png", "application/json; charset=utf-8"] {
        assert!(header_safe(value));
        assert!(tiny_http::Header::from_bytes("Content-Type", value).is_ok());
    }
    for byte in 0u8..=127 {
        let value = format!("image/png{}", char::from(byte));
        assert_eq!(header_safe(&value), (0x20..0x7f).contains(&byte), "{byte}");
    }
}

#[test]
fn bytes_response_keeps_content_type() {
    let bytes = vec![0, 0xff, b'\n', b'"'];
    let (actual, mime) = ApiResponse::Bytes {
        content_type: "image/png".into(),
        body: bytes.clone(),
    }
    .into_parts();
    assert_eq!(actual, bytes);
    assert_eq!(mime, "image/png");
    let (actual, mime) = ApiResponse::Json(json!({"ok": true})).into_parts();
    assert_eq!(
        serde_json::from_slice::<Value>(&actual).unwrap(),
        json!({"ok": true})
    );
    assert_eq!(mime, "application/json; charset=utf-8");
    let host = Host::new();
    for (path, mime) in [
        ("/panels.js", "text/javascript; charset=utf-8"),
        ("/panels.css", "text/css; charset=utf-8"),
        ("/api/state", "application/json; charset=utf-8"),
    ] {
        let response = ureq::get(&format!("{}{path}", host.url)).call().unwrap();
        assert_eq!(response.header("Content-Type"), Some(mime));
        assert_eq!(response.header("X-Content-Type-Options"), Some("nosniff"));
        let mut body = Vec::new();
        response.into_reader().read_to_end(&mut body).unwrap();
        if path == "/api/state" {
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap()["mode"],
                "simulated"
            );
        }
    }
}
