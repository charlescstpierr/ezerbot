use percent_encoding::percent_decode_str;
use std::{
    io::Read,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use url::Url;
use wry::http::{Method, Request, Response};

const RESOURCE_LIMIT: u64 = 64 * 1024 * 1024;
const RESOURCE_WORKERS: usize = 8;
const RESPONSE_HEADERS: &[&str] = &[
    "Content-Type",
    "Content-Length",
    "Content-Range",
    "Accept-Ranges",
    "Content-Disposition",
    "Cache-Control",
    "ETag",
    "Last-Modified",
];

#[derive(Clone)]
pub struct BackendOrigin {
    value: String,
    port: u16,
}

impl BackendOrigin {
    pub fn parse(raw: &str) -> Option<Self> {
        let authority = raw.strip_prefix("http://127.0.0.1:")?;
        let port = authority.strip_suffix('/').unwrap_or(authority);
        let port: u16 = port.parse().ok().filter(|port| *port != 0)?;
        let value = format!("http://127.0.0.1:{port}");
        if raw != value && raw != format!("{value}/") {
            return None;
        }
        Some(Self { value, port })
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn allows_navigation(&self, raw: &str) -> bool {
        if raw.starts_with("ezerbot-resource:") {
            return self.resource_target(raw, &Method::GET).is_some();
        }
        let raw = raw.strip_prefix("blob:").unwrap_or(raw);
        let Ok(url) = Url::parse(raw) else {
            return false;
        };
        let Some(remainder) = raw.strip_prefix(&self.value) else {
            return false;
        };
        (remainder.is_empty()
            || remainder.starts_with('/')
            || remainder.starts_with('#')
            || remainder.starts_with('?'))
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port_or_known_default() == Some(self.port)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() != "/api"
            && !url.path().starts_with("/api/")
    }

    /// The page the default browser may open for a new-window request.
    pub fn external_link(&self, raw: &str) -> Option<Url> {
        let url = Url::parse(raw).ok()?;
        let backend =
            url.host_str() == Some("127.0.0.1") && url.port_or_known_default() == Some(self.port);
        (matches!(url.scheme(), "http" | "https") && !backend).then_some(url)
    }

    fn resource_target(&self, raw: &str, method: &Method) -> Option<String> {
        if method != Method::GET && method != Method::HEAD {
            return None;
        }
        let prefix = format!("ezerbot-resource://127.0.0.1:{}", self.port);
        let suffix = raw.strip_prefix(&prefix)?;
        let suffix = suffix.split('#').next()?;
        let path = suffix.split('?').next()?;
        if !path.starts_with("/api/") || path.bytes().any(|byte| byte <= b' ' || byte == b'\\') {
            return None;
        }
        let lower = path.to_ascii_lowercase();
        if ["%2f", "%5c", "%25"]
            .iter()
            .any(|part| lower.contains(part))
        {
            return None;
        }
        for (index, byte) in path.bytes().enumerate() {
            if byte == b'%'
                && !path
                    .as_bytes()
                    .get(index + 1..index + 3)
                    .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
            {
                return None;
            }
        }
        let decoded = percent_decode_str(path).decode_utf8().ok()?;
        if decoded
            .bytes()
            .any(|byte| byte < b' ' || byte == 127 || byte == b'\\')
            || decoded.split('/').any(|part| part == "." || part == "..")
        {
            return None;
        }
        let url = Url::parse(raw).ok()?;
        if url.scheme() != "ezerbot-resource"
            || url.host_str() != Some("127.0.0.1")
            || url.port() != Some(self.port)
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        Some(format!("{}{suffix}", self.value))
    }
}

pub struct LocalApi {
    pub origin: BackendOrigin,
    identity: String,
    agent: ureq::Agent,
    active: AtomicUsize,
}

impl LocalApi {
    pub fn new(origin: BackendOrigin, identity: String) -> Self {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .try_proxy_from_env(false)
            .timeout(Duration::from_secs(20))
            .max_idle_connections(0)
            .build();
        Self {
            origin,
            identity,
            agent,
            active: AtomicUsize::new(0),
        }
    }

    pub fn initialization_script(&self) -> String {
        let arguments = serde_json::json!([
            self.origin.as_str(),
            format!("ezerbot-resource://127.0.0.1:{}", self.origin.port),
            self.identity
        ]);
        include_str!("local-api.js").replace("__EZERBOT_PARAMETERS__", &arguments.to_string())
    }

    pub fn serve(
        self: Arc<Self>,
        request: Request<Vec<u8>>,
        responder: wry::RequestAsyncResponder,
    ) {
        if self.active.fetch_add(1, Ordering::Relaxed) >= RESOURCE_WORKERS {
            self.active.fetch_sub(1, Ordering::Relaxed);
            responder.respond(resource_error(503));
            return;
        }
        std::thread::spawn(move || {
            let response = self
                .forward(&request)
                .unwrap_or_else(|| resource_error(502));
            self.active.fetch_sub(1, Ordering::Relaxed);
            responder.respond(response);
        });
    }

    fn forward(&self, request: &Request<Vec<u8>>) -> Option<Response<Vec<u8>>> {
        let target = self
            .origin
            .resource_target(&request.uri().to_string(), request.method())?;
        let mut forward = self
            .agent
            .request(request.method().as_str(), &target)
            .set("X-Ezerbot-Launch", &self.identity)
            .set("Origin", self.origin.as_str())
            .set("Accept-Encoding", "identity");
        for name in ["Range", "If-Range"] {
            if let Some(value) = request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
            {
                forward = forward.set(name, value);
            }
        }
        let response = match forward.call() {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(_) => return None,
        };
        if (300..400).contains(&response.status()) || response.header("Content-Encoding").is_some()
        {
            return None;
        }
        if request.method() != Method::HEAD
            && response.header("Content-Length").is_some_and(|length| {
                length
                    .parse::<u64>()
                    .map_or(true, |length| length > RESOURCE_LIMIT)
            })
        {
            return None;
        }
        let mut result = Response::builder()
            .status(response.status())
            .header("Access-Control-Allow-Origin", self.origin.as_str());
        for name in RESPONSE_HEADERS {
            if let Some(value) = response.header(name) {
                result = result.header(*name, value);
            }
        }
        let mut body = Vec::new();
        if request.method() != Method::HEAD {
            response
                .into_reader()
                .take(RESOURCE_LIMIT + 1)
                .read_to_end(&mut body)
                .ok()?;
            if body.len() as u64 > RESOURCE_LIMIT {
                return None;
            }
        }
        result.body(body).ok()
    }
}

fn resource_error(status: u16) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(b"Ressource locale indisponible.".to_vec())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn accepts_only_explicit_literal_loopback_origins() {
        for raw in ["http://127.0.0.1:32123", "http://127.0.0.1:80/"] {
            assert!(BackendOrigin::parse(raw).is_some());
        }
        for raw in [
            "http://127.0.0.1",
            "http://127.0.0.1:0",
            "http://127.0.0.1:65536",
            "http://127.0.0.1:080",
            "http://localhost:80",
            "http://2130706433:80",
            "http://127.1:80",
            "http://[::1]:80",
            "https://127.0.0.1:80",
            "http://user@127.0.0.1:80",
            "http://127.0.0.1:80/api/",
            "http://127.0.0.1:80?query",
            "http://127.0.0.1:80/#hash",
        ] {
            assert!(
                BackendOrigin::parse(raw).is_none(),
                "accepted untrusted origin"
            );
        }
    }

    #[test]
    fn only_web_pages_outside_the_backend_leave_for_the_browser() {
        let origin = BackendOrigin::parse("http://127.0.0.1:32123").unwrap();
        let oauth = "https://accounts.example/authorize?client_id=x&state=y";
        assert_eq!(origin.external_link(oauth).unwrap().as_str(), oauth);
        assert!(origin.external_link("http://127.0.0.1:9/fake").is_some());
        for raw in [
            "http://127.0.0.1:32123/floating",
            "http://127.0.0.1:32123/api/secret",
            "file:///etc/passwd",
            "ezerbot-resource://127.0.0.1:32123/api/file",
            "javascript:alert(1)",
            "ezerbot://oauth",
            "not a url",
        ] {
            assert!(origin.external_link(raw).is_none(), "{raw}");
        }
    }

    #[test]
    fn restricts_navigation_and_resource_paths() {
        let origin = BackendOrigin::parse("http://127.0.0.1:32123").unwrap();
        assert!(origin.allows_navigation("http://127.0.0.1:32123/#settings"));
        assert!(origin.allows_navigation("blob:http://127.0.0.1:32123/export-id"));
        assert!(origin.allows_navigation("ezerbot-resource://127.0.0.1:32123/api/file"));
        for raw in [
            "http://127.0.0.1:32123/api/file",
            "http://127.0.0.1:32123.evil.example/",
            "http://127.0.0.1:32124/",
            "file:///tmp/file",
            "javascript:alert(1)",
            "data:text/html,hello",
            "blob:https://outside.example/export-id",
            "blob:http://127.0.0.1:32124/export-id",
            "blob:null/export-id",
        ] {
            assert!(!origin.allows_navigation(raw));
        }
        assert!(origin
            .resource_target(
                "ezerbot-resource://127.0.0.1:32123/api/file?download=1",
                &Method::GET
            )
            .is_some());
        assert!(origin
            .resource_target("ezerbot-resource://127.0.0.1:32123/api/file", &Method::POST)
            .is_none());
        for path in [
            "/outside",
            "/api/../file",
            "/api/%2e%2e/file",
            "/api/%252e%252e/file",
            "/api/a%2fb",
            "/api/a%5cb",
            "/api/a\\b",
            "/api/%00",
            "/api/%zz",
        ] {
            assert!(origin
                .resource_target(
                    &format!("ezerbot-resource://127.0.0.1:32123{path}"),
                    &Method::GET
                )
                .is_none());
        }
        for host in [
            "user@127.0.0.1:32123",
            "127.0.0.1:32124",
            "localhost:32123",
            "127.0.0.1:32123.evil.example",
        ] {
            assert!(origin
                .resource_target(&format!("ezerbot-resource://{host}/api/file"), &Method::GET)
                .is_none());
        }
    }

    #[test]
    fn forwards_range_head_and_rejects_redirects_without_disclosing_identity() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = BackendOrigin::parse(&format!(
            "http://127.0.0.1:{}",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let backend_origin = origin.as_str().to_owned();
        let port = origin.port;
        let attacker = TcpListener::bind("127.0.0.1:0").unwrap();
        attacker.set_nonblocking(true).unwrap();
        let attacker_url = format!(
            "http://127.0.0.1:{}/stolen",
            attacker.local_addr().unwrap().port()
        );
        let server = thread::spawn(move || {
            for expected_method in ["GET", "HEAD", "GET", "GET"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut lines = Vec::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    lines.push(line);
                }
                assert!(lines[0].starts_with(expected_method));
                let headers = lines[1..].join("").to_lowercase();
                assert!(
                    headers.contains("x-ezerbot-launch: synthetic-test-identity\r\n"),
                    "missing launch authorization"
                );
                assert!(headers.contains(&format!("origin: {backend_origin}\r\n")));
                assert!(!headers.contains("cookie:"));
                if lines[0].contains("/redirect") {
                    write!(stream, "HTTP/1.1 302 Found\r\nLocation: {attacker_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else if lines[0].contains("/temporary") {
                    write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {attacker_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    assert!(headers.contains("range: bytes=1-2\r\n"));
                    assert!(headers.contains("if-range: resource-version\r\n"));
                    write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Length: 2\r\nContent-Range: bytes 1-2/4\r\nAccept-Ranges: bytes\r\nSet-Cookie: excluded=yes\r\nConnection: close\r\n\r\n").unwrap();
                    if expected_method != "HEAD" {
                        stream.write_all(b"bc").unwrap();
                    }
                }
            }
        });
        let api = LocalApi::new(origin, "synthetic-test-identity".into());
        for method in [Method::GET, Method::HEAD] {
            let request = Request::builder()
                .method(method.clone())
                .uri(format!("ezerbot-resource://127.0.0.1:{port}/api/file"))
                .header("Range", "bytes=1-2")
                .header("If-Range", "resource-version")
                .header("Cookie", "must-not-forward=yes")
                .body(Vec::new())
                .unwrap();
            let response = api.forward(&request).unwrap();
            assert_eq!(response.status(), 206);
            assert_eq!(response.headers()["Content-Range"], "bytes 1-2/4");
            assert!(!response.headers().contains_key("Set-Cookie"));
            assert_eq!(
                response.body().as_slice(),
                if method == Method::HEAD {
                    b"".as_slice()
                } else {
                    b"bc".as_slice()
                }
            );
        }
        for path in ["redirect", "temporary"] {
            let request = Request::builder()
                .uri(format!("ezerbot-resource://127.0.0.1:{port}/api/{path}"))
                .body(Vec::new())
                .unwrap();
            assert!(api.forward(&request).is_none());
        }
        server.join().unwrap();
        assert!(
            matches!(attacker.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}
