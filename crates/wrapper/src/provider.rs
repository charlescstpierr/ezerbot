//! Accès à l'abonnement ChatGPT par le parcours « Sign in with ChatGPT » d'OpenAI pour les
//! applications open source hébergées localement.
//! Référence : <https://developers.openai.com/siwc/token-sharing-open-source>

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const RESOURCE: &str = "https://api.openai.com/v1";
const REGISTRATION_CLIENT: &str = "dynamic_agent_client";
const APP_NAME: &str = "Grokbot";
// La documentation fixe l'hôte 127.0.0.1 et le chemin; seul le port peut varier.
const CALLBACK_PORTS: std::ops::RangeInclusive<u16> = 1455..=1465;
const CALLBACK_PATH: &str = "/auth/callback";
const SIGN_IN_WINDOW: Duration = Duration::from_secs(300);
const REFRESH_MARGIN: u64 = 300;
const ACCOUNT: &str = "local-workspace";
const TITLE_INSTRUCTIONS: &str = "Tu écris le titre d'une conversation pour la retrouver facilement dans une liste. Réponds seulement par le titre : 3 à 6 mots, dans la langue de l'utilisateur, sans guillemets, sans point final, sans mise en forme. Le titre nomme le sujet précis dont on parle, jamais une salutation ni une formule vague.";
const TITLE_LENGTH: usize = 60;

#[derive(Clone)]
pub struct Endpoints {
    pub authorize: String,
    pub token: String,
    pub revoke: String,
    pub jwks: String,
    pub issuer: String,
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            authorize: "https://auth.openai.com/api/accounts/authorize".into(),
            token: "https://auth.openai.com/api/accounts/oauth/token".into(),
            revoke: "https://auth.openai.com/api/accounts/oauth/revoke".into(),
            jwks: "https://auth.openai.com/.well-known/jwks.json".into(),
            issuer: "https://auth.openai.com".into(),
            api: "https://api.openai.com/v1".into(),
        }
    }
}

/// Ce qui est affiché à l'utilisateur quand une demande échoue. `kind` choisit l'action proposée :
/// `limit` ouvre les réglages d'utilisation de ChatGPT, `signin` propose de se reconnecter.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub message: String,
    pub kind: &'static str,
}

impl Failure {
    fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: "other",
        }
    }

    fn signin() -> Self {
        Self {
            message:
                "La connexion à ChatGPT n’est plus valide. Reconnectez-vous dans les paramètres."
                    .into(),
            kind: "signin",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Effort {
    pub effort: String,
    pub description: String,
}

/// Un palier de vitesse proposé par un modèle, par exemple `priority` (« Fast »).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Tier {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Un modèle du catalogue du compte. Rien n'est inventé ici : tout vient de `GET /v1/models`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Model {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub efforts: Vec<Effort>,
    pub default_effort: Option<String>,
    pub tiers: Vec<Tier>,
}

pub struct Run {
    pub conversation: String,
    pub message: String,
    /// Instructions composées à partir du bot de la conversation.
    pub instructions: String,
    /// Outils de proposition, en mode strict. Vide pour tout bot sauf le bot par défaut.
    pub tools: Vec<Value>,
    /// Paires (rôle, texte), de la plus ancienne à la plus récente.
    pub history: Vec<(&'static str, String)>,
    pub model: String,
    pub effort: Option<String>,
    /// Identifiant du palier de vitesse choisi; absent pour la vitesse standard.
    pub service_tier: Option<String>,
    /// Demande un titre de conversation tiré du sujet, une fois la réponse terminée.
    pub retitle: bool,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Credentials {
    host_id: String,
    client_id: Option<String>,
    email: Option<String>,
    subject: Option<String>,
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    #[serde(default)]
    expires_at: u64,
    #[serde(default)]
    scopes: String,
    #[serde(default)]
    saved_at: u64,
    #[serde(default)]
    welcomed: bool,
}

impl Credentials {
    fn connected(&self) -> bool {
        self.refresh_token.is_some()
            && self.access_token.is_some()
            && self
                .scopes
                .split_whitespace()
                .any(|scope| scope == PLAN_SCOPE)
    }

    fn clear_tokens(&mut self) {
        self.access_token = None;
        self.refresh_token = None;
        self.id_token = None;
        self.expires_at = 0;
        self.scopes.clear();
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: u64,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    email: Option<String>,
    nonce: Option<String>,
}

enum Phase {
    Idle,
    Pending,
    Failed(String),
}

struct SignIn {
    phase: Phase,
    cancel: Option<Arc<AtomicBool>>,
}

#[derive(Default)]
struct Catalog {
    models: Vec<Model>,
    loading: bool,
    loaded: bool,
}

struct Attempt {
    state: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    saved_client: Option<String>,
    saved_subject: Option<String>,
}

struct Inner {
    file: PathBuf,
    db: PathBuf,
    endpoints: Endpoints,
    open_browser: bool,
    // Sérialise les lectures, les écritures et les renouvellements : le jeton de renouvellement tourne.
    credentials: Mutex<()>,
    signin: Mutex<SignIn>,
    runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    // Titres en cours d'écriture : l'interface relit l'état tant qu'il en reste.
    titling: AtomicUsize,
    catalog: Mutex<Catalog>,
}

#[derive(Clone)]
pub struct Provider {
    inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0; bytes];
    getrandom::getrandom(&mut buffer).expect("le système fournit de l'aléa");
    URL_SAFE_NO_PAD.encode(buffer)
}

fn new_host_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("le système fournit de l'aléa");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn decode_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| value.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (escaped, bytes[index]) {
            (Some(byte), _) => {
                decoded.push(byte);
                index += 2;
            }
            (None, b'+') => decoded.push(b' '),
            (None, byte) => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn parse_query(url: &str) -> HashMap<String, String> {
    let query = url.split_once('?').map_or("", |(_, query)| query);
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (decode_component(key), decode_component(value)))
        .collect()
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(180))
        .build()
}

/// Lit le code d'erreur d'une réponse OpenAI. Les refus d'admission n'ont pas toujours la forme
/// `{"error": {...}}` : ils peuvent renvoyer `{"detail": "..."}` ou `{"error": "code"}`.
pub(crate) fn error_parts(body: &Value) -> (String, String, String) {
    let error = &body["error"];
    let code = error["code"]
        .as_str()
        .or_else(|| error.as_str())
        .or_else(|| body["code"].as_str())
        .unwrap_or_default();
    let message = error["message"]
        .as_str()
        .or_else(|| body["error_description"].as_str())
        .or_else(|| body["message"].as_str())
        .or_else(|| body["detail"].as_str())
        .unwrap_or_default();
    let param = error["param"].as_str().unwrap_or_default();
    (code.into(), message.into(), param.into())
}

pub(crate) fn failure(status: u16, code: &str, message: &str) -> Failure {
    match code {
        "subscription_sharing_usage_limit_exceeded" => Failure {
            message: "Limite d’utilisation atteinte. Vérifiez votre abonnement, ou la limite accordée à Grokbot, dans les réglages de ChatGPT.".into(),
            kind: "limit",
        },
        "subscription_sharing_user_not_eligible" => Failure::other("Ce compte ou cet espace de travail ne permet pas d’utiliser l’abonnement ChatGPT dans Grokbot."),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => Failure::other("Le service d’OpenAI est indisponible pour le moment. Réessayez dans quelques minutes."),
        "subscription_sharing_invalid_user" => Failure::signin(),
        _ if status == 401 => Failure::signin(),
        _ if status == 503 => Failure::other("Le service d’OpenAI est indisponible pour le moment. Réessayez dans quelques minutes."),
        _ => {
            let detail: String = if message.is_empty() { code } else { message }.chars().take(300).collect();
            Failure::other(if status == 0 {
                // Erreur annoncée dans le flux : il n'y a pas de code HTTP à citer.
                format!("OpenAI a signalé une erreur pendant la réponse : {detail}")
            } else if detail.is_empty() {
                format!("OpenAI a refusé la demande (erreur {status}).")
            } else {
                format!("OpenAI a refusé la demande (erreur {status}) : {detail}")
            })
        }
    }
}

fn request_failure(error: ureq::Error) -> (u16, String, String, String) {
    match error {
        ureq::Error::Status(status, response) => {
            let body: Value = response.into_json().unwrap_or(Value::Null);
            let (code, message, param) = error_parts(&body);
            (status, code, message, param)
        }
        ureq::Error::Transport(_) => (0, String::new(), String::new(), String::new()),
    }
}

fn network_failure() -> Failure {
    Failure::other("OpenAI est injoignable. Vérifiez votre connexion Internet.")
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// Lit le catalogue tel que le compte le renvoie : `supported_reasoning_levels`,
/// `default_reasoning_level` et `service_tiers` portent l'effort et la vitesse de chaque modèle.
fn parse_models(body: &Value) -> Vec<Model> {
    let items = |value: &Value| value.as_array().cloned().unwrap_or_default();
    items(&body["models"])
        .iter()
        .filter(|model| model["visibility"] == "list")
        .filter_map(|model| {
            let slug = model["slug"].as_str()?;
            Some(Model {
                slug: slug.into(),
                name: model["display_name"].as_str().unwrap_or(slug).into(),
                description: text(&model["description"]),
                efforts: items(&model["supported_reasoning_levels"])
                    .iter()
                    .filter_map(|level| {
                        Some(Effort {
                            effort: level["effort"].as_str()?.into(),
                            description: text(&level["description"]),
                        })
                    })
                    .collect(),
                default_effort: model["default_reasoning_level"].as_str().map(str::to_owned),
                tiers: items(&model["service_tiers"])
                    .iter()
                    .filter_map(|tier| {
                        let id = tier["id"].as_str()?;
                        Some(Tier {
                            id: id.into(),
                            name: tier["name"].as_str().unwrap_or(id).into(),
                            description: text(&tier["description"]),
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

/// Un appel de fonction émis par le modèle, tel que le flux le termine.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolCall {
    /// `item.id` du flux, pour rattacher les fragments d'arguments.
    pub item_id: String,
    /// `call_id` : l'identifiant que le modèle donne à l'appel.
    pub call_id: String,
    pub name: String,
    /// Les arguments, en JSON sérialisé.
    pub arguments: String,
}

/// Un événement du flux Responses, réduit à ce que l'affichage et les propositions utilisent.
#[derive(Debug, PartialEq)]
enum StreamEvent {
    Delta(String),
    /// `response.function_call_arguments.delta` : un fragment des arguments d'un appel.
    CallDelta {
        item_id: String,
        delta: String,
    },
    /// `response.output_item.done` d'un élément `function_call` : l'appel complet.
    Call(ToolCall),
    Completed,
    Incomplete,
    Failed(Failure),
    Other,
}

fn stream_event(data: &str) -> StreamEvent {
    let Ok(event) = serde_json::from_str::<Value>(data) else {
        return StreamEvent::Other;
    };
    match event["type"].as_str().unwrap_or_default() {
        "response.output_text.delta" => {
            StreamEvent::Delta(event["delta"].as_str().unwrap_or_default().into())
        }
        "response.function_call_arguments.delta" => StreamEvent::CallDelta {
            item_id: text(&event["item_id"]),
            delta: text(&event["delta"]),
        },
        // `response.output_item.added` et `response.function_call_arguments.done` annoncent le
        // même appel ; l'élément terminé porte à lui seul `call_id`, `name` et `arguments`.
        "response.output_item.done" if event["item"]["type"] == "function_call" => {
            let item = &event["item"];
            StreamEvent::Call(ToolCall {
                item_id: text(&item["id"]),
                call_id: text(&item["call_id"]),
                name: text(&item["name"]),
                arguments: text(&item["arguments"]),
            })
        }
        "response.completed" => StreamEvent::Completed,
        "response.incomplete" => StreamEvent::Incomplete,
        "response.failed" => {
            let (code, message, _) = error_parts(&event["response"]);
            StreamEvent::Failed(failure(0, &code, &message))
        }
        "error" => {
            let (code, message, _) = error_parts(&event);
            StreamEvent::Failed(failure(0, &code, &message))
        }
        _ => StreamEvent::Other,
    }
}

fn request_body(run: &Run, effort: Option<&str>, tier: Option<&str>) -> Value {
    let input: Vec<Value> = run
        .history
        .iter()
        .map(|(role, text)| json!({"role": role, "content": text}))
        .collect();
    // `store: false` et `stream: true` sont exigés par ce parcours; l'historique voyage dans `input`.
    let mut body = json!({
        "model": run.model, "instructions": run.instructions, "input": input, "store": false, "stream": true,
    });
    if let Some(effort) = effort {
        body["reasoning"] = json!({"effort": effort});
    }
    if let Some(tier) = tier {
        body["service_tier"] = json!(tier);
    }
    if !run.tools.is_empty() {
        body["tools"] = json!(run.tools);
    }
    body
}

/// Ce qu'un flux terminé laisse : des notes à afficher sous la réponse et les appels de fonction.
#[derive(Default)]
struct Streamed {
    notices: Vec<String>,
    calls: Vec<ToolCall>,
}

impl Streamed {
    fn notice(&self) -> Option<String> {
        (!self.notices.is_empty()).then(|| self.notices.join(" "))
    }
}

/// Ramène la réponse du modèle à une ligne courte et propre pour la liste des conversations.
fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|line| !line.is_empty())?;
    let line = line.trim_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '"' | '\'' | '«' | '»' | '“' | '”' | '`' | '*' | '#' | '_'
            )
    });
    let line = line.trim_end_matches(['.', ':', ' ']);
    let words: String = line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    if words.is_empty() {
        return None;
    }
    if words.chars().count() <= TITLE_LENGTH {
        return Some(words);
    }
    let prefix: String = words.chars().take(TITLE_LENGTH - 1).collect();
    Some(format!("{}…", prefix.trim_end()))
}

/// Le début de l'échange suffit pour nommer le sujet; chaque tour est tronqué.
fn title_transcript(history: &[(&'static str, String)], reply: &str) -> String {
    let turn = |role: &str, text: &str| {
        let excerpt: String = text.chars().take(700).collect();
        format!(
            "{} : {excerpt}",
            if role == "user" {
                "Utilisateur"
            } else {
                "Assistant"
            }
        )
    };
    let mut lines: Vec<String> = history
        .iter()
        .map(|(role, text)| turn(role, text))
        .collect();
    lines.push(turn("assistant", reply));
    lines.join("\n\n")
}

fn page(title: &str, detail: &str) -> String {
    format!("<!doctype html><html lang=\"fr\"><meta charset=\"utf-8\"><title>Grokbot</title><body style=\"margin:0;display:grid;place-items:center;height:100vh;background:#0a0b0d;color:#e6e7ea;font:16px -apple-system,sans-serif\"><main style=\"max-width:420px;padding:24px;text-align:center\"><h1 style=\"font-size:22px\">{title}</h1><p style=\"color:#9a9fa9\">{detail}</p></main></body></html>")
}

impl Provider {
    pub fn new(db: &Path) -> Self {
        Self::with_endpoints(db, Endpoints::default(), true)
    }

    pub fn with_endpoints(db: &Path, endpoints: Endpoints, open_browser: bool) -> Self {
        let directory = db.parent().filter(|parent| !parent.as_os_str().is_empty());
        let file = directory.unwrap_or(Path::new(".")).join(format!(
            "{}.chatgpt-auth.json",
            db.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("grokbot")
        ));
        Self {
            inner: Arc::new(Inner {
                file,
                db: db.to_owned(),
                endpoints,
                open_browser,
                credentials: Mutex::new(()),
                signin: Mutex::new(SignIn {
                    phase: Phase::Idle,
                    cancel: None,
                }),
                runs: Mutex::new(HashMap::new()),
                titling: AtomicUsize::new(0),
                catalog: Mutex::new(Catalog::default()),
            }),
        }
    }

    fn read(&self) -> Option<Credentials> {
        serde_json::from_slice(&fs::read(&self.inner.file).ok()?).ok()
    }

    /// Écriture atomique, lisible par le seul propriétaire.
    fn write(&self, credentials: &Credentials) -> std::io::Result<()> {
        let temporary = self.inner.file.with_extension("json.tmp");
        if let Some(parent) = self.inner.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(
            &serde_json::to_vec_pretty(credentials).expect("identifiants sérialisables"),
        )?;
        file.sync_all()?;
        fs::rename(temporary, &self.inner.file)
    }

    /// L'identifiant d'hôte doit exister et rester stable avant la première connexion.
    fn load_or_create(&self) -> std::io::Result<Credentials> {
        if let Some(credentials) = self.read() {
            return Ok(credentials);
        }
        let credentials = Credentials {
            host_id: new_host_id(),
            ..Credentials::default()
        };
        self.write(&credentials)?;
        Ok(credentials)
    }

    pub fn connected(&self) -> bool {
        let _guard = lock(&self.inner.credentials);
        self.read()
            .is_some_and(|credentials| credentials.connected())
    }

    pub fn status(&self) -> Value {
        let credentials = {
            let _guard = lock(&self.inner.credentials);
            self.read().unwrap_or_default()
        };
        let connected = credentials.connected();
        let (signin, error) = match &lock(&self.inner.signin).phase {
            Phase::Idle => ("idle", None),
            Phase::Pending => ("pending", None),
            Phase::Failed(message) => ("failed", Some(message.clone())),
        };
        let (models, models_ready) = {
            let catalog = lock(&self.inner.catalog);
            (catalog.models.clone(), catalog.loaded)
        };
        if connected && !models_ready {
            self.refresh_models();
        }
        json!({
            "connected": connected,
            "email": if connected { credentials.email } else { None },
            "signin": signin, "error": error,
            "welcomed": credentials.welcomed,
            "models": if connected { models } else { Vec::new() },
            "models_ready": models_ready,
            "titling": self.inner.titling.load(Ordering::SeqCst) > 0,
        })
    }

    pub fn mark_welcomed(&self) -> std::io::Result<()> {
        let _guard = lock(&self.inner.credentials);
        let mut credentials = self.load_or_create()?;
        credentials.welcomed = true;
        self.write(&credentials)
    }

    pub fn cancel_signin(&self) {
        let mut signin = lock(&self.inner.signin);
        if let Some(cancel) = signin.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        signin.phase = Phase::Idle;
    }

    /// Ouvre la page d'autorisation d'OpenAI et attend son retour sur une adresse de bouclage.
    pub fn start_signin(&self) -> Result<String, Failure> {
        self.cancel_signin();
        let credentials = {
            let _guard = lock(&self.inner.credentials);
            self.load_or_create().map_err(|_| {
                Failure::other("Impossible d’enregistrer les identifiants sur ce Mac.")
            })?
        };
        let (server, port) = CALLBACK_PORTS
            .filter_map(|port| {
                tiny_http::Server::http(("127.0.0.1", port))
                    .ok()
                    .map(|server| (server, port))
            })
            .next()
            .ok_or_else(|| {
                Failure::other(
                    "Aucun port local n’est libre pour terminer la connexion (1455 à 1465).",
                )
            })?;
        let attempt = Attempt {
            state: random_token(32),
            nonce: random_token(32),
            verifier: random_token(48),
            redirect_uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
            saved_client: credentials.client_id.clone(),
            saved_subject: credentials.subject.clone(),
        };
        let mut query = vec![
            (
                "client_id",
                attempt
                    .saved_client
                    .clone()
                    .unwrap_or_else(|| REGISTRATION_CLIENT.into()),
            ),
            ("ext_agent_host_id", credentials.host_id.clone()),
            ("response_type", "code".into()),
            ("redirect_uri", attempt.redirect_uri.clone()),
            ("scope", SCOPE.into()),
            ("resource", RESOURCE.into()),
            ("state", attempt.state.clone()),
            ("nonce", attempt.nonce.clone()),
            ("code_challenge_method", "S256".into()),
            ("code_challenge", challenge(&attempt.verifier)),
        ];
        if attempt.saved_client.is_none() {
            // Le nom de l'application n'accompagne que le premier enregistrement.
            query.push(("agent_name_hint", APP_NAME.into()));
        } else {
            if let Some(email) = &credentials.email {
                query.push(("login_hint", email.clone()));
            }
            if let Some(id_token) = &credentials.id_token {
                query.push(("id_token_hint", id_token.clone()));
            }
        }
        let url = format!(
            "{}?{}",
            self.inner.endpoints.authorize,
            query
                .iter()
                .map(|(key, value)| format!("{key}={}", encode(value)))
                .collect::<Vec<_>>()
                .join("&")
        );
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut signin = lock(&self.inner.signin);
            signin.phase = Phase::Pending;
            signin.cancel = Some(cancel.clone());
        }
        let provider = self.clone();
        thread::spawn(move || provider.await_callback(server, attempt, cancel));
        if self.inner.open_browser {
            // L'adresse peut contenir `id_token_hint` : elle va au navigateur, jamais dans un journal.
            let _ = std::process::Command::new("/usr/bin/open")
                .arg(&url)
                .spawn();
        }
        Ok(url)
    }

    fn await_callback(&self, server: tiny_http::Server, attempt: Attempt, cancel: Arc<AtomicBool>) {
        let deadline = Instant::now() + SIGN_IN_WINDOW;
        let outcome = loop {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            if Instant::now() >= deadline {
                break Err("La connexion a expiré. Recommencez.".to_owned());
            }
            let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(250)) else {
                continue;
            };
            let url = request.url().to_owned();
            if url.split('?').next() != Some(CALLBACK_PATH) {
                let _ = request.respond(tiny_http::Response::empty(404));
                continue;
            }
            let query = parse_query(&url);
            // Un retour dont l'état ne correspond pas n'appartient pas à cette tentative.
            if query.get("state") != Some(&attempt.state) {
                let _ = request.respond(tiny_http::Response::empty(400));
                continue;
            }
            let outcome = match (query.get("error"), query.get("code")) {
                (Some(error), _) if error == "access_denied" => Err(
                    "Vous avez refusé l’accès. Grokbot n’utilise pas votre abonnement.".to_owned(),
                ),
                (Some(error), _) => Err(format!("OpenAI a refusé la connexion ({error}).")),
                (None, Some(code)) => self
                    .complete(&attempt, code, query.get("client_id"), query.get("scope"))
                    .map_err(|failure| failure.message),
                (None, None) => Err("Le retour d’OpenAI est incomplet.".to_owned()),
            };
            let html = match &outcome {
                Ok(()) => page(
                    "Connexion réussie",
                    "Vous pouvez fermer cette page et revenir dans Grokbot.",
                ),
                Err(message) => page("Connexion non terminée", message),
            };
            let header = tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8")
                .expect("en-tête constant");
            let _ = request.respond(tiny_http::Response::from_string(html).with_header(header));
            break outcome;
        };
        let mut signin = lock(&self.inner.signin);
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        signin.cancel = None;
        signin.phase = match outcome {
            Ok(()) => Phase::Idle,
            Err(message) => Phase::Failed(message),
        };
        drop(signin);
        if self.connected() {
            self.refresh_models();
        }
    }

    fn complete(
        &self,
        attempt: &Attempt,
        code: &str,
        issued_client: Option<&String>,
        callback_scope: Option<&String>,
    ) -> Result<(), Failure> {
        let client_id = match (&attempt.saved_client, issued_client) {
            // Un premier enregistrement doit rendre un identifiant émis, jamais le gabarit.
            (None, Some(issued)) if issued != REGISTRATION_CLIENT => issued.clone(),
            (None, _) => {
                return Err(Failure::other(
                    "OpenAI n’a pas terminé l’enregistrement de Grokbot. Recommencez.",
                ))
            }
            (Some(saved), Some(issued)) if issued != saved => {
                return Err(Failure::other(
                    "OpenAI a répondu pour une autre application. Recommencez.",
                ))
            }
            (Some(saved), _) => saved.clone(),
        };
        let endpoints = &self.inner.endpoints;
        let response = agent()
            .post(&endpoints.token)
            .send_form(&[
                ("grant_type", "authorization_code"),
                ("client_id", &client_id),
                ("code", code),
                ("code_verifier", &attempt.verifier),
                ("redirect_uri", &attempt.redirect_uri),
                ("resource", RESOURCE),
            ])
            .map_err(|error| match request_failure(error) {
                (0, ..) => network_failure(),
                (status, code, message, _) => failure(status, &code, &message),
            })?;
        let tokens: TokenResponse = response
            .into_json()
            .map_err(|_| Failure::other("La réponse d’OpenAI est illisible."))?;
        let id_token = tokens
            .id_token
            .as_deref()
            .ok_or_else(|| Failure::other("OpenAI n’a pas fourni de preuve d’identité."))?;
        let claims = self.verify_identity(id_token, &client_id, &attempt.nonce)?;
        if attempt.saved_client.is_some()
            && attempt
                .saved_subject
                .as_ref()
                .is_some_and(|subject| *subject != claims.sub)
        {
            return Err(Failure::other(
                "Ce compte ChatGPT n’est pas celui déjà enregistré. Déconnectez-vous d’abord.",
            ));
        }
        let scopes = tokens
            .scope
            .clone()
            .or_else(|| callback_scope.cloned())
            .unwrap_or_default();
        let _guard = lock(&self.inner.credentials);
        let mut credentials = self
            .load_or_create()
            .map_err(|_| Failure::other("Impossible d’enregistrer les identifiants sur ce Mac."))?;
        credentials.client_id = Some(client_id);
        credentials.email = claims.email;
        credentials.subject = Some(claims.sub);
        credentials.id_token = Some(id_token.to_owned());
        credentials.access_token = Some(tokens.access_token);
        credentials.refresh_token = tokens.refresh_token;
        credentials.expires_at = seconds() + tokens.expires_in;
        credentials.scopes = scopes;
        credentials.saved_at = seconds();
        let granted = credentials.connected();
        self.write(&credentials)
            .map_err(|_| Failure::other("Impossible d’enregistrer les identifiants sur ce Mac."))?;
        if granted {
            Ok(())
        } else {
            // Une identité valide ne suffit pas : sans cette permission, aucune demande ne part.
            Err(Failure::other("Vous êtes identifié, mais l’utilisation de l’abonnement ChatGPT n’a pas été accordée à Grokbot."))
        }
    }

    fn verify_identity(
        &self,
        id_token: &str,
        client_id: &str,
        nonce: &str,
    ) -> Result<IdClaims, Failure> {
        let invalid = || Failure::other("La preuve d’identité d’OpenAI n’a pas pu être vérifiée.");
        let keys: JwkSet = agent()
            .get(&self.inner.endpoints.jwks)
            .call()
            .map_err(|_| network_failure())?
            .into_json()
            .map_err(|_| invalid())?;
        let key_id = decode_header(id_token)
            .map_err(|_| invalid())?
            .kid
            .ok_or_else(invalid)?;
        let key = DecodingKey::from_jwk(keys.find(&key_id).ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.inner.endpoints.issuer]);
        validation.set_audience(&[client_id]);
        let claims = decode::<IdClaims>(id_token, &key, &validation)
            .map_err(|_| invalid())?
            .claims;
        if claims.nonce.as_deref() != Some(nonce) {
            return Err(invalid());
        }
        Ok(claims)
    }

    /// Rend un jeton d'accès valide, renouvelé s'il expire bientôt ou si `force` le demande.
    fn bearer(&self, force: bool) -> Result<String, Failure> {
        let _guard = lock(&self.inner.credentials);
        let mut credentials = self
            .read()
            .filter(Credentials::connected)
            .ok_or_else(Failure::signin)?;
        let access = credentials
            .access_token
            .clone()
            .expect("vérifié par connected");
        if !force && credentials.expires_at > seconds() + REFRESH_MARGIN {
            return Ok(access);
        }
        let client_id = credentials.client_id.clone().ok_or_else(Failure::signin)?;
        let refresh = credentials
            .refresh_token
            .clone()
            .expect("vérifié par connected");
        let response = agent().post(&self.inner.endpoints.token).send_form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &client_id),
            ("refresh_token", &refresh),
            ("resource", RESOURCE),
        ]);
        let tokens: TokenResponse = match response {
            Ok(response) => response
                .into_json()
                .map_err(|_| Failure::other("La réponse d’OpenAI est illisible."))?,
            Err(error) => {
                let (status, code, message, _) = request_failure(error);
                if status == 0 {
                    // Une panne de réseau ne prouve rien : le jeton en place reste peut-être valide.
                    return if credentials.expires_at > seconds() {
                        Ok(access)
                    } else {
                        Err(network_failure())
                    };
                }
                if matches!(
                    code.as_str(),
                    "invalid_grant"
                        | "invalid_refresh_token"
                        | "token_expired"
                        | "refresh_token_expired"
                        | "refresh_token_invalidated"
                        | "refresh_token_reused"
                ) {
                    credentials.clear_tokens();
                    let _ = self.write(&credentials);
                    return Err(Failure::signin());
                }
                return Err(failure(status, &code, &message));
            }
        };
        // Le jeton d'accès, son échéance, ses permissions et le jeton de renouvellement changent ensemble.
        credentials.access_token = Some(tokens.access_token.clone());
        credentials.expires_at = seconds() + tokens.expires_in;
        if let Some(refresh) = tokens.refresh_token {
            credentials.refresh_token = Some(refresh);
        }
        if let Some(scope) = tokens.scope {
            credentials.scopes = scope;
        }
        if let Some(id_token) = tokens.id_token {
            credentials.id_token = Some(id_token);
        }
        credentials.saved_at = seconds();
        self.write(&credentials)
            .map_err(|_| Failure::other("Impossible d’enregistrer les identifiants sur ce Mac."))?;
        Ok(tokens.access_token)
    }

    /// Révoque la session renouvelable puis efface les jetons. Rend `false` si OpenAI n'a pas confirmé.
    pub fn signout(&self) -> bool {
        self.cancel_signin();
        for cancel in lock(&self.inner.runs).values() {
            cancel.store(true, Ordering::SeqCst);
        }
        let _guard = lock(&self.inner.credentials);
        let Some(mut credentials) = self.read() else {
            return true;
        };
        let revoked = match (&credentials.refresh_token, &credentials.client_id) {
            (Some(refresh), Some(client_id)) => agent()
                .post(&self.inner.endpoints.revoke)
                .send_form(&[
                    ("token", refresh),
                    ("token_type_hint", "refresh_token"),
                    ("client_id", client_id),
                ])
                .is_ok(),
            _ => true,
        };
        // L'enregistrement (client, hôte, courriel) reste : il sert à la prochaine connexion.
        credentials.clear_tokens();
        let _ = self.write(&credentials);
        *lock(&self.inner.catalog) = Catalog::default();
        revoked
    }

    pub fn models(&self) -> Vec<Model> {
        lock(&self.inner.catalog).models.clone()
    }

    pub fn refresh_models(&self) {
        {
            let mut catalog = lock(&self.inner.catalog);
            if catalog.loading {
                return;
            }
            catalog.loading = true;
        }
        let provider = self.clone();
        thread::spawn(move || {
            let models = provider.fetch_models();
            let mut catalog = lock(&provider.inner.catalog);
            catalog.loading = false;
            if let Ok(models) = models {
                catalog.models = models;
                catalog.loaded = true;
            }
        });
    }

    fn fetch_models(&self) -> Result<Vec<Model>, Failure> {
        let token = self.bearer(false)?;
        let body: Value = agent()
            .get(&format!("{}/models", self.inner.endpoints.api))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|_| network_failure())?
            .into_json()
            .map_err(|_| Failure::other("Le catalogue de modèles est illisible."))?;
        Ok(parse_models(&body))
    }

    /// Sends an unmodified Responses body and returns the raw JSON/SSE response.
    /// Authentication is renewed at most once after a 401; other failures are not retried.
    pub fn responses(&self, body: Value) -> Result<ureq::Response, Failure> {
        self.responses_with_options(body, false)
    }

    /// Only the engine adapter opts into the legacy effort/Fast compatibility fallback.
    /// One authentication refresh is shared across all attempts of this logical request.
    /// TODO(engine-cutover): ModelEvent needs a notice channel for downgraded settings.
    fn responses_with_options(
        &self,
        mut body: Value,
        capability_fallback: bool,
    ) -> Result<ureq::Response, Failure> {
        let mut token = self.bearer(false)?;
        let accept = if body["stream"] == true {
            "text/event-stream"
        } else {
            "application/json"
        };
        let mut refreshed = false;
        loop {
            let attempt = agent()
                .post(&format!("{}/responses", self.inner.endpoints.api))
                .set("Authorization", &format!("Bearer {token}"))
                .set("Accept", accept)
                .send_json(body.clone());
            let (status, code, message, param) = match attempt {
                Ok(response) => return Ok(response),
                Err(error) => request_failure(error),
            };
            if status == 401 && !refreshed {
                refreshed = true;
                token = self.bearer(true)?;
            } else if capability_fallback
                && status == 400
                && (param.contains("service_tier") || message.contains("service_tier"))
                && body.get("service_tier").is_some()
            {
                body.as_object_mut()
                    .expect("field belongs to an object")
                    .remove("service_tier");
            } else if capability_fallback
                && status == 400
                && (param.contains("reasoning") || message.contains("reasoning"))
                && body.get("reasoning").is_some()
            {
                body.as_object_mut()
                    .expect("field belongs to an object")
                    .remove("reasoning");
            } else if status == 0 {
                return Err(network_failure());
            } else {
                return Err(failure(status, &code, &message));
            }
        }
    }

    fn run_key(conversation: &str, message: &str) -> String {
        format!("{conversation}\n{message}")
    }

    pub fn running(&self, conversation: &str, message: &str) -> bool {
        lock(&self.inner.runs).contains_key(&Self::run_key(conversation, message))
    }

    pub fn stop(&self, conversation: &str, message: &str) {
        if let Some(cancel) = lock(&self.inner.runs).get(&Self::run_key(conversation, message)) {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Lance la génération en arrière-plan. Le texte partiel et l'état final vont dans SQLite,
    /// où la boucle HTTP les relit : elle ne reste jamais bloquée sur OpenAI.
    pub fn generate(&self, run: Run) {
        let key = Self::run_key(&run.conversation, &run.message);
        let cancel = Arc::new(AtomicBool::new(false));
        lock(&self.inner.runs).insert(key.clone(), cancel.clone());
        let provider = self.clone();
        thread::spawn(move || {
            let outcome = provider.stream(&run, &cancel);
            if !cancel.load(Ordering::SeqCst) {
                let completed = outcome.is_ok() && run.retitle;
                if completed {
                    // Compté avant de marquer la réponse terminée : l'interface ne cesse pas de relire.
                    provider.inner.titling.fetch_add(1, Ordering::SeqCst);
                }
                let connection = provider.database();
                let (status, error, kind, notice) = match outcome {
                    Ok(mut streamed) => {
                        // Les propositions sont enregistrées avant l'état `done` : l'interface les
                        // voit avec la réponse terminée. Un appel invalide devient une note.
                        for call in &streamed.calls {
                            let recorded =
                                connection.as_ref().map_err(|_| ()).and_then(|connection| {
                                    crate::record_proposal(
                                        connection,
                                        &run.conversation,
                                        &run.message,
                                        call,
                                    )
                                    .map_err(|_| ())
                                });
                            match recorded {
                                Ok(Some(notice)) => streamed.notices.push(notice),
                                Ok(None) => {}
                                Err(()) => streamed.notices.push(
                                    "Une proposition de bot n’a pas pu être enregistrée.".into(),
                                ),
                            }
                        }
                        ("done", None, None, streamed.notice())
                    }
                    Err(failure) => ("failed", Some(failure.message), Some(failure.kind), None),
                };
                if let Ok(connection) = connection {
                    let _ = connection.execute(
                        "UPDATE wrapper_generation SET status = ?4, error = ?5, error_kind = ?6, notice = ?7, updated_at = ?8 WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND status = 'streaming'",
                        params![ACCOUNT, run.conversation, run.message, status, error, kind, notice, millis()],
                    );
                }
                if completed {
                    lock(&provider.inner.runs).remove(&key);
                    let _ = provider.retitle(&run);
                    provider.inner.titling.fetch_sub(1, Ordering::SeqCst);
                }
            }
            lock(&provider.inner.runs).remove(&key);
        });
    }

    /// Demande au modèle un titre tiré du sujet et l'écrit, sauf si l'utilisateur a nommé la conversation.
    fn retitle(&self, run: &Run) -> Option<()> {
        let connection = self.database().ok()?;
        let reply: String = connection
            .query_row(
                "SELECT reply FROM wrapper_remote WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3",
                params![ACCOUNT, run.conversation, run.message],
                |row| row.get(0),
            )
            .ok()?;
        let token = self.bearer(false).ok()?;
        let body = json!({
            "model": run.model, "instructions": TITLE_INSTRUCTIONS, "store": false, "stream": true,
            "input": [{"role": "user", "content": title_transcript(&run.history, &reply)}],
        });
        let response = agent()
            .post(&format!("{}/responses", self.inner.endpoints.api))
            .set("Authorization", &format!("Bearer {token}"))
            .set("Accept", "text/event-stream")
            .send_json(body)
            .ok()?;
        let mut text = String::new();
        let mut data = String::new();
        for line in BufReader::new(response.into_reader()).lines() {
            let line = line.ok()?;
            if let Some(chunk) = line.strip_prefix("data:") {
                data.push_str(chunk.trim_start());
            } else if line.is_empty() && !data.is_empty() {
                match stream_event(&data) {
                    StreamEvent::Delta(delta) => text.push_str(&delta),
                    StreamEvent::Completed => break,
                    StreamEvent::Incomplete | StreamEvent::Failed(_) => return None,
                    StreamEvent::CallDelta { .. } | StreamEvent::Call(_) | StreamEvent::Other => {}
                }
                data.clear();
            }
        }
        let title = clean_title(&text)?;
        connection
            .execute(
                "UPDATE wrapper_conversations SET title = ?3 WHERE account_id = ?1 AND id = ?2 AND title_customized = 0 AND deleted = 0",
                params![ACCOUNT, run.conversation, title],
            )
            .ok()?;
        Some(())
    }

    fn database(&self) -> rusqlite::Result<Connection> {
        let connection = Connection::open(&self.inner.db)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        Ok(connection)
    }

    /// Rend les notes à afficher sous la réponse, par exemple quand un réglage a dû être retiré,
    /// et les appels de fonction terminés. Un flux annulé rend un résultat vide.
    fn stream(&self, run: &Run, cancel: &AtomicBool) -> Result<Streamed, Failure> {
        let connection = self
            .database()
            .map_err(|_| Failure::other("Le stockage local est indisponible."))?;
        let mut effort = run.effort.as_deref();
        let mut tier = run.service_tier.as_deref();
        let mut streamed = Streamed::default();
        let mut refreshed = false;
        let response = loop {
            let token = self.bearer(false)?;
            let attempt = agent()
                .post(&format!("{}/responses", self.inner.endpoints.api))
                .set("Authorization", &format!("Bearer {token}"))
                .set("Accept", "text/event-stream")
                .send_json(request_body(run, effort, tier));
            let (status, code, message, param) = match attempt {
                Ok(response) => break response,
                Err(error) => request_failure(error),
            };
            if status == 0 {
                return Err(network_failure());
            }
            if status == 401 && !refreshed {
                refreshed = true;
                self.bearer(true)?;
            } else if status == 400
                && tier.is_some()
                && (param.contains("service_tier") || message.contains("service_tier"))
            {
                tier = None;
                streamed.notices.push("OpenAI n’accepte pas la vitesse Fast avec cette connexion. La réponse a utilisé la vitesse Standard.".into());
            } else if status == 400
                && effort.is_some()
                && (param.contains("reasoning") || message.contains("reasoning"))
            {
                effort = None;
                streamed.notices.push("Ce modèle n’accepte pas ce niveau d’effort. La réponse a utilisé son réglage par défaut.".into());
            } else {
                return Err(failure(status, &code, &message));
            }
        };
        let mut text = String::new();
        let mut data = String::new();
        // Fragments d'arguments par élément, au cas où l'élément terminé arriverait sans eux.
        let mut fragments: HashMap<String, String> = HashMap::new();
        let mut written = Instant::now();
        let save = |text: &str| {
            connection.execute(
                "UPDATE wrapper_remote SET reply = ?4 WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3",
                params![ACCOUNT, run.conversation, run.message, text],
            )
        };
        let interrupted = || {
            Failure::other("La réponse a été interrompue avant la fin. Vous pouvez la relancer.")
        };
        for line in BufReader::new(response.into_reader()).lines() {
            if cancel.load(Ordering::SeqCst) {
                return Ok(Streamed::default());
            }
            let line = line.map_err(|_| interrupted())?;
            if let Some(chunk) = line.strip_prefix("data:") {
                data.push_str(chunk.trim_start());
                continue;
            }
            if !line.is_empty() || data.is_empty() {
                continue;
            }
            let event = stream_event(&data);
            data.clear();
            match event {
                StreamEvent::Delta(delta) => {
                    text.push_str(&delta);
                    if written.elapsed() >= Duration::from_millis(80) {
                        let _ = save(&text);
                        written = Instant::now();
                    }
                }
                StreamEvent::CallDelta { item_id, delta } => {
                    fragments.entry(item_id).or_default().push_str(&delta);
                }
                StreamEvent::Call(mut call) => {
                    if call.arguments.is_empty() {
                        call.arguments = fragments.remove(&call.item_id).unwrap_or_default();
                    }
                    streamed.calls.push(call);
                }
                StreamEvent::Completed | StreamEvent::Incomplete => {
                    if cancel.load(Ordering::SeqCst) {
                        return Ok(Streamed::default());
                    }
                    save(&text)
                        .map_err(|_| Failure::other("Le stockage local est indisponible."))?;
                    if matches!(event, StreamEvent::Incomplete) {
                        streamed
                            .notices
                            .push("OpenAI a arrêté cette réponse avant la fin.".into());
                    }
                    return Ok(streamed);
                }
                StreamEvent::Failed(failure) => {
                    let _ = save(&text);
                    return Err(failure);
                }
                StreamEvent::Other => {}
            }
        }
        if cancel.load(Ordering::SeqCst) {
            return Ok(Streamed::default());
        }
        // Une réponse ne compte comme terminée qu'après `response.completed`.
        let _ = save(&text);
        Err(interrupted())
    }
}

/// The engine adapter is independent of the legacy `Run`/`generate` path until cutover.
impl grokbot_engine::contracts::ModelGateway for Provider {
    fn stream(
        &self,
        request: &grokbot_engine::contracts::ModelRequest,
        emit: &mut dyn FnMut(
            grokbot_engine::contracts::ModelEvent,
        ) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        let body = crate::gateway::request_body(request)?;
        match self.responses_with_options(body, true) {
            Ok(response) => {
                crate::gateway::stream_response(BufReader::new(response.into_reader()), emit)
            }
            Err(failure) => emit(grokbot_engine::contracts::ModelEvent::Failed {
                reason: failure.message,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc_7636_example() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn host_id_is_a_version_4_uuid_urn() {
        let id = new_host_id();
        assert_eq!(id.len(), 45);
        assert!(id.starts_with("urn:uuid:"));
        assert_eq!(id.as_bytes()[23], b'4');
        assert!(matches!(id.as_bytes()[28], b'8' | b'9' | b'a' | b'b'));
        assert_ne!(id, new_host_id());
    }

    #[test]
    fn query_values_round_trip_through_encoding() {
        let value = "user+tag@example.com /é?&=";
        let query = parse_query(&format!(
            "/auth/callback?state={}&code=a%2Bb",
            encode(value)
        ));
        assert_eq!(query["state"], value);
        assert_eq!(query["code"], "a+b");
    }

    #[test]
    fn catalog_reads_efforts_and_speed_tiers_as_the_account_lists_them() {
        // Même forme que le catalogue réel; `default_reasoning_summary` ne doit pas passer pour un effort.
        let models = parse_models(&json!({"models": [
            {"slug": "gpt-a", "display_name": "GPT A", "description": "Modèle principal", "visibility": "list",
             "default_reasoning_level": "low", "default_reasoning_summary": "none", "multi_agent_reasoning_effort": "xhigh",
             "supported_reasoning_levels": [
                {"effort": "low", "description": "Fast responses"}, {"effort": "xhigh", "description": "Extra high"}, {"effort": "max", "description": "Maximum"}],
             "service_tiers": [{"id": "priority", "name": "Fast", "description": "2x speed, increased usage"}]},
            {"slug": "hidden", "display_name": "Hidden", "visibility": "hide"},
            {"slug": "plain", "visibility": "list"},
        ]}));
        assert_eq!(models.len(), 2);
        let efforts: Vec<_> = models[0]
            .efforts
            .iter()
            .map(|level| level.effort.as_str())
            .collect();
        assert_eq!(efforts, ["low", "xhigh", "max"]);
        assert_eq!(models[0].efforts[0].description, "Fast responses");
        assert_eq!(models[0].default_effort.as_deref(), Some("low"));
        assert_eq!(
            (
                models[0].tiers[0].id.as_str(),
                models[0].tiers[0].name.as_str()
            ),
            ("priority", "Fast")
        );
        assert_eq!(models[0].description, "Modèle principal");
        assert_eq!(models[1].name, "plain");
        assert!(
            models[1].efforts.is_empty()
                && models[1].tiers.is_empty()
                && models[1].default_effort.is_none()
        );
    }

    #[test]
    fn titles_are_reduced_to_one_short_clean_line() {
        assert_eq!(
            clean_title("\n« Service de notifications en Rust. »\nAutre ligne").as_deref(),
            Some("Service de notifications en Rust")
        );
        assert_eq!(
            clean_title("**Recette   de\tpain**").as_deref(),
            Some("Recette de pain")
        );
        assert_eq!(clean_title("  \n \"\" "), None);
        let long = clean_title(&"mot ".repeat(40)).unwrap();
        assert!(long.chars().count() <= TITLE_LENGTH && long.ends_with('…'));
        let transcript = title_transcript(&[("user", "Bonjour".into())], &"x".repeat(2000));
        assert!(
            transcript.starts_with("Utilisateur : Bonjour\n\nAssistant : x")
                && transcript.chars().count() < 800
        );
    }

    #[test]
    fn stream_events_separate_text_completion_and_plan_errors() {
        assert_eq!(
            stream_event(r#"{"type":"response.output_text.delta","delta":"Bon"}"#),
            StreamEvent::Delta("Bon".into())
        );
        assert_eq!(
            stream_event(r#"{"type":"response.completed"}"#),
            StreamEvent::Completed
        );
        assert_eq!(
            stream_event(r#"{"type":"response.created"}"#),
            StreamEvent::Other
        );
        let StreamEvent::Failed(failure) = stream_event(
            r#"{"type":"response.failed","response":{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"limit"}}}"#,
        ) else {
            panic!("expected a failure");
        };
        assert_eq!(failure.kind, "limit");
    }

    #[test]
    fn function_call_events_are_read_as_the_documentation_describes_them() {
        assert_eq!(
            stream_event(
                r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"fc_1","type":"function_call","status":"in_progress","call_id":"call_1","name":"propose_bot","arguments":""}}"#
            ),
            StreamEvent::Other
        );
        assert_eq!(
            stream_event(
                r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":1,"delta":"{\"na"}"#
            ),
            StreamEvent::CallDelta {
                item_id: "fc_1".into(),
                delta: "{\"na".into()
            }
        );
        assert_eq!(
            stream_event(
                r#"{"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":1,"arguments":"{}"}"#
            ),
            StreamEvent::Other
        );
        assert_eq!(
            stream_event(
                r#"{"type":"response.output_item.done","output_index":1,"item":{"id":"fc_1","type":"function_call","status":"completed","call_id":"call_1","name":"propose_bot","arguments":"{\"name\":\"X\"}"}}"#
            ),
            StreamEvent::Call(ToolCall {
                item_id: "fc_1".into(),
                call_id: "call_1".into(),
                name: "propose_bot".into(),
                arguments: "{\"name\":\"X\"}".into()
            })
        );
        // Un élément texte terminé n'est pas un appel.
        assert_eq!(
            stream_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message"}}"#
            ),
            StreamEvent::Other
        );
    }

    #[test]
    fn request_body_carries_tools_only_when_the_run_has_some() {
        let mut run = Run {
            conversation: "c".into(),
            message: "m".into(),
            instructions: String::new(),
            tools: Vec::new(),
            history: Vec::new(),
            model: "gpt-a".into(),
            effort: None,
            service_tier: None,
            retitle: false,
        };
        assert!(request_body(&run, None, None).get("tools").is_none());
        run.tools = vec![json!({"type": "function", "name": "propose_bot", "strict": true})];
        assert_eq!(
            request_body(&run, None, None)["tools"][0]["name"],
            "propose_bot"
        );
    }

    #[test]
    fn request_body_follows_the_plan_usage_rules() {
        let run = Run {
            conversation: "c".into(),
            message: "m".into(),
            instructions: "Tu t'appelles Test.".into(),
            tools: Vec::new(),
            history: vec![
                ("user", "Bonjour".into()),
                ("assistant", "Salut".into()),
                ("user", "Encore".into()),
            ],
            model: "gpt-a".into(),
            effort: Some("high".into()),
            service_tier: Some("priority".into()),
            retitle: false,
        };
        let body = request_body(&run, run.effort.as_deref(), run.service_tier.as_deref());
        assert_eq!(
            (body["store"].clone(), body["stream"].clone()),
            (json!(false), json!(true))
        );
        assert_eq!(body["instructions"], "Tu t'appelles Test.");
        assert_eq!(body["input"].as_array().unwrap().len(), 3);
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["service_tier"], "priority");
        let plain = request_body(&run, None, None);
        assert!(plain.get("reasoning").is_none() && plain.get("service_tier").is_none());
        // Champs refusés par ce parcours.
        for field in [
            "previous_response_id",
            "max_output_tokens",
            "temperature",
            "metadata",
            "user",
        ] {
            assert!(body.get(field).is_none());
        }
    }
}
