//! Jira Cloud connection and its closed read adapter. The token comes from Ezerbot's Atlassian
//! OAuth 2.0 (3LO) app: Atlassian's token endpoint asks for a `client_secret` that no public
//! client can avoid, so this one is read from the environment of the process only. It is never
//! compiled in, never written to the vault, and never sent anywhere but the token endpoint. The
//! two reading tools call the site's REST API with the connection's token and need no approval
//! card; commenting an issue only ever runs from a validated draft card (see `draft`).
use super::slack::{agent, clean, reply_of, Reply};
use super::*;

pub(in super::super) mod draft;

const ISSUES_TOOL: &str = "jira_list_issues";
const ISSUE_TOOL: &str = "jira_get_issue";
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 50;
/// Characters of one description or comment shown in the bubble.
const EXCERPT: usize = 3000;
/// Comments read with one issue.
const COMMENTS: usize = 20;
/// The loopback port Atlassian's app declares: its callback URL is registered, not negotiated.
pub(super) const REDIRECT_PORT: u16 = 52189;
const NOT_CONNECTED: &str =
    "Jira n’est pas connecté. Connectez-le dans Paramètres › Connecteurs, puis réessayez.";
const DISCONNECTED: &str =
    "Jira est déconnecté. Reconnectez Jira dans Paramètres › Connecteurs, puis réessayez.";
const ENABLE: &str =
    "Jira n’est pas activé pour ce bot. Activez-le dans Paramètres › Connecteurs, puis réessayez.";
pub(super) const RECONNECT: &str =
    "La connexion Jira a expiré ou a été révoquée. Reconnectez Jira dans Paramètres › Connecteurs.";

/// The reading tools of the closed list; the draft tool is in `draft`.
pub(in super::super) fn is_read(name: &str) -> bool {
    matches!(name, ISSUES_TOOL | ISSUE_TOOL)
}

/// Public value of the build, replaceable by the environment for tests and the judge.
/// It counts only beside the secret Atlassian asks for: one without the other is no client.
pub(super) fn client_id() -> Option<String> {
    client_secret()?;
    let value = std::env::var("EZERBOT_JIRA_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("EZERBOT_JIRA_CLIENT_ID").map(str::to_owned))?;
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .then(|| value.to_owned())
}
/// Read at run time from the process environment, never at build time: where the secret of a
/// shipped build lives is Charles's decision, and the code keeps none.
pub(super) fn client_secret() -> Option<String> {
    let value = std::env::var("EZERBOT_JIRA_CLIENT_SECRET").ok()?;
    let value = value.trim();
    (!value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}
pub(super) const NOT_CONFIGURED: &str = "Le client Jira d’Ezerbot n’est pas configuré dans cette version. Atlassian exige un identifiant et un secret d’application, et le rangement de ce secret reste à décider : il n’y a rien à saisir ici, et la connexion reste indisponible tant que le client n’est pas préparé.";
pub(super) fn not_configured() -> ApiError {
    invalid(NOT_CONFIGURED)
}
/// Tokens are written once, after Atlassian named the account and the site; a refused write
/// keeps nothing.
pub(super) fn write_tokens(
    ctx: &Context<'_>,
    server: &Server,
    env: &HashMap<String, String>,
) -> Result<(), ApiError> {
    save(ctx, &server.id, env).map_err(|_| {
        let locked = crate::features::forms::vault(ctx)
            .is_ok_and(|vault| vault.storage_status().backend == "file_fallback");
        invalid(if locked {
            "Le trousseau macOS est verrouillé ou inaccessible : la connexion Jira n’a pas été conservée et aucun jeton n’a été écrit. Déverrouillez-le, puis reconnectez Jira."
        } else {
            "Impossible de conserver la connexion Jira dans le coffre local. Vérifiez le stockage des secrets, puis reconnectez Jira."
        })
    })
}

fn issues_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "project":{"type":"string","description":"Clé du projet Jira (par exemple ENG)."},
        "limit":{"type":"integer","minimum":1,"maximum":MAX_LIMIT,"description":"Nombre d’issues à lire, les plus récemment modifiées d’abord ; 20 par défaut."}
    },"required":["project"]})
}
fn issue_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "issue":{"type":"string","description":"Clé de l’issue, par exemple ENG-12."}
    },"required":["issue"]})
}
const ISSUES_SUMMARY: &str =
    "Lister les issues d’un projet Jira, en lecture seule, avec le compte connecté.";
const ISSUE_SUMMARY: &str =
    "Lire une issue Jira par sa clé, avec ses commentaires, en lecture seule.";
fn read_specs() -> Vec<grokbot_engine::contracts::ToolSpec> {
    let read = |name: &str, description: &str, schema| grokbot_engine::contracts::ToolSpec {
        name: name.into(),
        description: description.into(),
        schema,
        requires_approval: false,
    };
    vec![
        read(ISSUES_TOOL, "Jira · lister les issues d’un projet (clé, résumé, état, assigné), les plus récemment modifiées d’abord, lecture seule, avec le compte Jira connecté dans Ezerbot. Reprenez le champ `texte` du résultat tel quel dans votre réponse.", issues_schema()),
        read(ISSUE_TOOL, "Jira · lire une issue par sa clé (par exemple ENG-12) avec sa description et ses commentaires, lecture seule. Reprenez le champ `texte` du résultat tel quel dans votre réponse.", issue_schema()),
    ]
}
/// Model-facing tools, advertised only while Jira is connected and enabled for the bot.
pub(in super::super) fn tool_specs() -> Vec<grokbot_engine::contracts::ToolSpec> {
    let mut specs = read_specs();
    specs.extend(draft::tool_specs());
    specs
}
/// The connection has no process: it is native, and its tools are this fixed list.
pub(super) fn attach(ctx: &Context<'_>, server: &Server) -> Result<Value, ApiError> {
    let mut tools = vec![
        json!({"name":"list_issues","description":ISSUES_SUMMARY,"inputSchema":issues_schema()}),
        json!({"name":"get_issue","description":ISSUE_SUMMARY,"inputSchema":issue_schema()}),
    ];
    tools.extend(draft::tool_definitions());
    let tools = json!(tools);
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
        json!({"message":"Jira est connecté. Activez les outils pour un bot : lire les issues d’un projet part sans carte d’approbation ; un commentaire n’est publié qu’après votre clic sur sa carte brouillon.","outils":tools}),
    )
}

/// The account of the live connection, and the server that holds its token.
struct Account {
    server: Server,
    /// The account and site Atlassian gave at sign-in: `Nom · Site`.
    name: String,
    /// Moves with every sign-in: a token is the one a draft was prepared with only while it holds.
    generation: i64,
    enabled: bool,
}
fn connected(ctx: &Context<'_>) -> Result<Option<Account>, ApiError> {
    let row: Option<(String, Option<String>, i64)> = ctx
        .connection
        .query_row(
            "SELECT server_id,account_email,generation FROM wrapper_connector_catalogue WHERE id='jira' AND phase='connected'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((server_id, name, generation)) = row else {
        return Ok(None);
    };
    Ok(super::super::servers(ctx.connection)?
        .into_iter()
        .find(|s| s.id == server_id)
        .map(|server| Account {
            enabled: ctx
                .bot_id
                .is_none_or(|bot| super::super::enabled(ctx.connection, &server.id, bot)),
            name: name.unwrap_or_else(|| "Jira".into()),
            generation,
            server,
        }))
}
/// Why the Jira tool is not offered to this bot: not connected, disconnected, or not enabled.
pub(in super::super) fn unavailable(ctx: &Context<'_>) -> String {
    match connected(ctx) {
        Ok(Some(_)) => ENABLE.into(),
        _ => {
            let phase: Option<String> = ctx
                .connection
                .query_row(
                    "SELECT phase FROM wrapper_connector_catalogue WHERE id='jira'",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .ok()
                .flatten();
            match phase.as_deref() {
                None => NOT_CONNECTED,
                Some("reconnect") => RECONNECT,
                Some(_) => DISCONNECTED,
            }
            .into()
        }
    }
}

fn unreachable_jira() -> ApiError {
    ApiError::new(
        503,
        "Jira est injoignable pour le moment. Vérifiez la connexion, puis réessayez.",
    )
}
/// One call, no redirect, bounded body. Transport failures carry no detail.
fn request(method: &str, url: &str, access: &str, body: Option<Value>) -> Result<Reply, ()> {
    let request = agent()
        .request(method, url)
        .set("Authorization", &format!("Bearer {access}"))
        .set("Accept", "application/json");
    reply_of(match body {
        Some(body) => request.send_json(body),
        None => request.call(),
    })
}
/// The REST root of one site: Atlassian routes by the site's cloud identifier.
fn site_api(cloud: &str) -> Result<String, ()> {
    let m = manifest("jira").map_err(|_| ())?;
    Ok(format!("{}/{cloud}/rest/api/3", endpoint(m, "api")))
}
/// A site the connection is bound to: what Atlassian named at sign-in.
struct Site {
    cloud: String,
    url: String,
}
/// The site lives beside the account in the catalogue row, not in the vault: it is no secret,
/// and a vault value is masked wherever it appears, which would hide every link to an issue.
fn site_of(ctx: &Context<'_>, server_id: &str) -> Result<Option<Site>, ApiError> {
    let kept: Option<String> = ctx
        .connection
        .query_row(
            "SELECT account_id FROM wrapper_connector_catalogue WHERE server_id=?1",
            [server_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(kept
        .as_deref()
        .and_then(|kept| kept.split_once(' '))
        .filter(|(cloud, url)| is_cloud_id(cloud) && is_site_url(url))
        .map(|(cloud, url)| Site {
            cloud: cloud.to_owned(),
            url: url.trim_end_matches('/').to_owned(),
        }))
}
/// Binds the connection to the site Atlassian named at sign-in.
pub(super) fn bind_site(
    ctx: &Context<'_>,
    server_id: &str,
    identity: &Identity,
) -> Result<(), ApiError> {
    ctx.connection.execute(
        "UPDATE wrapper_connector_catalogue SET account_id=?2 WHERE server_id=?1",
        params![server_id, format!("{} {}", identity.cloud, identity.url)],
    )?;
    Ok(())
}
fn is_cloud_id(text: &str) -> bool {
    (1..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn is_site_url(text: &str) -> bool {
    text.starts_with("https://")
        && text.len() <= 200
        && !text.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// What a sign-in learns from Atlassian: the account and the site every call will address.
pub(super) struct Identity {
    pub(super) display: String,
    pub(super) cloud: String,
    pub(super) url: String,
}
/// The account and the site a fresh token belongs to: `Nom · Site`, which the card shows.
pub(super) fn identity(access: &str) -> Option<Identity> {
    let m = manifest("jira").ok()?;
    let me = request("GET", &endpoint(m, "me"), access, None).ok()?;
    let resources = request("GET", &endpoint(m, "resources"), access, None).ok()?;
    if me.status != 200 || resources.status != 200 {
        return None;
    }
    let pick = |value: &Value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 100 && !s.chars().any(char::is_control))
            .map(str::to_owned)
    };
    let who = pick(&me.body["email"]).or_else(|| pick(&me.body["name"]))?;
    // The first site that carries both Jira permissions; the consent screen can grant several.
    let granted = |site: &&Value| {
        let scopes: Vec<&str> = site["scopes"]
            .as_array()
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_else(|| vec!["read:jira-work", "write:jira-work"]);
        scopes.contains(&"read:jira-work") && scopes.contains(&"write:jira-work")
    };
    let site = resources.body.as_array()?.iter().find(granted)?;
    let cloud = pick(&site["id"]).filter(|id| is_cloud_id(id))?;
    let url = pick(&site["url"]).filter(|url| is_site_url(url))?;
    let url = url.trim_end_matches('/').to_owned();
    let name = pick(&site["name"]).unwrap_or_else(|| {
        url.trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or("")
            .to_owned()
    });
    Some(Identity {
        display: format!("{who} · {name}"),
        cloud,
        url,
    })
}

/// What Atlassian's answer means for Charles. A dead token also moves the card to reconnect.
fn refusal(ctx: &Context<'_>, account: &Account, reply: &Reply, what: &str) -> ApiError {
    if reply.status == 429 {
        let wait = reply
            .retry_after
            .filter(|s| *s <= 3600)
            .map(|s| format!(" Réessayez dans environ {} s.", s.max(1)))
            .unwrap_or_default();
        return ApiError::new(429, format!("Jira limite le débit des requêtes.{wait}"));
    }
    if reply.status == 401 {
        // Only the connection that was refused moves to reconnect: a sign-in that began or
        // finished since belongs to another generation and is left as it is.
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        let _ = ctx.connection.execute(
            "UPDATE wrapper_connector_catalogue SET phase='reconnect',message=?3,authorization_url=NULL,device_user_code=NULL,device_expires_at=0 WHERE server_id=?1 AND generation=?2 AND phase='connected'",
            params![account.server.id, account.generation, RECONNECT],
        );
        return invalid(RECONNECT);
    }
    if reply.status == 403 {
        return invalid(&format!("Jira refuse l’accès à {what} avec ce compte. Vérifiez vos droits dans Jira, ou reconnectez Jira pour accorder de nouveau les autorisations."));
    }
    if reply.status == 404 {
        return invalid(&format!("Jira ne trouve pas {what} avec ce compte."));
    }
    if reply.status >= 500 {
        return unreachable_jira();
    }
    invalid(&format!(
        "Jira a refusé la requête concernant {what} ({}).",
        reply.status
    ))
}

/// The credentials of the live connection, read under the sign-in lock: a sign-in replaces
/// them there. The account the read started with must still be the one that is connected and
/// enabled for the bot, so a sign-in in between never lends its token to this read.
struct Access {
    token: String,
    site: Site,
}
fn token_of(ctx: &Context<'_>, account: &Account) -> Result<Access, ApiError> {
    super::ensure(ctx, &account.server)?;
    let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
    match connected(ctx)? {
        Some(now)
            if now.enabled && now.generation == account.generation && now.name == account.name => {}
        _ => {
            return Err(invalid(
                "Le compte Jira a changé pendant la lecture : rien n’a été lu. Réessayez.",
            ))
        }
    }
    let env = credentials(ctx, &account.server.id)?.ok_or_else(|| invalid(RECONNECT))?;
    match (env.get("ACCESS_TOKEN"), site_of(ctx, &account.server.id)?) {
        (Some(token), Some(site)) => Ok(Access {
            token: token.clone(),
            site,
        }),
        _ => Err(invalid(RECONNECT)),
    }
}

/// A project key such as `ENG`: capital letters, digits and underscores, starting with a
/// letter. Strict enough to sit inside a quoted JQL string.
pub(super) fn is_project_key(text: &str) -> bool {
    (2..=10).contains(&text.len())
        && text.as_bytes()[0].is_ascii_uppercase()
        && text
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}
/// An issue key such as `ENG-12`: the project key, a dash, the number.
pub(super) fn is_issue_key(text: &str) -> bool {
    text.split_once('-').is_some_and(|(project, number)| {
        is_project_key(project)
            && (1..=7).contains(&number.len())
            && number.bytes().all(|b| b.is_ascii_digit())
    })
}
/// `AAAA-MM-JJ HH:MM UTC` from Jira's timestamp (`2025-10-09T08:55:12.000+0000`).
fn when(iso: &str) -> String {
    let (Some(day), Some(time), Some(tail)) = (iso.get(..10), iso.get(11..16), iso.get(16..))
    else {
        return String::new();
    };
    let offset = tail.find(['+', '-', 'Z']).map(|at| &tail[at..]);
    let zone = match offset {
        Some("Z" | "+0000" | "+00:00" | "-0000" | "-00:00") => "UTC".to_owned(),
        Some(o) if o.is_ascii() && o.len() == 5 => format!("UTC{}:{}", &o[..3], &o[3..]),
        Some(o) if o.is_ascii() && o.len() == 6 && &o[3..4] == ":" => format!("UTC{o}"),
        _ => String::new(),
    };
    format!("{day} {time} {zone}").trim_end().to_owned()
}
fn name_of(value: &Value) -> String {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| clean(s, 100))
        .unwrap_or_default()
}
/// The text of an Atlassian document (the format of descriptions and comments): its text
/// nodes, a line break after each block. Marks, media and layout are not text.
fn document_text(node: &Value, out: &mut String, depth: usize) {
    if depth > 24 {
        return;
    }
    match node["type"].as_str().unwrap_or("") {
        "text" => out.push_str(node["text"].as_str().unwrap_or("")),
        "hardBreak" => out.push('\n'),
        "mention" => out.push_str(node["attrs"]["text"].as_str().unwrap_or("@quelqu’un")),
        "emoji" => out.push_str(
            node["attrs"]["text"]
                .as_str()
                .or_else(|| node["attrs"]["shortName"].as_str())
                .unwrap_or(""),
        ),
        "inlineCard" => out.push_str(node["attrs"]["url"].as_str().unwrap_or("")),
        kind => {
            for child in node["content"].as_array().into_iter().flatten() {
                document_text(child, out, depth + 1);
            }
            let block = matches!(
                kind,
                "paragraph" | "heading" | "listItem" | "blockquote" | "codeBlock" | "tableRow"
            );
            if block && !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
}
/// A description or a comment as the bubble shows it: the document's text, or the plain
/// string an older site answers.
fn body_text(body: &Value) -> String {
    let mut text = String::new();
    match body {
        Value::String(plain) => text.push_str(plain),
        Value::Object(_) => document_text(body, &mut text, 0),
        _ => {}
    }
    text.trim()
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .take(EXCERPT)
        .collect()
}
/// The document Jira takes as a comment: one paragraph per non-empty line.
pub(super) fn comment_document(text: &str) -> Value {
    let paragraphs: Vec<Value> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| json!({"type":"paragraph","content":[{"type":"text","text":line}]}))
        .collect();
    json!({"type":"doc","version":1,"content":paragraphs})
}

pub(in super::super) fn call(
    ctx: &Context<'_>,
    name: &str,
    arguments: &Value,
) -> grokbot_engine::contracts::ToolOutcome {
    let outcome = match name {
        ISSUES_TOOL => list_issues(ctx, arguments),
        _ => get_issue(ctx, arguments),
    };
    match outcome {
        Ok(output) => grokbot_engine::contracts::ToolOutcome::Done { output },
        Err(error) => grokbot_engine::contracts::ToolOutcome::Failed {
            reason: error.message,
        },
    }
}
/// The live account, when it may be read by this bot.
fn readable(ctx: &Context<'_>) -> Result<(Account, Access), ApiError> {
    let account = match connected(ctx)? {
        Some(account) if account.enabled => account,
        _ => return Err(invalid(&unavailable(ctx))),
    };
    let access = token_of(ctx, &account)?;
    Ok((account, access))
}
fn rest(access: &Access, path: &str) -> Result<String, ApiError> {
    site_api(&access.site.cloud)
        .map(|root| format!("{root}{path}"))
        .map_err(|_| unreachable_jira())
}
fn list_issues(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    let project = arguments["project"]
        .as_str()
        .map(|p| p.trim().to_uppercase())
        .filter(|p| is_project_key(p))
        .ok_or_else(|| invalid("Indiquez la clé du projet Jira à lire, par exemple ENG."))?;
    let limit = match arguments.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=MAX_LIMIT).contains(n))
            .ok_or_else(|| invalid("Le nombre d’issues à lire doit être compris entre 1 et 50."))?,
    };
    let (account, access) = readable(ctx)?;
    let reply = request(
        "POST",
        &rest(&access, "/search/jql")?,
        &access.token,
        Some(json!({
            // Quoted: a key such as `IN` or `OR` is a JQL word. The key is validated, so no quote can ride in it.
            "jql": format!("project = \"{project}\" ORDER BY updated DESC"),
            "maxResults": limit,
            "fields": ["summary", "status", "assignee", "updated"],
        })),
    )
    .map_err(|_| unreachable_jira())?;
    // A project the account does not see is a JQL error, not a missing page.
    if reply.status == 400 {
        return Err(invalid(&format!(
            "Jira ne trouve pas le projet {project} parmi ceux que ce compte voit."
        )));
    }
    if reply.status != 200 || !reply.body["issues"].is_array() {
        return Err(refusal(
            ctx,
            &account,
            &reply,
            &format!("le projet {project}"),
        ));
    }
    let shown: Vec<Value> = reply.body["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            let key = name_of(&i["key"]);
            json!({
                "cle": key,
                "resume": name_of(&i["fields"]["summary"]),
                "etat": name_of(&i["fields"]["status"]["name"]),
                "assigne": name_of(&i["fields"]["assignee"]["displayName"]),
                "modifiee": when(i["fields"]["updated"].as_str().unwrap_or("")),
                "url": format!("{}/browse/{key}", access.site.url),
            })
        })
        .collect();
    let more = reply.body["isLast"] == false || reply.body["nextPageToken"].is_string();
    let mut lines = vec![format!(
        "Jira · compte {} · projet {project} · {} issue(s), les plus récemment modifiées d’abord :",
        clean(&account.name, 100),
        shown.len()
    )];
    lines.extend(shown.iter().map(|i| {
        let field = |key: &str| i[key].as_str().unwrap_or("");
        format!(
            "{} · {} · état : {} · assigné : {} · modifiée le {} · {}",
            field("cle"),
            field("resume"),
            field("etat"),
            if field("assigne").is_empty() {
                "personne"
            } else {
                field("assigne")
            },
            field("modifiee"),
            field("url")
        )
    }));
    if shown.is_empty() {
        lines.push("Aucune issue à afficher dans ce projet.".into());
    }
    if more {
        lines.push("Il y a d’autres issues dans ce projet.".into());
    }
    Ok(json!({
        "compte": clean(&account.name, 100),
        "projet": project,
        "issues": shown,
        "autres_issues": more,
        "texte": lines.join("\n"),
    }))
}
fn get_issue(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    let wanted = arguments["issue"]
        .as_str()
        .map(|i| i.trim().to_uppercase())
        .filter(|i| is_issue_key(i))
        .ok_or_else(|| invalid("Indiquez la clé de l’issue Jira, par exemple ENG-12."))?;
    let (account, access) = readable(ctx)?;
    let reply = request(
        "GET",
        &rest(
            &access,
            &format!("/issue/{wanted}?fields=summary,status,assignee,description,created,updated,project,comment"),
        )?,
        &access.token,
        None,
    )
    .map_err(|_| unreachable_jira())?;
    if reply.status != 200 || !reply.body["fields"].is_object() {
        return Err(refusal(ctx, &account, &reply, &format!("l’issue {wanted}")));
    }
    let fields = &reply.body["fields"];
    let all = fields["comment"]["comments"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let comments: Vec<Value> = all
        .iter()
        .take(COMMENTS)
        .map(|c| {
            json!({
                "auteur": name_of(&c["author"]["displayName"]),
                "date": when(c["created"].as_str().unwrap_or("")),
                "texte": body_text(&c["body"]),
            })
        })
        .collect();
    let total = fields["comment"]["total"]
        .as_u64()
        .map_or(all.len(), |n| n as usize);
    let description = body_text(&fields["description"]);
    let (key, summary, state, assignee) = (
        name_of(&reply.body["key"]),
        name_of(&fields["summary"]),
        name_of(&fields["status"]["name"]),
        name_of(&fields["assignee"]["displayName"]),
    );
    let url = format!("{}/browse/{key}", access.site.url);
    let mut lines = vec![
        format!(
            "Jira · compte {} · issue {key} · projet {} :",
            clean(&account.name, 100),
            name_of(&fields["project"]["key"])
        ),
        format!("Résumé : {summary}"),
        format!("État : {state}"),
        format!(
            "Assigné : {}",
            if assignee.is_empty() {
                "personne"
            } else {
                &assignee
            }
        ),
        format!(
            "Créée le {} · modifiée le {}",
            when(fields["created"].as_str().unwrap_or("")),
            when(fields["updated"].as_str().unwrap_or(""))
        ),
        format!("Adresse : {url}"),
    ];
    if !description.is_empty() {
        lines.push(format!("Description : {description}"));
    }
    lines.push(if total > comments.len() {
        format!("{} commentaire(s) sur {total} :", comments.len())
    } else {
        format!("{} commentaire(s) :", comments.len())
    });
    lines.extend(comments.iter().map(|c| {
        format!(
            "[{}] {} : {}",
            c["date"].as_str().unwrap_or(""),
            c["auteur"].as_str().unwrap_or(""),
            c["texte"].as_str().unwrap_or("")
        )
    }));
    Ok(json!({
        "compte": clean(&account.name, 100),
        "cle": key,
        "resume": summary,
        "etat": state,
        "assigne": assignee,
        "projet": name_of(&fields["project"]["key"]),
        "description": description,
        "commentaires": comments,
        "url": url,
        "texte": lines.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_and_issue_keys_are_strict_enough_for_a_jql_string() {
        assert!(is_project_key("ENG") && is_project_key("PROD_2") && is_project_key("AB"));
        for bad in [
            "E",
            "eng",
            "1NG",
            "ENG-1",
            "ENG\"",
            "ENG OR",
            "ABCDEFGHIJK",
            "",
        ] {
            assert!(!is_project_key(bad), "{bad}");
        }
        assert!(is_issue_key("ENG-12") && is_issue_key("PROD_2-1"));
        for bad in [
            "ENG",
            "ENG-",
            "-12",
            "ENG-12a",
            "eng-12",
            "ENG 12",
            "ENG-12345678",
        ] {
            assert!(!is_issue_key(bad), "{bad}");
        }
    }
    #[test]
    fn timestamps_keep_their_offset() {
        assert_eq!(when("2025-10-09T08:55:12.000+0000"), "2025-10-09 08:55 UTC");
        assert_eq!(when("2025-10-09T08:55:12.000Z"), "2025-10-09 08:55 UTC");
        assert_eq!(
            when("2025-10-09T08:55:12.000-0400"),
            "2025-10-09 08:55 UTC-04:00"
        );
        assert_eq!(when(""), "");
    }
    #[test]
    fn a_document_reads_as_its_text_and_a_comment_is_one_paragraph_per_line() {
        let document = json!({"type":"doc","version":1,"content":[
            {"type":"paragraph","content":[{"type":"text","text":"Bonjour "},{"type":"mention","attrs":{"id":"1","text":"@Alice"}}]},
            {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"un"}]}]}]}
        ]});
        assert_eq!(body_text(&document), "Bonjour @Alice\nun");
        assert_eq!(body_text(&Value::Null), "");
        let sent = comment_document("a\n\n  \nb  ");
        assert_eq!(sent["content"].as_array().unwrap().len(), 2);
        assert_eq!(sent["content"][1]["content"][0]["text"], "b");
        assert_eq!(body_text(&sent), "a\nb");
    }
    #[test]
    fn a_site_is_bound_by_an_identifier_and_an_https_address() {
        assert!(is_cloud_id("1324a887-451b-4da2-b2e6-c0a0e1a3f8e0"));
        assert!(!is_cloud_id("../x") && !is_cloud_id(""));
        assert!(is_site_url("https://atelier.atlassian.net"));
        assert!(!is_site_url("http://atelier.atlassian.net") && !is_site_url("https://a b"));
    }
}
