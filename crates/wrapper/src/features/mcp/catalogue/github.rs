//! GitHub App connection and its closed read adapter. The user token comes from the
//! device flow, which sends only the public `client_id`: no `client_secret` is read,
//! sent or stored. The device code lives in the waiting thread's memory, never in
//! SQLite or the vault. Tokens are written once, after GitHub names the account, and
//! a refused write keeps nothing.
use super::*;
use std::{
    io::Read,
    time::{Duration, Instant},
};
pub(in super::super) mod comment;

/// Read tool of the closed list. It needs no approval card: it only reads.
pub(in super::super) const TOOL: &str = "github_list_issues";
pub(in super::super) use comment::TOOL as COMMENT_TOOL;
pub(super) const DEVICE_PAGE: &str = "https://github.com/login/device";
const CODE_LIFETIME: u64 = 900;
const PAGE: usize = 30;
const RECONNECT: &str =
    "La connexion GitHub a expiré ou a été révoquée. Reconnectez GitHub dans Paramètres › Connecteurs.";

/// Public value of the build, replaceable by the environment for tests and the judge.
pub(super) fn client_id() -> Option<String> {
    let value = std::env::var("EZERBOT_GITHUB_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("EZERBOT_GITHUB_CLIENT_ID").map(str::to_owned))?;
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .then(|| value.to_owned())
}
pub(super) fn not_configured() -> ApiError {
    invalid("L’application GitHub d’Ezerbot n’est pas configurée dans cette version. Il n’y a ni identifiant ni secret à saisir : la connexion reste indisponible tant que l’application n’est pas préparée.")
}
fn unreachable_github() -> ApiError {
    ApiError::new(
        503,
        "GitHub est injoignable pour le moment. Vérifiez la connexion, puis réessayez.",
    )
}
fn refused_connection() -> ApiError {
    invalid("GitHub a refusé la connexion. Cliquez sur Reconnecter pour obtenir un nouveau code.")
}
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(12))
        .build()
}

struct Reply {
    status: u16,
    next_page: bool,
    exhausted: bool,
    reset: Option<u64>,
    single_sign_on: bool,
    body: Value,
}
/// One call, no redirect, bounded body. Transport failures carry no detail.
fn send(request: ureq::Request, form: Option<&[(&str, &str)]>) -> Result<Reply, ()> {
    let request = request
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "Ezerbot")
        .set("X-GitHub-Api-Version", "2022-11-28");
    reply_of(match form {
        Some(fields) => request
            .set("Accept", "application/json")
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(
                &url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(fields.iter().copied())
                    .finish(),
            ),
        None => request.call(),
    })
}
fn send_json(request: ureq::Request, body: &Value) -> Result<Reply, ()> {
    reply_of(
        request
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "Ezerbot")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .send_json(body),
    )
}
fn reply_of(outcome: Result<ureq::Response, ureq::Error>) -> Result<Reply, ()> {
    let response = match outcome {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(_) => return Err(()),
    };
    let status = response.status();
    let next_page = response
        .header("link")
        .is_some_and(|link| link.contains("rel=\"next\""));
    let exhausted = response.header("x-ratelimit-remaining") == Some("0");
    let reset = response
        .header("x-ratelimit-reset")
        .and_then(|v| v.parse().ok());
    let single_sign_on = response.header("x-github-sso").is_some();
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(());
    }
    Ok(Reply {
        status,
        next_page,
        exhausted,
        reset,
        single_sign_on,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}

fn pace(interval: u64) -> Duration {
    if fake_base().is_some() {
        Duration::from_millis(interval.saturating_mul(1000).max(20))
    } else {
        Duration::from_secs(interval.clamp(1, 600))
    }
}

/// Ask GitHub for a code, show it, and wait for the authorization in a thread.
pub(super) fn begin(ctx: &Context<'_>, server: &Server, m: Manifest) -> Result<Value, ApiError> {
    let client = client_id().ok_or_else(not_configured)?;
    let reply = send(
        agent().post(&endpoint(m, "device")),
        Some(&[("client_id", &client)]),
    )
    .map_err(|_| unreachable_github())?;
    let text = |key: &str, limit: usize| {
        reply.body[key]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control))
            .map(str::to_owned)
    };
    let (Some(device), Some(code)) = (text("device_code", 256), text("user_code", 32)) else {
        return Err(refused_connection());
    };
    if reply.status != 200
        || reply.body["error"].is_string()
        || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(refused_connection());
    }
    let lifetime = reply.body["expires_in"]
        .as_u64()
        .unwrap_or(CODE_LIFETIME)
        .clamp(1, CODE_LIFETIME);
    let interval = reply.body["interval"].as_u64().unwrap_or(5);
    ctx.connection.execute(
        "UPDATE wrapper_connector_catalogue SET generation=generation+1,phase='authorizing',message='Entrez ce code chez GitHub (15 minutes).',authorization_url=?2,device_user_code=?3,device_expires_at=?4 WHERE server_id=?1",
        params![server.id, DEVICE_PAGE, code, wall_clock().saturating_add(lifetime * 1000) as i64],
    )?;
    let generation: i64 = ctx.connection.query_row(
        "SELECT generation FROM wrapper_connector_catalogue WHERE server_id=?1",
        [&server.id],
        |r| r.get(0),
    )?;
    let path = ctx.db_path.to_path_buf();
    let provider = ctx.provider.clone();
    let server = server.clone();
    let deadline = Instant::now() + Duration::from_secs(lifetime);
    std::thread::spawn(move || {
        // Never print provider errors, codes or token bodies.
        let _ = wait_for_authorization(
            &path, &provider, &server, m, generation, &client, &device, interval, deadline,
        );
    });
    #[cfg(target_os = "macos")]
    if fake_base().is_none()
        && std::process::Command::new("/usr/bin/open")
            .arg(DEVICE_PAGE)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
    {
        return Ok(
            json!({"message":"Ouvrez github.com/login/device dans votre navigateur, puis entrez le code affiché dans Paramètres › Connecteurs."}),
        );
    }
    Ok(
        json!({"message":"Entrez le code affiché dans Paramètres › Connecteurs sur github.com/login/device."}),
    )
}

#[allow(clippy::too_many_arguments)]
fn wait_for_authorization(
    path: &std::path::Path,
    provider: &crate::Provider,
    server: &Server,
    m: Manifest,
    generation: i64,
    client: &str,
    device: &str,
    mut interval: u64,
    deadline: Instant,
) -> Result<(), ApiError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(4))?;
    let ctx = Context {
        connection: &conn,
        provider,
        db_path: path,
        now_ms: wall_clock(),
        bot_id: None,
        conversation_id: None,
        run_id: None,
    };
    let current = || -> bool {
        conn.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_connector_catalogue WHERE server_id=?1 AND generation=?2 AND phase='authorizing')",params![server.id,generation],|r|r.get(0)).unwrap_or(false)
    };
    // Only the owner of the current generation may end the wait.
    let end = |message: &str| -> Result<(), ApiError> {
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        if current() {
            phase(&ctx, &server.id, "reconnect", message)?;
        }
        Ok(())
    };
    let mut failures = 0;
    loop {
        let resume = Instant::now() + pace(interval);
        while Instant::now() < resume.min(deadline) {
            if !current() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !current() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return end("Le code a expiré. Cliquez sur Reconnecter pour obtenir un nouveau code.");
        }
        let reply = send(
            agent().post(&endpoint(m, "token")),
            Some(&[
                ("client_id", client),
                ("device_code", device),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]),
        );
        let Ok(reply) = reply else {
            failures += 1;
            if failures >= 3 {
                return end(
                    "GitHub est injoignable. Cliquez sur Reconnecter pour obtenir un nouveau code.",
                );
            }
            continue;
        };
        failures = 0;
        match reply.body["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = reply.body["interval"]
                    .as_u64()
                    .filter(|next| *next > interval)
                    .unwrap_or(interval + 5);
                continue;
            }
            Some("expired_token") => {
                return end(
                    "Le code a expiré. Cliquez sur Reconnecter pour obtenir un nouveau code.",
                )
            }
            Some("access_denied") => {
                return end(
                    "Vous avez refusé l’autorisation chez GitHub. Aucun jeton n’a été enregistré.",
                )
            }
            Some(_) => return end(&refused_connection().message),
            None => (),
        }
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        if !current() {
            // Cancelled or replaced while GitHub answered: the token is dropped unread.
            return Ok(());
        }
        let outcome = (|| -> Result<(), ApiError> {
            if reply.status != 200 {
                return Err(refused_connection());
            }
            complete(&ctx, server, m, &reply.body)
        })();
        if let Err(error) = outcome {
            phase(&ctx, &server.id, "reconnect", &error.message)?;
        }
        return Ok(());
    }
}

fn tokens(raw: &Value, env: &mut HashMap<String, String>) -> Result<i64, ApiError> {
    if raw["token_type"]
        .as_str()
        .is_some_and(|v| !v.eq_ignore_ascii_case("bearer"))
    {
        return Err(refused_connection());
    }
    let valid = |key: &str| {
        raw[key]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 3000 && !s.chars().any(char::is_control))
    };
    let access = valid("access_token").ok_or_else(refused_connection)?;
    env.insert("ACCESS_TOKEN".into(), access.into());
    // GitHub rotates the refresh token: the old one is replaced, never kept.
    match valid("refresh_token") {
        Some(refresh) => env.insert("REFRESH_TOKEN".into(), refresh.into()),
        None => env.remove("REFRESH_TOKEN"),
    };
    Ok(raw["expires_in"]
        .as_u64()
        .map(|s| {
            wall_clock()
                .saturating_add(s.saturating_mul(1000))
                .min(i64::MAX as u64) as i64
        })
        .unwrap_or(0))
}
fn write_tokens(
    ctx: &Context<'_>,
    server: &Server,
    env: &HashMap<String, String>,
) -> Result<(), ApiError> {
    save(ctx, &server.id, env).map_err(|_| {
        let locked = crate::features::forms::vault(ctx)
            .is_ok_and(|vault| vault.storage_status().backend == "file_fallback");
        invalid(if locked {
            "Le trousseau macOS est verrouillé ou inaccessible : la connexion GitHub n’a pas été conservée et aucun jeton n’a été écrit. Déverrouillez-le, puis reconnectez GitHub."
        } else {
            "Impossible de conserver la connexion GitHub dans le coffre local. Vérifiez le stockage des secrets, puis reconnectez GitHub."
        })
    })
}
fn forget_tokens(ctx: &Context<'_>, server: &Server) {
    if let Ok(vault) = crate::features::forms::vault(ctx) {
        let _ = vault.remove(&secret_name(&server.id));
    }
}

/// GitHub named a token: learn the account, store, then attach. Nothing remains on error.
fn complete(ctx: &Context<'_>, server: &Server, m: Manifest, raw: &Value) -> Result<(), ApiError> {
    let mut env = HashMap::new();
    let expires = tokens(raw, &mut env)?;
    let (login, id) = account_of(m, &env["ACCESS_TOKEN"])?;
    env.insert("EMAIL".into(), login);
    write_tokens(ctx, server, &env)?;
    let stored = (|| -> Result<(), ApiError> {
        let tx = write_transaction(ctx.connection)?;
        // A new sign-in may select another account: tool activation does not follow it.
        tx.execute(
            "DELETE FROM wrapper_mcp_bots WHERE server_id=?1",
            [&server.id],
        )?;
        tx.execute(
            "UPDATE wrapper_connector_catalogue SET expires_at=?2,account_id=?3,device_user_code=NULL,device_expires_at=0 WHERE server_id=?1",
            params![server.id, expires, id],
        )?;
        tx.commit()?;
        attach(ctx, server).map(|_| ())
    })();
    if stored.is_err() {
        forget_tokens(ctx, server);
    }
    stored
}
fn account_of(m: Manifest, access: &str) -> Result<(String, String), ApiError> {
    let reply = send(
        agent()
            .get(&endpoint(m, "user"))
            .set("Authorization", &format!("Bearer {access}")),
        None,
    )
    .map_err(|_| unreachable_github())?;
    let login = reply.body["login"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 100 && !s.chars().any(char::is_control));
    let id = reply.body["id"].as_u64();
    match (reply.status, login, id) {
        (200, Some(login), Some(id)) => Ok((login.to_owned(), id.to_string())),
        _ => Err(refused_connection()),
    }
}

fn tool_definition() -> Value {
    json!({
        "name":"list_issues",
        "description":"Lister les issues d’un dépôt GitHub (une page de 30), en lecture seule, avec le compte GitHub connecté.",
        "inputSchema":schema(),
    })
}
fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "repository":{"type":"string","description":"Dépôt au format propriétaire/dépôt."},
        "state":{"type":"string","enum":["open","closed","all"],"description":"État des issues ; open par défaut."}
    },"required":["repository"]})
}
/// Model-facing tool, advertised only while GitHub is connected and enabled for the bot.
pub(in super::super) fn tool_spec() -> grokbot_engine::contracts::ToolSpec {
    grokbot_engine::contracts::ToolSpec {
        name: TOOL.into(),
        description: "GitHub · lister les issues d’un dépôt `propriétaire/dépôt` (lecture seule, une page de 30, compte GitHub connecté dans Ezerbot). Reprenez le champ `texte` du résultat tel quel dans votre réponse : il nomme le compte et donne numéro, titre, état et auteur de chaque issue.".into(),
        schema: schema(),
        requires_approval: false,
    }
}
/// The connection has no process: it is native, and its tools are this fixed list.
pub(super) fn attach(ctx: &Context<'_>, server: &Server) -> Result<Value, ApiError> {
    let tools = json!([tool_definition(), comment::tool_definition()]);
    ctx.connection.execute(
        "UPDATE wrapper_mcp_servers SET tools_json=?2,error=NULL WHERE id=?1",
        params![server.id, tools.to_string()],
    )?;
    phase(
        ctx,
        &server.id,
        "connected",
        "Connecté. Activez les outils pour les bots de votre choix.",
    )?;
    Ok(
        json!({"message":"GitHub est connecté. Activez ses outils pour un bot : les lectures partent sans carte d’approbation.","outils":tools}),
    )
}

/// Renew the access token from the refresh token. A dead refresh drops both tokens so
/// the expired access is never replayed; an unreachable GitHub changes nothing.
pub(super) fn refresh(
    ctx: &Context<'_>,
    server: &Server,
    m: Manifest,
    env: &mut HashMap<String, String>,
) -> Result<(), ApiError> {
    let client = client_id().ok_or_else(not_configured)?;
    let dead = |env: &mut HashMap<String, String>| {
        env.remove("ACCESS_TOKEN");
        env.remove("REFRESH_TOKEN");
        let rest: HashMap<_, _> = env
            .iter()
            .filter(|(k, _)| *k != "EMAIL")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if rest.is_empty() {
            forget_tokens(ctx, server);
        } else {
            let _ = save(ctx, &server.id, &rest);
        }
        let _ = ctx.connection.execute(
            "UPDATE wrapper_connector_catalogue SET expires_at=0 WHERE server_id=?1",
            [&server.id],
        );
        invalid(RECONNECT)
    };
    let Some(old) = env.get("REFRESH_TOKEN").cloned() else {
        return Err(dead(env));
    };
    let reply = send(
        agent().post(&endpoint(m, "token")),
        Some(&[
            ("client_id", &client),
            ("grant_type", "refresh_token"),
            ("refresh_token", &old),
        ]),
    );
    let reply = match reply {
        // A rate limit or an outage says nothing about the refresh token: keep it.
        Ok(reply)
            if reply.status < 500
                && !matches!(reply.status, 403 | 429)
                && !matches!(
                    reply.body["error"].as_str(),
                    Some("rate_limit_exceeded" | "slow_down")
                ) =>
        {
            reply
        }
        _ => return Err(unreachable_github()),
    };
    if reply.status != 200 || reply.body["error"].is_string() {
        return Err(dead(env));
    }
    let mut renewed = env.clone();
    let Ok(expires) = tokens(&reply.body, &mut renewed) else {
        return Err(dead(env));
    };
    // The previous refresh token is already invalid at GitHub: keep nothing of it.
    if write_tokens(ctx, server, &renewed).is_err() {
        let _ = dead(env);
        return Err(invalid("Le trousseau macOS n’a pas accepté le jeton renouvelé : reconnectez GitHub après l’avoir déverrouillé."));
    }
    ctx.connection.execute(
        "UPDATE wrapper_connector_catalogue SET expires_at=?2 WHERE server_id=?1",
        params![server.id, expires],
    )?;
    *env = renewed;
    Ok(())
}

/// Run one authenticated call. An access past its expiry is renewed first, and a 401
/// renews once more and calls again: a rejected call has not touched the provider.
fn authorized(
    ctx: &Context<'_>,
    server: &Server,
    m: Manifest,
    call: &dyn Fn(&str) -> Result<Reply, ApiError>,
) -> Result<Reply, ApiError> {
    super::ensure(ctx, server)?;
    let attempt = |ctx: &Context<'_>| -> Result<(String, Reply), ApiError> {
        let access = credentials(ctx, &server.id)?
            .and_then(|env| env.get("ACCESS_TOKEN").cloned())
            .ok_or_else(|| invalid(RECONNECT))?;
        let reply = call(&access)?;
        Ok((access, reply))
    };
    let (used, mut reply) = attempt(ctx)?;
    if reply.status == 401 {
        {
            let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
            let mut env = credentials(ctx, &server.id)?.ok_or_else(|| invalid(RECONNECT))?;
            if env.get("ACCESS_TOKEN") == Some(&used) {
                if let Err(error) = refresh(ctx, server, m, &mut env) {
                    if error.status != 503 {
                        phase(ctx, &server.id, "reconnect", RECONNECT)?;
                    }
                    return Err(error);
                }
            }
        }
        reply = attempt(ctx)?.1;
        if reply.status == 401 {
            phase(ctx, &server.id, "reconnect", RECONNECT)?;
            return Err(invalid(RECONNECT));
        }
    }
    Ok(reply)
}
fn limited(reply: &Reply) -> bool {
    reply.exhausted
        || reply.status == 429
        || reply.body["message"]
            .as_str()
            .is_some_and(|m| m.to_lowercase().contains("rate limit"))
}
fn limit_error(reply: &Reply, login: &str) -> ApiError {
    let wait = reply
        .reset
        .and_then(|at| at.checked_sub(wall_clock() / 1000))
        .filter(|s| *s <= 86_400)
        .map(|s| format!(" Réessayez dans environ {} min.", s.div_ceil(60).max(1)))
        .unwrap_or_default();
    invalid(&format!(
        "GitHub limite le débit des requêtes pour le compte {login}.{wait}"
    ))
}

fn repository(text: &str) -> Option<(&str, &str)> {
    let (owner, name) = text.split_once('/')?;
    let part = |s: &str| {
        (1..=100).contains(&s.len())
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (part(owner) && part(name)).then_some((owner, name))
}
fn clean(text: &str, limit: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(limit)
        .collect()
}

pub(in super::super) fn call(
    ctx: &Context<'_>,
    arguments: &Value,
) -> grokbot_engine::contracts::ToolOutcome {
    match list_issues(ctx, arguments) {
        Ok(output) => grokbot_engine::contracts::ToolOutcome::Done { output },
        Err(error) => grokbot_engine::contracts::ToolOutcome::Failed {
            reason: error.message,
        },
    }
}
/// The account this request resolved. Zero connected accounts is a sentence, not a call.
fn resolve(ctx: &Context<'_>) -> Result<(Server, Manifest, String), ApiError> {
    let row: Option<(String, String, Option<String>)> = ctx.connection.query_row("SELECT server_id,phase,account_email FROM wrapper_connector_catalogue WHERE id='github'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((id, phase_name, login)) = row else {
        return Err(invalid("GitHub n’est pas connecté. Connectez-le dans Paramètres › Connecteurs, puis réessayez."));
    };
    let login = match (phase_name.as_str(), login) {
        ("connected", Some(login)) => login,
        ("reconnect", _) => return Err(invalid(RECONNECT)),
        _ => return Err(invalid("GitHub n’est pas connecté. Connectez-le dans Paramètres › Connecteurs, puis réessayez.")),
    };
    let server = super::super::servers(ctx.connection)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| invalid("GitHub n’est pas connecté. Connectez-le dans Paramètres › Connecteurs, puis réessayez."))?;
    Ok((server, manifest("github")?, login))
}
fn list_issues(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        repository: String,
        state: Option<String>,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
        invalid("Arguments invalides : indiquez le dépôt sous la forme propriétaire/dépôt.")
    })?;
    let Some((owner, name)) = repository(args.repository.trim()) else {
        return Err(invalid(
            "Dépôt invalide : écrivez-le sous la forme propriétaire/dépôt.",
        ));
    };
    let state = match args.state.as_deref().unwrap_or("open") {
        s @ ("open" | "closed" | "all") => s,
        _ => return Err(invalid("État invalide : open, closed ou all.")),
    };
    let (server, m, login) = resolve(ctx)?;
    let url = format!(
        "{}/repos/{owner}/{name}/issues?state={state}&per_page={PAGE}",
        endpoint(m, "api")
    );
    let reply = authorized(ctx, &server, m, &|access| {
        send(
            agent()
                .get(&url)
                .set("Authorization", &format!("Bearer {access}")),
            None,
        )
        .map_err(|_| unreachable_github())
    })?;
    let repo = format!("{owner}/{name}");
    match reply.status {
        200 => (),
        404 => return Err(invalid(&format!("Le dépôt {repo} est introuvable pour le compte {login}, ou l’application GitHub d’Ezerbot n’y est pas installée. Vérifiez le nom et les dépôts choisis chez GitHub."))),
        403 | 429 if limited(&reply) => return Err(limit_error(&reply, &login)),
        403 => {
            let sso = if reply.single_sign_on { " Votre organisation exige une session SAML : autorisez-la chez GitHub." } else { "" };
            return Err(invalid(&format!("Accès refusé à {repo} pour le compte {login}. Si le dépôt appartient à une organisation, autorisez la session SAML ou élargissez l’installation de l’application GitHub d’Ezerbot.{sso}")));
        }
        500..=599 => return Err(ApiError::new(503, "GitHub est indisponible pour le moment. Réessayez plus tard.")),
        code => return Err(invalid(&format!("GitHub a répondu de façon inattendue (code {code}). Réessayez plus tard."))),
    }
    let items = reply
        .body
        .as_array()
        .ok_or_else(|| invalid("La réponse de GitHub est illisible."))?;
    let issues: Vec<Value> = items
        .iter()
        // The issues route also lists pull requests; they are read elsewhere.
        .filter(|item| item.get("pull_request").is_none())
        .filter_map(|item| {
            Some(json!({
                "numero":item["number"].as_u64()?,
                "titre":clean(item["title"].as_str()?, 200),
                "etat":if item["state"] == "closed" {"closed"} else {"open"},
                "auteur":clean(item["user"]["login"].as_str().unwrap_or("inconnu"), 100),
            }))
        })
        .collect();
    let heading = match state {
        "open" => "Issues ouvertes",
        "closed" => "Issues fermées",
        _ => "Issues",
    };
    let mut text = if issues.is_empty() {
        format!("Aucune issue trouvée dans {repo} (compte GitHub : {login}).")
    } else {
        let mut text = format!("{heading} de {repo} · compte GitHub : {login}");
        for issue in &issues {
            text.push_str(&format!(
                "\n#{} · {} · {} · par {}",
                issue["numero"],
                issue["titre"].as_str().unwrap_or(""),
                if issue["etat"] == "closed" {
                    "fermée"
                } else {
                    "ouverte"
                },
                issue["auteur"].as_str().unwrap_or("")
            ));
        }
        text
    };
    if reply.next_page {
        text.push_str(&format!(
            "\nIl y a d’autres issues sur GitHub : seule la première page de {PAGE} est affichée."
        ));
    }
    Ok(
        json!({"compte":login,"depot":repo,"etat":state,"issues":issues,"autres_issues":reply.next_page,"texte":text}),
    )
}
