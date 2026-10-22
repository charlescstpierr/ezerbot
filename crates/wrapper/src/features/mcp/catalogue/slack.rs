//! Slack connection and its closed read adapter. The user token comes from the public
//! client of Ezerbot's Slack app (PKCE, loopback return): no `client_secret` is read,
//! sent or stored, and a keychain that refuses the write keeps nothing. The one tool,
//! reading a channel, calls the Slack Web API with that token and needs no approval card;
//! the other, posting a message, only ever runs from a validated draft card (see `post`).
use super::*;
use std::{io::Read, time::Duration};

pub(in super::super) mod post;

pub(in super::super) const TOOL: &str = "slack_read_channel";
const SHORT: &str = "read_channel";
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 50;
/// Pages of channels looked through to find a name, 200 channels each.
const CHANNEL_PAGES: usize = 10;
/// People named in one reading; the rest keep their Slack identifier.
const PEOPLE: usize = 25;
/// Characters of one message shown in the bubble.
const EXCERPT: usize = 1500;
const NOT_CONNECTED: &str =
    "Slack n’est pas connecté. Connectez-le dans Paramètres › Connecteurs, puis réessayez.";
const DISCONNECTED: &str =
    "Slack est déconnecté. Reconnectez Slack dans Paramètres › Connecteurs, puis réessayez.";
const ENABLE: &str =
    "Slack n’est pas activé pour ce bot. Activez-le dans Paramètres › Connecteurs, puis réessayez.";
const MISSING_SCOPE: &str = "Slack n’a pas accordé toutes les autorisations nécessaires à Ezerbot. Reconnectez Slack dans Paramètres › Connecteurs pour les accorder de nouveau.";
const RECONNECT: &str =
    "La connexion Slack a expiré ou a été révoquée. Reconnectez Slack dans Paramètres › Connecteurs.";

/// Public value of the build, replaceable by the environment for tests and the judge.
pub(super) fn client_id() -> Option<String> {
    let value = std::env::var("EZERBOT_SLACK_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("EZERBOT_SLACK_CLIENT_ID").map(str::to_owned))?;
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .then(|| value.to_owned())
}
pub(super) const NOT_CONFIGURED: &str = "Le client Slack d’Ezerbot n’est pas configuré dans cette version. Il n’y a ni identifiant ni secret à saisir : la connexion reste indisponible tant que le client n’est pas préparé.";
pub(super) fn not_configured() -> ApiError {
    invalid(NOT_CONFIGURED)
}
/// Tokens are written once, after Slack named the workspace; a refused write keeps nothing.
pub(super) fn write_tokens(
    ctx: &Context<'_>,
    server: &Server,
    env: &HashMap<String, String>,
) -> Result<(), ApiError> {
    save(ctx, &server.id, env).map_err(|_| {
        let locked = crate::features::forms::vault(ctx)
            .is_ok_and(|vault| vault.storage_status().backend == "file_fallback");
        invalid(if locked {
            "Le trousseau macOS est verrouillé ou inaccessible : la connexion Slack n’a pas été conservée et aucun jeton n’a été écrit. Déverrouillez-le, puis reconnectez Slack."
        } else {
            "Impossible de conserver la connexion Slack dans le coffre local. Vérifiez le stockage des secrets, puis reconnectez Slack."
        })
    })
}

fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "channel":{"type":"string","description":"Nom du canal, avec ou sans #, par exemple general ; ou son identifiant Slack (C…, G…)."},
        "limit":{"type":"integer","minimum":1,"maximum":MAX_LIMIT,"description":"Nombre de derniers messages à lire ; 20 par défaut."}
    },"required":["channel"]})
}
const SUMMARY: &str =
    "Lire les derniers messages d’un canal Slack, en lecture seule, avec le compte Slack connecté.";
/// Model-facing tools, advertised only while Slack is connected and enabled for the bot.
pub(in super::super) fn tool_specs() -> Vec<grokbot_engine::contracts::ToolSpec> {
    vec![read_spec(), post::tool_spec()]
}
fn read_spec() -> grokbot_engine::contracts::ToolSpec {
    grokbot_engine::contracts::ToolSpec {
        name: TOOL.into(),
        description: "Slack · lire les derniers messages d’un canal nommé (lecture seule, compte Slack connecté dans Ezerbot). Reprenez le champ `texte` du résultat tel quel dans votre réponse : il nomme l’espace, le canal, l’auteur et la date de chaque message.".into(),
        schema: schema(),
        requires_approval: false,
    }
}
/// The connection has no process: it is native, and its tools are this fixed list.
pub(super) fn attach(ctx: &Context<'_>, server: &Server) -> Result<Value, ApiError> {
    let tools = json!([
        {"name":SHORT,"description":SUMMARY,"inputSchema":schema()},
        post::tool_definition(),
    ]);
    ctx.connection.execute(
        "UPDATE wrapper_mcp_servers SET tools_json=?2,error=NULL WHERE id=?1",
        params![server.id, tools.to_string()],
    )?;
    phase(
        ctx,
        &server.id,
        "connected",
        "Connecté. Activez l’outil pour les bots de votre choix.",
    )?;
    Ok(
        json!({"message":"Slack est connecté. Activez l’outil pour un bot : lire un canal part sans carte d’approbation ; un message n’est publié qu’après votre clic sur sa carte brouillon.","outils":tools}),
    )
}

/// The workspace of the live connection, and the server that holds its token.
struct Account {
    server: Server,
    workspace: String,
    enabled: bool,
}
fn connected(ctx: &Context<'_>) -> Result<Option<Account>, ApiError> {
    let row: Option<(String, Option<String>)> = ctx
        .connection
        .query_row(
            "SELECT server_id,account_email FROM wrapper_connector_catalogue WHERE id='slack' AND phase='connected'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((server_id, workspace)) = row else {
        return Ok(None);
    };
    Ok(super::super::servers(ctx.connection)?
        .into_iter()
        .find(|s| s.id == server_id)
        .map(|server| Account {
            enabled: ctx
                .bot_id
                .is_none_or(|bot| super::super::enabled(ctx.connection, &server.id, bot)),
            workspace: workspace.unwrap_or_else(|| "Slack".into()),
            server,
        }))
}
/// Why the Slack tool is not offered to this bot: not connected, disconnected, or not enabled.
pub(in super::super) fn unavailable(ctx: &Context<'_>) -> String {
    match connected(ctx) {
        Ok(Some(_)) => ENABLE.into(),
        _ => {
            let phase: Option<String> = ctx
                .connection
                .query_row(
                    "SELECT phase FROM wrapper_connector_catalogue WHERE id='slack'",
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

struct Reply {
    status: u16,
    retry_after: Option<u64>,
    body: Value,
}
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(12))
        .build()
}
/// One Web API read, no redirect, bounded body. Transport failures carry no detail.
fn get(m: Manifest, access: &str, method: &str, query: &[(&str, &str)]) -> Result<Reply, ApiError> {
    let mut url = url::Url::parse(&format!("{}/{method}", endpoint(m, "api")))
        .map_err(|_| unreachable_slack())?;
    url.query_pairs_mut().extend_pairs(query);
    reply_of(
        agent()
            .get(url.as_str())
            .set("Authorization", &format!("Bearer {access}"))
            .set("Accept", "application/json")
            .call(),
    )
    .map_err(|_| unreachable_slack())
}
/// What Slack answered, whatever the status: no redirect, bounded body. A transport failure
/// or an unreadable body carries no detail.
fn reply_of(outcome: Result<ureq::Response, ureq::Error>) -> Result<Reply, ()> {
    let response = match outcome {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(_) => return Err(()),
    };
    let status = response.status();
    let retry_after = response.header("retry-after").and_then(|v| v.parse().ok());
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
        retry_after,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}
/// The workspace a fresh token belongs to, when the exchange named only its identifier.
pub(super) fn workspace_of(m: Manifest, access: &str) -> Option<String> {
    let reply = get(m, access, "auth.test", &[]).ok()?;
    reply.body["team"]
        .as_str()
        .map(str::trim)
        .filter(|name| reply.body["ok"] == true && !name.is_empty() && name.len() <= 100)
        .filter(|name| !name.chars().any(char::is_control))
        .map(str::to_owned)
}
fn unreachable_slack() -> ApiError {
    ApiError::new(
        503,
        "Slack est injoignable pour le moment. Vérifiez la connexion, puis réessayez.",
    )
}
fn clean(text: &str, limit: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(limit)
        .collect()
}

/// What Slack's error code means for Charles. A dead token also moves the card to reconnect.
fn refusal(ctx: &Context<'_>, server: &Server, reply: &Reply, channel: &str) -> ApiError {
    if reply.status == 429 || reply.body["error"] == "ratelimited" {
        let wait = reply
            .retry_after
            .filter(|s| *s <= 3600)
            .map(|s| format!(" Réessayez dans environ {} s.", s.max(1)))
            .unwrap_or_default();
        return ApiError::new(429, format!("Slack limite le débit des requêtes.{wait}"));
    }
    let code = reply.body["error"].as_str().unwrap_or("");
    match code {
        "invalid_auth" | "not_authed" | "token_revoked" | "token_expired" | "account_inactive" => {
            let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
            let _ = phase(ctx, &server.id, "reconnect", RECONNECT);
            invalid(RECONNECT)
        }
        "missing_scope" => {
            // A token that lacks a permission cannot be reused or renewed into one that has
            // it: it goes, so that reconnecting asks Slack for consent again.
            let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(vault) = crate::features::forms::vault(ctx) {
                let _ = vault.remove(&secret_name(&server.id));
            }
            let _ = ctx.connection.execute(
                "UPDATE wrapper_connector_catalogue SET expires_at=0 WHERE server_id=?1",
                [&server.id],
            );
            let _ = phase(ctx, &server.id, "reconnect", MISSING_SCOPE);
            invalid(MISSING_SCOPE)
        }
        "not_in_channel" => invalid(&format!(
            "Vous n’êtes pas membre du canal {channel}. Rejoignez-le dans Slack, puis réessayez."
        )),
        "channel_not_found" => invalid(&format!(
            "Slack ne trouve pas le canal {channel} avec ce compte."
        )),
        _ if reply.status >= 500 => unreachable_slack(),
        _ => {
            let code: String = code
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(40)
                .collect();
            invalid(&format!("Slack a refusé la lecture ({code})."))
        }
    }
}
fn checked(
    ctx: &Context<'_>,
    server: &Server,
    reply: Reply,
    channel: &str,
) -> Result<Value, ApiError> {
    if reply.status == 200 && reply.body["ok"] == true {
        Ok(reply.body)
    } else {
        Err(refusal(ctx, server, &reply, channel))
    }
}

/// A Slack channel identifier: public (`C…`) or private (`G…`).
fn is_identifier(text: &str) -> bool {
    (9..=16).contains(&text.len())
        && matches!(text.as_bytes()[0], b'C' | b'G')
        && text
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}
fn channel_id(
    ctx: &Context<'_>,
    account: &Account,
    m: Manifest,
    access: &str,
    wanted: &str,
) -> Result<String, ApiError> {
    if is_identifier(wanted) {
        return Ok(wanted.to_owned());
    }
    let name = wanted.to_lowercase();
    let mut cursor = String::new();
    for _ in 0..CHANNEL_PAGES {
        let mut query = vec![
            ("types", "public_channel,private_channel"),
            ("exclude_archived", "true"),
            ("limit", "200"),
        ];
        if !cursor.is_empty() {
            query.push(("cursor", &cursor));
        }
        let page = checked(
            ctx,
            &account.server,
            get(m, access, "conversations.list", &query)?,
            wanted,
        )?;
        let found = page["channels"].as_array().and_then(|channels| {
            channels
                .iter()
                .find(|c| c["name"].as_str().is_some_and(|n| n.to_lowercase() == name))
        });
        if let Some(id) = found.and_then(|c| c["id"].as_str()) {
            return Ok(id.to_owned());
        }
        cursor = page["response_metadata"]["next_cursor"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        if cursor.is_empty() {
            break;
        }
    }
    Err(invalid(&format!(
        "Aucun canal Slack ne s’appelle {} parmi ceux que ce compte voit.",
        clean(wanted, 80)
    )))
}

/// `AAAA-MM-JJ HH:MM UTC` from a Slack timestamp (`1760000100.000200`).
fn when(ts: &str) -> String {
    let seconds: i64 = ts
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// Names of the people a reading mentions, looked up once each.
struct People {
    names: HashMap<String, String>,
    lookups: usize,
}
impl People {
    fn name(
        &mut self,
        ctx: &Context<'_>,
        account: &Account,
        m: Manifest,
        access: &str,
        id: &str,
    ) -> String {
        if let Some(known) = self.names.get(id) {
            return known.clone();
        }
        let mut shown = id.to_owned();
        if self.lookups < PEOPLE {
            self.lookups += 1;
            if let Ok(body) = get(m, access, "users.info", &[("user", id)])
                .and_then(|reply| checked(ctx, &account.server, reply, id))
            {
                let user = &body["user"];
                let pick = |value: &Value| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| clean(s, 100))
                };
                if let Some(name) = pick(&user["real_name"])
                    .or_else(|| pick(&user["profile"]["display_name"]))
                    .or_else(|| pick(&user["name"]))
                {
                    shown = name;
                }
            }
        }
        self.names.insert(id.to_owned(), shown.clone());
        shown
    }
}
/// Mentions and channel links as words, and Slack's three escapes undone.
fn plain(
    text: &str,
    ctx: &Context<'_>,
    account: &Account,
    m: Manifest,
    access: &str,
    people: &mut People,
) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            rest = &rest[start..];
            break;
        };
        let inner = &rest[start + 1..start + end];
        let (target, label) = inner.split_once('|').unwrap_or((inner, ""));
        if let Some(id) = target.strip_prefix('@') {
            let shown = if label.is_empty() {
                people.name(ctx, account, m, access, id)
            } else {
                label.to_owned()
            };
            out.push('@');
            out.push_str(&shown);
        } else if let Some(channel) = target.strip_prefix('#') {
            out.push('#');
            out.push_str(if label.is_empty() { channel } else { label });
        } else if !label.is_empty() {
            out.push_str(&format!("{label} ({target})"));
        } else {
            out.push_str(target);
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

pub(in super::super) fn call(
    ctx: &Context<'_>,
    arguments: &Value,
) -> grokbot_engine::contracts::ToolOutcome {
    match read_channel(ctx, arguments) {
        Ok(output) => grokbot_engine::contracts::ToolOutcome::Done { output },
        Err(error) => grokbot_engine::contracts::ToolOutcome::Failed {
            reason: error.message,
        },
    }
}
fn read_channel(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    let wanted = arguments["channel"]
        .as_str()
        .map(|c| c.trim().trim_start_matches('#').trim())
        .filter(|c| !c.is_empty() && c.len() <= 100 && !c.chars().any(char::is_control))
        .ok_or_else(|| invalid("Indiquez le nom du canal Slack à lire."))?;
    let limit = match arguments.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=MAX_LIMIT).contains(n))
            .ok_or_else(|| {
                invalid("Le nombre de messages à lire doit être compris entre 1 et 50.")
            })?,
    };
    let account = match connected(ctx)? {
        Some(account) if account.enabled => account,
        _ => return Err(invalid(&unavailable(ctx))),
    };
    let m = manifest("slack")?;
    super::ensure(ctx, &account.server)?;
    // A sign-in replaces the token under this lock: the read below keeps the one it took.
    let access = {
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        credentials(ctx, &account.server.id)?
            .and_then(|env| env.get("ACCESS_TOKEN").cloned())
            .ok_or_else(|| invalid(RECONNECT))?
    };
    let id = channel_id(ctx, &account, m, &access, wanted)?;
    let label = if is_identifier(wanted) {
        wanted.to_owned()
    } else {
        format!("#{}", clean(wanted, 80))
    };
    let count = limit.to_string();
    let body = checked(
        ctx,
        &account.server,
        get(
            m,
            &access,
            "conversations.history",
            &[("channel", &id), ("limit", &count)],
        )?,
        &label,
    )?;
    let mut people = People {
        names: HashMap::new(),
        lookups: 0,
    };
    // Slack answers newest first; a conversation reads oldest first.
    let mut shown = Vec::new();
    for message in body["messages"].as_array().into_iter().flatten().rev() {
        let subtype = message["subtype"].as_str();
        if subtype.is_some_and(|s| {
            !matches!(
                s,
                "bot_message" | "thread_broadcast" | "file_share" | "me_message"
            )
        }) {
            continue;
        }
        let Some(text) = message["text"].as_str().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let author = match message["user"].as_str() {
            Some(user) => people.name(ctx, &account, m, &access, user),
            None => clean(message["username"].as_str().unwrap_or("Application"), 100),
        };
        shown.push(json!({
            "auteur": author,
            "date": when(message["ts"].as_str().unwrap_or("")),
            "texte": clean(&plain(text, ctx, &account, m, &access, &mut people), EXCERPT),
        }));
    }
    let more = body["has_more"] == true;
    let mut lines = vec![format!(
        "Slack · espace {} · {label} · {} dernier(s) message(s), du plus ancien au plus récent :",
        clean(&account.workspace, 100),
        shown.len()
    )];
    lines.extend(shown.iter().map(|s| {
        format!(
            "[{}] {} : {}",
            s["date"].as_str().unwrap_or(""),
            s["auteur"].as_str().unwrap_or(""),
            s["texte"].as_str().unwrap_or("")
        )
    }));
    if shown.is_empty() {
        lines.push("Aucun message à afficher dans ce canal.".into());
    }
    if more {
        lines.push("Il y a d’autres messages plus anciens dans ce canal.".into());
    }
    Ok(json!({
        "compte": clean(&account.workspace, 100),
        "canal": label.trim_start_matches('#'),
        "messages": shown,
        "autres_messages": more,
        "texte": lines.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamps_become_utc_dates() {
        assert_eq!(when("1760000100.000200"), "2025-10-09 08:55 UTC");
        assert_eq!(when("0.0"), "1970-01-01 00:00 UTC");
        assert_eq!(when("951782400.000000"), "2000-02-29 00:00 UTC");
    }
    #[test]
    fn identifiers_are_told_from_names() {
        assert!(is_identifier("C0GENERAL"));
        assert!(is_identifier("G01234ABCDE"));
        assert!(!is_identifier("general"));
        assert!(!is_identifier("Cgeneral"));
        assert!(!is_identifier("C1"));
        assert!(
            !is_identifier("D0DIRECT01"),
            "direct messages are out of this reading"
        );
    }
}
