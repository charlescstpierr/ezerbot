use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

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
    let db = directory.join("app.sqlite");
    let child = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
        .args(["--port", "0", "--db"])
        .arg(&db)
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
