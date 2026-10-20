//! Local reconstruction of the registry/broker/grant separation in REA protocol.md.
//! Fixed manifests, no service credentials or third-party code bundled. All I/O
//! with a provider is reachable only from an admitted lifecycle/tool call.
use super::*;
use std::sync::Mutex;
mod oauth;

static EFFECTS: Mutex<()> = Mutex::new(());
const PURPOSE: &str = grokbot_forms::vault::MCP_ENV_PURPOSE;
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
    Manifest { id:"gmail", name:"Gmail", description:"Rechercher et lire vos courriels, préparer et envoyer des messages.", permissions:"Lecture et modification des messages, envoi de courriels ; adresse du compte. Chaque outil reste soumis à vos approbations.", scopes:"openid email https://www.googleapis.com/auth/gmail.modify", source:"https://github.com/taylorwilsdon/google_workspace_mcp", license:"MIT · workspace-mcp 2.1.1 (communautaire)", setup:GOOGLE_SETUP, endpoint:"" },
    Manifest { id:"calendar", name:"Google Calendar", description:"Consulter vos agendas et gérer vos événements.", permissions:"Lecture et modification des agendas et événements ; adresse du compte.", scopes:"openid email https://www.googleapis.com/auth/calendar", source:"https://github.com/taylorwilsdon/google_workspace_mcp", license:"MIT · workspace-mcp 2.1.1 (communautaire)", setup:GOOGLE_SETUP, endpoint:"" },
    Manifest { id:"drive", name:"Google Drive", description:"Rechercher, lire et organiser les fichiers de votre Drive.", permissions:"Lecture et modification de tous les fichiers Drive accessibles au compte ; adresse du compte.", scopes:"openid email https://www.googleapis.com/auth/drive", source:"https://github.com/taylorwilsdon/google_workspace_mcp", license:"MIT · workspace-mcp 2.1.1 (communautaire)", setup:GOOGLE_SETUP, endpoint:"" },
    Manifest { id:"slack", name:"Slack", description:"Rechercher des discussions, lire les fils et envoyer des messages.", permissions:"Recherche des messages et fichiers, lecture des conversations et fichiers, envoi de messages, lecture des utilisateurs.", scopes:"search:read.public search:read.private search:read.im search:read.mpim search:read.files search:read.users channels:history groups:history im:history mpim:history files:read chat:write users:read users:read.email", source:"https://docs.slack.dev/ai/slack-mcp-server/", license:"Service officiel Slack · aucun logiciel redistribué", setup:"Une app Slack interne (ou publiée) autorisée par votre administrateur est nécessaire. Ajoutez les portées indiquées et le retour OAuth http://127.0.0.1:52187/oauth/slack/callback ; saisissez l’identifiant et le secret de cette app.", endpoint:"https://mcp.slack.com/mcp" },
    Manifest { id:"notion", name:"Notion", description:"Rechercher et modifier les pages de votre espace Notion.", permissions:"Accès aux pages et espaces choisis pendant le consentement Notion, en lecture et écriture.", scopes:"default", source:"https://developers.notion.com/docs/mcp", license:"Service officiel Notion · aucun logiciel redistribué", setup:"Connexion dans le navigateur. Vous choisirez les espaces et pages autorisés chez Notion ; aucun identifiant d’application à saisir.", endpoint:"https://mcp.notion.com/mcp" },
    Manifest { id:"github", name:"GitHub", description:"Consulter et gérer les dépôts, issues et demandes de fusion.", permissions:"Accès en lecture et écriture aux dépôts privés et publics (repo), lecture des organisations (read:org).", scopes:"repo read:org", source:"https://github.com/github/github-mcp-server", license:"MIT · serveur officiel GitHub hébergé, aucun logiciel redistribué", setup:"Créez une OAuth App GitHub avec une URL de retour http://127.0.0.1/oauth/github/callback. Saisissez son identifiant et son secret. GitHub autorise le port local attribué à la connexion.", endpoint:"https://api.githubcopilot.com/mcp/" },
];
fn manifest(id: &str) -> Result<Manifest, ApiError> {
    MANIFESTS
        .iter()
        .find(|m| m.id == id)
        .copied()
        .ok_or_else(|| invalid("Ce connecteur n’est pas dans le catalogue."))
}
fn server_id(id: &str) -> String {
    format!("c{}", &format!("{:x}", Sha256::digest(id.as_bytes()))[..15])
}
fn record(
    ctx: &Context<'_>,
    server: &str,
) -> Result<Option<(Manifest, String, i64, i64)>, ApiError> {
    let row: Option<(String,String,i64,i64)> = ctx.connection.query_row("SELECT id,phase,expires_at,generation FROM wrapper_connector_catalogue WHERE server_id=?1", [server], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    row.map(|(id, phase, expires, g)| Ok((manifest(&id)?, phase, expires, g)))
        .transpose()
}
pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_connector_catalogue (id TEXT PRIMARY KEY,server_id TEXT NOT NULL UNIQUE,phase TEXT NOT NULL DEFAULT 'disconnected',message TEXT,expires_at INTEGER NOT NULL DEFAULT 0,generation INTEGER NOT NULL DEFAULT 0,authorization_url TEXT,account_email TEXT);
    UPDATE wrapper_connector_catalogue SET phase='reconnect',message='La connexion a été interrompue. Reconnectez le service.',authorization_url=NULL,generation=generation+1 WHERE phase='authorizing';")?;
    Ok(())
}
pub(super) fn state(ctx: &Context<'_>) -> Result<Value, ApiError> {
    let local = super::state_servers(ctx)?;
    let mut items = Vec::new();
    for m in MANIFESTS {
        let id = server_id(m.id);
        let row: Option<(String,Option<String>,i64,Option<String>)> = ctx.connection.query_row("SELECT phase,message,expires_at,authorization_url FROM wrapper_connector_catalogue WHERE id=?1", [m.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let server = local.iter().find(|s| s["id"] == id).cloned();
        let (phase, message, expires, url) =
            row.unwrap_or_else(|| ("disconnected".into(), None, 0, None));
        let status = if phase == "authorizing"
            || phase == "reconnect"
            || phase == "error"
            || phase == "disconnected"
        {
            phase.as_str()
        } else if expires > 0 && expires <= wall_clock() as i64 + 30_000 {
            "reconnect"
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
            Some("Le jeton a expiré. Reconnectez le service ; les appels déjà autorisés peuvent renouveler un jeton valide.".to_owned())
        } else if status == "error" && phase == "connected" {
            server
                .as_ref()
                .and_then(|s| s["error"].as_str())
                .map(str::to_owned)
        } else {
            message
        };
        items.push(json!({"id":m.id,"name":m.name,"description":m.description,"permissions":m.permissions,"scopes":m.scopes,"source":m.source,"license":m.license,"setup":m.setup,"needs_client":m.id!="notion","status":status,"message":message,"authorization_url":url,"expires_at":expires,"server":server}));
    }
    Ok(json!(items))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    id: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
}
pub(super) fn routes(app: &App, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    let ctx = app.feature_context();
    if path == "/api/connectors/disconnect" {
        let input: Identity = parse(body)?;
        let m = manifest(&input.id)?;
        return super::route(
            app,
            "/api/mcp/remove",
            json!({"id":server_id(m.id)}).to_string().as_bytes(),
        );
    }
    let input: Connect = parse(body)?;
    let m = manifest(&input.id)?;
    if input.client_id.len() > 512
        || input.client_secret.len() > 1024
        || input.client_id.chars().any(char::is_control)
        || input.client_secret.chars().any(char::is_control)
    {
        return Err(invalid("Identifiants OAuth invalides."));
    }
    let id = server_id(m.id);
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
        if !input.client_id.is_empty() {
            if env.get("CLIENT_ID") != Some(&input.client_id) {
                env.clear();
            }
            env.insert("CLIENT_ID".into(), input.client_id);
        }
        if !input.client_secret.is_empty() {
            env.insert("CLIENT_SECRET".into(), input.client_secret);
        }
        if m.id != "notion"
            && (!env.contains_key("CLIENT_ID") || !env.contains_key("CLIENT_SECRET"))
            && fake_base().is_none()
        {
            return Err(invalid("Saisissez l’identifiant et le secret de l’application OAuth. Ils seront conservés dans le coffre, jamais réaffichés."));
        }
        let tx = write_transaction(ctx.connection)?;
        tx.execute("INSERT OR IGNORE INTO wrapper_mcp_servers(id,name,command,arguments_json,environment_names_json) VALUES (?1,?2,?3,?4,'[\"ACCESS_TOKEN\",\"REFRESH_TOKEN\",\"CLIENT_ID\",\"CLIENT_SECRET\"]')",params![id,m.name,if m.endpoint.is_empty(){"uvx · workspace-mcp 2.1.1"}else{m.endpoint},json!([]).to_string()])?;
        tx.execute("INSERT INTO wrapper_connector_catalogue(id,server_id) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET generation=generation+1,authorization_url=NULL,phase=CASE WHEN phase='authorizing' THEN 'reconnect' ELSE phase END",params![m.id,id])?;
        save(&ctx, &id, &env)?;
        tx.commit()?;
    }
    super::route(
        app,
        "/api/mcp/restart",
        json!({"id":id}).to_string().as_bytes(),
    )
}
pub(super) fn preview(ctx: &Context<'_>, server: &Server) -> Option<Value> {
    let (m, _, _, revision) = record(ctx, &server.id).ok()??;
    Some(
        json!({"service":m.name,"permissions":m.permissions,"portées":m.scopes,"source":m.source,"licence":m.license,"révision":revision,"connexion":"Ouverture du navigateur, échange OAuth, stockage privé et découverte des outils. Une nouvelle identification réinitialise l’activation par bot et les règles de ce service. Le serveur Google est téléchargé et lancé avec uvx ; les autres serveurs sont contactés par HTTPS.","déconnexion":"Révocation distante, suppression locale des jetons et des règles, arrêt du connecteur."}),
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
    ctx.connection.execute("UPDATE wrapper_connector_catalogue SET phase=?2,message=?3,authorization_url=NULL WHERE server_id=?1",params![id,phase,message])?;
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
    match kind {
        "mcp" => m.endpoint.into(),
        "authorize" if m.endpoint.is_empty() => {
            "https://accounts.google.com/o/oauth2/v2/auth".into()
        }
        "token" if m.endpoint.is_empty() => "https://oauth2.googleapis.com/token".into(),
        "revoke" if m.endpoint.is_empty() => "https://oauth2.googleapis.com/revoke".into(),
        "userinfo" => "https://openidconnect.googleapis.com/v1/userinfo".into(),
        "authorize" if m.id == "slack" => "https://slack.com/oauth/v2_user/authorize".into(),
        "token" if m.id == "slack" => "https://slack.com/api/oauth.v2.user.access".into(),
        "revoke" if m.id == "slack" => "https://slack.com/api/auth.revoke".into(),
        "authorize" if m.id == "github" => "https://github.com/login/oauth/authorize".into(),
        "token" if m.id == "github" => "https://github.com/login/oauth/access_token".into(),
        "revoke" if m.id == "github" => "https://api.github.com/applications".into(),
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
    let mut env = credentials(ctx, &server.id)?
        .ok_or_else(|| invalid("Identifiants OAuth absents du coffre. Reconnectez le service."))?;
    if phase_name != "reconnect"
        && env.contains_key("ACCESS_TOKEN")
        && (expires == 0 || expires > wall_clock() as i64 + 30_000)
    {
        return attach(ctx, server, m, &env).map(Some);
    }
    if env.contains_key("REFRESH_TOKEN") && oauth::refresh(ctx, server, m, &mut env).is_ok() {
        return attach(ctx, server, m, &env).map(Some);
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
    let mut env =
        credentials(ctx, &server.id)?.ok_or_else(|| invalid(crate::mcp::AUTH_REQUIRED))?;
    if oauth::refresh(ctx, server, m, &mut env).is_err() {
        phase(ctx, &server.id, "reconnect", crate::mcp::AUTH_REQUIRED)?;
        super::execution::retire(ctx, &server.id);
        return Err(invalid(crate::mcp::AUTH_REQUIRED));
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
    // Invalidate any browser callback before revocation/deletion. All callback
    // publication and tool refreshes share EFFECTS, so no late token resurrection.
    ctx.connection.execute("UPDATE wrapper_connector_catalogue SET generation=generation+1,authorization_url=NULL WHERE server_id=?1",[&server.id])?;
    let revoked = oauth::revoke(m, &env);
    let tx = write_transaction(ctx.connection)?;
    super::super::forms::vault(ctx)?.remove(&secret_name(&server.id))?;
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
    let message = if revoked {
        "Déconnecté. Les jetons et règles de ce connecteur ont été supprimés."
    } else {
        "Déconnecté sur ce Mac ; révocation distante non confirmée. Retirez aussi l’accès d’Ezerbot dans les réglages du service."
    };
    tx.execute("UPDATE wrapper_connector_catalogue SET phase='disconnected',message=?2,expires_at=0,account_email=NULL WHERE server_id=?1",params![server.id,message])?;
    tx.commit()?;
    super::execution::retire(ctx, &server.id);
    Ok(Some(
        json!({"message":message,"revocation_confirmed":revoked}),
    ))
}
