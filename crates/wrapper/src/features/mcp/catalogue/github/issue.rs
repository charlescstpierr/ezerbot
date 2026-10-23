//! Issue creation as a draft card, on the table and the rules of the comment draft: preparing
//! the card calls no provider, the send is bound to one fingerprint of the validated title,
//! body, account and repository, and the row is the single claim on that send, so a replayed
//! Envoyer or a replayed run never creates a second issue.
use super::comment::bound;
use super::*;
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};

/// Write tool of the closed list. The local policy keeps it on a confirmation card that no
/// standing rule can cover, whatever its name says.
pub(in super::super::super) const TOOL: &str = "github_create_issue";
const MAX_TITLE: usize = 256;
const MAX_BODY: usize = 65_536;
const ACTION: &str = "Créer une issue";

pub(in super::super::super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    let present: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('wrapper_github_drafts') WHERE name='title')",
        [],
        |r| r.get(0),
    )?;
    if !present {
        conn.execute(
            "ALTER TABLE wrapper_github_drafts ADD COLUMN title TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    Ok(())
}

fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "repository":{"type":"string","description":"Dépôt au format propriétaire/dépôt."},
        "title":{"type":"string","description":"Titre exact de l’issue, que Charles pourra corriger avant l’envoi."},
        "body":{"type":"string","description":"Corps exact de l’issue, que Charles pourra corriger avant l’envoi."},
        "compte":account_property()
    },"required":["repository","title"]})
}
pub(super) fn tool_definition() -> Value {
    json!({
        "name":"create_issue",
        "description":"Préparer une issue GitHub : une carte brouillon attend la validation de Charles, rien n’est créé avant son clic.",
        "inputSchema":schema(),
    })
}
pub(in super::super::super) fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: TOOL.into(),
        description: format!("GitHub · créer une issue dans un dépôt `propriétaire/dépôt` avec le compte GitHub connecté. Une carte brouillon montre le titre et le corps, que Charles peut corriger ; rien n’est créé avant son clic sur Envoyer, et un refus ne crée rien. Le résultat dit quel titre et quel corps ont été publiés, et le numéro de l’issue. {ACCOUNTS_NOTE}"),
        schema: schema(),
        requires_approval: true,
    }
}

struct Draft {
    repository: String,
    title: String,
    login: String,
    account_id: String,
    body: String,
}
fn fingerprint(repository: &str, title: &str, login: &str, account_id: &str, body: &str) -> String {
    let bytes = serde_json::to_vec(&json!([
        "issue", repository, title, login, account_id, body
    ]))
    .unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}
/// The card shows the stored text: a value masked there would not be what leaves.
fn unmasked(ctx: &Context<'_>, text: &str, what: &str) -> Result<(), ApiError> {
    let shown = redact_value(&crate::features::forms::vault(ctx)?, &json!(text))?;
    if shown != json!(text) {
        return Err(invalid(&format!(
            "{what} contient une valeur privée du coffre : l’issue n’a pas été préparée."
        )));
    }
    Ok(())
}
fn check_title(ctx: &Context<'_>, title: &str) -> Result<(), ApiError> {
    if title.trim().is_empty() {
        return Err(invalid(
            "Le titre est vide : écrivez un titre avant d’envoyer.",
        ));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(invalid(
            "Le titre dépasse 256 caractères, la limite de GitHub.",
        ));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid(
            "Le titre tient sur une seule ligne, sans caractère de contrôle.",
        ));
    }
    unmasked(ctx, title, "Le titre")
}
fn check_body(ctx: &Context<'_>, body: &str) -> Result<(), ApiError> {
    if body.chars().count() > MAX_BODY {
        return Err(invalid(
            "Le corps dépasse 65 536 caractères, la limite de GitHub.",
        ));
    }
    if body
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid("Le corps contient des caractères de contrôle."));
    }
    unmasked(ctx, body, "Le corps")
}
fn parse(ctx: &Context<'_>, arguments: &Value) -> Result<Draft, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        repository: String,
        title: String,
        #[serde(default)]
        body: String,
        compte: Option<String>,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
        invalid("Arguments invalides : indiquez le dépôt (propriétaire/dépôt), le titre et le corps de l’issue.")
    })?;
    let Some((owner, name)) = repository(args.repository.trim()) else {
        return Err(invalid(
            "Dépôt invalide : écrivez-le sous la forme propriétaire/dépôt.",
        ));
    };
    check_title(ctx, &args.title)?;
    check_body(ctx, &args.body)?;
    let account = resolve(ctx, args.compte.as_deref())?;
    Ok(Draft {
        repository: format!("{owner}/{name}"),
        title: args.title,
        login: account.login,
        account_id: account.id,
        body: args.body,
    })
}

/// Checked before any card: a bad target or a missing account never opens one.
pub(in super::super::super) fn validate(
    ctx: &Context<'_>,
    call: &ToolCall,
) -> Result<(), ApiError> {
    // A draft that already exists is bound to its account: resuming it must not resolve the
    // optional `compte` of the original call again, which other accounts may now make ambiguous.
    if let Some(run) = ctx.run_id {
        if let Ok((request_id, _)) =
            crate::features::forms::approval_identity_in(ctx.connection, run, call)
        {
            let bound: Option<(String, String)> = ctx
                .connection
                .query_row(
                    "SELECT login,account_id FROM wrapper_github_drafts WHERE request_id=?1",
                    [&request_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((login, account)) = bound {
                return held(ctx, &login, &account).map(|_| ());
            }
        }
    }
    parse(ctx, &call.arguments).map(|_| ())
}
fn display(draft: &Draft, fingerprint: &str) -> Value {
    json!({"brouillon":{
        "service":"GitHub","action":ACTION,"depot":draft.repository,"titre":draft.title,
        "login":draft.login,"texte":draft.body,"empreinte":fingerprint,
    }})
}
/// Called while the approval request is created, in the same transaction: no network.
pub(in super::super::super) fn prepare(
    ctx: &Context<'_>,
    request_id: &str,
    call: &ToolCall,
) -> Result<Value, ApiError> {
    let draft = parse(ctx, &call.arguments)?;
    let print = fingerprint(
        &draft.repository,
        &draft.title,
        &draft.login,
        &draft.account_id,
        &draft.body,
    );
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_github_drafts(request_id,repository,issue,login,account_id,text,fingerprint,call_id,title) VALUES (?1,?2,0,?3,?4,?5,?6,?7,?8)",
        params![request_id, draft.repository, draft.login, draft.account_id, draft.body, print, call.call_id, draft.title],
    )?;
    Ok(display(&draft, &print))
}
/// The title and the body Charles validated replace the proposal, and the fingerprint follows.
pub(in super::super::super) fn edit(
    ctx: &Context<'_>,
    request_id: &str,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<(), ApiError> {
    let row: Option<(String, String, String, String, String, String)> = ctx.connection.query_row(
        "SELECT repository,login,account_id,state,title,text FROM wrapper_github_drafts WHERE request_id=?1",
        [request_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    ).optional()?;
    let Some((repository, login, account, state, kept_title, kept_body)) = row else {
        return Err(invalid("Ce brouillon est introuvable."));
    };
    if state != "draft" {
        return Err(ApiError::new(409, "Ce brouillon a déjà été envoyé."));
    }
    let title = title.unwrap_or(&kept_title);
    let body = body.unwrap_or(&kept_body);
    check_title(ctx, title)?;
    check_body(ctx, body)?;
    let print = fingerprint(&repository, title, &login, &account, body);
    ctx.connection.execute(
        "UPDATE wrapper_github_drafts SET title=?2,text=?3,fingerprint=?4 WHERE request_id=?1 AND state='draft'",
        params![request_id, title, body, print],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.titre',?2,'$.brouillon.texte',?3,'$.brouillon.empreinte',?4) WHERE request_id=?1",
        params![request_id, title, body, print],
    )?;
    Ok(())
}

pub(in super::super::super) fn call(ctx: &Context<'_>, call: &ToolCall) -> ToolOutcome {
    match publish(ctx, call) {
        Ok(output) => ToolOutcome::Done { output },
        Err(error) => ToolOutcome::Failed {
            reason: error.message,
        },
    }
}
fn uncertain() -> ApiError {
    ApiError::new(
        502,
        "GitHub n’a pas confirmé la réception : l’issue a peut-être été créée. Vérifiez sur GitHub avant d’en préparer une autre.",
    )
}
/// `number` is kept in the `issue` column once GitHub has named it.
fn settle(
    ctx: &Context<'_>,
    request_id: &str,
    state: &str,
    sent: Option<(u64, &str)>,
) -> Result<(), ApiError> {
    ctx.connection.execute(
        "UPDATE wrapper_github_drafts SET state=?2,issue=?3,comment_url=?4 WHERE request_id=?1",
        params![
            request_id,
            state,
            sent.map_or(0, |(number, _)| number as i64),
            sent.map(|(_, url)| url)
        ],
    )?;
    Ok(())
}
fn published(
    login: &str,
    repository: &str,
    number: u64,
    url: &str,
    title: &str,
    body: &str,
) -> Value {
    let target = if number == 0 {
        repository.to_owned()
    } else {
        format!("{repository}#{number}")
    };
    json!({"publie":true,"compte":login,"depot":repository,"numero":number,"url":url,"titre":title,"texte":body,
        "message":format!("Issue créée sur {target} sous le compte {login} : {url}")})
}
struct Stored {
    repository: String,
    number: i64,
    login: String,
    account: String,
    title: String,
    body: String,
    print: String,
    state: String,
    url: Option<String>,
}
fn publish(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let run = ctx.run_id.ok_or_else(|| {
        invalid("Rien n’a été créé : cette demande n’appartient à aucune exécution.")
    })?;
    let (request_id, _) = crate::features::forms::approval_identity_in(ctx.connection, run, call)?;
    let row: Option<Stored> = ctx.connection.query_row(
        "SELECT repository,issue,login,account_id,title,text,fingerprint,state,comment_url FROM wrapper_github_drafts WHERE request_id=?1",
        [&request_id],
        |r| Ok(Stored { repository: r.get(0)?, number: r.get(1)?, login: r.get(2)?, account: r.get(3)?, title: r.get(4)?, body: r.get(5)?, print: r.get(6)?, state: r.get(7)?, url: r.get(8)? }),
    ).optional()?;
    let Some(Stored {
        repository,
        number,
        login,
        account,
        title,
        body,
        print,
        state,
        url,
    }) = row
    else {
        return Err(invalid("Brouillon introuvable : rien n’a été créé."));
    };
    match state.as_str() {
        "draft" => {}
        // A replay answers with the first result and never calls GitHub.
        "sent" => {
            return Ok(published(
                &login,
                &repository,
                number as u64,
                url.as_deref().unwrap_or(""),
                &title,
                &body,
            ))
        }
        _ => return Err(uncertain()),
    }
    let held = match held(ctx, &login, &account) {
        Ok(held) if fingerprint(&repository, &title, &login, &account, &body) == print => held,
        other => {
            settle(ctx, &request_id, "failed", None)?;
            return Err(other.err().unwrap_or_else(|| {
                invalid(
                    "Le compte GitHub a changé depuis l’affichage du brouillon : rien n’a été créé.",
                )
            }));
        }
    };
    let (server, m) = (held.server, held.m);
    // One claim before the call: only the run that wins it may reach GitHub.
    let claimed = ctx.connection.execute(
        "UPDATE wrapper_github_drafts SET state='sending' WHERE request_id=?1 AND state='draft'",
        [&request_id],
    )?;
    if claimed != 1 {
        return Err(uncertain());
    }
    let endpoint = format!("{}/repos/{repository}/issues", endpoint(m, "api"));
    let sent = json!({"title": title, "body": body});
    let reply = match authorized(
        ctx,
        &server,
        m,
        &|ctx| bound(ctx, &server.id, &login, &account),
        &|access| {
            send_json(
                agent()
                    .post(&endpoint)
                    .set("Authorization", &format!("Bearer {access}")),
                &sent,
            )
            .map_err(|_| uncertain())
        },
    ) {
        Ok(reply) => reply,
        Err(error) => {
            settle(
                ctx,
                &request_id,
                if error.status == 502 {
                    "uncertain"
                } else {
                    "failed"
                },
                None,
            )?;
            return Err(error);
        }
    };
    let link = reply.body["html_url"].as_str().filter(|u| {
        u.starts_with("https://") && u.len() <= 500 && !u.chars().any(char::is_control)
    });
    let created = reply.body["number"].as_u64().filter(|n| *n > 0);
    match reply.status {
        // Created but unreadable: it exists on GitHub, so it must not be sent again.
        201 => {
            let (number, url) = (created.unwrap_or(0), link.unwrap_or(""));
            settle(ctx, &request_id, "sent", Some((number, url)))?;
            Ok(published(&login, &repository, number, url, &title, &body))
        }
        status => {
            let error = refusal(&reply, status, &repository, &login);
            settle(
                ctx,
                &request_id,
                if status >= 500 { "uncertain" } else { "failed" },
                None,
            )?;
            Err(error)
        }
    }
}
fn refusal(reply: &Reply, status: u16, repository: &str, login: &str) -> ApiError {
    match status {
        404 => invalid(&format!("Le dépôt {repository} est introuvable pour le compte {login}, ou l’application GitHub d’Ezerbot n’y est pas installée. Rien n’a été créé.")),
        403 | 429 if limited(reply) => limit_error(reply, login),
        403 => invalid(&format!("Accès refusé à {repository} pour le compte {login} : l’application GitHub d’Ezerbot doit avoir le droit d’écrire dans les issues de ce dépôt. Rien n’a été créé.")),
        410 => invalid(&format!("Les issues sont désactivées sur {repository}. Rien n’a été créé.")),
        422 => invalid(&format!("GitHub a refusé cette issue sur {repository} (titre ou texte refusé, ou issues désactivées). Rien n’a été créé.")),
        500..=599 => uncertain(),
        code => invalid(&format!("GitHub a répondu de façon inattendue (code {code}). Rien n’a été créé.")),
    }
}
