//! Minimal diagnostics for upstream refusals. Never retain a body, free-form message or headers.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::provider::{error_parts, failure, Failure};

pub(crate) struct Diagnostics {
    path: PathBuf,
    status: u16,
    request_id: String,
}

impl Diagnostics {
    pub(crate) fn new(directory: PathBuf, response: &ureq::Response) -> Self {
        Self {
            path: directory.join("logs/engine.log"),
            status: response.status(),
            request_id: response.header("x-request-id").unwrap_or_default().into(),
        }
    }

    /// One append per rejected HTTP attempt or terminal SSE error, including retries.
    pub(crate) fn rejected(&self, body: &Value) {
        let (code, _, param) = error_parts(body);
        let field = |value: &str| {
            if !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-[]".contains(&c))
            {
                value.to_owned()
            } else {
                "-".into()
            }
        };
        let request_id = if self.request_id.starts_with("req_") {
            field(&self.request_id)
        } else {
            "-".into()
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let line = format!(
            "[{timestamp}] OpenAI refus : statut={} code={} param={} request_id={request_id}\n",
            self.status,
            field(&code),
            field(&param)
        );
        // The native shell already redirects stderr to this file. Use its open
        // handle so its non-append offset cannot later overwrite a direct append.
        #[cfg(unix)]
        {
            use std::os::{fd::AsFd, unix::fs::MetadataExt};
            let stderr = std::io::stderr()
                .as_fd()
                .try_clone_to_owned()
                .and_then(|fd| fs::File::from(fd).metadata());
            if let (Ok(log), Ok(stderr)) = (fs::metadata(&self.path), stderr) {
                if log.dev() == stderr.dev() && log.ino() == stderr.ino() {
                    eprint!("{line}");
                    return;
                }
            }
        }
        let Some(directory) = self.path.parent() else {
            return;
        };
        if fs::create_dir_all(directory).is_err() {
            return;
        }
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut file) = options.open(&self.path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

/// Only explicit upstream reset fields are used, never the code or a retry/rate-limit header.
/// Accept Unix seconds or an ISO date; normalize with SQLite's existing date parser.
fn reset_time(body: &Value) -> Option<String> {
    let error = body.get("error").filter(|v| v.is_object()).unwrap_or(body);
    let value = error.get("reset_at").or_else(|| error.get("resets_at"))?;
    let connection = rusqlite::Connection::open_in_memory().ok()?;
    let result: Option<String> =
        if let Some(seconds) = value.as_i64().or_else(|| value.as_str()?.parse().ok()) {
            if !(946684800..=4133980799).contains(&seconds) {
                return None;
            }
            connection
                .query_row(
                    "SELECT strftime('%d/%m/%Y à %H:%M:%S UTC', ?1, 'unixepoch')",
                    [seconds],
                    |r| r.get(0),
                )
                .ok()?
        } else {
            let date = value.as_str()?;
            if date.len() < 20
                || date.len() > 40
                || date.as_bytes().get(4) != Some(&b'-')
                || date.as_bytes().get(7) != Some(&b'-')
                || date.as_bytes().get(10) != Some(&b'T')
                || !date
                    .bytes()
                    .all(|c| c.is_ascii_digit() || b"-:TtZz+. ".contains(&c))
            {
                return None;
            }
            connection
                .query_row(
                    "SELECT strftime('%d/%m/%Y à %H:%M:%S UTC', ?1)",
                    [date],
                    |r| r.get(0),
                )
                .ok()?
        };
    result
}

pub(crate) fn failure_from_body(status: u16, body: &Value) -> Failure {
    let (code, message, _) = error_parts(body);
    let mut result = failure(status, &code, &message);
    if result.kind == "limit" {
        if let Some(reset) = reset_time(body) {
            result.message.push_str(&format!(
                " Réinitialisation annoncée par OpenAI : le {reset}."
            ));
        }
    }
    result
}
