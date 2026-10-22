//! Local JSON-RPC 2.0 / newline-delimited stdio client (REA NM04, NM07).
//! No shell, inherited credentials or server-request execution.
mod http;
pub(crate) use http::{approved_endpoint, AUTH_REQUIRED};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub(crate) const DISCONNECTED: &str =
    "Le serveur MCP s’est arrêté ou déconnecté. Redémarrez le connecteur.";
const FRAME_LIMIT: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);
#[derive(Debug)]
pub(crate) enum Error {
    Transport(String),
    Rpc { code: i64, message: String },
}
impl From<String> for Error {
    fn from(s: String) -> Self {
        Self::Transport(s)
    }
}
impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Self::Transport(s.into())
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(s) => f.write_str(s),
            Self::Rpc { code, message } => write!(f, "{message} ({code})"),
        }
    }
}
type Result<T> = std::result::Result<T, Error>;

pub(crate) struct Client {
    child: Option<Child>,
    remote: Option<http::Remote>,
    identity: u64,
    input: Option<mpsc::SyncSender<Vec<u8>>>,
    frames: mpsc::Receiver<Result<Value>>,
    next: u64,
    catalog_dirty: bool,
    // Retain the launched credential snapshot for redaction until EOF: a vault
    // rotation must not reveal a value still held by the running child.
    environment: HashMap<String, String>,
}
impl Client {
    pub(crate) fn spawn(
        command: &str,
        arguments: &[String],
        environment: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(arguments).env_clear();
        // Deliberately small OS bootstrap environment; no parent API tokens.
        for key in ["PATH", "HOME", "TMPDIR", "LANG", "CFFIXED_USER_HOME"] {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        let mut child = cmd
            .envs(environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                "Impossible de démarrer le serveur MCP. Vérifiez la commande et ses arguments."
                    .to_owned()
            })?;
        let mut stdin = child.stdin.take().expect("stdin pipe");
        let (input, writes) = mpsc::sync_channel::<Vec<u8>>(8);
        // A child that stops reading cannot block the host's timeout or shutdown.
        std::thread::spawn(move || {
            while let Ok(bytes) = writes.recv() {
                if stdin
                    .write_all(&bytes)
                    .and_then(|_| stdin.write_all(b"\n"))
                    .and_then(|_| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let output = child.stdout.take().expect("stdout pipe");
        let (send, frames) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                let read = reader
                    .by_ref()
                    .take((FRAME_LIMIT + 1) as u64)
                    .read_until(b'\n', &mut line);
                let value = match read {
                    Ok(0) => break,
                    Ok(_) if line.len() <= FRAME_LIMIT && line.last() == Some(&b'\n') => {
                        serde_json::from_slice(&line).map_err(|_| {
                            "Le serveur MCP a envoyé un message JSON illisible.".into()
                        })
                    }
                    _ => Err("Le message MCP est trop volumineux ou incomplet.".into()),
                };
                let failed = value.is_err();
                if send.send(value).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child: Some(child),
            remote: None,
            identity: NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed),
            input: Some(input),
            frames,
            next: 0,
            catalog_dirty: false,
            environment: environment.clone(),
        })
    }
    pub(crate) fn remote(endpoint: String, token: String) -> Self {
        let (_, frames) = mpsc::sync_channel(1);
        Self {
            child: None,
            remote: Some(http::Remote::new(endpoint, token.clone())),
            identity: NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed),
            input: None,
            frames,
            next: 0,
            catalog_dirty: false,
            environment: if token.is_empty() {
                HashMap::new()
            } else {
                HashMap::from([("ACCESS_TOKEN".into(), token)])
            },
        }
    }
    fn send(&mut self, value: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&value).map_err(|_| "Requête MCP invalide.".to_owned())?;
        if bytes.len() > FRAME_LIMIT {
            return Err("La requête MCP dépasse 1 Mio.".into());
        }
        let input = self
            .input
            .as_mut()
            .ok_or("Le serveur MCP est déconnecté.")?;
        input.try_send(bytes).map_err(|error| match error {
            mpsc::TrySendError::Disconnected(_) => DISCONNECTED.into(),
            mpsc::TrySendError::Full(_) => "Le serveur MCP ne lit plus les requêtes.".into(),
        })
    }
    fn notification(&mut self, frame: &Value) -> Result<()> {
        if frame["jsonrpc"] != "2.0" || !frame["method"].is_string() {
            return Err("Notification MCP invalide.".into());
        }
        if frame["method"] == "notifications/tools/list_changed" {
            self.catalog_dirty = true;
        }
        if let Some(id) = frame.get("id") {
            if frame["method"] == "ping" {
                self.send(json!({"jsonrpc":"2.0","id":id,"result":{}}))?;
            } else {
                self.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Méthode non prise en charge"}}))?;
            }
        }
        Ok(())
    }
    pub(crate) fn catalog_changed(&mut self) -> Result<bool> {
        if self.remote.is_some() {
            return Ok(false);
        }
        // Only idle connections call this; no response is consumed out of turn.
        for _ in 0..32 {
            match self.frames.try_recv() {
                Ok(frame) => self.notification(&frame?)?,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(_) => return Err(DISCONNECTED.into()),
            }
        }
        Ok(self.catalog_dirty)
    }
    pub(crate) fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        if let Some(remote) = &mut self.remote {
            return remote.notify(method, params);
        }
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    pub(crate) fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.request_with_timeout(method, params, TIMEOUT)
    }
    pub(crate) fn request_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.next += 1;
        let id = self.next;
        if let Some(remote) = &mut self.remote {
            return remote.exchange(
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
                timeout,
            );
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let frame = match self.frames.recv_timeout(remaining) {
                Ok(value) => value?,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = self.notify(
                        "notifications/cancelled",
                        json!({"requestId":id,"reason":"Délai dépassé"}),
                    );
                    return Err("Le serveur MCP n’a pas répondu dans le délai prévu (10 secondes). Redémarrez le connecteur.".into());
                }
                Err(_) => return Err(DISCONNECTED.into()),
            };
            if frame["jsonrpc"] != "2.0" {
                return Err("Réponse MCP incompatible avec JSON-RPC 2.0.".into());
            }
            if frame.get("method").is_some() {
                // Sampling/elicitation cannot bypass the app's model/approval boundary.
                self.notification(&frame)?;
                // Notifications are not executable.
                if Instant::now() >= deadline {
                    return Err("Délai de réponse MCP dépassé.".into());
                }
                continue;
            }
            if frame["id"] != id {
                // A timed-out call can still deliver its frame. Skip only an
                // older id; a newer one is a real protocol error.
                if frame["id"].as_u64().is_some_and(|late| late < id) {
                    if Instant::now() >= deadline {
                        return Err("Délai de réponse MCP dépassé.".into());
                    }
                    continue;
                }
                return Err("Le serveur MCP a renvoyé un identifiant de réponse inattendu.".into());
            }
            if let Some(error) = frame.get("error") {
                let code = error["code"]
                    .as_i64()
                    .ok_or("Erreur JSON-RPC MCP invalide.")?;
                if !error["message"].is_string() || frame.get("result").is_some() {
                    return Err("Erreur JSON-RPC MCP invalide.".into());
                }
                let message = if method == "tools/call" && matches!(code, -32601 | -32602) {
                    "Le serveur MCP a refusé l’outil : outil inconnu ou arguments invalides."
                } else {
                    "Le serveur MCP a refusé la requête."
                };
                return Err(Error::Rpc {
                    code,
                    message: message.into(),
                });
            }
            return frame
                .get("result")
                .cloned()
                .ok_or_else(|| "Le résultat MCP est absent.".into());
        }
    }
    pub(crate) fn initialize(&mut self) -> Result<Vec<Value>> {
        self.initialize_with_timeout(TIMEOUT)
    }
    pub(crate) fn initialize_with_timeout(&mut self, timeout: Duration) -> Result<Vec<Value>> {
        let version = if self.remote.is_some() {
            "2025-06-18"
        } else {
            "2024-11-05"
        };
        let result = self.request_with_timeout("initialize", json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"Ezerbot","version":"0.1.0"}}), timeout)?;
        if !matches!(
            result["protocolVersion"].as_str(),
            Some("2024-11-05" | "2025-03-26" | "2025-06-18")
        ) {
            return Err("La version du protocole MCP n’est pas prise en charge.".into());
        }
        self.notify("notifications/initialized", json!({}))?;
        self.list_tools()
    }
    pub(crate) fn list_tools(&mut self) -> Result<Vec<Value>> {
        // A notification arriving DURING discovery must schedule another pass.
        self.catalog_dirty = false;
        let mut tools = Vec::new();
        let mut cursor = None;
        for _ in 0..16 {
            let params = cursor
                .as_ref()
                .map_or(json!({}), |c: &String| json!({"cursor":c}));
            let result = self.request("tools/list", params)?;
            let page = result["tools"]
                .as_array()
                .ok_or("La liste des outils MCP est invalide.")?;
            for tool in page {
                let name = tool["name"]
                    .as_str()
                    .ok_or("Un outil MCP n’a pas de nom.")?;
                if name.is_empty()
                    || name.len() > 256
                    || !tool["inputSchema"].is_object()
                    || tool["inputSchema"]["type"] != "object"
                    || tools.iter().any(|t: &Value| t["name"] == name)
                {
                    return Err(
                        "Le catalogue d’outils MCP est invalide ou contient des doublons.".into(),
                    );
                }
                tools.push(tool.clone());
                if tools.len() > 250 {
                    return Err("Ce connecteur dépasse la limite de 250 outils MCP.".into());
                }
            }
            cursor = result["nextCursor"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
        Err("La pagination des outils MCP ne se termine pas.".into())
    }
    pub(crate) fn identity(&self) -> u64 {
        self.identity
    }
    /// Only fixed, trusted catalogue adapters may classify non-secret bootstrap
    /// configuration as public. Manual servers keep every variable private.
    pub(crate) fn retain_private_environment(&mut self, names: &[&str]) {
        self.environment
            .retain(|key, _| names.contains(&key.as_str()));
    }
    pub(crate) fn private_environment(&self) -> &HashMap<String, String> {
        &self.environment
    }
    pub(crate) fn alive(&mut self) -> bool {
        if let Some(remote) = &self.remote {
            return remote.alive();
        }
        self.child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    }
    pub(crate) fn stop(&mut self) {
        // Dropping a remote handle performs no unapproved session DELETE.
        self.remote.take();
        // MCP stdio shutdown: close stdin, allow EOF cleanup, then reap/terminate.
        self.input.take();
        let deadline = Instant::now() + Duration::from_millis(300);
        while self.alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.alive() {
            if let Some(child) = &mut self.child {
                let _ = child.kill();
            }
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
    }
}
pub(crate) struct Connection {
    pub(crate) client: Mutex<Client>,
    pub(crate) environment: HashMap<String, String>,
    pub(crate) identity: u64,
    pub(crate) dirty: AtomicBool,
    pub(crate) ready: AtomicBool,
}
impl Connection {
    pub(crate) fn new(client: Client) -> Arc<Self> {
        Arc::new(Self {
            identity: client.identity(),
            environment: client.private_environment().clone(),
            client: Mutex::new(client),
            dirty: AtomicBool::new(false),
            ready: AtomicBool::new(false),
        })
    }
}
#[derive(Default)]
pub(crate) struct Manager(
    pub(crate) Mutex<HashMap<String, Arc<Connection>>>,
    pub(crate) AtomicBool,
    pub(crate) AtomicBool,
    pub(crate) AtomicU64,
);

#[cfg(test)]
mod tests {
    use super::*;
    fn fake(mode: &str) -> Client {
        Client::spawn(
            "python3",
            &[
                format!(
                    "{}/tests/fixtures/mcp_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
            ],
            &HashMap::new(),
        )
        .unwrap()
    }
    #[test]
    fn initialize_notifications_call_and_clean_eof() {
        let mut client = fake("normal");
        assert_eq!(client.initialize().unwrap().len(), 2);
        let result = client
            .request(
                "tools/call",
                json!({"name":"echo","arguments":{"text":"Bonjour"}}),
            )
            .unwrap();
        assert_eq!(result["content"][0]["text"], "Bonjour");
        assert!(client
            .request("tools/call", json!({"name":"unknown","arguments":{}}))
            .unwrap_err()
            .to_string()
            .contains("refusé"));
        client.input.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        while client.alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !client.alive(),
            "Le serveur doit quitter proprement sur EOF."
        );
        drop(client);
    }
    #[test]
    fn restarted_processes_have_distinct_connection_identities() {
        let old = fake("normal");
        let identity = old.identity();
        drop(old);
        let restarted = fake("normal");
        assert_ne!(identity, restarted.identity());
    }
    #[test]
    fn list_changed_during_discovery_is_not_lost() {
        let mut client = fake("change-during-list");
        client.initialize().unwrap();
        assert!(client.catalog_changed().unwrap());
        let tools = client.list_tools().unwrap();
        assert_eq!(tools[0]["description"], "Catalogue actualisé.");
        assert!(!client.catalog_changed().unwrap());
    }
    #[test]
    fn process_environment_preserves_empty_values_spaces_and_prototype_like_names() {
        let env = HashMap::from([
            ("EMPTY".into(), "".into()),
            ("SPACE".into(), "  avec espaces  ".into()),
            ("__proto__".into(), "private-proto-4444".into()),
        ]);
        let mut client = Client::spawn(
            "python3",
            &[format!(
                "{}/tests/fixtures/mcp_server.py",
                env!("CARGO_MANIFEST_DIR")
            )],
            &env,
        )
        .unwrap();
        client.initialize().unwrap();
        for (name, value) in env {
            let result = client
                .request(
                    "tools/call",
                    json!({"name":"echo","arguments":{"text":format!("env:{name}")}}),
                )
                .unwrap();
            assert_eq!(result["content"][0]["text"], value);
        }
    }
    #[test]
    fn child_that_never_reads_stdin_cannot_block_timeout() {
        let mut client = fake("no-read");
        let before = Instant::now();
        assert!(client
            .request_with_timeout(
                "tools/call",
                json!({"payload":"a".repeat(256*1024)}),
                Duration::from_millis(100)
            )
            .unwrap_err()
            .to_string()
            .contains("délai"));
        assert!(before.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn crash_timeout_and_invalid_json_are_explicit() {
        for (mode, expected) in [
            ("crash", "déconnecté"),
            ("bad-json", "illisible"),
            ("timeout", "délai"),
        ] {
            let mut client = fake(mode);
            let error = client
                .request_with_timeout(
                    "initialize",
                    json!({}),
                    if mode == "timeout" {
                        Duration::from_millis(100)
                    } else {
                        TIMEOUT
                    },
                )
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
    #[test]
    fn late_response_after_timeout_does_not_poison_the_next_call() {
        let mut client = fake("normal");
        client
            .initialize_with_timeout(Duration::from_secs(2))
            .unwrap();
        let late = client
            .request_with_timeout(
                "tools/call",
                json!({"name":"echo","arguments":{"text":"slow-late"}}),
                Duration::from_millis(30),
            )
            .unwrap_err();
        assert!(late.to_string().contains("délai"), "{late}");
        // The fixture answers 150 ms later. Wait until that frame is queued.
        std::thread::sleep(Duration::from_millis(250));
        let result = client
            .request_with_timeout(
                "tools/call",
                json!({"name":"echo","arguments":{"text":"Bonjour"}}),
                Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(result["content"][0]["text"], "Bonjour");
    }
}
