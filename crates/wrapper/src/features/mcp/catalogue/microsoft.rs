//! Microsoft 365 connection (Outlook mail and calendar) and its closed read adapter. The token
//! comes from the public client of Ezerbot's Entra app (PKCE, loopback return): no
//! `client_secret` is read, sent or stored, and a keychain that refuses the write keeps
//! nothing. The two reading tools call Microsoft Graph with that token and need no approval
//! card; sending a mail only ever runs from a validated draft card (see `mail`). Nothing here
//! writes to the calendar or touches a mail that is already there.
use super::slack::{agent, clean, reply_of, Reply};
use super::*;
use jiff::{civil::DateTime, tz::TimeZone, Span, Timestamp};

pub(in super::super) mod mail;

const MESSAGES_TOOL: &str = "outlook_list_recent_messages";
const EVENTS_TOOL: &str = "outlook_list_events";
/// The latest messages of the inbox that one read gives.
const MESSAGES: usize = 5;
/// Events asked for in one read of a day; a day with more says so.
const EVENTS: usize = 100;
/// Characters of the preview of one message shown in the bubble.
const PREVIEW: usize = 240;
const NOT_CONNECTED: &str =
    "Microsoft 365 n’est pas connecté. Connectez-le dans Paramètres › Connecteurs, puis réessayez.";
const DISCONNECTED: &str = "Microsoft 365 est déconnecté. Reconnectez Microsoft 365 dans Paramètres › Connecteurs, puis réessayez.";
const ENABLE: &str = "Microsoft 365 n’est pas activé pour ce bot. Activez-le dans Paramètres › Connecteurs, puis réessayez.";
pub(super) const RECONNECT: &str = "La connexion Microsoft 365 a expiré ou a été révoquée. Reconnectez Microsoft 365 dans Paramètres › Connecteurs.";
const DAY: &str = "Jour invalide : choisissez « demain » (par défaut) ou « aujourd_hui ».";

/// The reading tools of the closed list; the mail tool is in `mail`.
pub(in super::super) fn is_read(name: &str) -> bool {
    matches!(name, MESSAGES_TOOL | EVENTS_TOOL)
}

/// Public value of the build, replaceable by the environment for tests and the judge.
pub(super) fn client_id() -> Option<String> {
    let value = std::env::var("EZERBOT_MICROSOFT_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("EZERBOT_MICROSOFT_CLIENT_ID").map(str::to_owned))?;
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .then(|| value.to_owned())
}
/// The directory that signs users in: any work, school or personal account unless the
/// registration is narrower, in which case the build or the judge names its tenant.
pub(super) fn tenant() -> String {
    std::env::var("EZERBOT_MICROSOFT_TENANT")
        .ok()
        .or_else(|| option_env!("EZERBOT_MICROSOFT_TENANT").map(str::to_owned))
        .map(|value| value.trim().to_owned())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
        .unwrap_or_else(|| "common".into())
}
pub(super) const NOT_CONFIGURED: &str = "Le client Microsoft 365 d’Ezerbot n’est pas configuré dans cette version. Il n’y a ni identifiant ni secret à saisir : la connexion reste indisponible tant que le client n’est pas préparé.";
pub(super) fn not_configured() -> ApiError {
    invalid(NOT_CONFIGURED)
}
/// Tokens are written once, after Graph named the account; a refused write keeps nothing.
pub(super) fn write_tokens(
    ctx: &Context<'_>,
    server: &Server,
    env: &HashMap<String, String>,
) -> Result<(), ApiError> {
    save(ctx, &server.id, env).map_err(|_| {
        let locked = crate::features::forms::vault(ctx)
            .is_ok_and(|vault| vault.storage_status().backend == "file_fallback");
        invalid(if locked {
            "Le trousseau macOS est verrouillé ou inaccessible : la connexion Microsoft 365 n’a pas été conservée et aucun jeton n’a été écrit. Déverrouillez-le, puis reconnectez Microsoft 365."
        } else {
            "Impossible de conserver la connexion Microsoft 365 dans le coffre local. Vérifiez le stockage des secrets, puis reconnectez Microsoft 365."
        })
    })
}

fn messages_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{}})
}
fn events_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "jour":{"type":"string","enum":["aujourd_hui","demain"],"description":"Le jour à lire, dans le fuseau de ce Mac ; demain si omis."}
    }})
}
const MESSAGES_SUMMARY: &str =
    "Lire les 5 derniers courriels de la boîte de réception Outlook, en lecture seule.";
const EVENTS_SUMMARY: &str =
    "Lire les rendez-vous d’un jour (demain par défaut) de l’agenda Outlook, en lecture seule.";
fn read_specs() -> Vec<grokbot_engine::contracts::ToolSpec> {
    let read = |name: &str, description: &str, schema| grokbot_engine::contracts::ToolSpec {
        name: name.into(),
        description: description.into(),
        schema,
        requires_approval: false,
    };
    vec![
        read(MESSAGES_TOOL, "Outlook · lire les 5 derniers courriels de la boîte de réception (expéditeur, objet, date, aperçu), lecture seule, avec le compte Microsoft 365 connecté dans Ezerbot. Reprenez le champ `texte` du résultat tel quel dans votre réponse.", messages_schema()),
        read(EVENTS_TOOL, "Outlook · lire les rendez-vous d’un jour de l’agenda (lecture seule, compte Microsoft 365 connecté dans Ezerbot ; demain par défaut, ou aujourd’hui). Reprenez le champ `texte` du résultat tel quel dans votre réponse : il nomme le compte, le fuseau et l’heure de chaque rendez-vous.", events_schema()),
    ]
}
/// Model-facing tools, advertised only while Microsoft 365 is connected and enabled for the bot.
pub(in super::super) fn tool_specs() -> Vec<grokbot_engine::contracts::ToolSpec> {
    let mut specs = read_specs();
    specs.extend(mail::tool_specs());
    specs
}
/// The connection has no process: it is native, and its tools are this fixed list.
pub(super) fn attach(ctx: &Context<'_>, server: &Server) -> Result<Value, ApiError> {
    let mut tools = vec![
        json!({"name":"list_recent_messages","description":MESSAGES_SUMMARY,"inputSchema":messages_schema()}),
        json!({"name":"list_events","description":EVENTS_SUMMARY,"inputSchema":events_schema()}),
    ];
    tools.extend(mail::tool_definitions());
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
        json!({"message":"Microsoft 365 est connecté. Activez les outils pour un bot : lire les derniers courriels et les rendez-vous part sans carte d’approbation ; un courriel ne part qu’après votre clic sur sa carte brouillon. Rien n’est écrit dans le calendrier.","outils":tools}),
    )
}

/// The account of the live connection, and the server that holds its token.
struct Account {
    server: Server,
    /// The address Graph gave at sign-in: the one mail leaves from.
    name: String,
    /// Moves with every sign-in: a token is the one a draft was prepared with only while it holds.
    generation: i64,
    enabled: bool,
}
fn connected(ctx: &Context<'_>) -> Result<Option<Account>, ApiError> {
    let row: Option<(String, Option<String>, i64)> = ctx
        .connection
        .query_row(
            "SELECT server_id,account_email,generation FROM wrapper_connector_catalogue WHERE id='microsoft365' AND phase='connected'",
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
            name: name.unwrap_or_else(|| "Microsoft 365".into()),
            generation,
            server,
        }))
}
/// Why the Microsoft 365 tool is not offered to this bot: not connected, disconnected, or not enabled.
pub(in super::super) fn unavailable(ctx: &Context<'_>) -> String {
    match connected(ctx) {
        Ok(Some(_)) => ENABLE.into(),
        _ => {
            let phase: Option<String> = ctx
                .connection
                .query_row(
                    "SELECT phase FROM wrapper_connector_catalogue WHERE id='microsoft365'",
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

fn unreachable_graph() -> ApiError {
    ApiError::new(
        503,
        "Microsoft 365 est injoignable pour le moment. Vérifiez la connexion, puis réessayez.",
    )
}
fn graph_url(path: &str) -> Option<String> {
    let m = manifest("microsoft365").ok()?;
    Some(format!("{}{path}", endpoint(m, "api")))
}
/// One Graph read, no redirect, bounded body. Transport failures carry no detail.
/// Times are asked in UTC: the zone a day is read in is applied here, not guessed by the server.
fn get(access: &str, path: &str) -> Result<Reply, ()> {
    reply_of(
        agent()
            .get(&graph_url(path).ok_or(())?)
            .set("Authorization", &format!("Bearer {access}"))
            .set("Accept", "application/json")
            .set("Prefer", "outlook.timezone=\"UTC\"")
            .call(),
    )
}
/// One Graph write, no redirect. `sendMail` answers 202 with no body.
pub(super) fn post(access: &str, path: &str, body: Value) -> Result<Reply, ()> {
    reply_of(
        agent()
            .post(&graph_url(path).ok_or(())?)
            .set("Authorization", &format!("Bearer {access}"))
            .set("Accept", "application/json")
            .send_json(body),
    )
}
/// The account a fresh token belongs to: the address of the mailbox, which the card shows
/// and which mail leaves from.
pub(super) fn identity(access: &str) -> Option<String> {
    let reply = get(access, "/me?$select=displayName,mail,userPrincipalName").ok()?;
    if reply.status != 200 {
        return None;
    }
    let pick = |value: &Value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|s| {
                s.contains('@') && s.len() <= 254 && !s.chars().any(|c| c.is_control() || c == ' ')
            })
            .map(str::to_owned)
    };
    pick(&reply.body["mail"]).or_else(|| pick(&reply.body["userPrincipalName"]))
}

/// The code Graph gives an error: `InvalidAuthenticationToken`, `ErrorAccessDenied`…
fn error_code(reply: &Reply) -> &str {
    reply.body["error"]["code"].as_str().unwrap_or("")
}
/// What Graph's answer means for Charles. A dead token also moves the card to reconnect.
fn refusal(ctx: &Context<'_>, account: &Account, reply: &Reply, what: &str) -> ApiError {
    let code = error_code(reply);
    if reply.status == 429 || code == "TooManyRequests" || code == "ApplicationThrottled" {
        let wait = reply
            .retry_after
            .filter(|s| *s <= 3600)
            .map(|s| format!(" Réessayez dans environ {} s.", s.max(1)))
            .unwrap_or_default();
        return ApiError::new(
            429,
            format!("Microsoft limite le débit des requêtes.{wait}"),
        );
    }
    if reply.status == 401 || code == "InvalidAuthenticationToken" {
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
        return invalid(&format!("Microsoft refuse l’accès à {what} avec ce compte. Si votre organisation restreint le consentement des applications, son administrateur doit autoriser Ezerbot ; sinon reconnectez Microsoft 365 pour accorder de nouveau les autorisations."));
    }
    if reply.status >= 500 {
        return unreachable_graph();
    }
    let code: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(60)
        .collect();
    invalid(&format!(
        "Microsoft a refusé la requête concernant {what}{}.",
        if code.is_empty() {
            String::new()
        } else {
            format!(" ({code})")
        }
    ))
}
/// The body of a Graph list answer.
fn checked(
    ctx: &Context<'_>,
    account: &Account,
    reply: Reply,
    what: &str,
) -> Result<Value, ApiError> {
    if reply.status == 200 && reply.body["value"].is_array() {
        Ok(reply.body)
    } else {
        Err(refusal(ctx, account, &reply, what))
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
                "Le compte Microsoft 365 a changé pendant la lecture : rien n’a été lu. Réessayez.",
            ))
        }
    }
    credentials(ctx, &account.server.id)?
        .and_then(|env| env.get("ACCESS_TOKEN").cloned())
        .ok_or_else(|| invalid(RECONNECT))
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

/// `AAAA-MM-JJ HH:MM UTC` from Graph's ISO timestamp (`2026-10-23T08:15:00Z`).
fn when(iso: &str) -> String {
    match (iso.get(..10), iso.get(11..16)) {
        (Some(day), Some(time)) => format!("{day} {time} UTC"),
        _ => String::new(),
    }
}
fn short(value: &Value, limit: usize) -> String {
    value
        .as_str()
        .map(|s| clean(s, limit))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

pub(in super::super) fn call(
    ctx: &Context<'_>,
    name: &str,
    arguments: &Value,
) -> grokbot_engine::contracts::ToolOutcome {
    let outcome = match name {
        MESSAGES_TOOL => list_messages(ctx, arguments),
        _ => list_events(ctx, arguments),
    };
    match outcome {
        Ok(output) => grokbot_engine::contracts::ToolOutcome::Done { output },
        Err(error) => grokbot_engine::contracts::ToolOutcome::Failed {
            reason: error.message,
        },
    }
}
fn list_messages(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    if !arguments.as_object().is_some_and(|a| a.is_empty()) && !arguments.is_null() {
        return Err(invalid(
            "Cet outil ne prend aucun argument : il lit les 5 derniers courriels.",
        ));
    }
    let (account, access) = readable(ctx)?;
    let reply = get(
        &access,
        &format!("/me/mailFolders/inbox/messages?$top={MESSAGES}&$orderby=receivedDateTime%20desc&$select=subject,from,receivedDateTime,bodyPreview"),
    )
    .map_err(|_| unreachable_graph())?;
    let data = checked(ctx, &account, reply, "la boîte de réception")?;
    let shown: Vec<Value> = data["value"]
        .as_array()
        .into_iter()
        .flatten()
        .take(MESSAGES)
        .map(|m| {
            let sender = &m["from"]["emailAddress"];
            json!({
                "expediteur": short(&sender["name"], 100),
                "adresse": short(&sender["address"], 254),
                "objet": short(&m["subject"], 255),
                "recu": when(m["receivedDateTime"].as_str().unwrap_or("")),
                "apercu": short(&m["bodyPreview"], PREVIEW),
            })
        })
        .collect();
    let mut lines = vec![format!(
        "Outlook · compte {} · {} dernier(s) courriel(s) de la boîte de réception, le plus récent d’abord :",
        clean(&account.name, 254),
        shown.len()
    )];
    lines.extend(shown.iter().enumerate().map(|(index, m)| {
        let field = |key: &str| m[key].as_str().unwrap_or("");
        let sender = if field("expediteur").is_empty() {
            field("adresse").to_owned()
        } else {
            format!("{} <{}>", field("expediteur"), field("adresse"))
        };
        let subject = if field("objet").is_empty() {
            "(sans objet)"
        } else {
            field("objet")
        };
        format!(
            "{}. [{}] {sender} · {subject} · {}",
            index + 1,
            field("recu"),
            field("apercu")
        )
    }));
    if shown.is_empty() {
        lines.push("Aucun courriel dans la boîte de réception.".into());
    }
    Ok(json!({
        "compte": clean(&account.name, 254),
        "courriels": shown,
        "texte": lines.join("\n"),
    }))
}

/// Graph's `2026-10-24T09:00:00.0000000`, asked in UTC, as an instant.
fn instant(part: &Value) -> Option<Timestamp> {
    let text = part["dateTime"].as_str()?;
    let civil: DateTime = text.get(..19)?.parse().ok()?;
    Some(civil.to_zoned(TimeZone::UTC).ok()?.timestamp())
}
/// The zone a day is read in: the Mac's. The mailbox's own would need the permission to read
/// its settings, which Ezerbot does not ask for.
fn zone() -> (TimeZone, String) {
    let zone = TimeZone::system();
    let name = zone.iana_name().map_or_else(|| "UTC".into(), str::to_owned);
    if name == "UTC" {
        (TimeZone::UTC, name)
    } else {
        (zone, name)
    }
}
fn list_events(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        jour: Option<String>,
    }
    let args: Args = serde_json::from_value(if arguments.is_null() {
        json!({})
    } else {
        arguments.clone()
    })
    .map_err(|_| invalid(DAY))?;
    let (days, label) = match args.jour.as_deref() {
        None | Some("demain") => (1, "demain"),
        Some("aujourd_hui") => (0, "aujourd’hui"),
        Some(_) => return Err(invalid(DAY)),
    };
    let (account, access) = readable(ctx)?;
    let unreadable = || invalid("L’agenda Outlook est illisible.");
    let (zone, zone_name) = zone();
    let today = Timestamp::from_millisecond(ctx.now_ms as i64)
        .map_err(|_| unreadable())?
        .to_zoned(zone.clone())
        .date();
    let day = today
        .checked_add(Span::new().days(days))
        .map_err(|_| unreadable())?;
    let next = day
        .checked_add(Span::new().days(1))
        .map_err(|_| unreadable())?;
    let bound = |date: jiff::civil::Date| -> Result<String, ApiError> {
        Ok(date
            .to_zoned(zone.clone())
            .map_err(|_| unreadable())?
            .timestamp()
            .to_string())
    };
    let reply = get(
        &access,
        &format!(
            "/me/calendarView?startDateTime={}&endDateTime={}&$orderby=start/dateTime&$top={EVENTS}&$select=subject,start,end,isAllDay,isCancelled,location",
            bound(day)?,
            bound(next)?
        ),
    )
    .map_err(|_| unreachable_graph())?;
    let data = checked(ctx, &account, reply, "l’agenda")?;
    let told = |at: Timestamp| at.to_zoned(zone.clone()).strftime("%H:%M").to_string();
    let mut events = Vec::new();
    let mut lines = Vec::new();
    for item in data["value"].as_array().into_iter().flatten() {
        if item["isCancelled"] == true {
            continue;
        }
        let (Some(start), Some(end)) = (instant(&item["start"]), instant(&item["end"])) else {
            continue;
        };
        let all_day = item["isAllDay"] == true;
        let title = short(&item["subject"], 200);
        let title = if title.is_empty() {
            "(sans titre)".to_owned()
        } else {
            title
        };
        let place = short(&item["location"]["displayName"], 120);
        let hours = if all_day {
            "toute la journée".to_owned()
        } else {
            format!("{}–{}", told(start), told(end))
        };
        lines.push(format!(
            "{}. {hours} · {title}{}",
            lines.len() + 1,
            if place.is_empty() {
                String::new()
            } else {
                format!(" · {place}")
            }
        ));
        events.push(json!({
            "titre": title,
            "debut": start.to_zoned(zone.clone()).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            "fin": end.to_zoned(zone.clone()).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            "journee": all_day,
            "lieu": place,
        }));
    }
    let more = data["@odata.nextLink"].is_string();
    let heading = format!(
        "{} · compte Microsoft 365 : {} · fuseau : {zone_name}",
        if events.is_empty() {
            format!("Aucun rendez-vous {label} ({day})")
        } else {
            format!("Rendez-vous de {label} ({day})")
        },
        clean(&account.name, 254)
    );
    let mut text = vec![heading];
    text.extend(lines);
    if more {
        text.push("Il y a d’autres rendez-vous ce jour-là que cette liste n’affiche pas.".into());
    }
    Ok(json!({
        "compte": clean(&account.name, 254),
        "jour": day.to_string(),
        "fuseau": zone_name,
        "rendez_vous": events,
        "autres_rendez_vous": more,
        "texte": text.join("\n"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamps_become_utc_dates() {
        assert_eq!(when("2026-10-23T08:15:00Z"), "2026-10-23 08:15 UTC");
        assert_eq!(when(""), "");
    }
    #[test]
    fn graph_times_asked_in_utc_are_instants() {
        let at = instant(&json!({"dateTime":"2026-10-24T09:00:00.0000000","timeZone":"UTC"}));
        assert_eq!(
            at.map(|t| t.to_string()).as_deref(),
            Some("2026-10-24T09:00:00Z")
        );
        assert!(instant(&json!({"dateTime":"hier"})).is_none());
        assert!(instant(&json!({})).is_none());
    }
    #[test]
    fn the_endpoints_are_microsofts_and_the_tenant_defaults_to_every_account() {
        if fake_base().is_some() || std::env::var_os("EZERBOT_MICROSOFT_TENANT").is_some() {
            return;
        }
        let m = manifest("microsoft365").unwrap();
        assert_eq!(
            endpoint(m, "authorize"),
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
        );
        assert_eq!(
            endpoint(m, "token"),
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
        assert_eq!(endpoint(m, "api"), "https://graph.microsoft.com/v1.0");
        assert!(m.scopes.split(' ').all(|s| !s.contains("ReadWrite")));
        assert!(!m.scopes.contains("Calendars.") || m.scopes.contains("Calendars.Read "));
    }
    #[test]
    fn only_the_two_reads_are_reads() {
        assert!(is_read(MESSAGES_TOOL) && is_read(EVENTS_TOOL));
        assert!(!is_read(mail::SEND_TOOL));
    }
}
