//! Streamable HTTP for the fixed, reviewed catalogue endpoints and for the one
//! address a user typed. No redirects, sampling, elicitation or
//! credential-bearing error text; the token only goes to the approved origin.
use super::{Error, Result, FRAME_LIMIT};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read},
    time::Duration,
};

pub(crate) const AUTH_REQUIRED: &str = "La connexion au service a expiré ou a été révoquée. Utilisez « Reconnecter » dans les paramètres.";
pub(crate) const REDIRECTED: &str = "Le serveur MCP a répondu par une redirection. Elle n’est pas suivie : le jeton n’est envoyé qu’à l’adresse saisie.";
/// The address a user typed, as it will be stored and called: `https`, or `http`
/// only for `127.0.0.1` once the user confirmed that the server is on this Mac.
/// No credentials, query or fragment in the address: the secret has its own field.
pub(crate) fn approved_endpoint(
    raw: &str,
    on_this_mac: bool,
) -> std::result::Result<String, String> {
    let url = url::Url::parse(raw.trim()).map_err(|_| {
        "L’adresse du serveur MCP est invalide. Exemple : https://exemple.fr/mcp".to_owned()
    })?;
    if url.as_str().len() > 2048 {
        return Err("L’adresse du serveur MCP dépasse 2 048 caractères.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "L’adresse ne doit pas contenir d’identifiants. Saisissez le jeton dans son champ."
                .into(),
        );
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("L’adresse ne doit pas contenir de paramètres (?…) ni d’ancre (#…). Saisissez le jeton dans son champ.".into());
    }
    match (url.scheme(), url.host()) {
        ("https", Some(_)) => {}
        ("http", Some(url::Host::Ipv4(ip))) if ip == std::net::Ipv4Addr::LOCALHOST => {
            if !on_this_mac {
                return Err("Confirmez que ce serveur est sur ce Mac pour accepter une adresse http://127.0.0.1.".into());
            }
        }
        _ => {
            return Err("Seules les adresses https:// sont acceptées. http:// n’est permis que pour 127.0.0.1, après confirmation que le serveur est sur ce Mac.".into());
        }
    }
    Ok(url.into())
}
pub(super) struct Remote {
    endpoint: String,
    token: String,
    session: Option<String>,
    version: String,
    alive: bool,
}
impl Remote {
    pub(super) fn new(endpoint: String, token: String) -> Self {
        Self {
            endpoint,
            token,
            session: None,
            version: "2025-06-18".into(),
            alive: true,
        }
    }
    pub(super) fn alive(&self) -> bool {
        self.alive
    }
    pub(super) fn exchange(&mut self, frame: Value, timeout: Duration) -> Result<Value> {
        if serde_json::to_vec(&frame).map_or(true, |b| b.len() > FRAME_LIMIT) {
            return Err("La requête MCP dépasse 1 Mo.".into());
        }
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(timeout)
            .build();
        let mut request = agent
            .post(&self.endpoint)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream")
            .set("MCP-Protocol-Version", &self.version);
        if !self.token.is_empty() {
            request = request.set("Authorization", &format!("Bearer {}", self.token));
        }
        if let Some(session) = &self.session {
            request = request.set("Mcp-Session-Id", session);
        }
        let response = match request.send_json(frame.clone()) {
            Ok(r) => r,
            Err(ureq::Error::Status(401 | 403, _)) => {
                self.alive = false;
                return Err(AUTH_REQUIRED.into());
            }
            Err(ureq::Error::Status(404, _)) => {
                self.alive = false;
                return Err("La session MCP a expiré. Reconnectez le service.".into());
            }
            Err(_) => return Err("Le service MCP est indisponible. Réessayez la connexion.".into()),
        };
        // With redirects disabled a 3xx is still an `Ok`: refuse it, never read it.
        if (300..400).contains(&response.status()) {
            return Err(REDIRECTED.into());
        }
        if let Some(session) = response.header("Mcp-Session-Id") {
            if session.len() > 512 || session.chars().any(char::is_control) {
                return Err("Session MCP invalide.".into());
            }
            self.session = Some(session.to_owned());
        }
        if frame.get("id").is_none() {
            return Ok(Value::Null);
        }
        let sse = response
            .header("Content-Type")
            .is_some_and(|s| s.starts_with("text/event-stream"));
        let matching = |v: &Value| {
            v["jsonrpc"] == "2.0" && v.get("id") == frame.get("id") && v.get("method").is_none()
        };
        let response = if sse {
            // A Streamable HTTP response can remain open after our result. Stop
            // at the matching SSE event, not at EOF (and bound the whole stream).
            let mut reader = BufReader::new(response.into_reader().take((FRAME_LIMIT + 1) as u64));
            let mut total = 0;
            let mut data = String::new();
            loop {
                let mut line = String::new();
                let n = reader
                    .read_line(&mut line)
                    .map_err(|_| Error::from("Flux MCP incomplet."))?;
                total += n;
                if total > FRAME_LIMIT {
                    return Err("Le flux MCP dépasse 1 Mo.".into());
                }
                if n == 0 {
                    return Err("Flux MCP fermé sans résultat correspondant.".into());
                }
                let line = line.trim_end_matches(['\r', '\n']);
                if line.is_empty() && !data.is_empty() {
                    let value: Value = serde_json::from_str(&data)
                        .map_err(|_| Error::from("Flux MCP invalide."))?;
                    if matching(&value) {
                        break value;
                    }
                    data.clear();
                } else if let Some(part) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(part.strip_prefix(' ').unwrap_or(part));
                }
            }
        } else {
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take((FRAME_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::from("Réponse HTTP MCP incomplète."))?;
            if bytes.len() > FRAME_LIMIT {
                return Err("La réponse HTTP MCP dépasse 1 Mo.".into());
            }
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| Error::from("Réponse HTTP MCP illisible."))?;
            if !matching(&value) {
                return Err("Réponse HTTP MCP sans identifiant correspondant.".into());
            }
            value
        };
        if let Some(e) = response.get("error") {
            return Err(Error::Rpc {
                code: e["code"].as_i64().unwrap_or(-32000),
                message: "Le service MCP a refusé la requête.".into(),
            });
        }
        let result = response
            .get("result")
            .cloned()
            .ok_or_else(|| Error::from("Résultat MCP absent."))?;
        if frame["method"] == "initialize" {
            self.version = result["protocolVersion"].as_str().unwrap_or("").to_owned();
        }
        Ok(result)
    }
    pub(super) fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.exchange(
            json!({"jsonrpc":"2.0","method":method,"params":params}),
            Duration::from_secs(10),
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener};
    #[test]
    fn sse_returns_at_matching_event_without_waiting_for_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse::<usize>().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let event="data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\r\n\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\r\n\r\n";
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{event}").unwrap();
            socket.flush().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let mut remote = Remote::new(format!("http://{address}/mcp"), "fake-token-only".into());
        let result = remote
            .exchange(
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}),
                Duration::from_millis(500),
            )
            .unwrap();
        assert_eq!(result, json!({"ok":true}));
    }
}
