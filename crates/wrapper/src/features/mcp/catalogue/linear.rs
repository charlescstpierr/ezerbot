//! Linear connection and its closed read adapter. The token comes from the public client of
//! Ezerbot's Linear app (PKCE, loopback return): no `client_secret` is read, sent or stored,
//! and a keychain that refuses the write keeps nothing. The three reading tools call the
//! Linear GraphQL API with that token and need no approval card; creating an issue and
//! commenting on one only ever run from a validated draft card (see `draft`).
use super::slack::{agent, clean, reply_of, Reply};
use super::*;

pub(in super::super) mod draft;

const TEAMS_TOOL: &str = "linear_list_teams";
const ISSUES_TOOL: &str = "linear_list_issues";
const ISSUE_TOOL: &str = "linear_get_issue";
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 50;
/// Characters of one description or comment shown in the bubble.
const EXCERPT: usize = 3000;
/// Comments read with one issue.
const COMMENTS: usize = 20;
/// Pages of 100 teams looked through: 2 000 teams, far beyond any workspace.
const TEAM_PAGES: usize = 20;
/// The loopback port Linear's app declares: its callback URLs are registered, not negotiated.
pub(super) const REDIRECT_PORT: u16 = 52188;
const NOT_CONNECTED: &str =
    "Linear n’est pas connecté. Connectez-le dans Paramètres › Connecteurs, puis réessayez.";
const DISCONNECTED: &str =
    "Linear est déconnecté. Reconnectez Linear dans Paramètres › Connecteurs, puis réessayez.";
const ENABLE: &str =
    "Linear n’est pas activé pour ce bot. Activez-le dans Paramètres › Connecteurs, puis réessayez.";
pub(super) const RECONNECT: &str =
    "La connexion Linear a expiré ou a été révoquée. Reconnectez Linear dans Paramètres › Connecteurs.";

/// The reading tools of the closed list; the two draft tools are in `draft`.
pub(in super::super) fn is_read(name: &str) -> bool {
    matches!(name, TEAMS_TOOL | ISSUES_TOOL | ISSUE_TOOL)
}

/// Public value of the build, replaceable by the environment for tests and the judge.
pub(super) fn client_id() -> Option<String> {
    let value = std::env::var("EZERBOT_LINEAR_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("EZERBOT_LINEAR_CLIENT_ID").map(str::to_owned))?;
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .then(|| value.to_owned())
}
pub(super) const NOT_CONFIGURED: &str = "Le client Linear d’Ezerbot n’est pas configuré dans cette version. Il n’y a ni identifiant ni secret à saisir : la connexion reste indisponible tant que le client n’est pas préparé.";
pub(super) fn not_configured() -> ApiError {
    invalid(NOT_CONFIGURED)
}
/// Tokens are written once, after Linear named the account; a refused write keeps nothing.
pub(super) fn write_tokens(
    ctx: &Context<'_>,
    server: &Server,
    env: &HashMap<String, String>,
) -> Result<(), ApiError> {
    save(ctx, &server.id, env).map_err(|_| {
        let locked = crate::features::forms::vault(ctx)
            .is_ok_and(|vault| vault.storage_status().backend == "file_fallback");
        invalid(if locked {
            "Le trousseau macOS est verrouillé ou inaccessible : la connexion Linear n’a pas été conservée et aucun jeton n’a été écrit. Déverrouillez-le, puis reconnectez Linear."
        } else {
            "Impossible de conserver la connexion Linear dans le coffre local. Vérifiez le stockage des secrets, puis reconnectez Linear."
        })
    })
}

fn issues_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "team":{"type":"string","description":"Clé de l’équipe (par exemple ENG) ou son nom."},
        "limit":{"type":"integer","minimum":1,"maximum":MAX_LIMIT,"description":"Nombre d’issues à lire, les plus récemment modifiées d’abord ; 20 par défaut."}
    },"required":["team"]})
}
fn issue_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "issue":{"type":"string","description":"Identifiant de l’issue, par exemple ENG-12."}
    },"required":["issue"]})
}
fn teams_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{}})
}
const TEAMS_SUMMARY: &str = "Lister les équipes Linear visibles avec le compte connecté.";
const ISSUES_SUMMARY: &str =
    "Lister les issues d’une équipe Linear, en lecture seule, avec le compte connecté.";
const ISSUE_SUMMARY: &str =
    "Lire une issue Linear par son identifiant, avec ses commentaires, en lecture seule.";
fn read_specs() -> Vec<grokbot_engine::contracts::ToolSpec> {
    let read = |name: &str, description: &str, schema| grokbot_engine::contracts::ToolSpec {
        name: name.into(),
        description: description.into(),
        schema,
        requires_approval: false,
    };
    vec![
        read(TEAMS_TOOL, "Linear · lister les équipes visibles (clé et nom), lecture seule, avec le compte Linear connecté dans Ezerbot. Reprenez le champ `texte` du résultat tel quel dans votre réponse.", teams_schema()),
        read(ISSUES_TOOL, "Linear · lister les issues d’une équipe (identifiant, titre, état, assigné), les plus récemment modifiées d’abord, lecture seule. Reprenez le champ `texte` du résultat tel quel dans votre réponse.", issues_schema()),
        read(ISSUE_TOOL, "Linear · lire une issue par son identifiant (par exemple ENG-12) avec sa description et ses commentaires, lecture seule. Reprenez le champ `texte` du résultat tel quel dans votre réponse.", issue_schema()),
    ]
}
/// Model-facing tools, advertised only while Linear is connected and enabled for the bot.
pub(in super::super) fn tool_specs() -> Vec<grokbot_engine::contracts::ToolSpec> {
    let mut specs = read_specs();
    specs.extend(draft::tool_specs());
    specs
}
/// The connection has no process: it is native, and its tools are this fixed list.
pub(super) fn attach(ctx: &Context<'_>, server: &Server) -> Result<Value, ApiError> {
    let mut tools = vec![
        json!({"name":"list_teams","description":TEAMS_SUMMARY,"inputSchema":teams_schema()}),
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
        json!({"message":"Linear est connecté. Activez les outils pour un bot : lire les équipes et les issues part sans carte d’approbation ; une issue ou un commentaire n’est publié qu’après votre clic sur sa carte brouillon.","outils":tools}),
    )
}

/// The account of the live connection, and the server that holds its token.
struct Account {
    server: Server,
    /// The name and organization Linear gave at sign-in.
    name: String,
    /// Moves with every sign-in: a token is the one a draft was prepared with only while it holds.
    generation: i64,
    enabled: bool,
}
fn connected(ctx: &Context<'_>) -> Result<Option<Account>, ApiError> {
    let row: Option<(String, Option<String>, i64)> = ctx
        .connection
        .query_row(
            "SELECT server_id,account_email,generation FROM wrapper_connector_catalogue WHERE id='linear' AND phase='connected'",
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
            name: name.unwrap_or_else(|| "Linear".into()),
            generation,
            server,
        }))
}
/// Why the Linear tool is not offered to this bot: not connected, disconnected, or not enabled.
pub(in super::super) fn unavailable(ctx: &Context<'_>) -> String {
    match connected(ctx) {
        Ok(Some(_)) => ENABLE.into(),
        _ => {
            let phase: Option<String> = ctx
                .connection
                .query_row(
                    "SELECT phase FROM wrapper_connector_catalogue WHERE id='linear'",
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

fn unreachable_linear() -> ApiError {
    ApiError::new(
        503,
        "Linear est injoignable pour le moment. Vérifiez la connexion, puis réessayez.",
    )
}
/// One GraphQL call, no redirect, bounded body. Transport failures carry no detail.
fn graphql(access: &str, query: &str, variables: Value) -> Result<Reply, ()> {
    let m = manifest("linear").map_err(|_| ())?;
    reply_of(
        agent()
            .post(&endpoint(m, "api"))
            .set("Authorization", &format!("Bearer {access}"))
            .set("Accept", "application/json")
            .send_json(json!({"query":query,"variables":variables})),
    )
}
/// The account a fresh token belongs to: `Nom · Organisation`, which the card shows.
pub(super) fn identity(access: &str) -> Option<String> {
    let reply = graphql(
        access,
        "query{viewer{name email organization{name}}}",
        json!({}),
    )
    .ok()?;
    let viewer = &reply.body["data"]["viewer"];
    let pick = |value: &Value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 100 && !s.chars().any(char::is_control))
            .map(str::to_owned)
    };
    let who = pick(&viewer["email"]).or_else(|| pick(&viewer["name"]))?;
    Some(match pick(&viewer["organization"]["name"]) {
        Some(organization) => format!("{who} · {organization}"),
        None => who,
    })
}

/// The code Linear gives a GraphQL error: `AUTHENTICATION_ERROR`, `RATELIMITED`, `FORBIDDEN`…
pub(super) fn error_code(reply: &Reply) -> &str {
    reply.body["errors"][0]["extensions"]["code"]
        .as_str()
        .unwrap_or("")
}
/// What Linear's answer means for Charles. A dead token also moves the card to reconnect.
pub(super) fn refusal(ctx: &Context<'_>, server: &Server, reply: &Reply, what: &str) -> ApiError {
    let code = error_code(reply);
    if reply.status == 429 || code == "RATELIMITED" {
        let wait = reply
            .retry_after
            .filter(|s| *s <= 3600)
            .map(|s| format!(" Réessayez dans environ {} s.", s.max(1)))
            .unwrap_or_default();
        return ApiError::new(429, format!("Linear limite le débit des requêtes.{wait}"));
    }
    if reply.status == 401 || code == "AUTHENTICATION_ERROR" {
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        let _ = phase(ctx, &server.id, "reconnect", RECONNECT);
        return invalid(RECONNECT);
    }
    if code == "FORBIDDEN" {
        return invalid(&format!("Linear refuse l’accès à {what} avec ce compte. Vérifiez vos droits dans Linear, ou reconnectez Linear pour accorder de nouveau les autorisations."));
    }
    if reply.status >= 500 {
        return unreachable_linear();
    }
    let code: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(40)
        .collect();
    invalid(&format!(
        "Linear a refusé la requête concernant {what}{}.",
        if code.is_empty() {
            String::new()
        } else {
            format!(" ({code})")
        }
    ))
}
/// The `data` of an answer that carries no error.
fn checked(
    ctx: &Context<'_>,
    server: &Server,
    reply: Reply,
    what: &str,
) -> Result<Value, ApiError> {
    let failed = reply.body["errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty());
    if reply.status == 200 && !failed && reply.body["data"].is_object() {
        Ok(reply.body["data"].clone())
    } else {
        Err(refusal(ctx, server, &reply, what))
    }
}

/// The token of the live connection, read under the sign-in lock: a sign-in replaces it there.
/// The account the read started with must still be the one that is connected and enabled
/// for the bot, so a sign-in in between never lends its token to this read.
fn token_of(ctx: &Context<'_>, account: &Account) -> Result<String, ApiError> {
    super::ensure(ctx, &account.server)?;
    let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
    match connected(ctx)? {
        Some(now)
            if now.enabled && now.generation == account.generation && now.name == account.name => {}
        _ => {
            return Err(invalid(
                "Le compte Linear a changé pendant la lecture : rien n’a été lu. Réessayez.",
            ))
        }
    }
    credentials(ctx, &account.server.id)?
        .and_then(|env| env.get("ACCESS_TOKEN").cloned())
        .ok_or_else(|| invalid(RECONNECT))
}
fn query(
    ctx: &Context<'_>,
    account: &Account,
    access: &str,
    document: &str,
    variables: Value,
    what: &str,
) -> Result<Value, ApiError> {
    let reply = graphql(access, document, variables).map_err(|_| unreachable_linear())?;
    checked(ctx, &account.server, reply, what)
}

struct Team {
    id: String,
    key: String,
    name: String,
}
impl Team {
    fn label(&self) -> String {
        format!("{} ({})", self.key, self.name)
    }
}
/// The teams the account sees, page after page, up to the first one `until` accepts.
fn teams_until(
    ctx: &Context<'_>,
    account: &Account,
    access: &str,
    until: impl Fn(&Value) -> bool,
) -> Result<Vec<Value>, ApiError> {
    let mut teams = Vec::new();
    let mut after = Value::Null;
    for _ in 0..TEAM_PAGES {
        let data = query(
            ctx,
            account,
            access,
            "query($after:String){teams(first:100,after:$after){nodes{id key name} pageInfo{hasNextPage endCursor}}}",
            json!({"after":after}),
            "les équipes",
        )?;
        let page = &data["teams"];
        teams.extend(page["nodes"].as_array().into_iter().flatten().cloned());
        let next = page["pageInfo"]["endCursor"].as_str();
        if teams.iter().any(&until) || page["pageInfo"]["hasNextPage"] != true || next.is_none() {
            break;
        }
        after = json!(next);
    }
    Ok(teams)
}
/// The team a key or a name designates among those the account sees.
fn find_team(
    ctx: &Context<'_>,
    account: &Account,
    access: &str,
    wanted: &str,
) -> Result<Team, ApiError> {
    let wanted_lower = wanted.to_lowercase();
    let named = |t: &Value| {
        [&t["key"], &t["name"]]
            .iter()
            .any(|v| v.as_str().is_some_and(|v| v.to_lowercase() == wanted_lower))
    };
    let teams = teams_until(ctx, account, access, named)?;
    teams
        .iter()
        .find(|t| named(t))
        .and_then(|t| {
            Some(Team {
                id: t["id"].as_str()?.to_owned(),
                key: clean(t["key"].as_str()?, 20),
                name: clean(t["name"].as_str().unwrap_or(""), 100),
            })
        })
        .ok_or_else(|| {
            invalid(&format!(
                "Aucune équipe Linear ne s’appelle {} parmi celles que ce compte voit.",
                clean(wanted, 80)
            ))
        })
}
/// The internal identifier of an issue that its readable one (`ENG-12`) designates.
fn issue_id(
    ctx: &Context<'_>,
    account: &Account,
    access: &str,
    identifier: &str,
) -> Result<String, ApiError> {
    let reply = graphql(
        access,
        "query($id:String!){issue(id:$id){id}}",
        json!({"id":identifier}),
    )
    .map_err(|_| unreachable_linear())?;
    let missing = || {
        invalid(&format!(
            "Linear ne trouve pas l’issue {identifier} avec ce compte."
        ))
    };
    if reply.body["errors"][0]["message"]
        .as_str()
        .is_some_and(|m| m.to_lowercase().contains("not found"))
    {
        return Err(missing());
    }
    let data = checked(
        ctx,
        &account.server,
        reply,
        &format!("l’issue {identifier}"),
    )?;
    data["issue"]["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(missing)
}
/// An issue identifier such as `ENG-12`: the team key, a dash, the number.
pub(super) fn is_issue_identifier(text: &str) -> bool {
    text.split_once('-').is_some_and(|(key, number)| {
        (1..=10).contains(&key.len())
            && key.bytes().all(|b| b.is_ascii_alphanumeric())
            && key.as_bytes()[0].is_ascii_alphabetic()
            && (1..=7).contains(&number.len())
            && number.bytes().all(|b| b.is_ascii_digit())
    })
}
/// `AAAA-MM-JJ HH:MM UTC` from Linear's ISO timestamp (`2025-10-09T08:55:12.000Z`).
fn when(iso: &str) -> String {
    match (iso.get(..10), iso.get(11..16)) {
        (Some(day), Some(time)) => format!("{day} {time} UTC"),
        _ => String::new(),
    }
}
fn name_of(value: &Value) -> String {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| clean(s, 100))
        .unwrap_or_default()
}

pub(in super::super) fn call(
    ctx: &Context<'_>,
    name: &str,
    arguments: &Value,
) -> grokbot_engine::contracts::ToolOutcome {
    let outcome = match name {
        TEAMS_TOOL => list_teams(ctx),
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
fn readable(ctx: &Context<'_>) -> Result<(Account, String), ApiError> {
    let account = match connected(ctx)? {
        Some(account) if account.enabled => account,
        _ => return Err(invalid(&unavailable(ctx))),
    };
    let access = token_of(ctx, &account)?;
    Ok((account, access))
}
fn list_teams(ctx: &Context<'_>) -> Result<Value, ApiError> {
    let (account, access) = readable(ctx)?;
    let teams: Vec<Value> = teams_until(ctx, &account, &access, |_| false)?
        .iter()
        .map(|t| json!({"cle":name_of(&t["key"]),"nom":name_of(&t["name"])}))
        .collect();
    let mut lines = vec![format!(
        "Linear · compte {} · {} équipe(s) :",
        clean(&account.name, 100),
        teams.len()
    )];
    lines.extend(teams.iter().map(|t| {
        format!(
            "{} · {}",
            t["cle"].as_str().unwrap_or(""),
            t["nom"].as_str().unwrap_or("")
        )
    }));
    Ok(json!({"compte":clean(&account.name,100),"equipes":teams,"texte":lines.join("\n")}))
}
fn list_issues(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    let wanted = arguments["team"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty() && t.len() <= 100 && !t.chars().any(char::is_control))
        .ok_or_else(|| invalid("Indiquez la clé ou le nom de l’équipe Linear à lire."))?;
    let limit = match arguments.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=MAX_LIMIT).contains(n))
            .ok_or_else(|| invalid("Le nombre d’issues à lire doit être compris entre 1 et 50."))?,
    };
    let (account, access) = readable(ctx)?;
    let team = find_team(ctx, &account, &access, wanted)?;
    let data = query(
        ctx,
        &account,
        &access,
        "query($id:String!,$n:Int!){team(id:$id){issues(first:$n,orderBy:updatedAt){nodes{identifier title url updatedAt state{name} assignee{name}} pageInfo{hasNextPage}}}}",
        json!({"id":team.id,"n":limit}),
        &format!("l’équipe {}", team.key),
    )?;
    let issues = &data["team"]["issues"];
    let shown: Vec<Value> = issues["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            json!({
                "identifiant": name_of(&i["identifier"]),
                "titre": name_of(&i["title"]),
                "etat": name_of(&i["state"]["name"]),
                "assigne": name_of(&i["assignee"]["name"]),
                "modifiee": when(i["updatedAt"].as_str().unwrap_or("")),
                "url": name_of(&i["url"]),
            })
        })
        .collect();
    let more = issues["pageInfo"]["hasNextPage"] == true;
    let mut lines = vec![format!(
        "Linear · compte {} · équipe {} · {} issue(s), les plus récemment modifiées d’abord :",
        clean(&account.name, 100),
        team.label(),
        shown.len()
    )];
    lines.extend(shown.iter().map(|i| {
        let field = |key: &str| i[key].as_str().unwrap_or("");
        format!(
            "{} · {} · état : {} · assigné : {} · modifiée le {} · {}",
            field("identifiant"),
            field("titre"),
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
        lines.push("Aucune issue à afficher dans cette équipe.".into());
    }
    if more {
        lines.push("Il y a d’autres issues dans cette équipe.".into());
    }
    Ok(json!({
        "compte": clean(&account.name, 100),
        "equipe": team.key,
        "issues": shown,
        "autres_issues": more,
        "texte": lines.join("\n"),
    }))
}
fn get_issue(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    let wanted = arguments["issue"]
        .as_str()
        .map(|i| i.trim().to_uppercase())
        .filter(|i| is_issue_identifier(i))
        .ok_or_else(|| invalid("Indiquez l’identifiant de l’issue Linear, par exemple ENG-12."))?;
    let (account, access) = readable(ctx)?;
    let reply = graphql(
        &access,
        &format!("query($id:String!){{issue(id:$id){{identifier title description url createdAt updatedAt state{{name}} assignee{{name}} team{{key name}} comments(first:{COMMENTS}){{nodes{{body createdAt user{{name}}}}}}}}}}"),
        json!({"id":wanted}),
    )
    .map_err(|_| unreachable_linear())?;
    let missing = || {
        invalid(&format!(
            "Linear ne trouve pas l’issue {wanted} avec ce compte."
        ))
    };
    if reply.body["errors"][0]["message"]
        .as_str()
        .is_some_and(|m| m.to_lowercase().contains("not found"))
    {
        return Err(missing());
    }
    let data = checked(ctx, &account.server, reply, &format!("l’issue {wanted}"))?;
    let issue = &data["issue"];
    if !issue.is_object() {
        return Err(missing());
    }
    let comments: Vec<Value> = issue["comments"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            json!({
                "auteur": name_of(&c["user"]["name"]),
                "date": when(c["createdAt"].as_str().unwrap_or("")),
                "texte": clean(c["body"].as_str().unwrap_or(""), EXCERPT),
            })
        })
        .collect();
    let description = clean(issue["description"].as_str().unwrap_or(""), EXCERPT);
    let (identifier, title, state, assignee, url) = (
        name_of(&issue["identifier"]),
        name_of(&issue["title"]),
        name_of(&issue["state"]["name"]),
        name_of(&issue["assignee"]["name"]),
        name_of(&issue["url"]),
    );
    let mut lines = vec![
        format!(
            "Linear · compte {} · issue {identifier} · équipe {} :",
            clean(&account.name, 100),
            name_of(&issue["team"]["key"])
        ),
        format!("Titre : {title}"),
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
            when(issue["createdAt"].as_str().unwrap_or("")),
            when(issue["updatedAt"].as_str().unwrap_or(""))
        ),
        format!("Adresse : {url}"),
    ];
    if !description.is_empty() {
        lines.push(format!("Description : {description}"));
    }
    lines.push(format!("{} commentaire(s) :", comments.len()));
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
        "identifiant": identifier,
        "titre": title,
        "etat": state,
        "assigne": assignee,
        "equipe": name_of(&issue["team"]["key"]),
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
    fn issue_identifiers_are_a_team_key_and_a_number() {
        assert!(is_issue_identifier("ENG-12"));
        assert!(is_issue_identifier("PROD2-1"));
        assert!(!is_issue_identifier("ENG"));
        assert!(!is_issue_identifier("ENG-"));
        assert!(!is_issue_identifier("-12"));
        assert!(!is_issue_identifier("ENG-12a"));
        assert!(!is_issue_identifier("1NG-12"));
        assert!(!is_issue_identifier("ENG 12"));
    }
    #[test]
    fn timestamps_become_utc_dates() {
        assert_eq!(when("2025-10-09T08:55:12.000Z"), "2025-10-09 08:55 UTC");
        assert_eq!(when(""), "");
    }
}
