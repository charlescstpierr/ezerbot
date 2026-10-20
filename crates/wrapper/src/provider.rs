//! Sessions indépendantes : quota Codex (protocole pi openai-codex) et partage
//! d’abonnement « Sign in with ChatGPT ». Aucun identifiant d’un autre logiciel n’est lu.
mod codex;
mod connection;
pub use connection::Provider;

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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const RESOURCE: &str = "https://api.openai.com/v1";
const REGISTRATION_CLIENT: &str = "dynamic_agent_client";
const APP_NAME: &str = "Ezerbot";
// La documentation fixe l'hôte 127.0.0.1 et le chemin; seul le port peut varier.
const CALLBACK_PORTS: std::ops::RangeInclusive<u16> = 1455..=1465;
const CALLBACK_PATH: &str = "/auth/callback";
const SIGN_IN_WINDOW: Duration = Duration::from_secs(300);
const REFRESH_MARGIN: u64 = 300;

#[derive(Clone)]
pub struct Endpoints {
    pub authorize: String,
    pub token: String,
    pub revoke: String,
    pub jwks: String,
    pub issuer: String,
    pub api: String,
}

impl Endpoints {
    pub fn codex() -> Self {
        Self {
            authorize: "https://auth.openai.com/oauth/authorize".into(),
            token: "https://auth.openai.com/oauth/token".into(),
            api: "https://chatgpt.com/backend-api/codex".into(),
            ..Self::default()
        }
    }
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

type MetadataSink<'a> =
    dyn FnMut(Option<&str>, Option<&Failure>) -> grokbot_engine::contracts::Result<()> + 'a;

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

/// Catalogue du compte (`GET /v1/models`) ou catalogue Codex statique repris de pi.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Model {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub efforts: Vec<Effort>,
    pub default_effort: Option<String>,
    pub tiers: Vec<Tier>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Credentials {
    host_id: String,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    plan: Option<String>,
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

    fn usable(&self, codex: bool) -> bool {
        if codex {
            self.access_token.is_some()
                && self.refresh_token.is_some()
                && self.account_id.as_ref().is_some_and(|id| !id.is_empty())
        } else {
            self.connected()
        }
    }

    fn clear_tokens(&mut self) {
        self.access_token = None;
        self.refresh_token = None;
        self.id_token = None;
        self.expires_at = 0;
        self.scopes.clear();
        self.account_id = None;
        self.plan = None;
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
    cancel: Arc<AtomicBool>,
}

struct Inner {
    codex: bool,
    local_revoked: AtomicBool,
    file: PathBuf,
    endpoints: Endpoints,
    open_browser: bool,
    // Sérialise les lectures, les écritures et les renouvellements : le jeton de renouvellement tourne.
    credentials: Mutex<()>,
    signin: Mutex<SignIn>,
    runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    interrupting: AtomicBool,
    // Titres en cours d'écriture : l'interface relit l'état tant qu'il en reste.
    titling: AtomicUsize,
    catalog: Mutex<Catalog>,
    mcp: crate::mcp::Manager,
}

#[derive(Clone)]
struct Session {
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
        .or_else(|| error["type"].as_str())
        .or_else(|| error.as_str())
        .or_else(|| body["code"].as_str())
        .unwrap_or_default();
    let message = error["message"]
        .as_str()
        .or_else(|| body["error_description"].as_str())
        .or_else(|| body["message"].as_str())
        .or_else(|| body["detail"].as_str())
        .unwrap_or_default();
    let param = error["param"]
        .as_str()
        .or_else(|| body["param"].as_str())
        .unwrap_or_default();
    (code.into(), message.into(), param.into())
}

pub(crate) fn failure(status: u16, code: &str, message: &str) -> Failure {
    match code {
        "usage_not_included" => Failure::other("Votre forfait ChatGPT n’inclut pas Codex. Choisissez « Sign in with ChatGPT » dans les paramètres ou vérifiez votre forfait."),
        "usage_limit_reached" => Failure {
            message: "La limite d’utilisation Codex de ce compte ChatGPT est atteinte. Attendez la réinitialisation du quota ou consultez Codex → Réglages → Utilisation.".into(),
            kind: "limit",
        },
        "subscription_sharing_usage_limit_exceeded" => Failure {
            message: "Ezerbot a atteint la limite d’utilisation que lui accorde votre abonnement ChatGPT. Votre quota Codex peut rester disponible. Pour ajuster cette limite : ChatGPT → Réglages → Utilisation.".into(),
            kind: "limit",
        },
        "subscription_sharing_user_not_eligible" => Failure::other("Ce compte ou cet espace de travail ne permet pas d’utiliser l’abonnement ChatGPT dans Ezerbot."),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => Failure::other("Le service d’OpenAI est indisponible pour le moment. Réessayez dans quelques minutes."),
        "subscription_sharing_invalid_user" => Failure::signin(),
        _ if status == 401 => Failure::signin(),
        _ if status == 503 => Failure::other("Le service d’OpenAI est indisponible pour le moment. Réessayez dans quelques minutes."),
        _ if status == 400 && message.to_ascii_lowercase().contains("model is not supported") => {
            let model = message.split('\'').nth(1)
                .filter(|slug| !slug.is_empty() && slug.len() <= 128
                    && slug.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)))
                .map(|slug| format!(" « {slug} »"))
                .unwrap_or_default();
            Failure::other(format!("OpenAI n’accepte pas ce modèle{model} avec votre connexion. Choisissez un autre modèle avec le bouton « Modèle » sous la zone de message, puis cliquez sur Relancer."))
        }
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

fn page(title: &str, detail: &str) -> String {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let title = escape(title);
    let detail = escape(detail);
    format!("<!doctype html><html lang=\"fr\"><meta charset=\"utf-8\"><title>Ezerbot</title><body style=\"margin:0;display:grid;place-items:center;height:100vh;background:#0a0b0d;color:#e6e7ea;font:16px -apple-system,sans-serif\"><main style=\"max-width:420px;padding:24px;text-align:center\"><h1 style=\"font-size:22px\">{title}</h1><p style=\"color:#9a9fa9\">{detail}</p></main></body></html>")
}

impl Session {
    fn with_endpoints(db: &Path, endpoints: Endpoints, open_browser: bool, codex: bool) -> Self {
        let directory = db.parent().filter(|parent| !parent.as_os_str().is_empty());
        let file = directory.unwrap_or(Path::new(".")).join(format!(
            "{}.{}-auth.json",
            db.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("grokbot"),
            if codex { "codex" } else { "chatgpt" }
        ));
        Self {
            inner: Arc::new(Inner {
                codex,
                local_revoked: AtomicBool::new(false),
                file,
                endpoints,
                open_browser,
                credentials: Mutex::new(()),
                signin: Mutex::new(SignIn {
                    phase: Phase::Idle,
                    cancel: None,
                }),
                runs: Mutex::new(HashMap::new()),
                interrupting: AtomicBool::new(false),
                titling: AtomicUsize::new(0),
                catalog: Mutex::new(Catalog::default()),
                mcp: crate::mcp::Manager::default(),
            }),
        }
    }

    pub(crate) fn diagnostics(&self, response: &ureq::Response) -> crate::upstream::Diagnostics {
        crate::upstream::Diagnostics::new(
            self.inner
                .file
                .parent()
                .unwrap_or(Path::new("."))
                .to_owned(),
            response,
        )
    }

    fn request_failure(&self, error: ureq::Error) -> (u16, String, String, String, Failure) {
        match error {
            ureq::Error::Status(status, response) => {
                let diagnostics = self.diagnostics(&response);
                let body: Value = response.into_json().unwrap_or(Value::Null);
                diagnostics.rejected(&body);
                let (code, message, param) = error_parts(&body);
                (
                    status,
                    code,
                    message,
                    param,
                    crate::upstream::failure_from_body(status, &body),
                )
            }
            ureq::Error::Transport(_) => (
                0,
                String::new(),
                String::new(),
                String::new(),
                network_failure(),
            ),
        }
    }

    pub(crate) fn mcp(&self) -> &crate::mcp::Manager {
        &self.inner.mcp
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
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
        if self.inner.local_revoked.load(Ordering::SeqCst) {
            return false;
        }
        let _guard = lock(&self.inner.credentials);
        self.read()
            .is_some_and(|credentials| credentials.usable(self.inner.codex))
    }

    pub fn status(&self) -> Value {
        let credentials = {
            let _guard = lock(&self.inner.credentials);
            self.read().unwrap_or_default()
        };
        let retained =
            self.inner.local_revoked.load(Ordering::SeqCst) && credentials.usable(self.inner.codex);
        let connected = credentials.usable(self.inner.codex)
            && !self.inner.local_revoked.load(Ordering::SeqCst);
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
            "retained": retained,
            "backend": if self.inner.codex { "codex" } else { "sharing" },
            "email": if connected { credentials.email } else { None },
            "plan": if connected { credentials.plan } else { None },
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
        if self.inner.local_revoked.load(Ordering::SeqCst)
            && self.read().is_some_and(|c| c.usable(self.inner.codex))
        {
            return Err(Failure::other(
                "Terminez d’abord la déconnexion sur ce Mac.",
            ));
        }
        self.cancel_signin();
        let credentials = {
            let _guard = lock(&self.inner.credentials);
            self.load_or_create().map_err(|_| {
                Failure::other("Impossible d’enregistrer les identifiants sur ce Mac.")
            })?
        };
        let ports = if self.inner.codex {
            1455..=1455
        } else {
            CALLBACK_PORTS
        };
        let (server, port) = (0..if self.inner.codex { 8 } else { 1 })
            .find_map(|attempt| {
                if attempt > 0 { thread::sleep(Duration::from_millis(50)); }
                ports.clone().find_map(|port| {
                    tiny_http::Server::http(("127.0.0.1", port)).ok().map(|server| (server, port))
                })
            })
            .ok_or_else(|| {
                Failure::other(
                    "Le port de connexion local est occupé. Fermez toute autre connexion ChatGPT en cours, puis recommencez.",
                )
            })?;
        let cancel = Arc::new(AtomicBool::new(false));
        let attempt = Attempt {
            cancel: cancel.clone(),
            state: random_token(32),
            nonce: random_token(32),
            verifier: random_token(48),
            redirect_uri: format!(
                "http://{}:{port}{CALLBACK_PATH}",
                if self.inner.codex {
                    "localhost"
                } else {
                    "127.0.0.1"
                }
            ),
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
        if self.inner.codex {
            query = codex::authorization_query(&attempt);
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
        let mut server = Some(server);
        let deadline = Instant::now() + SIGN_IN_WINDOW;
        let outcome = loop {
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            if Instant::now() >= deadline {
                break Err("La connexion a expiré. Recommencez.".to_owned());
            }
            let Ok(Some(request)) = server
                .as_ref()
                .expect("serveur actif")
                .recv_timeout(Duration::from_millis(250))
            else {
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
            // Libérer le port avant l’échange réseau, qui peut être lent ou annulé.
            drop(server.take());
            let outcome = match (query.get("error"), query.get("code")) {
                (Some(error), _) if error == "access_denied" => Err(if self.inner.codex {
                    "Vous avez refusé l’accès. Ezerbot n’utilise pas votre quota Codex.".to_owned()
                } else {
                    "Vous avez refusé l’accès. Ezerbot n’utilise pas votre abonnement.".to_owned()
                }),
                (Some(_), _) => Err("OpenAI a refusé la connexion. Recommencez.".into()),
                (None, Some(code)) => self
                    .complete(&attempt, code, query.get("client_id"), query.get("scope"))
                    .map_err(|failure| failure.message),
                (None, None) => Err("Le retour d’OpenAI est incomplet.".to_owned()),
            };
            let html = match &outcome {
                Ok(()) => page(
                    "Connexion réussie",
                    "Vous pouvez fermer cette page et revenir dans Ezerbot.",
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
        if self.inner.codex {
            return self.complete_codex(attempt, code);
        }
        let client_id = match (&attempt.saved_client, issued_client) {
            // Un premier enregistrement doit rendre un identifiant émis, jamais le gabarit.
            (None, Some(issued)) if issued != REGISTRATION_CLIENT => issued.clone(),
            (None, _) => {
                return Err(Failure::other(
                    "OpenAI n’a pas terminé l’enregistrement d’Ezerbot. Recommencez.",
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
            .map_err(|error| self.request_failure(error).4)?;
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
        let _signin = lock(&self.inner.signin);
        if attempt.cancel.load(Ordering::SeqCst) {
            return Err(Failure::other("Connexion annulée."));
        }
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
            self.inner.local_revoked.store(false, Ordering::SeqCst);
            Ok(())
        } else {
            // Une identité valide ne suffit pas : sans cette permission, aucune demande ne part.
            Err(Failure::other("Vous êtes identifié, mais l’utilisation de l’abonnement ChatGPT n’a pas été accordée à Ezerbot."))
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
        if self.inner.local_revoked.load(Ordering::SeqCst) {
            return Err(Failure::signin());
        }
        let mut credentials = self
            .read()
            .filter(|c| c.usable(self.inner.codex))
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
        let mut fields = vec![
            ("grant_type", "refresh_token"),
            ("client_id", &client_id),
            ("refresh_token", &refresh),
        ];
        if !self.inner.codex {
            fields.push(("resource", RESOURCE));
        }
        let response = agent().post(&self.inner.endpoints.token).send_form(&fields);
        let tokens: TokenResponse = match response {
            Ok(response) => response
                .into_json()
                .map_err(|_| Failure::other("La réponse d’OpenAI est illisible."))?,
            Err(error) => {
                let (status, code, _, _, failure) = self.request_failure(error);
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
                return Err(failure);
            }
        };
        // Le jeton d'accès, son échéance, ses permissions et le jeton de renouvellement changent ensemble.
        if self.inner.codex {
            codex::update_profile(&mut credentials, &tokens)?;
        }
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
    pub fn signout(&self) -> Result<bool, Failure> {
        self.cancel_signin();
        self.inner.local_revoked.store(true, Ordering::SeqCst);
        // The engine route durably stops only provider-consuming runs. A blanket
        // signal here could cancel an already-suspended form during owner release.
        let _guard = lock(&self.inner.credentials);
        let Some(mut credentials) = self.read() else {
            return Ok(true);
        };
        let refresh_token = credentials.refresh_token.clone();
        let client_id = credentials.client_id.clone();
        credentials.clear_tokens();
        self.write(&credentials).map_err(|_| {
            Failure::other(
                "Impossible d’effacer la connexion sur ce Mac. Réessayez avant de fermer Ezerbot.",
            )
        })?;
        *lock(&self.inner.catalog) = Catalog::default();
        // Comme pi, Codex déconnecte localement; aucune révocation distante n’est documentée.
        let revoked = self.inner.codex
            || match (&refresh_token, &client_id) {
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
        // L’enregistrement local reste, sans aucun jeton.
        Ok(revoked)
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
        if self.inner.codex {
            return Ok(codex::models());
        }
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

    /// One small, bounded best-effort call. No Fast tier, history, tools, retries
    /// or user-visible failure. The catalogue is authoritative; prefer nano/mini
    /// when available. If no small model is advertised, skip rather than charge
    /// a large model implicitly.
    pub(crate) fn extract_memory(
        &self,
        instructions: &str,
        input: &str,
    ) -> Result<String, Failure> {
        use grokbot_engine::contracts::ModelEvent;
        use std::io::Read;
        let models = self.models();
        let model = models
            .iter()
            .filter(|m| {
                m.slug.contains("nano")
                    || m.slug.contains("mini")
                    || (self.inner.codex && m.slug == "gpt-6-luna")
            })
            .min_by_key(|m| {
                if m.slug.contains("nano") {
                    0
                } else if m.slug.contains("mini") {
                    1
                } else {
                    2
                }
            })
            .ok_or_else(|| Failure::other("Modèle indisponible."))?;
        let mut body = json!({"model":model.slug,"instructions":instructions,
            "input":[{"role":"user","content":input}],"store":false,"stream":true,
            "max_output_tokens":900});
        if let Some(effort) = model
            .efforts
            .iter()
            .min_by_key(|e| match e.effort.as_str() {
                "none" => 0,
                "minimal" => 1,
                "low" => 2,
                "medium" => 3,
                _ => 4,
            })
        {
            body["reasoning"] = json!({"effort":effort.effort});
        }
        // Background work never waits for credential rotation or refreshes it.
        // A foreground call will renew an expiring token; skipping is harmless.
        // Credential writes use atomic rename. A read-only snapshot needs no
        // rotation lock (and cannot randomly lose a job to the UI's status read).
        let credentials = self
            .read()
            .filter(|c| c.usable(self.inner.codex))
            .filter(|c| c.expires_at > seconds() + 15)
            .ok_or_else(Failure::signin)?;
        let token = credentials.access_token.ok_or_else(Failure::signin)?;
        let response = self
            .request(&token, &mut body)?
            .timeout(Duration::from_secs(12))
            .send_json(body)
            .map_err(|error| {
                self.request_failure(error);
                Failure::other("Extraction indisponible.")
            })?;
        let diagnostics = self.diagnostics(&response);
        let mut text = String::new();
        let mut completed = false;
        // A real Responses stream repeats metadata/instructions and sends one
        // envelope per token. Leave room for 900 such deltas while still bounding
        // transport bytes independently of the 4096-byte useful-text limit.
        crate::gateway::stream_response(
            BufReader::new(response.into_reader().take(512 * 1024)),
            &diagnostics,
            &mut |event| {
                match event {
                    ModelEvent::TextDelta { text: delta } if text.len() + delta.len() <= 4096 => {
                        text.push_str(&delta)
                    }
                    ModelEvent::Completed => completed = true,
                    ModelEvent::Usage { .. } | ModelEvent::Reasoning { .. } => {}
                    _ => {
                        return Err(grokbot_engine::contracts::Error::InvalidInput(
                            "memory extraction rejected".into(),
                        ))
                    }
                }
                Ok(())
            },
        )
        .map_err(|_| Failure::other("Extraction ignorée."))?;
        if !completed {
            return Err(Failure::other("Extraction incomplète."));
        }
        Ok(text)
    }

    /// Sends an unmodified Responses body and returns the raw JSON/SSE response.
    /// Authentication is renewed at most once after a 401; other failures are not retried.
    pub fn responses(&self, body: Value) -> Result<ureq::Response, Failure> {
        self.responses_with_options(body, false, &mut Vec::new())
    }

    /// Isolated, bounded policy call. No transcript, tools, cached conversation,
    /// effort/Fast retry or provider error text enters the reviewer boundary.
    pub(crate) fn review(&self, mut body: Value) -> Result<String, Failure> {
        use grokbot_engine::contracts::ModelEvent;
        let token = self.bearer(false)?;
        let response = self
            .request(&token, &mut body)?
            .timeout(Duration::from_secs(10))
            .send_json(body)
            .map_err(|error| {
                self.request_failure(error);
                Failure::other("Revue indisponible.")
            })?;
        let diagnostics = self.diagnostics(&response);
        let mut text = String::new();
        let mut completed = false;
        let mut invalid = false;
        // Bound both the total wire response and the collected answer.
        let mut wire = String::new();
        std::io::Read::read_to_string(
            &mut std::io::Read::take(response.into_reader(), 32_769),
            &mut wire,
        )
        .map_err(|_| Failure::other("Revue interrompue."))?;
        if wire.len() >= 32_769 {
            return Err(Failure::other("Revue trop longue."));
        }
        // The chat parser intentionally ignores non-text events; a policy reply
        // must additionally reject refusals, unknown events and opaque items.
        let mut terminal = false;
        for line in wire.lines().filter_map(|line| line.strip_prefix("data:")) {
            if terminal {
                return Err(Failure::other("Revue contradictoire."));
            }
            let event: Value = serde_json::from_str(line.trim())
                .map_err(|_| Failure::other("Revue illisible."))?;
            let kind = event["type"].as_str().unwrap_or_default();
            if kind == "response.failed" || kind == "error" {
                diagnostics.rejected(if kind == "response.failed" {
                    &event["response"]
                } else {
                    &event
                });
            }
            if ![
                "response.created",
                "response.in_progress",
                "response.output_item.added",
                "response.output_item.done",
                "response.content_part.added",
                "response.content_part.done",
                "response.output_text.delta",
                "response.output_text.done",
                "response.completed",
                "response.reasoning_summary_part.added",
                "response.reasoning_summary_part.done",
                "response.reasoning_summary_text.delta",
                "response.reasoning_summary_text.done",
            ]
            .contains(&kind)
                || (kind.starts_with("response.output_item.")
                    && !["message", "reasoning"]
                        .contains(&event["item"]["type"].as_str().unwrap_or_default()))
                || (kind.starts_with("response.content_part.")
                    && event["part"]["type"] != "output_text")
                || (kind == "response.output_text.delta" && !event["delta"].is_string())
                || (kind == "response.output_text.done" && !event["text"].is_string())
                || (kind == "response.completed"
                    && event["response"]
                        .get("status")
                        .is_some_and(|status| status != "completed"))
                || event.to_string().contains("\"refusal\"")
            {
                return Err(Failure::other("Revue incertaine."));
            }
            terminal = kind == "response.completed";
        }
        crate::gateway::stream_response(std::io::Cursor::new(wire), &diagnostics, &mut |event| {
            match event {
                ModelEvent::TextDelta { text: delta }
                    if !completed && text.len() + delta.len() <= 4096 =>
                {
                    text.push_str(&delta)
                }
                ModelEvent::Completed if !completed => completed = true,
                ModelEvent::Usage { .. } => {}
                _ => invalid = true,
            }
            Ok(())
        })
        .map_err(|_| Failure::other("Revue illisible."))?;
        if !completed || invalid {
            return Err(Failure::other("Revue incomplète."));
        }
        Ok(text)
    }

    /// Only the engine adapter opts into the legacy effort/Fast compatibility fallback.
    /// One authentication refresh is shared across all attempts of this logical request.
    fn responses_with_options(
        &self,
        mut body: Value,
        capability_fallback: bool,
        notices: &mut Vec<String>,
    ) -> Result<ureq::Response, Failure> {
        let mut token = self.bearer(false)?;
        let mut refreshed = false;
        loop {
            let attempt = self.request(&token, &mut body)?.send_json(body.clone());
            let (status, _, message, param, failure) = match attempt {
                Ok(response) => return Ok(response),
                Err(error) => self.request_failure(error),
            };
            if status == 401 && !refreshed {
                refreshed = true;
                token = self.bearer(true)?;
            } else if capability_fallback
                && !self.inner.codex
                && status == 400
                && (param.contains("service_tier") || message.contains("service_tier"))
                && body.get("service_tier").is_some()
            {
                body.as_object_mut()
                    .expect("field belongs to an object")
                    .remove("service_tier");
                notices.push("OpenAI n’accepte pas la vitesse Fast avec cette connexion. La réponse a utilisé la vitesse Standard.".into());
            } else if capability_fallback
                && !self.inner.codex
                && status == 400
                && (param.contains("reasoning") || message.contains("reasoning"))
                && body.get("reasoning").is_some()
            {
                body.as_object_mut()
                    .expect("field belongs to an object")
                    .remove("reasoning");
                notices.push("Ce modèle n’accepte pas ce niveau d’effort. La réponse a utilisé son réglage par défaut.".into());
            } else if status == 0 {
                return Err(network_failure());
            } else {
                return Err(failure);
            }
        }
    }

    /// Process-local ownership only; durable lifecycle belongs to grokbot-engine.
    pub(crate) fn claim_run(&self, id: &str) -> Option<Arc<AtomicBool>> {
        let mut runs = lock(&self.inner.runs);
        if self.interrupting() || runs.contains_key(id) {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        runs.insert(id.into(), cancel.clone());
        Some(cancel)
    }
    pub(crate) fn interrupting(&self) -> bool {
        self.inner.interrupting.load(Ordering::SeqCst)
    }
    /// Freeze admission and ask all workers to persist Interrupted, not Stopped.
    pub(crate) fn interrupt_runs(&self) {
        let runs = lock(&self.inner.runs);
        self.inner.interrupting.store(true, Ordering::SeqCst);
        for cancel in runs.values() {
            cancel.store(true, Ordering::SeqCst);
        }
    }
    pub(crate) fn has_run_workers(&self) -> bool {
        !lock(&self.inner.runs).is_empty()
    }
    pub(crate) fn release_run(&self, id: &str) {
        lock(&self.inner.runs).remove(id);
    }
    pub(crate) fn cancel_run(&self, id: &str) {
        if let Some(cancel) = lock(&self.inner.runs).get(id) {
            cancel.store(true, Ordering::SeqCst);
        }
    }
    pub(crate) fn title_started(&self) {
        self.inner.titling.fetch_add(1, Ordering::SeqCst);
    }
    pub(crate) fn title_finished(&self) {
        self.inner.titling.fetch_sub(1, Ordering::SeqCst);
    }

    /// Wrapper metadata is an adapter side channel, not another text generator.
    /// This preserves Fast notices and reconnect/usage actions without changing
    /// the shared ModelEvent contract or bypassing its one Responses parser.
    pub(crate) fn stream_run(
        &self,
        request: &grokbot_engine::contracts::ModelRequest,
        cancel: &AtomicBool,
        metadata: &mut MetadataSink<'_>,
        emit: &mut dyn FnMut(
            grokbot_engine::contracts::ModelEvent,
        ) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        use grokbot_engine::contracts::{Error, ModelEvent};
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::InvalidInput("run cancelled".into()));
        }
        let mut body = crate::gateway::request_body(request)?;
        // Keep the accepted legacy plain-text input shape for ordinary chats.
        for item in body["input"].as_array_mut().expect("gateway input") {
            if let Some(content) = item["content"].as_array() {
                if content.len() == 1 && content[0]["type"] == "input_text" {
                    item["content"] = content[0]["text"].clone();
                }
            }
        }
        let mut notices = Vec::new();
        let response = match self.responses_with_options(body, true, &mut notices) {
            Ok(response) => response,
            Err(failure) => {
                metadata(None, Some(&failure))?;
                return emit(ModelEvent::Failed {
                    reason: failure.message,
                });
            }
        };
        if !notices.is_empty() {
            metadata(Some(&notices.join(" ")), None)?;
        }
        let diagnostics = self.diagnostics(&response);
        let reader = Cancellable {
            reader: BufReader::new(response.into_reader()),
            cancel,
        };
        crate::gateway::stream_response(reader, &diagnostics, &mut |event| {
            if matches!(event, ModelEvent::Incomplete { .. }) {
                metadata(
                    Some("OpenAI a arrêté cette réponse avant la fin."),
                    Some(&Failure {
                        message: "OpenAI a arrêté cette réponse avant la fin.".into(),
                        kind: "incomplete",
                    }),
                )?;
            }
            if let ModelEvent::Failed { reason } = &event {
                // Preserve the upstream Failure's classification independently
                // of its French wording. Parser/transport failures stay "other".
                let kind = diagnostics.failure_kind();
                metadata(
                    None,
                    Some(&Failure {
                        message: reason.clone(),
                        kind,
                    }),
                )?;
            }
            emit(event)
        })
    }
}

/// Responses is the sole model transport; the engine owns generation and replay.
impl grokbot_engine::contracts::ModelGateway for Provider {
    fn stream(
        &self,
        request: &grokbot_engine::contracts::ModelRequest,
        emit: &mut dyn FnMut(
            grokbot_engine::contracts::ModelEvent,
        ) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        let body = crate::gateway::request_body(request)?;
        match self.responses_with_options(body, true, &mut Vec::new()) {
            Ok(response) => {
                let diagnostics = self.diagnostics(&response);
                crate::gateway::stream_response(
                    BufReader::new(response.into_reader()),
                    &diagnostics,
                    emit,
                )
            }
            Err(failure) => emit(grokbot_engine::contracts::ModelEvent::Failed {
                reason: failure.message,
            }),
        }
    }
}

/// Probe cancellation even while the shared parser ignores a keepalive or an
/// argument delta. A blocked network read still obeys the transport deadline.
struct Cancellable<'a, R> {
    reader: R,
    cancel: &'a AtomicBool,
}
impl<R: BufRead> std::io::Read for Cancellable<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("run cancelled"));
        }
        self.reader.read(bytes)
    }
}
impl<R: BufRead> BufRead for Cancellable<'_, R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("run cancelled"));
        }
        self.reader.fill_buf()
    }
    fn consume(&mut self, amount: usize) {
        self.reader.consume(amount);
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
}
