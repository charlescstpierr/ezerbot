//! Static UI files over the actual local HTTP host: compression negotiation
//! and the closed set of served paths. Never launch a release host here.
#![cfg(debug_assertions)]
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::Value;

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'";

struct Host {
    child: Child,
    directory: std::path::PathBuf,
    address: String,
}

impl Host {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("grokbot-assets-{}", std::process::id()));
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
            address: String::new(),
        };
        let mut line = String::new();
        BufReader::new(host.child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let url = serde_json::from_str::<Value>(&line).unwrap()["url"]
            .as_str()
            .unwrap()
            .to_owned();
        host.address = url.strip_prefix("http://").unwrap().to_owned();
        host
    }

    fn get(&self, path: &str, accept_encoding: Option<&str>) -> Reply {
        let mut socket = TcpStream::connect(&self.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let encoding = accept_encoding
            .map(|value| format!("Accept-Encoding: {value}\r\n"))
            .unwrap_or_default();
        write!(
            socket,
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n{encoding}\r\n",
            self.address
        )
        .unwrap();
        let mut raw = Vec::new();
        socket.read_to_end(&mut raw).unwrap();
        let split = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("end of headers");
        let head = String::from_utf8(raw[..split].to_vec()).unwrap();
        let mut lines = head.lines();
        let status = lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let mut reply = Reply {
            status,
            headers,
            body: raw[split + 4..].to_vec(),
        };
        if reply.header("transfer-encoding") == Some("chunked") {
            reply.body = dechunk(&reply.body);
        }
        reply
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn dechunk(mut rest: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let line_end = rest.windows(2).position(|w| w == b"\r\n").unwrap();
        let size =
            usize::from_str_radix(std::str::from_utf8(&rest[..line_end]).unwrap(), 16).unwrap();
        if size == 0 {
            return body;
        }
        body.extend_from_slice(&rest[line_end + 2..line_end + 2 + size]);
        rest = &rest[line_end + 2 + size + 2..];
    }
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn decoded(&self) -> Vec<u8> {
        match self.header("content-encoding") {
            None => self.body.clone(),
            Some("gzip") => {
                let mut bytes = Vec::new();
                flate2::read::GzDecoder::new(self.body.as_slice())
                    .read_to_end(&mut bytes)
                    .unwrap();
                bytes
            }
            Some(other) => panic!("unexpected Content-Encoding {other}"),
        }
    }
}

const COMPRESSIBLE: [(&str, &str); 6] = [
    ("/", "text/html; charset=utf-8"),
    ("/app.js", "text/javascript; charset=utf-8"),
    ("/styles.css", "text/css; charset=utf-8"),
    ("/chat.js", "text/javascript; charset=utf-8"),
    ("/panels.js", "text/javascript; charset=utf-8"),
    ("/manifest.webmanifest", "application/manifest+json"),
];

#[test]
fn gzip_is_sent_only_when_accepted_and_decodes_to_the_identity_body() {
    let host = Host::new();
    for (path, content_type) in COMPRESSIBLE {
        let plain = host.get(path, None);
        assert_eq!(plain.status, 200, "{path}");
        assert_eq!(plain.header("content-encoding"), None, "{path}");
        assert_eq!(plain.header("vary"), Some("Accept-Encoding"), "{path}");
        assert!(!plain.body.is_empty(), "{path}");

        let gzip = host.get(path, Some("gzip, deflate, br"));
        assert_eq!(gzip.status, 200, "{path}");
        assert_eq!(gzip.header("content-encoding"), Some("gzip"), "{path}");
        assert_eq!(gzip.header("vary"), Some("Accept-Encoding"), "{path}");
        assert_eq!(gzip.header("content-type"), Some(content_type), "{path}");
        assert_eq!(gzip.header("cache-control"), Some("no-cache"), "{path}");
        assert_eq!(gzip.header("content-security-policy"), Some(CSP), "{path}");
        assert_eq!(gzip.header("x-frame-options"), Some("DENY"), "{path}");
        assert_eq!(gzip.header("x-content-type-options"), Some("nosniff"));
        if gzip.header("transfer-encoding").is_none() {
            assert_eq!(
                gzip.header("content-length"),
                Some(gzip.body.len().to_string().as_str()),
                "{path}"
            );
        }
        assert!(gzip.body.len() < plain.body.len(), "{path}");
        let decoded = gzip.decoded();
        assert_eq!(decoded, plain.body, "{path}");

        for refusal in ["gzip;q=0", "deflate, br", "identity", "*;q=0"] {
            let reply = host.get(path, Some(refusal));
            assert_eq!(reply.header("content-encoding"), None, "{path} {refusal}");
            assert_eq!(reply.header("vary"), Some("Accept-Encoding"));
            assert_eq!(reply.body, plain.body, "{path} {refusal}");
        }
        for acceptance in ["GZIP", "deflate,  gzip;q=0.5", "*"] {
            let reply = host.get(path, Some(acceptance));
            assert_eq!(
                reply.header("content-encoding"),
                Some("gzip"),
                "{acceptance}"
            );
            assert_eq!(reply.decoded(), plain.body, "{path} {acceptance}");
        }
    }
}

#[test]
fn images_and_api_answers_are_never_compressed_and_carry_no_vary() {
    let host = Host::new();
    let icon = host.get("/icon-192.png", Some("gzip"));
    assert_eq!(icon.status, 200);
    assert_eq!(icon.header("content-type"), Some("image/png"));
    assert_eq!(icon.header("content-encoding"), None);
    assert_eq!(icon.header("vary"), None);
    assert_eq!(&icon.body[..4], b"\x89PNG");
    let state = host.get("/api/state", Some("gzip"));
    assert_eq!(state.status, 200);
    assert_eq!(state.header("content-encoding"), None);
    assert_eq!(state.header("vary"), None);
    assert_eq!(state.header("cache-control"), Some("no-store"));
    serde_json::from_slice::<Value>(&state.body).unwrap();
}

#[test]
fn every_path_the_service_worker_precaches_is_served_and_unlisted_ones_are_not() {
    let host = Host::new();
    let script = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../app/web/public/sw.js"
    ))
    .unwrap();
    let list = |name: &str| {
        let prefix = format!("const {name} = [");
        script
            .lines()
            .find_map(|line| line.trim().strip_prefix(prefix.as_str()))
            .and_then(|rest| rest.split_once(']'))
            .unwrap_or_else(|| panic!("{name} array in sw.js"))
            .0
            .split('"')
            .skip(1)
            .step_by(2)
            .collect::<Vec<_>>()
    };
    let shell = list("ASSETS");
    let on_demand = list("ON_DEMAND");
    assert!(shell.len() >= 10, "{shell:?}");
    assert!(on_demand.len() >= 20, "{on_demand:?}");
    let precached: Vec<&str> = shell.into_iter().chain(on_demand).collect();
    for path in &precached {
        assert_eq!(host.get(path, None).status, 200, "{path}");
        assert_eq!(host.get(path, Some("gzip")).status, 200, "{path}");
    }
    for path in [
        "/chat.js.gz",
        "/panels/x.js",
        "/../Cargo.toml",
        "/katex/",
        "/katex/KaTeX_Main-Regular.woff",
        "/katex/KaTeX_Main-Regular.ttf",
        "/index.html",
        "/gzip/ui/app.js.gz",
        "/app.js/",
    ] {
        assert_eq!(host.get(path, Some("gzip")).status, 404, "{path}");
    }
}
