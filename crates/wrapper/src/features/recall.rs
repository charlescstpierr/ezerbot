//! Réunions : un bot Recall.ai invité dans Meet, Zoom ou Teams
//! (docs/specs/reunions-voix-iphone.md). Le Mac n’écoute pas l’appel : seul le
//! bot Recall le capte. Rien ne part vers Recall avant le clic sur la carte de
//! consentement. Le suivi vit dans Ezerbot ouvert, sans file ni exécuteur distant.
use super::{write_transaction, Context};
use crate::{parse, ApiError, ApiResponse, App};
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use url::Url;

empty_hooks!(
    state,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

pub const JOIN: &str = "recall_join";
pub const LEAVE: &str = "recall_leave";
pub const REMOVE: &str = "recall_remove";
pub const CONSENT: &str = "Prévenez les participants avant d’ajouter le bot à la réunion. Ils peuvent voir que le bot transcrit.";
pub const RESUME: &str = "Cette réunion n’est plus suivie. Le bot a pu rester dans l’appel jusqu’à ce que Recall le retire.";
const SECRET: &str = "REUNIONS_RECALL_KEY";
const PURPOSE: &str = "Clé API Recall.ai de l’entrée Réunions";
const NOT_CONFIGURED: &str = "La clé Recall n’est pas configurée. Collez votre clé API Recall.ai dans le champ prévu, puis cliquez sur Connecter.";
const BAD_LINK: &str = "Ce lien n’est pas une réunion Google Meet, Zoom ou Microsoft Teams. Collez le lien d’invitation de l’un de ces services.";
const BUSY: &str =
    "Une réunion est déjà en cours. Faites d’abord sortir le bot avant d’en inviter un autre.";
const REVOCATION: &str = "La clé Recall est retirée de ce Mac. Révoquez-la aussi dans votre compte Recall.ai si vous ne l’utilisez plus.";
const LEAVE_UNCONFIRMED: &str =
    "Recall n’a pas confirmé la sortie du bot. La réunion reste suivie : réessayez de quitter.";
const HOSTS: [&str; 4] = [
    "meet.google.com",
    "zoom.us",
    "teams.microsoft.com",
    "teams.live.com",
];
/// Une clé Recall n’est valable que dans la région de son compte. Seuls ces
/// hôtes fixes reçoivent la clé : aucune adresse ne vient d’une saisie.
const REGIONS: [&str; 4] = ["us-west-2", "us-east-1", "eu-central-1", "ap-northeast-1"];
const POLL_MS: u64 = 4_000;
const MAX_FAILURES: i64 = 3;
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY: u64 = 1 << 20;
/// Invitations dont la réponse de Recall est attendue : la fermeture les attend.
static CREATING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
const CREATION_WAIT: Duration = Duration::from_secs(12);
static POLLING: AtomicBool = AtomicBool::new(false);
static NEXT_POLL: AtomicU64 = AtomicU64::new(0);

pub fn is_tool(name: &str) -> bool {
    matches!(name, JOIN | LEAVE | REMOVE)
}
/// Cartes qui exigent un clic ponctuel, jamais une règle permanente.
pub fn is_consent(name: &str) -> bool {
    matches!(name, JOIN | REMOVE)
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_recall_settings (id INTEGER PRIMARY KEY CHECK(id=1), connected INTEGER NOT NULL DEFAULT 0, region TEXT);
        INSERT OR IGNORE INTO wrapper_recall_settings(id) VALUES (1);
        CREATE TABLE IF NOT EXISTS wrapper_recall_meetings (
            id TEXT PRIMARY KEY, link TEXT NOT NULL, bot_name TEXT NOT NULL, bot_id TEXT, region TEXT,
            state TEXT NOT NULL, message TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
            needs_leave INTEGER NOT NULL DEFAULT 0, failures INTEGER NOT NULL DEFAULT 0);
        CREATE UNIQUE INDEX IF NOT EXISTS wrapper_recall_one_active ON wrapper_recall_meetings((1)) WHERE state IN ('joining','in_call');",
    )?;
    // Un arrêt brutal, une veille ou une extinction n’ont pas pu faire sortir le
    // bot. Au retour, aucune réunion ne reste En cours ; la sortie est redemandée.
    conn.execute(
        "UPDATE wrapper_recall_meetings SET state='failed',message=?1,needs_leave=(bot_id IS NOT NULL),updated_at=?2 WHERE state IN ('joining','in_call')",
        params![RESUME, crate::now()],
    )?;
    Ok(())
}

fn wall_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn invalid(message: &str) -> ApiError {
    ApiError::new(400, message)
}

/// Liens acceptés : https, sans identifiants ni port, sur Meet, Zoom ou Teams.
pub fn validate_link(raw: &str) -> Result<String, ApiError> {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.len() > 2048
        || raw.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(invalid(BAD_LINK));
    }
    let url = Url::parse(raw).map_err(|_| invalid(BAD_LINK))?;
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let known = HOSTS
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !known
    {
        return Err(invalid(BAD_LINK));
    }
    Ok(url.to_string())
}
fn link_host(link: &str) -> String {
    Url::parse(link)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn connected(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT connected FROM wrapper_recall_settings WHERE id=1",
        [],
        |r| r.get(0),
    )
    .unwrap_or(false)
}
fn require_connected(conn: &Connection) -> Result<(), ApiError> {
    if connected(conn) {
        Ok(())
    } else {
        Err(invalid(NOT_CONFIGURED))
    }
}
fn active(conn: &Connection) -> Result<Option<Meeting>, ApiError> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE state IN ('joining','in_call')"),
            [],
            Meeting::read,
        )
        .optional()?)
}
fn require_idle(conn: &Connection) -> Result<(), ApiError> {
    if active(conn)?.is_some() {
        return Err(ApiError::new(409, BUSY));
    }
    Ok(())
}
fn key(ctx: &Context<'_>) -> Result<String, ApiError> {
    super::forms::vault(ctx)
        .and_then(|vault| Ok(vault.get(SECRET)?))
        .map_err(|_| ApiError::new(503, "Le trousseau est inaccessible : la clé Recall n’a pas pu être lue. Déverrouillez-le puis réessayez."))?
        .ok_or_else(|| invalid(NOT_CONFIGURED))
}

const SELECT: &str = "SELECT id,link,bot_name,bot_id,region,state,message,created_at,updated_at,failures,needs_leave FROM wrapper_recall_meetings";
struct Meeting {
    id: String,
    link: String,
    bot_name: String,
    bot_id: Option<String>,
    region: Option<String>,
    state: String,
    message: Option<String>,
    created_at: i64,
    updated_at: i64,
    failures: i64,
    needs_leave: bool,
}
impl Meeting {
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            link: r.get(1)?,
            bot_name: r.get(2)?,
            bot_id: r.get(3)?,
            region: r.get(4)?,
            state: r.get(5)?,
            message: r.get(6)?,
            created_at: r.get(7)?,
            updated_at: r.get(8)?,
            failures: r.get(9)?,
            needs_leave: r.get(10)?,
        })
    }
    fn json(&self) -> Value {
        json!({"id":self.id,"state":self.state,"label":label(&self.state),
            "host":link_host(&self.link),"bot_name":self.bot_name,"message":self.message,
            "active":matches!(self.state.as_str(),"joining"|"in_call"),
            "created_at":self.created_at,"updated_at":self.updated_at})
    }
}
fn label(state: &str) -> &'static str {
    match state {
        "joining" => "Connexion",
        "in_call" => "En cours",
        "done" => "Terminée",
        "cancelled" => "Annulée",
        _ => "Échec",
    }
}

/// Entrée du catalogue : une carte, un état connecté, une clé dans le coffre.
pub(super) fn item(ctx: &Context<'_>) -> Result<Value, ApiError> {
    let connected = connected(ctx.connection);
    let meeting = ctx
        .connection
        .query_row(
            &format!("{SELECT} ORDER BY created_at DESC,rowid DESC LIMIT 1"),
            [],
            Meeting::read,
        )
        .optional()?
        .map(|m| m.json());
    Ok(json!({
        "id":"reunions","kind":"recall","name":"Réunions",
        "description":"Invitez un bot dans une réunion Google Meet, Zoom ou Microsoft Teams : il transcrit l’appel.",
        "permissions":"Le bot rejoint les réunions dont vous collez le lien, après une carte de consentement à chaque invitation. Il est visible des participants. Le micro du Mac n’est pas utilisé.",
        "setup":"Créez une clé d’API dans votre compte Recall.ai, puis collez-la ci-dessous. Elle est conservée dans le coffre local, jamais réaffichée, et ne part que vers Recall.ai.",
        "source":"https://docs.recall.ai/","license":"Service Recall.ai · aucun logiciel redistribué",
        "status":if connected {"connected"} else {"disconnected"},
        "message":Value::Null,"has_key":connected,"consent":CONSENT,
        "meeting":meeting,"server":Value::Null,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    #[serde(default)]
    key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Join {
    link: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Leave {
    #[serde(default)]
    id: Option<String>,
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if method != "POST" || !path.starts_with("/api/recall/") {
        return None;
    }
    let ctx = app.feature_context();
    Some(
        (|| -> Result<Value, ApiError> {
            match path {
                "/api/recall/connect" => connect(&ctx, &parse::<Connect>(body)?.key),
                "/api/recall/join" => {
                    let link = validate_link(&parse::<Join>(body)?.link)?;
                    require_connected(ctx.connection)?;
                    require_idle(ctx.connection)?;
                    // Aucun appel à Recall ici : la carte de consentement précède l’invitation.
                    super::engine::queue_approved_action(&ctx, JOIN, json!({"lien":link}))
                }
                "/api/recall/leave" => leave(&ctx, parse::<Leave>(body)?.id.as_deref()),
                "/api/recall/disconnect" => {
                    require_connected(ctx.connection)?;
                    super::engine::queue_approved_action(&ctx, REMOVE, json!({}))
                }
                "/api/recall/shutdown" => Ok(shutdown(&ctx)),
                _ => Err(ApiError::new(404, "Route inconnue.")),
            }
        })()
        .map(ApiResponse::Json),
    )
}

/// Écrit la clé au coffre. Un trousseau qui refuse l’écriture échoue sans
/// repli : rien n’est conservé dans un fichier de secours.
fn store_key(vault: &grokbot_forms::vault::Vault, key: &str) -> Result<(), ApiError> {
    vault.set(SECRET, PURPOSE, key).map(|_| ()).map_err(|_| {
        ApiError::new(
            503,
            "Impossible de conserver la clé Recall dans le trousseau. Rien n’a été enregistré : déverrouillez le trousseau puis réessayez.",
        )
    })
}
fn connect(ctx: &Context<'_>, key: &str) -> Result<Value, ApiError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(invalid(NOT_CONFIGURED));
    }
    if key.len() < 8 || key.len() > 256 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid(
            "Cette clé Recall n’a pas le bon format. Copiez-la de nouveau depuis votre compte Recall.ai.",
        ));
    }
    require_idle(ctx.connection)?;
    store_key(&super::forms::vault(ctx)?, key)?;
    ctx.connection.execute(
        "UPDATE wrapper_recall_settings SET connected=1,region=NULL WHERE id=1",
        [],
    )?;
    Ok(json!({"connected":true,"message":"Réunions est connecté. La clé n’est plus affichée."}))
}

// ---- Recall.ai -------------------------------------------------------------

struct Failure {
    status: Option<u16>,
}
/// Débogage seulement : un fournisseur de boucle sur 127.0.0.1, jamais Internet.
fn fake_base() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("GROKBOT_FAKE_RECALL") {
        let port = value
            .strip_prefix("http://127.0.0.1:")
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p != 0)
            .expect("GROKBOT_FAKE_RECALL doit viser http://127.0.0.1:<port>.");
        return Some(format!("http://127.0.0.1:{port}"));
    }
    None
}
/// Régions à essayer, la région connue d’abord. Une valeur inconnue est ignorée.
fn regions(hint: Option<&str>) -> Vec<(String, String)> {
    if let Some(base) = fake_base() {
        return vec![("fake".into(), base)];
    }
    let known = hint.and_then(|h| REGIONS.iter().copied().find(|r| *r == h));
    let order: Vec<&str> = known
        .into_iter()
        .chain(REGIONS.iter().copied().filter(|r| Some(*r) != known))
        .collect();
    order
        .into_iter()
        .map(|r| (r.to_owned(), format!("https://{r}.recall.ai")))
        .collect()
}
fn call(
    base: &str,
    key: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, Failure> {
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .redirects(0)
        .build();
    let request = agent
        .request(method, &format!("{base}{path}"))
        .set("Authorization", &format!("Token {key}"))
        .set("Accept", "application/json");
    let response = match body {
        Some(body) => request.send_json(body),
        None => request.call(),
    };
    match response {
        Ok(response) => {
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take(MAX_BODY)
                .read_to_end(&mut bytes)
                .map_err(|_| Failure { status: None })?;
            Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
        }
        Err(ureq::Error::Status(status, _)) => Err(Failure {
            status: Some(status),
        }),
        Err(_) => Err(Failure { status: None }),
    }
}
fn refusal(failure: &Failure) -> &'static str {
    match failure.status {
        Some(400) | Some(422) => "Recall a refusé ce lien de réunion.",
        Some(401) | Some(403) => {
            "Recall a refusé la clé. Déconnectez Réunions, puis reconnectez-le avec une clé valide."
        }
        Some(429) => "Recall limite les demandes pour le moment. Réessayez dans un instant.",
        Some(507) => "Recall n’a pas de bot disponible pour le moment. Réessayez dans un instant.",
        _ => "Recall est injoignable ou a répondu par une erreur. L’invitation n’a pas abouti.",
    }
}
/// (région, identifiant du bot) quand Recall accepte l’invitation.
fn create_bot(
    conn: &Connection,
    key: &str,
    link: &str,
    name: &str,
) -> Result<(String, String), Failure> {
    let hint: Option<String> = conn
        .query_row(
            "SELECT region FROM wrapper_recall_settings WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten();
    let body = json!({"meeting_url":link,"bot_name":name,"recording_config":{"transcript":{"provider":{"recallai_streaming":{}}}}});
    let mut last = Failure { status: None };
    for (region, base) in regions(hint.as_deref()) {
        match call(&base, key, "POST", "/api/v1/bot/", Some(&body)) {
            Ok(created) => {
                let id = created["id"].as_str().filter(|id| {
                    !id.is_empty()
                        && id.len() <= 128
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                });
                return match id {
                    Some(id) => {
                        let _ = conn.execute(
                            "UPDATE wrapper_recall_settings SET region=?1 WHERE id=1",
                            [&region],
                        );
                        Ok((region, id.to_owned()))
                    }
                    None => Err(Failure { status: None }),
                };
            }
            // Mauvaise région : la clé n’y existe pas. Les autres erreurs s’arrêtent là.
            Err(failure) if matches!(failure.status, Some(401) | Some(403)) => last = failure,
            Err(failure) => return Err(failure),
        }
    }
    Err(last)
}
fn base_for(region: Option<&str>) -> Option<String> {
    regions(region).into_iter().next().map(|(_, base)| base)
}
fn leave_bot(key: &str, region: Option<&str>, bot: &str) -> Result<(), Failure> {
    let base = base_for(region).ok_or(Failure { status: None })?;
    call(
        &base,
        key,
        "POST",
        &format!("/api/v1/bot/{bot}/leave_call/"),
        None,
    )
    .map(|_| ())
}
fn bot_code(key: &str, region: Option<&str>, bot: &str) -> Result<String, Failure> {
    let base = base_for(region).ok_or(Failure { status: None })?;
    let value = call(&base, key, "GET", &format!("/api/v1/bot/{bot}/"), None)?;
    Ok(value["status_changes"]
        .as_array()
        .and_then(|changes| changes.last())
        .and_then(|change| change["code"].as_str())
        .unwrap_or("joining_call")
        .to_owned())
}
/// Un état Recall encore inconnu du Mac devient Échec, jamais un mot anglais brut.
fn map_status(code: &str) -> (&'static str, Option<&'static str>) {
    match code {
        "joining_call" | "in_waiting_room" | "in_call_not_recording" | "recording_permission_allowed" => ("joining", None),
        "in_call_recording" => ("in_call", None),
        "call_ended" | "done" | "analysis_done" => ("done", None),
        "recording_permission_denied" => ("failed", Some("Les participants n’ont pas autorisé le bot à transcrire la réunion.")),
        "fatal" => ("failed", Some("Le bot n’a pas pu rester dans la réunion : il a été refusé, expulsé, ou Recall a rencontré une erreur.")),
        _ => ("failed", Some("Recall a signalé un état que ce Mac ne reconnaît pas. La réunion n’est plus suivie.")),
    }
}

// ---- Cycle de vie ------------------------------------------------------------

fn set_state(
    conn: &Connection,
    id: &str,
    state: &str,
    message: Option<&str>,
) -> Result<bool, ApiError> {
    Ok(conn.execute(
        "UPDATE wrapper_recall_meetings SET state=?2,message=?3,updated_at=?4,failures=0 WHERE id=?1 AND state IN ('joining','in_call')",
        params![id, state, message, crate::now()],
    )? > 0)
}
/// Fait sortir le bot, puis fixe l’état final. Sans confirmation de Recall,
/// l’état ne change pas : l’écran ne dit jamais que le bot est sorti sans preuve.
fn leave_meeting(
    ctx: &Context<'_>,
    meeting: &Meeting,
    joining_state: &str,
) -> Result<(), ApiError> {
    if let Some(bot) = &meeting.bot_id {
        let key = key(ctx)?;
        if leave_bot(&key, meeting.region.as_deref(), bot).is_err() {
            // Le bot a pu déjà partir : un état terminal de Recall vaut confirmation.
            let gone = bot_code(&key, meeting.region.as_deref(), bot)
                .map(|code| map_status(&code).0 == "done")
                .unwrap_or(false);
            if !gone {
                return Err(ApiError::new(502, LEAVE_UNCONFIRMED));
            }
        }
    }
    let state = if meeting.state == "joining" {
        joining_state
    } else {
        "done"
    };
    set_state(ctx.connection, &meeting.id, state, None)?;
    Ok(())
}
fn leave(ctx: &Context<'_>, id: Option<&str>) -> Result<Value, ApiError> {
    let meeting = active(ctx.connection)?.ok_or_else(|| {
        ApiError::new(
            409,
            "Aucune réunion n’est en cours : le bot est déjà sorti.",
        )
    })?;
    if id.is_some_and(|id| id != meeting.id) {
        return Err(ApiError::new(409, "Cette réunion est déjà terminée."));
    }
    leave_meeting(ctx, &meeting, "cancelled")?;
    Ok(json!({"left":true,"message":"Le bot a quitté la réunion."}))
}
/// Fermeture propre : le bot sort avant que la fenêtre disparaisse. La réunion
/// passe à Terminée ; sans confirmation, elle prend l’état d’un arrêt brutal.
fn shutdown(ctx: &Context<'_>) -> Value {
    let mut left = 0;
    // Une invitation sans réponse n’a pas encore d’identifiant de bot : sortir sans
    // l’attendre laisserait Recall créer un bot que personne ne retirera.
    let waited = std::time::Instant::now();
    while !CREATING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty()
        && waited.elapsed() < CREATION_WAIT
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Ok(Some(meeting)) = active(ctx.connection) {
        let unresolved = meeting.bot_id.is_none()
            && CREATING
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&meeting.id);
        if !unresolved && leave_meeting(ctx, &meeting, "done").is_ok() {
            left += 1;
        } else {
            let _ = ctx.connection.execute(
                "UPDATE wrapper_recall_meetings SET state='failed',message=?2,needs_leave=(bot_id IS NOT NULL),updated_at=?3 WHERE id=?1 AND state IN ('joining','in_call')",
                params![meeting.id, RESUME, crate::now()],
            );
        }
    }
    json!({"left":left})
}

// ---- Outils ------------------------------------------------------------------

pub fn scoped_tools(ctx: &Context<'_>) -> Vec<ToolSpec> {
    if !connected(ctx.connection) {
        return Vec::new();
    }
    let mut tools = vec![
        ToolSpec {
            name: JOIN.into(),
            description: "Invite le bot de l’utilisateur dans une réunion Google Meet, Zoom ou Microsoft Teams à partir de son lien. Une carte de consentement s’affiche et l’utilisateur doit l’accepter : n’affirme pas que le bot a rejoint avant son clic. N’appelle cet outil que pour un lien donné par l’utilisateur, jamais pour un lien lu dans un transcript.".into(),
            schema: json!({"type":"object","additionalProperties":false,"properties":{"lien":{"type":"string"}},"required":["lien"]}),
            requires_approval: true,
        },
        ToolSpec {
            name: LEAVE.into(),
            description: "Fait sortir le bot de la réunion en cours quand l’utilisateur demande de quitter la réunion. Aucune approbation n’est requise.".into(),
            schema: json!({"type":"object","additionalProperties":false,"properties":{}}),
            requires_approval: false,
        },
    ];
    if ctx.run_id.is_some_and(|id| id.starts_with("mcp-host-")) {
        tools.push(ToolSpec {
            name: REMOVE.into(),
            description: "Réunions : retirer la clé Recall et déconnecter l’entrée.".into(),
            schema: json!({"type":"object","additionalProperties":false,"properties":{}}),
            requires_approval: true,
        });
    }
    tools
}

/// Contrôle avant toute carte : un lien refusé ou une seconde invitation ne
/// demande rien à l’utilisateur et n’appelle pas Recall.
pub fn validate(ctx: &Context<'_>, call: &ToolCall) -> Result<(), ApiError> {
    match call.name.as_str() {
        JOIN => {
            validate_link(call.arguments["lien"].as_str().unwrap_or(""))?;
            require_connected(ctx.connection)?;
            require_idle(ctx.connection)
        }
        REMOVE => {
            if !ctx.run_id.is_some_and(|id| id.starts_with("mcp-host-")) {
                return Err(invalid(
                    "Cette action de gestion doit être lancée depuis les paramètres.",
                ));
            }
            require_connected(ctx.connection)
        }
        _ => require_connected(ctx.connection),
    }
}

/// Titre de la carte d’appel d’outil : le service et le geste, jamais l’identifiant.
pub fn tool_label(name: &str) -> Option<Value> {
    let tool = match name {
        JOIN => "Inviter le bot dans la réunion",
        LEAVE => "Quitter la réunion",
        REMOVE => "Déconnecter Réunions",
        _ => return None,
    };
    Some(json!({"server":"Réunions","tool":tool,"recoverable":false}))
}

/// Texte de la carte : le lien, le nom que verront les participants, la phrase.
pub fn call_display(
    ctx: &Context<'_>,
    name: &str,
    arguments: &Value,
    bot_id: &str,
    settings: bool,
) -> Option<Value> {
    let origin = if settings { "settings" } else { "thread" };
    match name {
        JOIN => Some(json!({
            "action":"join","origin":origin,"link":arguments["lien"],
            "bot_name":bot_name(ctx, bot_id),"phrase":CONSENT,
        })),
        REMOVE => Some(json!({"action":"remove","origin":origin,"name":"Réunions"})),
        _ => None,
    }
}
fn bot_name(ctx: &Context<'_>, bot_id: &str) -> String {
    super::forms::bot_display_name(ctx, bot_id)
        .chars()
        .take(100)
        .collect()
}

pub fn execute(ctx: &Context<'_>, call: &ToolCall) -> ToolOutcome {
    let result = match call.name.as_str() {
        JOIN => join(ctx, call),
        LEAVE => leave(ctx, None),
        REMOVE => remove(ctx),
        _ => Err(invalid("Outil de réunion inconnu.")),
    };
    match result {
        Ok(output) => ToolOutcome::Done { output },
        Err(error) => ToolOutcome::Failed {
            reason: error.message,
        },
    }
}
fn join(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    validate(ctx, call)?;
    let link = validate_link(call.arguments["lien"].as_str().unwrap_or(""))?;
    let key = key(ctx)?;
    let name = bot_name(ctx, ctx.bot_id.unwrap_or(crate::DEFAULT_BOT.id));
    let id: String =
        ctx.connection
            .query_row("SELECT 'recall_'||lower(hex(randomblob(16)))", [], |r| {
                r.get(0)
            })?;
    // Inscrite avant la ligne : la fermeture qui voit la réunion attend aussi la réponse de Recall.
    struct Creating(String);
    impl Drop for Creating {
        fn drop(&mut self) {
            CREATING
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|id| *id != self.0);
        }
    }
    CREATING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(id.clone());
    let _creating = Creating(id.clone());
    {
        let tx = write_transaction(ctx.connection)?;
        // L’index unique n’admet qu’une réunion active : la seconde invitation échoue ici.
        tx.execute(
            "INSERT INTO wrapper_recall_meetings(id,link,bot_name,state,created_at,updated_at) VALUES (?1,?2,?3,'joining',?4,?4)",
            params![id, link, name, crate::now()],
        )
        .map_err(|_| ApiError::new(409, BUSY))?;
        tx.commit()?;
    }
    // Aucune transaction ouverte pendant l’appel réseau.
    match create_bot(ctx.connection, &key, &link, &name) {
        Ok((region, bot)) => {
            let stored = ctx.connection.execute(
                "UPDATE wrapper_recall_meetings SET bot_id=?2,region=?3 WHERE id=?1 AND state='joining'",
                params![id, bot, region],
            )?;
            if stored == 0 {
                // Quitter a devancé la réponse de Recall : ce bot ne doit pas rester.
                let _ = leave_bot(&key, Some(&region), &bot);
            }
            Ok(
                json!({"meeting_id":id,"message":"Le bot rejoint la réunion. L’état passe à En cours quand il est dans l’appel."}),
            )
        }
        Err(failure) => {
            let message = refusal(&failure);
            set_state(ctx.connection, &id, "failed", Some(message))?;
            Err(ApiError::new(502, message))
        }
    }
}
fn remove(ctx: &Context<'_>) -> Result<Value, ApiError> {
    validate(
        ctx,
        &ToolCall {
            call_id: String::new(),
            name: REMOVE.into(),
            arguments: json!({}),
        },
    )?;
    // Une réunion en cours est quittée avant le retrait de la clé.
    if let Some(meeting) = active(ctx.connection)? {
        leave_meeting(ctx, &meeting, "cancelled")?;
    }
    super::forms::vault(ctx)?.remove(SECRET).map_err(|_| {
        ApiError::new(503, "Suppression non confirmée : rétablissez l’accès au trousseau macOS puis réessayez. La clé est conservée.")
    })?;
    ctx.connection.execute(
        "UPDATE wrapper_recall_settings SET connected=0,region=NULL WHERE id=1",
        [],
    )?;
    Ok(json!({"message":REVOCATION}))
}

// ---- Suivi en arrière-plan -----------------------------------------------------

/// Hors du fil HTTP : aucune requête réseau sous le verrou SQLite ni sous le tick.
pub fn tick(ctx: &Context<'_>) {
    let work: bool = ctx
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM wrapper_recall_meetings WHERE bot_id IS NOT NULL AND (state IN ('joining','in_call') OR needs_leave=1))",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    let now = wall_clock();
    if !work || NEXT_POLL.load(Ordering::Acquire) > now || POLLING.swap(true, Ordering::AcqRel) {
        return;
    }
    NEXT_POLL.store(now + POLL_MS, Ordering::Release);
    let provider = ctx.provider.clone();
    let path = ctx.db_path.to_path_buf();
    std::thread::spawn(move || {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                POLLING.store(false, Ordering::Release);
            }
        }
        let _reset = Reset;
        let Ok(conn) = Connection::open(&path) else {
            return;
        };
        let _ = conn.busy_timeout(Duration::from_secs(3));
        let ctx = Context {
            connection: &conn,
            provider: &provider,
            db_path: &path,
            now_ms: 0,
            bot_id: None,
            conversation_id: None,
            run_id: None,
        };
        sync(&ctx);
    });
}
/// Une passe de suivi, synchrone. Le fil de tick l’appelle hors du fil HTTP.
#[doc(hidden)]
pub fn sync(ctx: &Context<'_>) {
    let rows: Vec<Meeting> = (|| -> rusqlite::Result<Vec<Meeting>> {
        let mut q = ctx.connection.prepare(&format!(
            "{SELECT} WHERE bot_id IS NOT NULL AND (state IN ('joining','in_call') OR needs_leave=1)"
        ))?;
        let rows = q.query_map([], Meeting::read)?.collect();
        rows
    })()
    .unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let Ok(key) = key(ctx) else {
        return;
    };
    for meeting in rows {
        let Some(bot) = meeting.bot_id.as_deref() else {
            continue;
        };
        if meeting.needs_leave {
            // Une seule demande de sortie après un arrêt brutal, sans boucle.
            let _ = leave_bot(&key, meeting.region.as_deref(), bot);
            let _ = ctx.connection.execute(
                "UPDATE wrapper_recall_meetings SET needs_leave=0 WHERE id=?1",
                [&meeting.id],
            );
            continue;
        }
        match bot_code(&key, meeting.region.as_deref(), bot) {
            Ok(code) => {
                let (state, message) = map_status(&code);
                if state != meeting.state {
                    // Comparaison avec l’état lu : un état posé entre-temps (Quitter) gagne.
                    let _ = ctx.connection.execute(
                        "UPDATE wrapper_recall_meetings SET state=?3,message=?4,updated_at=?5,failures=0,needs_leave=(?3='failed') WHERE id=?1 AND state=?2",
                        params![meeting.id, meeting.state, state, message, crate::now()],
                    );
                } else if meeting.failures > 0 {
                    let _ = ctx.connection.execute(
                        "UPDATE wrapper_recall_meetings SET failures=0 WHERE id=?1",
                        [&meeting.id],
                    );
                }
            }
            Err(failure) => {
                let refused = matches!(failure.status, Some(401) | Some(403) | Some(404));
                if refused || meeting.failures + 1 >= MAX_FAILURES {
                    let _ = ctx.connection.execute(
                        "UPDATE wrapper_recall_meetings SET state='failed',message=?2,needs_leave=1,updated_at=?3 WHERE id=?1 AND state IN ('joining','in_call')",
                        params![meeting.id, RESUME, crate::now()],
                    );
                } else {
                    let _ = ctx.connection.execute(
                        "UPDATE wrapper_recall_meetings SET failures=failures+1 WHERE id=?1",
                        [&meeting.id],
                    );
                }
            }
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn only_meet_zoom_and_teams_links_are_accepted() {
        for ok in [
            "https://meet.google.com/abc-defg-hij",
            "https://us02web.zoom.us/j/123456789?pwd=abc",
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting",
            "https://teams.live.com/meet/9361234",
        ] {
            assert!(validate_link(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "http://meet.google.com/abc",
            "https://example.com/meet.google.com",
            "https://meet.google.com.evil.test/abc",
            "https://evilzoom.us/j/1",
            "https://user:pw@zoom.us/j/1",
            "https://zoom.us:8443/j/1",
            "javascript:alert(1)",
            "https://zoom.us/j/1 https://example.com",
        ] {
            assert_eq!(validate_link(bad).unwrap_err().message, BAD_LINK, "{bad}");
        }
    }

    #[test]
    fn a_keychain_that_refuses_the_write_leaves_no_fallback_file() {
        let dir = std::env::temp_dir().join(format!("ezerbot-recall-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("workspace.secrets.json");
        let vault = grokbot_forms::vault::Vault::open_refusing_keychain(&file).unwrap();
        let secret = "recall-key-must-never-reach-a-file-0123456789";
        let error = store_key(&vault, secret).unwrap_err();
        assert!(error.message.contains("Rien n’a été enregistré"));
        assert!(!error.message.contains(secret));
        let on_disk = std::fs::read(&file).unwrap();
        assert!(!on_disk
            .windows(secret.len())
            .any(|w| w == secret.as_bytes()));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn unknown_recall_states_fail_in_french() {
        let (state, message) = map_status("something_new");
        assert_eq!(state, "failed");
        assert!(message.unwrap().contains("ne reconnaît pas"));
        assert_eq!(map_status("in_call_recording").0, "in_call");
        assert_eq!(map_status("joining_call").0, "joining");
        assert_eq!(map_status("call_ended").0, "done");
    }
}
