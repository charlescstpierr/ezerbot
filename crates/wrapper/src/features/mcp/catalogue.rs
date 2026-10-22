//! Local reconstruction of the registry/broker/grant separation in REA protocol.md.
//! Fixed manifests, no service credentials or third-party code bundled. All I/O
//! with a provider is reachable only from an admitted lifecycle/tool call.
use super::*;
use std::sync::Mutex;
pub(super) mod github;
pub(super) mod gmail;
mod oauth;
pub(super) mod slack;

static EFFECTS: Mutex<()> = Mutex::new(());
const PURPOSE: &str = grokbot_forms::vault::MCP_ENV_PURPOSE;
const REVOCATION_WARNING: &str = "Déconnecté sur ce Mac ; révocation distante non confirmée. Retirez aussi l’accès d’Ezerbot dans les réglages du service.";
fn scopes_label(m: Manifest) -> &'static str {
    match m.id {
        "notion" => "Choisies pendant le consentement Notion",
        "github" => "Permissions de la GitHub App d’Ezerbot, choisies chez GitHub",
        "slack" => "Lecture seule des canaux, choisie pendant le consentement Slack",
        _ => m.scopes,
    }
}
pub(super) fn id_for_server(ctx: &Context<'_>, id: &str) -> Option<&'static str> {
    record(ctx, id).ok().flatten().map(|(m, _, _, _)| m.id)
}
/// A service that Ezerbot talks to itself, with no process and no client to type.
fn is_native_service(id: &str) -> bool {
    matches!(id, "github" | "gmail" | "slack")
}
/// A connection with no process: GitHub, Gmail and Slack are native adapters, connected or not.
pub(super) fn native_status(ctx: &Context<'_>, server: &str) -> Option<&'static str> {
    let (m, phase, _, _) = record(ctx, server).ok().flatten()?;
    is_native_service(m.id).then(|| {
        if phase == "connected" {
            "connected"
        } else {
            "disconnected"
        }
    })
}
/// The service a native connection belongs to; several GitHub accounts share one list of tools.
pub(super) fn native_service(ctx: &Context<'_>, server: &str) -> Option<&'static str> {
    let (m, _, _, _) = record(ctx, server).ok().flatten()?;
    is_native_service(m.id).then_some(m.id)
}
/// Tools of the native connection, offered to a bot only while it is connected
/// and enabled for that bot.
pub(super) fn native_tools(
    ctx: &Context<'_>,
    server: &Server,
) -> Vec<grokbot_engine::contracts::ToolSpec> {
    if native_status(ctx, &server.id) != Some("connected") {
        return Vec::new();
    }
    match native_service(ctx, &server.id) {
        Some("gmail") => gmail::tool_specs(),
        Some("slack") => slack::tool_specs(),
        _ => github::tool_specs(),
    }
}
#[derive(Clone, Copy)]
pub(super) struct Manifest {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub permissions: &'static str,
    pub scopes: &'static str,
    pub source: &'static str,
    pub license: &'static str,
    pub setup: &'static str,
    pub endpoint: &'static str,
}
const GOOGLE_SETUP: &str = "Prérequis : uv installé sur ce Mac. Dans Google Cloud, activez l’API du service, configurez l’écran de consentement et créez un client OAuth « Application de bureau ». Saisissez son identifiant et son secret ci-dessous. Aucun compte Google n’est fourni par Ezerbot.";
pub(super) const MANIFESTS: &[Manifest] = &[
    Manifest { id:"gmail", name:"Gmail", description:"Lire vos derniers courriels, vos rendez-vous et un fichier de votre Drive, et préparer un envoi, depuis le chat, avec votre compte Google.", permissions:"Lecture de la boîte de réception, de l’agenda et des fichiers Drive, brouillons et modification des messages, adresse du compte. Les bots peuvent lire les derniers courriels, les rendez-vous d’un jour et un fichier Drive nommé, sans carte d’approbation, et préparer un courriel : il ne part qu’après votre clic sur sa carte brouillon. Aucun courriel n’est supprimé, et rien n’est écrit dans l’agenda ni dans Drive.", scopes:"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose https://www.googleapis.com/auth/gmail.modify https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/drive.readonly", source:"https://developers.google.com/gmail/api/auth/scopes", license:"Service Google · client Google d’Ezerbot, aucun logiciel redistribué", setup:"Aucune application OAuth à créer et aucun secret à saisir. Connecter ouvre la page de connexion de Google dans le navigateur : vous choisissez votre compte et acceptez les autorisations demandées, courriel, agenda et Drive en lecture. Un seul consentement suffit pour les trois.", endpoint:"" },
    Manifest { id:"calendar", name:"Google Calendar", description:"Consulter vos agendas et gérer vos événements.", permissions:"Lecture et modification des agendas et événements ; adresse du compte.", scopes:"openid email https://www.googleapis.com/auth/calendar", source:"https://github.com/taylorwilsdon/google_workspace_mcp", license:"MIT · workspace-mcp 2.1.1 (communautaire)", setup:GOOGLE_SETUP, endpoint:"" },
    Manifest { id:"drive", name:"Google Drive", description:"Rechercher, lire et organiser les fichiers de votre Drive.", permissions:"Lecture et modification de tous les fichiers Drive accessibles au compte ; adresse du compte.", scopes:"openid email https://www.googleapis.com/auth/drive", source:"https://github.com/taylorwilsdon/google_workspace_mcp", license:"MIT · workspace-mcp 2.1.1 (communautaire)", setup:GOOGLE_SETUP, endpoint:"" },
    Manifest { id:"slack", name:"Slack", description:"Lire les derniers messages d’un canal Slack depuis le chat, avec votre compte Slack.", permissions:"Lecture des messages des canaux publics et privés que votre compte voit, et des noms des personnes ; rien n’est envoyé ni modifié dans Slack.", scopes:"channels:history groups:history channels:read groups:read users:read", source:"https://docs.slack.dev/authentication/installing-with-oauth/", license:"Service Slack · client public Ezerbot, PKCE, aucun logiciel redistribué", setup:"Aucune application OAuth à créer et aucun secret à saisir. Connecter ouvre Slack dans le navigateur : vous choisissez l’espace de travail et autorisez le client Slack d’Ezerbot à lire vos canaux.", endpoint:"https://slack.com/api" },
    Manifest { id:"notion", name:"Notion", description:"Rechercher et modifier les pages de votre espace Notion.", permissions:"Accès aux pages et espaces choisis pendant le consentement Notion, en lecture et écriture.", scopes:"default", source:"https://developers.notion.com/docs/mcp", license:"Service officiel Notion · aucun logiciel redistribué", setup:"Connexion dans le navigateur. Vous choisirez les espaces et pages autorisés chez Notion ; aucun identifiant d’application à saisir.", endpoint:"https://mcp.notion.com/mcp" },
    Manifest { id:"github", name:"GitHub", description:"Lister les issues de vos dépôts et préparer des commentaires et des issues depuis le chat, avec votre compte GitHub.", permissions:"Issues (lecture ; un commentaire ou une nouvelle issue n’est publié qu’après votre clic sur sa carte brouillon) et pull requests (lecture), contenu des dépôts en lecture, identité du compte ; seulement les dépôts que vous choisirez chez GitHub.", scopes:"", source:"https://docs.github.com/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow", license:"Service GitHub · GitHub App Ezerbot, flux par code, aucun logiciel redistribué", setup:"Aucune application OAuth à créer et aucun secret à saisir. Connecter affiche un code à 8 caractères et ouvre github.com/login/device : vous vous identifiez et choisissez les dépôts chez GitHub.", endpoint:"https://api.github.com" },
];
fn manifest(id: &str) -> Result<Manifest, ApiError> {
    MANIFESTS
        .iter()
        .find(|m| m.id == id)
        .copied()
        .ok_or_else(|| invalid("Ce connecteur n’est pas dans le catalogue."))
}
/// A catalogue row is a service plus, for GitHub, one account slot: `github`, `github:2`…
fn row_manifest(row: &str) -> Result<Manifest, ApiError> {
    manifest(row.split_once(':').map_or(row, |(service, _)| service))
}
/// The GitHub account slots, the first one first.
fn github_rows(ctx: &Context<'_>) -> Result<Vec<(String, String)>, ApiError> {
    let mut stmt = ctx.connection.prepare("SELECT id,phase FROM wrapper_connector_catalogue WHERE id='github' OR id GLOB 'github:*' ORDER BY id='github' DESC,rowid")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
fn server_id(id: &str) -> String {
    format!("c{}", &format!("{:x}", Sha256::digest(id.as_bytes()))[..15])
}
fn record(
    ctx: &Context<'_>,
    server: &str,
) -> Result<Option<(Manifest, String, i64, i64)>, ApiError> {
    let row: Option<(String,String,i64,i64)> = ctx.connection.query_row("SELECT id,phase,expires_at,generation FROM wrapper_connector_catalogue WHERE server_id=?1", [server], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    row.map(|(id, phase, expires, g)| Ok((row_manifest(&id)?, phase, expires, g)))
        .transpose()
}
pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_connector_catalogue (id TEXT PRIMARY KEY,server_id TEXT NOT NULL UNIQUE,phase TEXT NOT NULL DEFAULT 'disconnected',message TEXT,expires_at INTEGER NOT NULL DEFAULT 0,generation INTEGER NOT NULL DEFAULT 0,authorization_url TEXT,account_email TEXT);")?;
    // The code to type and the numeric account id are public attributes. The device
    // code itself is never stored: it lives in the waiting thread.
    for (column, definition) in [
        ("account_id", "TEXT"),
        ("device_user_code", "TEXT"),
        ("device_expires_at", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let present: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('wrapper_connector_catalogue') WHERE name=?1)",
            [column],
            |r| r.get(0),
        )?;
        if !present {
            conn.execute(
                &format!(
                    "ALTER TABLE wrapper_connector_catalogue ADD COLUMN {column} {definition}"
                ),
                [],
            )?;
        }
    }
    github::migrate(conn)?;
    github::comment::migrate(conn)?;
    github::issue::migrate(conn)?;
    gmail::migrate(conn)?;
    conn.execute_batch("UPDATE wrapper_connector_catalogue SET phase='reconnect',message='La connexion a été interrompue. Reconnectez le service.',authorization_url=NULL,device_user_code=NULL,device_expires_at=0,generation=generation+1 WHERE phase='authorizing';")?;
    Ok(())
}
pub(super) fn state(ctx: &Context<'_>) -> Result<Value, ApiError> {
    let local = super::state_servers(ctx)?;
    let mut items = Vec::new();
    for m in MANIFESTS {
        if m.id != "github" {
            items.push(item_of(ctx, *m, m.id, &local)?);
            continue;
        }
        // The card follows the first account that is still there; a disconnected
        // first slot gives way to the next one instead of hiding it.
        let rows = github_rows(ctx)?;
        let lead = rows
            .iter()
            .find(|(_, phase)| phase != "disconnected")
            .map_or("github", |(row, _)| row.as_str());
        let mut item = item_of(ctx, *m, lead, &local)?;
        let mut accounts = Vec::new();
        for (row, _) in rows.iter().filter(|(_, phase)| phase != "disconnected") {
            let account = item_of(ctx, *m, row, &local)?;
            accounts.push(json!({"id":row,"account":account["account"],"status":account["status"],"message":account["message"],"warning":account["warning"],"device":account["device"],"authorization_url":account["authorization_url"],"has_tokens":account["has_tokens"],"server":account["server"]}));
        }
        item["accounts"] = json!(accounts);
        // A slot still waiting for its first approval belongs to this card too.
        item["slot_servers"] = json!(rows
            .iter()
            .map(|(row, _)| server_id(row))
            .collect::<Vec<_>>());
        items.push(item);
    }
    items.push(super::super::recall::item(ctx)?);
    Ok(json!(items))
}
/// One card, or one GitHub account slot, as the settings panel shows it.
fn item_of(
    ctx: &Context<'_>,
    m: Manifest,
    row_id: &str,
    local: &[Value],
) -> Result<Value, ApiError> {
    let id = server_id(row_id);
    let row: Option<(String,Option<String>,i64,Option<String>)> = ctx.connection.query_row("SELECT phase,message,expires_at,authorization_url FROM wrapper_connector_catalogue WHERE id=?1", [row_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let device: Option<(Option<String>, i64)> = ctx.connection.query_row("SELECT device_user_code,device_expires_at FROM wrapper_connector_catalogue WHERE id=?1", [row_id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let server = local.iter().find(|s| s["id"] == id).cloned();
    let configured = row.is_some();
    let (phase, message, expires, url) =
        row.unwrap_or_else(|| ("disconnected".into(), None, 0, None));
    let private = if configured {
        credentials(ctx, &id)?.unwrap_or_default()
    } else {
        HashMap::new()
    };
    // GitHub, Gmail and Slack have no client to type: it is configured by the build, or not at all.
    let has_client = if m.id == "github" {
        github::client_id().is_some()
    } else if m.id == "gmail" {
        gmail::client_id().is_some()
    } else if m.id == "slack" {
        slack::client_id().is_some()
    } else {
        private.contains_key("CLIENT_ID")
            && (m.id == "notion" || private.contains_key("CLIENT_SECRET"))
    };
    let has_tokens = private.contains_key("ACCESS_TOKEN") || private.contains_key("REFRESH_TOKEN");
    let can_refresh = private.contains_key("REFRESH_TOKEN");
    let expired = expires > 0 && expires <= wall_clock() as i64 + 30_000;
    let warning = message.as_deref() == Some(REVOCATION_WARNING);
    let status = if phase == "authorizing"
        || phase == "reconnect"
        || phase == "error"
        || phase == "disconnected"
    {
        phase.as_str()
    } else if expired && !can_refresh {
        "reconnect"
    } else if is_native_service(m.id) && server.is_some() {
        // Native connection: there is no process to be alive or crashed.
        "connected"
    } else if server
        .as_ref()
        .is_some_and(|s| s["status"] == "error" || s["status"] == "crashed")
    {
        "error"
    } else if server
        .as_ref()
        .is_some_and(|s| s["status"] == "connected" || s["status"] == "busy")
    {
        "connected"
    } else {
        "saved"
    };
    let message = if status == "saved" {
        Some("Identifiants conservés dans le coffre. Reconnectez le service pour démarrer ses outils.".to_owned())
    } else if status == "reconnect" && phase == "connected" {
        Some("Le jeton a expiré. Reconnectez le service.".to_owned())
    } else if status == "connected" && expired && can_refresh {
        Some("Le jeton sera renouvelé au prochain appel autorisé.".to_owned())
    } else if status == "error" && phase == "connected" {
        server
            .as_ref()
            .and_then(|s| s["error"].as_str())
            .map(str::to_owned)
    } else {
        message.filter(|s| !s.starts_with("Connecté."))
    };
    let message = message.map(|s| {
        if s == crate::mcp::AUTH_REQUIRED {
            "La connexion a expiré ou a été révoquée. Reconnectez le service.".to_owned()
        } else {
            s
        }
    });
    let device = device
        .filter(|_| phase == "authorizing")
        .and_then(|(code, expires)| code.map(|code| json!({"user_code":code,"verification_uri":github::DEVICE_PAGE,"expires_at":expires})))
        .unwrap_or(Value::Null);
    Ok(
        json!({"device":device,"device_flow":m.id=="github","configured":!matches!(m.id,"github"|"gmail"|"slack")||has_client,"unconfigured_message":(!has_client).then(||match m.id{"gmail"=>Some(gmail::not_configured().message),"slack"=>Some(slack::not_configured().message),_=>None}).flatten(),"id":m.id,"slot":row_id,"name":m.name,"description":m.description,"permissions":m.permissions,"scopes":m.scopes,"scopes_label":scopes_label(m),"redirect_uri":Value::Null,"source":m.source,"license":m.license,"setup":m.setup,"needs_client":!matches!(m.id,"notion"|"github"|"gmail"|"slack"),"has_client":has_client,"has_tokens":has_tokens,"can_refresh":can_refresh,"account":private.get("EMAIL"),"warning":warning,"status":status,"message":message,"authorization_url":url,"expires_at":expires,"server":server}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    id: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    /// The GitHub account slot to (re)connect; the first one when absent.
    #[serde(default)]
    account: Option<String>,
    /// Add a GitHub account beside the ones already connected.
    #[serde(default)]
    another: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Slot {
    id: String,
    #[serde(default)]
    account: Option<String>,
}
/// The catalogue row a request names: the service itself, or one of its GitHub slots.
fn slot_row(ctx: &Context<'_>, m: Manifest, account: Option<&str>) -> Result<String, ApiError> {
    let Some(account) = account.filter(|a| *a != m.id) else {
        return Ok(m.id.to_owned());
    };
    if m.id == "github" && github_rows(ctx)?.iter().any(|(row, _)| row == account) {
        return Ok(account.to_owned());
    }
    Err(invalid("Ce compte GitHub n’existe plus."))
}
/// Where a new GitHub account goes: a free slot, never over a live connection or a wait.
fn another_slot(ctx: &Context<'_>) -> Result<String, ApiError> {
    let rows = github_rows(ctx)?;
    if rows.iter().any(|(_, phase)| phase == "authorizing") {
        return Err(ApiError::new(
            409,
            "Une connexion GitHub attend déjà un code. Terminez-la ou annulez-la avant d’ajouter un autre compte.",
        ));
    }
    if rows.is_empty() {
        return Ok("github".into());
    }
    if let Some((row, _)) = rows.iter().find(|(_, phase)| phase == "disconnected") {
        return Ok(row.clone());
    }
    Ok((2..)
        .map(|n| format!("github:{n}"))
        .find(|row| rows.iter().all(|(taken, _)| taken != row))
        .expect("an unbounded range has a free slot"))
}
pub(super) fn routes(app: &App, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    let ctx = app.feature_context();
    if path == "/api/connectors/disconnect" {
        let input: Slot = parse(body)?;
        let m = manifest(&input.id)?;
        let row = slot_row(&ctx, m, input.account.as_deref())?;
        return super::route(
            app,
            "/api/mcp/remove",
            json!({"id":server_id(&row)}).to_string().as_bytes(),
        );
    }
    if path == "/api/connectors/cancel" {
        let input: Slot = parse(body)?;
        let m = manifest(&input.id)?;
        let row = slot_row(&ctx, m, input.account.as_deref())?;
        if m.id != "github" {
            return Err(invalid(
                "Ce service ne se connecte pas par un code à entrer.",
            ));
        }
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        // Bumping the generation retires the waiting thread: a late authorization,
        // even one GitHub still accepts, can no longer reach the vault.
        let cancelled = ctx.connection.execute("UPDATE wrapper_connector_catalogue SET generation=generation+1,phase='disconnected',message='Connexion annulée. Aucun jeton n’a été conservé.',authorization_url=NULL,device_user_code=NULL,device_expires_at=0 WHERE id=?1 AND phase='authorizing'",[&row])?;
        if cancelled == 0 {
            return Err(ApiError::new(
                409,
                "Aucune connexion GitHub n’est en attente d’un code.",
            ));
        }
        return Ok(json!({"message":"Connexion annulée. Aucun jeton n’a été conservé."}));
    }
    let input: Connect = parse(body)?;
    let m = manifest(&input.id)?;
    if m.id == "github" {
        // Nothing typed here is ever read: the application is the build's.
        github::client_id().ok_or_else(github::not_configured)?;
        if !input.client_id.is_empty() || !input.client_secret.is_empty() {
            return Err(invalid("GitHub n’utilise ni identifiant ni secret saisis ici. Cliquez simplement sur Connecter."));
        }
    }
    if matches!(m.id, "gmail" | "slack") {
        // Nothing typed here is ever read: the client is the build's.
        let (name, client) = if m.id == "gmail" {
            (
                "Gmail",
                gmail::client_id().ok_or_else(gmail::not_configured),
            )
        } else {
            (
                "Slack",
                slack::client_id().ok_or_else(slack::not_configured),
            )
        };
        client?;
        if !input.client_id.is_empty() || !input.client_secret.is_empty() {
            return Err(invalid(&format!("{name} n’utilise ni identifiant ni secret saisis ici. Cliquez simplement sur Connecter.")));
        }
    }
    if input.client_id.len() > 512
        || input.client_secret.len() > 1024
        || input.client_id.chars().any(char::is_control)
        || input.client_secret.chars().any(char::is_control)
    {
        return Err(invalid("Identifiants OAuth invalides."));
    }
    if (input.another || input.account.is_some()) && m.id != "github" {
        return Err(invalid("Ce service ne garde qu’un compte."));
    }
    let row = if input.another {
        another_slot(&ctx)?
    } else {
        slot_row(&ctx, m, input.account.as_deref())?
    };
    let id = server_id(&row);
    {
        // Settings runs on the HTTP thread: never wait behind a provider's
        // token exchange or first package download. State polling stays usable.
        let _guard = EFFECTS.try_lock().map_err(|_| {
            ApiError::new(
                409,
                "Une connexion est en cours de préparation. Réessayez dans quelques instants.",
            )
        })?;
        let mut env = credentials(&ctx, &id)?.unwrap_or_default();
        // A setup form is not a disconnect decision. Never replace credentials
        // or discard a live account's tokens before revocation was approved.
        let has_tokens = env.contains_key("ACCESS_TOKEN") || env.contains_key("REFRESH_TOKEN");
        if has_tokens
            && ((!input.client_id.is_empty() && env.get("CLIENT_ID") != Some(&input.client_id))
                || !input.client_secret.is_empty())
        {
            return Err(ApiError::new(
                409,
                "Déconnectez d’abord le service pour changer d’application OAuth.",
            ));
        }
        if !input.client_id.is_empty() {
            if env.get("CLIENT_ID") != Some(&input.client_id) {
                env.clear();
            }
            env.insert("CLIENT_ID".into(), input.client_id);
        }
        if !input.client_secret.is_empty() {
            env.insert("CLIENT_SECRET".into(), input.client_secret);
        }
        if !matches!(m.id, "notion" | "github" | "gmail" | "slack")
            && (!env.contains_key("CLIENT_ID") || !env.contains_key("CLIENT_SECRET"))
            && fake_base().is_none()
        {
            return Err(invalid("Saisissez l’identifiant et le secret de l’application OAuth. Ils seront conservés dans le coffre, jamais réaffichés."));
        }
        let tx = write_transaction(ctx.connection)?;
        tx.execute("INSERT OR IGNORE INTO wrapper_mcp_servers(id,name,command,arguments_json,environment_names_json) VALUES (?1,?2,?3,?4,?5)",params![id,m.name,if m.id=="gmail"{gmail::API}else if m.endpoint.is_empty(){"uvx · workspace-mcp 2.1.1"}else{m.endpoint},json!([]).to_string(),if is_native_service(m.id){json!(["ACCESS_TOKEN","REFRESH_TOKEN"])}else{json!(["ACCESS_TOKEN","REFRESH_TOKEN","CLIENT_ID","CLIENT_SECRET"])}.to_string()])?;
        tx.execute("INSERT INTO wrapper_connector_catalogue(id,server_id) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET generation=generation+1,authorization_url=NULL,phase=CASE WHEN phase='authorizing' THEN 'reconnect' ELSE phase END",params![row,id])?;
        // A native connection has no setup to keep before its tokens exist.
        if !is_native_service(m.id) {
            save(&ctx, &id, &env)?;
        }
        tx.commit()?;
    }
    super::route(
        app,
        "/api/mcp/restart",
        json!({"id":id}).to_string().as_bytes(),
    )
}
pub(super) fn preview(ctx: &Context<'_>, server: &Server, action: &str) -> Option<Value> {
    let (m, phase, _, revision) = record(ctx, &server.id).ok()??;
    // GitHub has nothing in the vault before its first connection.
    let private = credentials(ctx, &server.id).ok()?.unwrap_or_default();
    let mut names: Vec<_> = private
        .keys()
        .filter(|k| k.as_str() != "EMAIL")
        .cloned()
        .collect();
    names.sort();
    let remove = action == "remove";
    let action = format!(
        "{} {}",
        if remove {
            "Déconnecter"
        } else if phase != "disconnected" || private.contains_key("ACCESS_TOKEN") {
            "Reconnecter"
        } else {
            "Connecter"
        },
        m.name
    );
    if m.id == "github" {
        let connection = if remove {
            ""
        } else {
            "GitHub affichera un code à 8 caractères qu’Ezerbot vous montre ici ; la page https://github.com/login/device s’ouvre dans le navigateur. Vous vous identifiez, autorisez l’application GitHub d’Ezerbot et choisissez les dépôts chez GitHub. Seul l’identifiant public de l’application est envoyé : aucun secret n’existe dans Ezerbot. Les jetons reçus sont conservés dans le coffre. Une nouvelle identification réinitialise l’activation par bot et les règles de ce service."
        };
        let mut card = json!({"id":m.id,"service":m.name,"action":action,"transport":"https","variables_privées":names,"permissions":m.permissions,"portées":"aucune ; les permissions sont celles de la GitHub App d’Ezerbot, choisies chez GitHub","source":m.source,"licence":m.license,"révision":revision,"connexion":connection});
        card["déconnexion"] = match private.get("EMAIL").filter(|_| remove) {
            // The confirmation names the account it would disconnect.
            Some(login) => {
                card["compte"] = json!(login);
                json!(format!("Le compte GitHub {login} sera déconnecté : ses jetons locaux, l’activation par bot et les règles de ce service sont supprimés, et le connecteur s’arrête. Ezerbot demande aussi à GitHub de révoquer le jeton ; sans le secret d’une application, GitHub peut refuser, et le message le dira. Retirez alors l’autorisation d’Ezerbot dans les réglages de votre compte GitHub."))
            }
            None => json!("Les jetons locaux, l’activation par bot et les règles de ce service sont supprimés, et le connecteur s’arrête. Ezerbot demande aussi à GitHub de révoquer le jeton ; sans le secret d’une application, GitHub peut refuser, et le message le dira."),
        };
        return Some(card);
    }
    if m.id == "gmail" {
        let connection = if remove {
            ""
        } else {
            "Le navigateur s’ouvre sur la page de connexion de Google. Seul l’identifiant public du client Google d’Ezerbot est envoyé, avec une preuve PKCE ; le retour se fait sur ce Mac (127.0.0.1) et aucun secret n’existe dans Ezerbot. Les jetons reçus sont conservés dans le trousseau macOS : s’il refuse l’écriture, la connexion échoue et rien n’est conservé. Une nouvelle identification réinitialise l’activation par bot et les règles de ce service."
        };
        let mut card = json!({"id":m.id,"service":m.name,"action":action,"transport":"https","variables_privées":names,"permissions":m.permissions,"portées":scopes_label(m),"source":m.source,"licence":m.license,"révision":revision,"connexion":connection});
        card["déconnexion"] = match private.get("EMAIL").filter(|_| remove) {
            // The confirmation names the account it would disconnect.
            Some(email) => {
                card["compte"] = json!(email);
                json!(format!("Le compte Gmail {email} sera déconnecté : ses jetons locaux, l’activation par bot et les règles de ce service sont supprimés, et le connecteur s’arrête. Ezerbot demande aussi à Google de révoquer le jeton ; si Google ne le confirme pas, le message le dira. Retirez alors l’accès d’Ezerbot dans les réglages de votre compte Google."))
            }
            None => json!("Les jetons locaux, l’activation par bot et les règles de ce service sont supprimés, et le connecteur s’arrête. Ezerbot demande aussi à Google de révoquer le jeton ; si Google ne le confirme pas, le message le dira."),
        };
        return Some(card);
    }
    if m.id == "slack" {
        let connection = if remove {
            ""
        } else {
            "Slack s’ouvre dans le navigateur : vous choisissez l’espace de travail et autorisez le client Slack d’Ezerbot à lire vos canaux. Seul l’identifiant public du client est envoyé, avec une preuve PKCE : aucun secret n’existe dans Ezerbot. Le jeton reçu est conservé dans le coffre local (le trousseau macOS quand il est disponible). Une nouvelle identification réinitialise l’activation par bot et les règles de ce service."
        };
        let mut card = json!({"id":m.id,"service":m.name,"action":action,"transport":"https","variables_privées":names,"permissions":m.permissions,"portées":scopes_label(m),"source":m.source,"licence":m.license,"révision":revision,"connexion":connection});
        card["déconnexion"] = match private.get("EMAIL").filter(|_| remove) {
            Some(workspace) => {
                card["compte"] = json!(workspace);
                json!(format!("L’espace Slack {workspace} sera déconnecté : le jeton local, l’activation par bot et les règles de ce service sont supprimés, et le connecteur s’arrête. Ezerbot demande aussi à Slack de révoquer le jeton, et le message dit si Slack l’a confirmé."))
            }
            None => json!("Le jeton local, l’activation par bot et les règles de ce service sont supprimés, et le connecteur s’arrête. Ezerbot demande aussi à Slack de révoquer le jeton, et le message dit si Slack l’a confirmé."),
        };
        return Some(card);
    }
    let connection = if m.endpoint.is_empty() {
        "Le serveur Google workspace-mcp 2.1.1 est téléchargé si nécessaire, puis lancé sur ce Mac avec uvx et l’adaptateur Ezerbot (identifiants en mémoire)."
    } else {
        "Le serveur officiel du service est contacté par HTTPS ; aucun programme supplémentaire n’est installé."
    };
    Some(
        json!({"id":m.id,"service":m.name,"action":action,"transport":if m.endpoint.is_empty(){"stdio"}else{"https"},"variables_privées":names,"permissions":m.permissions,"portées":scopes_label(m),"source":m.source,"licence":m.license,"révision":revision,"connexion":format!("{connection} Si une identification est nécessaire, le navigateur s’ouvre pour OAuth et les jetons sont conservés dans le coffre. Une nouvelle identification réinitialise l’activation par bot et les règles de ce service."),"déconnexion":"Révocation distante, suppression locale des jetons et des règles, arrêt du connecteur. La configuration de l’application OAuth est conservée pour une prochaine connexion."}),
    )
}
fn credentials(ctx: &Context<'_>, id: &str) -> Result<Option<HashMap<String, String>>, ApiError> {
    let vault = super::super::forms::vault(ctx)?;
    let mut env: Option<HashMap<String, String>> = vault
        .get(&secret_name(id))?
        .map(|raw| {
            serde_json::from_str::<Value>(&raw)
                .ok()
                .and_then(|v| serde_json::from_value(v["variables"].clone()).ok())
                .ok_or_else(|| invalid("Les identifiants du connecteur sont illisibles."))
        })
        .transpose()?;
    if let Some(env) = &mut env {
        let email: Option<String> = ctx
            .connection
            .query_row(
                "SELECT account_email FROM wrapper_connector_catalogue WHERE server_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(email) = email {
            env.insert("EMAIL".into(), email);
        }
    }
    Ok(env)
}
fn save(ctx: &Context<'_>, id: &str, env: &HashMap<String, String>) -> Result<(), ApiError> {
    let mut private = env.clone();
    private.remove("EMAIL");
    super::super::forms::vault(ctx)?.set(&secret_name(id), PURPOSE, &json!({"variables":private}).to_string()).map_err(|_|invalid("Impossible de conserver les identifiants OAuth dans le coffre local (4 Ko maximum). Vérifiez le stockage des secrets, puis reconnectez le service."))?;
    if let Some(email) = env.get("EMAIL") {
        ctx.connection.execute(
            "UPDATE wrapper_connector_catalogue SET account_email=?2 WHERE server_id=?1",
            params![id, email],
        )?;
    }
    Ok(())
}
fn phase(ctx: &Context<'_>, id: &str, phase: &str, message: &str) -> Result<(), ApiError> {
    ctx.connection.execute("UPDATE wrapper_connector_catalogue SET phase=?2,message=?3,authorization_url=NULL,device_user_code=NULL,device_expires_at=0 WHERE server_id=?1",params![id,phase,message])?;
    Ok(())
}
/// Debug-only QA overrides cannot point at credentials collectors on the Internet.
fn fake_base() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("GROKBOT_FAKE_CONNECTORS") {
        let port = value
            .strip_prefix("http://127.0.0.1:")
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p != 0)
            .expect("GROKBOT_FAKE_CONNECTORS doit viser http://127.0.0.1:<port>.");
        return Some(format!("http://127.0.0.1:{port}"));
    }
    None
}
fn endpoint(m: Manifest, kind: &str) -> String {
    if let Some(base) = fake_base() {
        return format!("{base}/{}/{kind}", m.id);
    }
    if m.id == "github" {
        return match kind {
            "device" => "https://github.com/login/device/code",
            "token" => "https://github.com/login/oauth/access_token",
            "user" => "https://api.github.com/user",
            _ => "https://api.github.com",
        }
        .into();
    }
    match kind {
        "api" if m.id == "gmail" => gmail::API.into(),
        "calendar" | "drive" if m.id == "gmail" => "https://www.googleapis.com".into(),
        "mcp" => m.endpoint.into(),
        "authorize" if m.endpoint.is_empty() => {
            "https://accounts.google.com/o/oauth2/v2/auth".into()
        }
        "token" if m.endpoint.is_empty() => "https://oauth2.googleapis.com/token".into(),
        "revoke" if m.endpoint.is_empty() => "https://oauth2.googleapis.com/revoke".into(),
        "userinfo" => "https://openidconnect.googleapis.com/v1/userinfo".into(),
        "api" if m.id == "slack" => "https://slack.com/api".into(),
        "authorize" if m.id == "slack" => "https://slack.com/oauth/v2_user/authorize".into(),
        "token" if m.id == "slack" => "https://slack.com/api/oauth.v2.user.access".into(),
        "revoke" if m.id == "slack" => "https://slack.com/api/auth.revoke".into(),
        "authorize" => "https://mcp.notion.com/authorize".into(),
        "register" => "https://mcp.notion.com/register".into(),
        _ => "https://mcp.notion.com/token".into(),
    }
}
fn attach(
    ctx: &Context<'_>,
    server: &Server,
    m: Manifest,
    env: &HashMap<String, String>,
) -> Result<Value, ApiError> {
    if m.id == "github" {
        return github::attach(ctx, server);
    }
    if m.id == "gmail" {
        return gmail::attach(ctx, server);
    }
    if m.id == "slack" {
        return slack::attach(ctx, server);
    }
    let token = env
        .get("ACCESS_TOKEN")
        .ok_or_else(|| invalid("Reconnectez ce service pour obtenir un nouveau jeton."))?;
    let client = if fake_base().is_some() || !m.endpoint.is_empty() {
        Client::remote(endpoint(m, "mcp"), token.clone())
    } else {
        let mut candidates = vec![
            std::path::PathBuf::from("/opt/homebrew/bin/uvx"),
            std::path::PathBuf::from("/usr/local/bin/uvx"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            let home = std::path::PathBuf::from(home);
            candidates.extend([home.join(".local/bin/uvx"), home.join(".cargo/bin/uvx")]);
        }
        let uvx = candidates
            .into_iter()
            .find(|p| p.is_file())
            .unwrap_or_else(|| "uvx".into());
        let service = if m.id == "calendar" { "calendar" } else { m.id };
        let mut child_env = HashMap::from([
            ("ACCESS_TOKEN".into(), token.clone()),
            (
                "GOOGLE_EMAIL".into(),
                env.get("EMAIL").cloned().unwrap_or_default(),
            ),
            ("GOOGLE_SCOPES".into(), m.scopes.into()),
            ("MCP_SINGLE_USER_MODE".into(), "1".into()),
            ("PYTHON_DOTENV_DISABLED".into(), "1".into()),
        ]);
        if let Some(home) = std::env::var_os("HOME") {
            child_env.insert(
                "WORKSPACE_MCP_CREDENTIALS_DIR".into(),
                std::path::PathBuf::from(&home)
                    .join("Library/Caches/Ezerbot/connecteurs")
                    .join(&server.id)
                    .to_string_lossy()
                    .into_owned(),
            );
            child_env.insert(
                "UV_CACHE_DIR".into(),
                std::path::PathBuf::from(home)
                    .join("Library/Caches/Ezerbot/connecteurs/uv")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        child_env.insert(
            "USER_GOOGLE_EMAIL".into(),
            env.get("EMAIL").cloned().unwrap_or_default(),
        );
        let mut client = Client::spawn(&uvx.to_string_lossy(),&["--from".into(),"workspace-mcp==2.1.1".into(),"python".into(),"-c".into(),include_str!("google_bridge.py").into(),"--transport".into(),"stdio".into(),"--tools".into(),service.into(),"--tool-tier".into(),"core".into()],&child_env).map_err(|_| invalid("Impossible de lancer le serveur Google. Installez uv (uvx), puis reconnectez le service. Le premier démarrage télécharge workspace-mcp 2.1.1."))?;
        // Account identity/scopes/cache location are public configuration, not
        // tokens. Keep only the token in this trusted adapter's redaction snapshot.
        client.retain_private_environment(&["ACCESS_TOKEN"]);
        client
    };
    let handle = crate::mcp::Connection::new(client);
    let old = ctx
        .provider
        .mcp()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(server.id.clone(), handle.clone());
    drop(old);
    let discovered = handle
        .client
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .initialize_with_timeout(std::time::Duration::from_secs(
            if m.endpoint.is_empty() && fake_base().is_none() {
                120
            } else {
                10
            },
        ))
        .map_err(|e| ApiError::new(503, e.to_string()));
    let mut private = env.clone();
    private.remove("EMAIL");
    let tools = discovered.and_then(|tools| safe_catalogue(ctx, tools, &private));
    super::execution::publish(ctx, &server.id, &handle, &tools)?;
    match tools {
        Ok(tools) => {
            phase(
                ctx,
                &server.id,
                "connected",
                "Connecté. Activez les outils pour les bots de votre choix.",
            )?;
            Ok(
                json!({"message":"Service connecté. Les outils restent soumis à vos approbations.","outils":tools}),
            )
        }
        Err(e) => {
            phase(
                ctx,
                &server.id,
                if e.message == crate::mcp::AUTH_REQUIRED {
                    "reconnect"
                } else {
                    "error"
                },
                &e.message,
            )?;
            Err(e)
        }
    }
}
pub(super) fn start(
    ctx: &Context<'_>,
    server: &Server,
    arguments: &Value,
) -> Result<Option<Value>, ApiError> {
    let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some((m, phase_name, expires, _)) = record(ctx, &server.id)? else {
        return Ok(None);
    };
    if arguments != &lifecycle_arguments(ctx, server, "start", false)
        && arguments != &lifecycle_arguments(ctx, server, "start", true)
    {
        return Err(invalid("La configuration OAuth a changé. Approuvez une nouvelle demande depuis les paramètres."));
    }
    let mut env = if is_native_service(m.id) {
        // No client is stored: before the first connection there is nothing in the vault.
        credentials(ctx, &server.id)?.unwrap_or_default()
    } else {
        credentials(ctx, &server.id)?.ok_or_else(|| {
            invalid("Identifiants OAuth absents du coffre. Reconnectez le service.")
        })?
    };
    if phase_name != "reconnect"
        && env.contains_key("ACCESS_TOKEN")
        && (expires == 0 || expires > wall_clock() as i64 + 30_000)
    {
        return attach(ctx, server, m, &env).map(Some);
    }
    if env.contains_key("REFRESH_TOKEN") {
        match oauth::refresh(ctx, server, m, &mut env) {
            Ok(()) => return attach(ctx, server, m, &env).map(Some),
            // An unreachable GitHub is not a dead refresh: start no second sign-in.
            Err(error) if m.id == "github" && error.status == 503 => return Err(error),
            Err(_) => (),
        }
    }
    if m.id == "github" {
        return match github::begin(ctx, server, m) {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                phase(ctx, &server.id, "error", &error.message)?;
                Err(error)
            }
        };
    }
    match oauth::begin(ctx, server, m, env) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            phase(ctx, &server.id, "error", &error.message)?;
            Err(error)
        }
    }
}
/// Called only after the existing gate admitted this service tool. Refresh never
/// replays a failed tools/call: a provider may already have applied its effect.
pub(super) fn ensure(ctx: &Context<'_>, server: &Server) -> Result<(), ApiError> {
    let Some((m, phase_name, expires, _)) = record(ctx, &server.id)? else {
        return Ok(());
    };
    if phase_name == "authorizing" || phase_name == "disconnected" || phase_name == "reconnect" {
        return Err(invalid(crate::mcp::AUTH_REQUIRED));
    }
    if expires == 0 || expires > wall_clock() as i64 + 30_000 {
        return Ok(());
    }
    let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
    if m.id == "github" {
        // A concurrent read may have renewed while this one waited: one rotation only.
        if record(ctx, &server.id)?.is_some_and(|(_, _, expires, _)| {
            expires == 0 || expires > wall_clock() as i64 + 30_000
        }) {
            return Ok(());
        }
    }
    let mut env =
        credentials(ctx, &server.id)?.ok_or_else(|| invalid(crate::mcp::AUTH_REQUIRED))?;
    if let Err(error) = oauth::refresh(ctx, server, m, &mut env) {
        if m.id == "github" && error.status == 503 {
            return Err(error);
        }
        phase(
            ctx,
            &server.id,
            "reconnect",
            if m.id == "github" {
                "La connexion GitHub a expiré ou a été révoquée. Reconnectez GitHub."
            } else if m.id == "gmail" {
                gmail::RECONNECT
            } else {
                crate::mcp::AUTH_REQUIRED
            },
        )?;
        super::execution::retire(ctx, &server.id);
        return Err(if m.id == "github" {
            invalid("La connexion GitHub a expiré ou a été révoquée. Reconnectez GitHub dans Paramètres › Connecteurs.")
        } else if m.id == "gmail" {
            invalid(gmail::RECONNECT)
        } else {
            invalid(crate::mcp::AUTH_REQUIRED)
        });
    }
    attach(ctx, server, m, &env).map(|_| ())
}
pub(super) fn auth_error(ctx: &Context<'_>, id: &str, message: &str) {
    if message == crate::mcp::AUTH_REQUIRED {
        let _ = phase(ctx, id, "reconnect", message);
    }
}
pub(super) fn remove(
    ctx: &Context<'_>,
    server: &Server,
    arguments: &Value,
) -> Result<Option<Value>, ApiError> {
    let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some((m, _, _, _)) = record(ctx, &server.id)? else {
        return Ok(None);
    };
    if arguments != &lifecycle_arguments(ctx, server, "remove", false) {
        return Err(invalid("La configuration OAuth a changé. Approuvez une nouvelle demande depuis les paramètres."));
    }
    let env = credentials(ctx, &server.id)?.unwrap_or_default();
    let had_tokens = env.contains_key("ACCESS_TOKEN");
    // Invalidate any browser callback before revocation/deletion. All callback
    // publication and tool refreshes share EFFECTS, so no late token resurrection.
    ctx.connection.execute("UPDATE wrapper_connector_catalogue SET generation=generation+1,authorization_url=NULL WHERE server_id=?1",[&server.id])?;
    let revoked = oauth::revoke(m, &env);
    let tx = write_transaction(ctx.connection)?;
    // A provider app's registration is configuration, not an account token.
    // Keep just it so a deliberate reconnect need not ask for the same setup.
    // Slack has no registration to keep: whatever client an earlier version stored goes too.
    let client: HashMap<_, _> = env
        .into_iter()
        .filter(|(key, _)| m.id != "slack" && matches!(key.as_str(), "CLIENT_ID" | "CLIENT_SECRET"))
        .collect();
    if client.is_empty() {
        super::super::forms::vault(ctx)?.remove(&secret_name(&server.id))?;
    } else {
        save(ctx, &server.id, &client)?;
    }
    tx.execute(
        "DELETE FROM wrapper_mcp_bots WHERE server_id=?1",
        [&server.id],
    )?;
    tx.execute("DELETE FROM wrapper_mcp_servers WHERE id=?1", [&server.id])?;
    tx.execute(
        "DELETE FROM forms_approval_rules WHERE tool GLOB ?1 OR tool IN (?2,?3)",
        params![
            format!("mcp_{}_*", server.id),
            format!("mcp_start_{}", server.id),
            format!("mcp_remove_{}", server.id)
        ],
    )?;
    let message = if is_native_service(m.id) && !had_tokens {
        "Déconnecté. Aucun jeton n’était conservé ; les règles de ce connecteur ont été supprimées."
    } else if revoked && m.id == "gmail" {
        "Déconnecté. Google a confirmé la révocation ; les jetons et règles de ce connecteur ont été supprimés."
    } else if revoked && m.id == "github" {
        "Déconnecté. GitHub a confirmé la révocation ; les jetons et règles de ce connecteur ont été supprimés."
    } else if revoked && m.id == "slack" {
        "Déconnecté. Slack a confirmé la révocation ; le jeton et les règles de ce connecteur ont été supprimés."
    } else if revoked {
        "Déconnecté. Les jetons et règles de ce connecteur ont été supprimés. La configuration OAuth est conservée."
    } else {
        REVOCATION_WARNING
    };
    // A conversation that had chosen this account is asked again, never sent to another.
    tx.execute("DELETE FROM wrapper_github_account_choice WHERE account_id=(SELECT account_id FROM wrapper_connector_catalogue WHERE server_id=?1)",[&server.id])?;
    if m.id == "gmail" {
        gmail::forget_mailbox(&tx, &server.id)?;
    }
    tx.execute("UPDATE wrapper_connector_catalogue SET phase='disconnected',message=?2,expires_at=0,account_email=NULL,account_id=NULL,device_user_code=NULL,device_expires_at=0 WHERE server_id=?1",params![server.id,message])?;
    tx.commit()?;
    super::execution::retire(ctx, &server.id);
    Ok(Some(
        json!({"message":message,"revocation_confirmed":revoked}),
    ))
}
