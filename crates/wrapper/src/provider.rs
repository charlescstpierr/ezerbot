//! Compte ChatGPT par OAuth Codex, avec une session propre à Ezerbot.
//! Aucun identifiant d’un autre logiciel n’est lu. L'ancien protocole /v1
//! n'est accessible que par injection explicite de serveurs de test locaux.
mod codex;
mod connection;
mod reliable;
pub use connection::Provider;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
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
/// Attente courte : un flock tenu par un processus vivant n’est jamais volé.
const REFRESH_LOCK_WAIT: Duration = Duration::from_millis(400);
/// Dernier recours absolu si le dossier personnel est inconnu. Jamais le répertoire courant.
const FALLBACK_HOME: &str = "/var/empty";

thread_local! {
    static HELD_LOCKS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

fn holds_lock(path: &Path) -> bool {
    HELD_LOCKS.with(|held| held.borrow().iter().any(|item| item == path))
}

/// Même dossier que l’application Cocoa : `Application Support/Grokbot`.
/// `Grokbot Local` n’est plus une destination. Une session déjà là est fusionnée
/// une fois sous verrou, puis le fichier d’origine reste sans `refresh_token`.
///
/// Les jetons restent dans le JSON propriétaire voisin de la base
/// (`workspace.codex-auth.json` ou `workspace.chatgpt-auth.json`, mode 0600).
/// Ils ne vont pas dans le trousseau : une signature ad hoc change de cdhash
/// à chaque compilation et le lancement suivant lirait un trousseau vide.
pub fn database_path_from_home(home: Option<&OsStr>, fixed: Option<&OsStr>) -> PathBuf {
    fn usable(value: Option<&OsStr>) -> Option<&Path> {
        let path = Path::new(value?);
        (path.is_absolute() && path != Path::new("/") && path.as_os_str().len() > 1).then_some(path)
    }
    let root = usable(home)
        .or_else(|| usable(fixed))
        .map(Path::to_path_buf)
        .or_else(passwd_home)
        .unwrap_or_else(|| PathBuf::from(FALLBACK_HOME));
    root.join("Library/Application Support/Grokbot/workspace.sqlite")
}

/// `getpwuid_r` via l’API sûre de `nix`. Un dossier `/` ou relatif est ignoré.
fn passwd_home() -> Option<PathBuf> {
    let user = nix::unistd::User::from_uid(nix::unistd::Uid::current())
        .ok()
        .flatten()?;
    let dir = user.dir;
    (dir.is_absolute() && dir != Path::new("/") && dir.as_os_str().len() > 1).then_some(dir)
}

pub fn default_database_path() -> PathBuf {
    database_path_from_home(
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("CFFIXED_USER_HOME").as_deref(),
    )
}

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
                "La connexion à ChatGPT n’est plus valide. Reconnectez-vous à votre compte ChatGPT."
                    .into(),
            kind: "signin",
        }
    }

    fn expired() -> Self {
        Self {
            message: "Votre session ChatGPT a expiré. Reconnectez-vous à votre compte ChatGPT."
                .into(),
            kind: "signin_expired",
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
    deadlines: Mutex<reliable::Deadlines>,
    transports: AtomicUsize,
    closing: AtomicUsize,
    simulation: AtomicBool,
    probe: Mutex<Value>,
    probe_generation: AtomicUsize,
    local_revoked: AtomicBool,
    file: PathBuf,
    endpoints: Endpoints,
    open_browser: bool,
    // Mutex du processus, pris après le verrou fichier.
    credentials: Mutex<()>,
    signin: Mutex<SignIn>,
    runs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Lectures encore ouvertes, pour les compter dès l'annulation.
    live_reads: Mutex<HashMap<String, Arc<Mutex<reliable::Settled>>>>,
    #[cfg(test)]
    paused_turns: Mutex<std::collections::HashSet<String>>,
    interrupting: AtomicBool,
    // Titres en cours d'écriture : l'interface relit l'état tant qu'il en reste.
    titling: AtomicUsize,
    catalog: Mutex<Catalog>,
    quota: Mutex<codex::UsageCache>,
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
        .timeout_read(Duration::from_secs(12))
        .timeout_write(Duration::from_secs(12))
        .timeout(Duration::from_secs(12))
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
        "subscription_sharing_invalid_user" => Failure::other("OpenAI a refusé cet utilisateur pour cette demande. La session enregistrée sur ce Mac est conservée."),
        // Ces codes, hors refus explicite du refresh_token stocké, ne demandent pas de reconnexion.
        "invalid_grant" | "invalid_refresh_token" | "token_expired" | "refresh_token_expired"
        | "refresh_token_invalidated" | "refresh_token_reused" => Failure::other("OpenAI a refusé la demande. La session enregistrée sur ce Mac est conservée."),
        _ if status == 401 => Failure::other("OpenAI a refusé la demande (erreur 401). La session enregistrée sur ce Mac est conservée."),
        _ if status == 503 => Failure::other(format!("Le service d’OpenAI est indisponible (HTTP 503). Réessayez dans quelques minutes.{}", if message.is_empty() { String::new() } else { format!(" Détail : {}", message.chars().take(300).collect::<String>()) })),
        _ if status == 400 && message.to_ascii_lowercase().contains("model is not supported") => {
            let model = message.split('\'').nth(1)
                .filter(|slug| !slug.is_empty() && slug.len() <= 128
                    && slug.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)))
                .map(|slug| format!(" « {slug} »"))
                .unwrap_or_default();
            Failure::other(format!("OpenAI n’accepte pas ce modèle{model} avec votre connexion. Choisissez un autre modèle avec le bouton « Modèle » sous la zone de message, puis relancez la réponse ou envoyez un nouveau message."))
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

fn explicit_refresh_refusal(code: &str) -> bool {
    matches!(
        code,
        "invalid_grant"
            | "invalid_refresh_token"
            | "token_expired"
            | "refresh_token_expired"
            | "refresh_token_invalidated"
            | "refresh_token_reused"
    )
}

fn preserved_scope(codex: bool, stored: &str, granted: String) -> String {
    let drops_plan = !codex
        && stored.split_whitespace().any(|scope| scope == PLAN_SCOPE)
        && !granted.split_whitespace().any(|scope| scope == PLAN_SCOPE);
    if drops_plan {
        stored.to_owned()
    } else {
        granted
    }
}

/// Verrou `flock` tenu par le processus. Le noyau le lâche à la mort du
/// processus. Un second descripteur n’est jamais ouvert pour le même fichier :
/// le fermer libérerait le verrou du processus sur macOS.
struct AuthLock {
    path: PathBuf,
    _file: nix::fcntl::Flock<fs::File>,
}

impl AuthLock {
    /// `None` si ce fil tient déjà ce fichier, ou si un autre processus vivant
    /// le tient encore après une courte attente. Jamais de vol.
    fn acquire(auth_file: &Path) -> Option<Self> {
        let path = auth_file.with_extension("lock");
        if holds_lock(&path) {
            return None;
        }
        let started = Instant::now();
        loop {
            let file = open_lock(&path)?;
            match nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock) {
                Ok(file) => {
                    HELD_LOCKS.with(|held| held.borrow_mut().push(path.clone()));
                    return Some(Self { path, _file: file });
                }
                Err((file, _)) => {
                    drop(file);
                    if started.elapsed() > REFRESH_LOCK_WAIT {
                        return None;
                    }
                    thread::sleep(Duration::from_millis(15));
                }
            }
        }
    }
}

impl Drop for AuthLock {
    fn drop(&mut self) {
        HELD_LOCKS.with(|held| {
            let mut held = held.borrow_mut();
            if let Some(index) = held.iter().rposition(|item| item == &self.path) {
                held.remove(index);
            }
        });
    }
}

fn open_lock(path: &Path) -> Option<fs::File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).ok()?;
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
    Some(file)
}

fn legacy_auth_path(primary: &Path) -> Option<PathBuf> {
    let parent = primary.parent()?;
    if parent.file_name().and_then(|name| name.to_str()) != Some("Grokbot") {
        return None;
    }
    let name = primary.file_name()?;
    Some(parent.parent()?.join("Grokbot Local").join(name))
}

fn read_credentials(path: &Path) -> Option<Credentials> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn legacy_has_refresh(path: &Path) -> bool {
    read_credentials(path)
        .and_then(|credentials| credentials.refresh_token)
        .is_some_and(|token| !token.is_empty())
}

fn temporary_is_newer(primary: &Path, temporary: &Path) -> bool {
    if !temporary.is_file() {
        return false;
    }
    let Some(credentials) = read_credentials(temporary) else {
        return false;
    };
    if credentials
        .refresh_token
        .as_deref()
        .is_none_or(|token| token.is_empty())
    {
        return false;
    }
    let Ok(temporary_time) = fs::metadata(temporary).and_then(|meta| meta.modified()) else {
        return false;
    };
    match fs::metadata(primary).and_then(|meta| meta.modified()) {
        Ok(primary_time) => temporary_time > primary_time,
        Err(_) => true,
    }
}

fn promote_temporary(temporary: &Path, primary: &Path) -> std::io::Result<()> {
    if let Some(parent) = primary.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(temporary, primary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(primary, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn tombstone(path: &Path, source: &Credentials) {
    let mut cleared = source.clone();
    cleared.clear_tokens();
    let _ = write_credentials(path, &cleared);
}

fn write_credentials(path: &Path, credentials: &Credentials) -> std::io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
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
    file.write_all(&serde_json::to_vec_pretty(credentials).expect("identifiants sérialisables"))?;
    file.sync_all()?;
    fs::rename(temporary, path)
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
        // Session ChatGPT : JSON propriétaire, jamais le trousseau lié à la signature.
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
                deadlines: Mutex::new(reliable::Deadlines::default()),
                transports: AtomicUsize::new(0),
                closing: AtomicUsize::new(0),
                simulation: AtomicBool::new(
                    cfg!(debug_assertions)
                        && std::env::var_os("GROKBOT_DISABLE_SIMULATION").is_none(),
                ),
                probe: Mutex::new(json!({"state":"idle"})),
                probe_generation: AtomicUsize::new(0),
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
                live_reads: Mutex::new(HashMap::new()),
                #[cfg(test)]
                paused_turns: Mutex::new(std::collections::HashSet::new()),
                interrupting: AtomicBool::new(false),
                titling: AtomicUsize::new(0),
                catalog: Mutex::new(Catalog::default()),
                quota: Mutex::new(codex::UsageCache::default()),
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
        self.adopt_pending_write();
        self.import_legacy_session();
        self.read_raw()
    }

    fn read_raw(&self) -> Option<Credentials> {
        serde_json::from_slice(&fs::read(&self.inner.file).ok()?).ok()
    }

    /// Un processus tué après `sync` et avant `rename` laisse un `*.json.tmp`
    /// plus récent. On le reprend au prochain démarrage s’il contient un jeton.
    fn adopt_pending_write(&self) {
        let temporary = self.inner.file.with_extension("json.tmp");
        if !temporary_is_newer(&self.inner.file, &temporary) {
            return;
        }
        let promote = || {
            if temporary_is_newer(&self.inner.file, &temporary) {
                let _ = promote_temporary(&temporary, &self.inner.file);
            }
        };
        if holds_lock(&self.inner.file.with_extension("lock")) {
            promote();
            return;
        }
        let Some(_lock) = AuthLock::acquire(&self.inner.file) else {
            return;
        };
        promote();
    }

    /// Fusionne `Grokbot Local/<fichier>` vers `Grokbot/<fichier>`. Jamais
    /// l’inverse. Les deux chemins sont verrouillés. L’ancien fichier reste,
    /// sans `refresh_token`.
    fn import_legacy_session(&self) {
        let Some(legacy) = legacy_auth_path(&self.inner.file) else {
            return;
        };
        if !legacy_has_refresh(&legacy) {
            return;
        }
        if holds_lock(&self.inner.file.with_extension("lock")) {
            self.migrate_assuming_locked();
            return;
        }
        let Some(_lock) = AuthLock::acquire(&self.inner.file) else {
            return;
        };
        self.migrate_assuming_locked();
    }

    fn migrate_assuming_locked(&self) {
        let Some(legacy) = legacy_auth_path(&self.inner.file) else {
            return;
        };
        if !legacy.is_file() {
            return;
        }
        let Some(_source) = AuthLock::acquire(&legacy) else {
            return;
        };
        let Some(source) = read_credentials(&legacy) else {
            return;
        };
        if source
            .refresh_token
            .as_deref()
            .is_none_or(|token| token.is_empty())
        {
            return;
        }
        if self
            .read_raw()
            .and_then(|credentials| credentials.refresh_token)
            .is_some_and(|token| !token.is_empty())
        {
            tombstone(&legacy, &source);
            return;
        }
        let mut destination = self.read_raw().unwrap_or_default();
        let host = if destination.host_id.is_empty() {
            source.host_id.clone()
        } else {
            destination.host_id.clone()
        };
        destination = source.clone();
        destination.host_id = host;
        if self.write(&destination).is_err() {
            return;
        }
        tombstone(&legacy, &source);
    }

    /// Écriture atomique, lisible par le seul propriétaire.
    fn write(&self, credentials: &Credentials) -> std::io::Result<()> {
        write_credentials(&self.inner.file, credentials)
    }

    /// L'identifiant d'hôte doit exister et rester stable avant la première connexion.
    fn load_or_create(&self) -> std::io::Result<Credentials> {
        let _lock = self.lock_auth()?;
        self.migrate_assuming_locked();
        if let Some(credentials) = self.read_raw() {
            return Ok(credentials);
        }
        let credentials = Credentials {
            host_id: new_host_id(),
            ..Credentials::default()
        };
        self.write(&credentials)?;
        Ok(credentials)
    }

    /// Verrou déjà pris par l’appelant : `Ok(None)`. Sinon le garde doit vivre
    /// jusqu’à la fin de l’écriture. Échec = ne rien écrire et ne rien renouveler.
    fn lock_auth(&self) -> std::io::Result<Option<AuthLock>> {
        if holds_lock(&self.inner.file.with_extension("lock")) {
            return Ok(None);
        }
        AuthLock::acquire(&self.inner.file)
            .map(Some)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "verrou d’authentification occupé",
                )
            })
    }

    pub fn connected(&self) -> bool {
        if self.inner.local_revoked.load(Ordering::SeqCst) {
            return false;
        }
        // Atomic rename makes a read-only snapshot safe. Never wait behind a
        // network refresh on the UI/server thread.
        self.read()
            .is_some_and(|credentials| credentials.usable(self.inner.codex))
    }

    pub fn status(&self) -> Value {
        let credentials = self.read().unwrap_or_default();
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
        let _lock = self.lock_auth()?;
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
        let before = self
            .read()
            .and_then(|credentials| credentials.refresh_token);
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
        let email = claims.email;
        let subject = claims.sub;
        let id_token = id_token.to_owned();
        let access_token = tokens.access_token;
        let refresh_token = tokens.refresh_token;
        let expires_at = seconds() + tokens.expires_in;
        let mut granted = false;
        let kept = self.store_signin(before.as_deref(), |credentials| {
            credentials.client_id = Some(client_id);
            credentials.email = email;
            credentials.subject = Some(subject);
            credentials.id_token = Some(id_token);
            credentials.access_token = Some(access_token);
            credentials.refresh_token = refresh_token;
            credentials.expires_at = expires_at;
            credentials.scopes = scopes;
            credentials.saved_at = seconds();
            granted = credentials.connected();
            Ok(())
        })?;
        if kept || granted {
            self.reset_probe();
            self.inner.local_revoked.store(false, Ordering::SeqCst);
            Ok(())
        } else {
            // Une identité valide ne suffit pas : sans cette permission, aucune demande ne part.
            Err(Failure::other("Vous êtes identifié, mais l’utilisation de l’abonnement ChatGPT n’a pas été accordée à Ezerbot."))
        }
    }

    /// Écriture de connexion sous le verrou, après relecture. `true` si une paire
    /// plus récente est déjà sur le disque : elle n’est pas écrasée.
    fn store_signin(
        &self,
        before: Option<&str>,
        apply: impl FnOnce(&mut Credentials) -> Result<(), Failure>,
    ) -> Result<bool, Failure> {
        let Some(_lock) = AuthLock::acquire(&self.inner.file) else {
            return Err(Failure::other(
                "L’enregistrement de la session est déjà en cours. Réessayez.",
            ));
        };
        let _guard = lock(&self.inner.credentials);
        let current = self.read();
        if current.as_ref().and_then(|c| c.refresh_token.as_deref()) != before
            && current.as_ref().is_some_and(|c| c.usable(self.inner.codex))
        {
            return Ok(true);
        }
        let mut credentials = self
            .load_or_create()
            .map_err(|_| Failure::other("Impossible d’enregistrer les identifiants sur ce Mac."))?;
        apply(&mut credentials)?;
        self.write(&credentials)
            .map_err(|_| Failure::other("Impossible d’enregistrer les identifiants sur ce Mac."))?;
        Ok(false)
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
        if !force && !self.inner.local_revoked.load(Ordering::SeqCst) {
            if let Some(credentials) = self
                .read()
                .filter(|c| c.usable(self.inner.codex) && c.expires_at > seconds() + REFRESH_MARGIN)
            {
                return credentials.access_token.ok_or_else(|| {
                    Failure::other("Aucune session ChatGPT enregistrée sur ce Mac.")
                });
            }
        }
        let Some(_refresh_lock) = AuthLock::acquire(&self.inner.file) else {
            return Err(Failure::other(
                "Le renouvellement de la session est déjà en cours. Réessayez. La connexion reste enregistrée sur ce Mac.",
            ));
        };
        let _guard = lock(&self.inner.credentials);
        self.bearer_locked(force, false)
    }

    fn bearer_locked(&self, force: bool, retried: bool) -> Result<String, Failure> {
        if self.inner.local_revoked.load(Ordering::SeqCst) {
            return Err(Failure::other(
                "La connexion à ChatGPT n’est plus valide. Reconnectez-vous via Codex.",
            ));
        }
        let Some(credentials) = self.read().filter(|c| c.usable(self.inner.codex)) else {
            return Err(Failure::other(
                "Aucune session ChatGPT enregistrée sur ce Mac.",
            ));
        };
        let access = credentials
            .access_token
            .clone()
            .ok_or_else(|| Failure::other("Aucune session ChatGPT enregistrée sur ce Mac."))?;
        if !force && credentials.expires_at > seconds() + REFRESH_MARGIN {
            return Ok(access);
        }
        let Some(client_id) = credentials.client_id.clone() else {
            return Err(Failure::other(
                "Aucune session ChatGPT enregistrée sur ce Mac.",
            ));
        };
        let refresh = credentials
            .refresh_token
            .clone()
            .ok_or_else(|| Failure::other("Aucune session ChatGPT enregistrée sur ce Mac."))?;
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
                let (status, code, _, _, _) = self.request_failure(error);
                if status == 0 {
                    return if credentials.expires_at > seconds() {
                        Ok(access)
                    } else {
                        Err(network_failure())
                    };
                }
                if explicit_refresh_refusal(&code) {
                    return self.rejected_refresh(&refresh, retried);
                }
                return if credentials.expires_at > seconds() {
                    Ok(access)
                } else {
                    Err(Failure::other(
                        "Le renouvellement de la session ChatGPT a échoué. Réessayez. La connexion reste enregistrée sur ce Mac.",
                    ))
                };
            }
        };
        let Some(mut credentials) = self.read() else {
            return Err(Failure::other(
                "Impossible d’enregistrer les identifiants sur ce Mac. La connexion reste enregistrée.",
            ));
        };
        if credentials.refresh_token.as_deref() != Some(refresh.as_str()) {
            return self.use_disk_session(&credentials, retried);
        }
        credentials.access_token = Some(tokens.access_token.clone());
        credentials.expires_at = seconds() + tokens.expires_in;
        if let Some(refresh) = tokens.refresh_token.clone() {
            credentials.refresh_token = Some(refresh);
        }
        if let Some(scope) = tokens.scope.clone() {
            credentials.scopes = preserved_scope(self.inner.codex, &credentials.scopes, scope);
        }
        if let Some(id_token) = tokens.id_token.clone() {
            credentials.id_token = Some(id_token);
        }
        credentials.saved_at = seconds();
        // Le nouveau refresh_token est durable avant update_profile : un échec
        // ou un kill ensuite ne doit pas laisser l’ancien jeton, déjà invalidé.
        if self.write(&credentials).is_err() {
            return Err(Failure::other(
                "Impossible d’enregistrer les identifiants sur ce Mac. La connexion reste enregistrée.",
            ));
        }
        crate::upstream::turn(
            self.inner.file.parent().unwrap_or(Path::new(".")),
            "oauth",
            200,
            "token_refresh_saved",
        );
        if self.inner.codex {
            codex::update_profile(&mut credentials, &tokens)?;
            if self.write(&credentials).is_err() {
                return Err(Failure::other(
                    "Impossible d’enregistrer les identifiants sur ce Mac. La connexion reste enregistrée.",
                ));
            }
        }
        Ok(tokens.access_token)
    }

    fn use_disk_session(&self, stored: &Credentials, retried: bool) -> Result<String, Failure> {
        if !stored.usable(self.inner.codex) {
            return Err(Failure::other(
                "La session enregistrée sur ce Mac est conservée. Réessayez.",
            ));
        }
        if stored.expires_at > seconds() + REFRESH_MARGIN || retried {
            return stored.access_token.clone().ok_or_else(|| {
                Failure::other("La session enregistrée sur ce Mac est conservée. Réessayez.")
            });
        }
        self.bearer_locked(true, true)
    }

    /// Un seul refus ne suffit pas. On relit, on réessaie le jeton encore sur le
    /// disque, et on n’efface qu’après un second refus de ce même jeton.
    fn rejected_refresh(&self, sent: &str, retried: bool) -> Result<String, Failure> {
        let Some(stored) = self.read() else {
            return Err(Failure::other(
                "La session enregistrée sur ce Mac est conservée. Réessayez.",
            ));
        };
        if stored.refresh_token.as_deref() != Some(sent) {
            return self.use_disk_session(&stored, retried);
        }
        if !retried {
            return self.bearer_locked(true, true);
        }
        let mut cleared = stored;
        cleared.clear_tokens();
        if self.write(&cleared).is_err() {
            return Err(Failure::other(
                "Impossible d’enregistrer les identifiants sur ce Mac. La connexion reste enregistrée.",
            ));
        }
        Err(Failure::expired())
    }

    fn reset_probe(&self) {
        let mut probe = lock(&self.inner.probe);
        self.inner.probe_generation.fetch_add(1, Ordering::SeqCst);
        *probe = json!({"state":"idle"});
    }

    /// Révoque la session renouvelable puis efface les jetons. Rend `false` si OpenAI n'a pas confirmé.
    pub fn signout(&self) -> Result<bool, Failure> {
        self.cancel_signin();
        self.reset_probe();
        self.reset_quota();
        let snapshot = self
            .read()
            .and_then(|credentials| credentials.refresh_token);
        let Some(_lock) = AuthLock::acquire(&self.inner.file) else {
            return Err(Failure::other(
                "La déconnexion est déjà en cours. Réessayez. La connexion reste enregistrée sur ce Mac.",
            ));
        };
        let _guard = lock(&self.inner.credentials);
        let Some(mut credentials) = self.read() else {
            self.inner.local_revoked.store(true, Ordering::SeqCst);
            return Ok(true);
        };
        if credentials.refresh_token != snapshot {
            return Ok(true);
        }
        // L’effacement a été demandé. Tant que l’écriture échoue, les jetons
        // restent sur le disque mais aucune requête ne part.
        self.inner.local_revoked.store(true, Ordering::SeqCst);
        let refresh_token = credentials.refresh_token.clone();
        let client_id = credentials.client_id.clone();
        credentials.clear_tokens();
        self.write(&credentials).map_err(|_| {
            Failure::other(
                "Impossible d’effacer la connexion sur ce Mac. Réessayez avant de fermer Ezerbot.",
            )
        })?;
        self.inner.local_revoked.store(true, Ordering::SeqCst);
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
        let generation = self.inner.probe_generation.load(Ordering::SeqCst);
        thread::spawn(move || {
            let models = provider.fetch_models();
            let mut catalog = lock(&provider.inner.catalog);
            if provider.inner.probe_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            catalog.loading = false;
            if let Ok(models) = models {
                catalog.models = models;
                catalog.loaded = true;
            }
        });
    }

    fn fetch_models(&self) -> Result<Vec<Model>, Failure> {
        if self.inner.codex {
            return self.fetch_codex_models();
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
            .ok_or_else(|| Failure::other("Extraction indisponible."))?;
        let token = credentials
            .access_token
            .ok_or_else(|| Failure::other("Extraction indisponible."))?;
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

    fn responses_background(&self, mut body: Value) -> Result<ureq::Response, Failure> {
        // Optional preparation never owns credential rotation or blocks a chat.
        let credentials = self
            .read()
            .filter(|c| c.usable(self.inner.codex) && c.expires_at > seconds())
            .ok_or_else(|| Failure::other("Extraction indisponible."))?;
        let token = credentials
            .access_token
            .ok_or_else(|| Failure::other("Extraction indisponible."))?;
        self.request(&token, &mut body)?
            .timeout(Duration::from_secs(12))
            .send_json(body)
            .map_err(|error| self.request_failure(error).4)
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
        body: Value,
        capability_fallback: bool,
        notices: &mut Vec<String>,
    ) -> Result<ureq::Response, Failure> {
        self.responses_checked(
            body,
            capability_fallback,
            notices,
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }

    fn responses_checked(
        &self,
        mut body: Value,
        capability_fallback: bool,
        notices: &mut Vec<String>,
        abandoned: &AtomicBool,
        on_status: &mut dyn FnMut(u16),
    ) -> Result<ureq::Response, Failure> {
        let mut token = self.bearer(false)?;
        let mut refreshed = false;
        loop {
            if abandoned.load(Ordering::SeqCst) {
                return Err(Failure::other("Réponse annulée."));
            }
            let attempt = self.request(&token, &mut body)?.send_json(body.clone());
            let (status, _, message, param, failure) = match attempt {
                Ok(response) => {
                    on_status(response.status());
                    return Ok(response);
                }
                Err(error) => self.request_failure(error),
            };
            on_status(status);
            if status == 401 && !refreshed {
                refreshed = true;
                token = self.bearer(true)?;
            } else if status == 401 {
                // Le renouvellement a déjà eu lieu, ou le jeton renvoyé est encore refusé
                // par l’API. Ce n’est pas un refus du refresh_token : la session reste.
                return Err(Failure::other(
                    "OpenAI a refusé la demande après le renouvellement de la session. Réessayez. La connexion reste enregistrée.",
                ));
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
        turn: Option<&str>,
        metadata: &mut MetadataSink<'_>,
        emit: &mut dyn FnMut(
            grokbot_engine::contracts::ModelEvent,
        ) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        use grokbot_engine::contracts::Error;
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
        self.reliable_stream(body, cancel, turn, metadata, emit)
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
        self.stream_canonical(request, emit)
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
    use std::ffi::OsStr;

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
    fn database_path_matches_cocoa_and_never_falls_back_to_a_relative_directory() {
        let fixed = Path::new("/Users/charles");
        let suffix = Path::new("Library/Application Support/Grokbot/workspace.sqlite");
        let from_root =
            database_path_from_home(Some(Path::new("/").as_os_str()), Some(fixed.as_os_str()));
        assert_eq!(from_root, fixed.join(suffix));
        let home = database_path_from_home(
            Some(Path::new("/Users/ada").as_os_str()),
            Some(fixed.as_os_str()),
        );
        assert_eq!(home, PathBuf::from("/Users/ada").join(suffix));
        let empty = database_path_from_home(Some(OsStr::new("")), Some(fixed.as_os_str()));
        assert_eq!(empty, from_root);
        let relative = database_path_from_home(Some(OsStr::new("relative")), Some(OsStr::new("/")));
        assert!(relative.is_absolute());
        assert!(!relative.starts_with("."));
        assert_eq!(relative, database_path_from_home(None, None));
        let passwd = passwd_home().unwrap_or_else(|| PathBuf::from(FALLBACK_HOME));
        assert_eq!(relative, passwd.join(suffix));
        assert!(!relative.to_string_lossy().contains("Grokbot Local"));
    }
}
