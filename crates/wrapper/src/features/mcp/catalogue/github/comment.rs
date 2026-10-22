//! Issue comment as a draft card. Nothing leaves before the click: preparing the card
//! calls no provider, and the send is bound to one fingerprint of the validated text,
//! the account and the target. The row below is the single claim on that send, so a
//! replayed Envoyer or a replayed run never reaches GitHub twice.
use super::*;
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};

/// Write tool of the closed list. Its name says a comment, so the local policy keeps it
/// on a confirmation card that no standing rule can cover.
pub(in super::super::super) const TOOL: &str = "github_add_issue_comment";
const MAX_TEXT: usize = 65_536;
const ACTION: &str = "Commenter une issue";

pub(in super::super::super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_github_drafts (request_id TEXT PRIMARY KEY,repository TEXT NOT NULL,issue INTEGER NOT NULL,login TEXT NOT NULL,account_id TEXT NOT NULL,text TEXT NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'draft',comment_url TEXT,call_id TEXT NOT NULL DEFAULT '');")?;
    // A send that was in flight when the app stopped may or may not have reached GitHub.
    conn.execute(
        "UPDATE wrapper_github_drafts SET state='uncertain' WHERE state='sending'",
        [],
    )?;
    Ok(())
}

fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "repository":{"type":"string","description":"Dépôt au format propriétaire/dépôt."},
        "issue":{"type":"integer","minimum":1,"description":"Numéro de l’issue."},
        "text":{"type":"string","description":"Texte exact du commentaire, que Charles pourra corriger avant l’envoi."},
        "compte":account_property()
    },"required":["repository","issue","text"]})
}
pub(super) fn tool_definition() -> Value {
    json!({
        "name":"add_issue_comment",
        "description":"Préparer un commentaire d’issue GitHub : une carte brouillon attend la validation de Charles, rien n’est publié avant son clic.",
        "inputSchema":schema(),
    })
}
pub(in super::super::super) fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: TOOL.into(),
        description: format!("GitHub · commenter une issue d’un dépôt `propriétaire/dépôt` avec le compte GitHub connecté. Une carte brouillon montre le texte, que Charles peut corriger ; rien n’est publié avant son clic sur Envoyer, et un refus ne publie rien. Le résultat dit quel texte a été publié. {ACCOUNTS_NOTE}"),
        schema: schema(),
        requires_approval: true,
    }
}

struct Draft {
    repository: String,
    issue: u64,
    login: String,
    account_id: String,
    text: String,
}
fn fingerprint(repository: &str, issue: u64, login: &str, account_id: &str, text: &str) -> String {
    let bytes = serde_json::to_vec(&json!([repository, issue, login, account_id, text]))
        .unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}
fn check_text(ctx: &Context<'_>, text: &str) -> Result<(), ApiError> {
    if text.trim().is_empty() {
        return Err(invalid(
            "Le commentaire est vide : écrivez un texte avant d’envoyer.",
        ));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(invalid(
            "Le commentaire dépasse 65 536 caractères, la limite de GitHub.",
        ));
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid(
            "Le commentaire contient des caractères de contrôle.",
        ));
    }
    // The card shows the stored text: a value masked there would not be what leaves.
    let shown = redact_value(&crate::features::forms::vault(ctx)?, &json!(text))?;
    if shown != json!(text) {
        return Err(invalid(
            "Le commentaire contient une valeur privée du coffre : il n’a pas été préparé.",
        ));
    }
    Ok(())
}
fn parse(ctx: &Context<'_>, arguments: &Value) -> Result<Draft, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        repository: String,
        issue: u64,
        text: String,
        compte: Option<String>,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
        invalid("Arguments invalides : indiquez le dépôt (propriétaire/dépôt), le numéro de l’issue et le texte.")
    })?;
    let Some((owner, name)) = repository(args.repository.trim()) else {
        return Err(invalid(
            "Dépôt invalide : écrivez-le sous la forme propriétaire/dépôt.",
        ));
    };
    if !(1..=i32::MAX as u64).contains(&args.issue) {
        return Err(invalid("Numéro d’issue invalide."));
    }
    check_text(ctx, &args.text)?;
    let account = resolve(ctx, args.compte.as_deref())?;
    Ok(Draft {
        repository: format!("{owner}/{name}"),
        issue: args.issue,
        login: account.login,
        account_id: account.id,
        text: args.text,
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
        if let Ok((request_id, _)) = crate::features::forms::approval_identity(run, call) {
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
        "service":"GitHub","action":ACTION,"depot":draft.repository,"numero":draft.issue,
        "login":draft.login,"texte":draft.text,"empreinte":fingerprint,
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
        draft.issue,
        &draft.login,
        &draft.account_id,
        &draft.text,
    );
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_github_drafts(request_id,repository,issue,login,account_id,text,fingerprint,call_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![request_id, draft.repository, draft.issue as i64, draft.login, draft.account_id, draft.text, print, call.call_id],
    )?;
    Ok(display(&draft, &print))
}
/// The tool call this draft came from: its result is the only one the card may show.
pub(in super::super::super) fn call_id(ctx: &Context<'_>, request_id: &str) -> Option<String> {
    ctx.connection
        .query_row(
            "SELECT call_id FROM wrapper_github_drafts WHERE request_id=?1",
            [request_id],
            |r| r.get(0),
        )
        .ok()
}
/// Under the connection lock, the credentials in use are still those of the approved account.
fn bound(ctx: &Context<'_>, server: &str, login: &str, account: &str) -> Result<(), ApiError> {
    let row: Option<(String, Option<String>, Option<String>)> = ctx.connection.query_row(
        "SELECT phase,account_email,account_id FROM wrapper_connector_catalogue WHERE server_id=?1",
        [server],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).optional()?;
    match row {
        Some((phase, Some(current), Some(id)))
            if phase == "connected" && current == login && id == account =>
        {
            Ok(())
        }
        _ => Err(invalid(
            "Le compte GitHub a changé depuis l’affichage du brouillon : rien n’a été publié.",
        )),
    }
}
/// The text Charles validated replaces the proposal, and the fingerprint follows it.
pub(in super::super::super) fn edit(
    ctx: &Context<'_>,
    request_id: &str,
    text: &str,
) -> Result<(), ApiError> {
    check_text(ctx, text)?;
    let row: Option<(String, i64, String, String, String)> = ctx.connection.query_row(
        "SELECT repository,issue,login,account_id,state FROM wrapper_github_drafts WHERE request_id=?1",
        [request_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    ).optional()?;
    let Some((repository, issue, login, account, state)) = row else {
        return Err(invalid("Ce brouillon est introuvable."));
    };
    if state != "draft" {
        return Err(ApiError::new(409, "Ce brouillon a déjà été envoyé."));
    }
    let print = fingerprint(&repository, issue as u64, &login, &account, text);
    ctx.connection.execute(
        "UPDATE wrapper_github_drafts SET text=?2,fingerprint=?3 WHERE request_id=?1 AND state='draft'",
        params![request_id, text, print],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.texte',?2,'$.brouillon.empreinte',?3) WHERE request_id=?1",
        params![request_id, text, print],
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
        "GitHub n’a pas confirmé la réception : le commentaire a peut-être été publié. Vérifiez sur GitHub avant d’en préparer un autre.",
    )
}
fn settle(
    ctx: &Context<'_>,
    request_id: &str,
    state: &str,
    url: Option<&str>,
) -> Result<(), ApiError> {
    ctx.connection.execute(
        "UPDATE wrapper_github_drafts SET state=?2,comment_url=?3 WHERE request_id=?1",
        params![request_id, state, url],
    )?;
    Ok(())
}
fn published(login: &str, repository: &str, issue: u64, url: &str, text: &str) -> Value {
    json!({"publie":true,"compte":login,"depot":repository,"numero":issue,"url":url,"texte":text,
        "message":format!("Commentaire publié sur {repository}#{issue} sous le compte {login} : {url}")})
}
struct Stored {
    repository: String,
    issue: i64,
    login: String,
    account: String,
    text: String,
    print: String,
    state: String,
    url: Option<String>,
}
fn publish(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let run = ctx.run_id.ok_or_else(|| {
        invalid("Rien n’a été publié : cette demande n’appartient à aucune exécution.")
    })?;
    let (request_id, _) = crate::features::forms::approval_identity(run, call)?;
    let row: Option<Stored> = ctx.connection.query_row(
        "SELECT repository,issue,login,account_id,text,fingerprint,state,comment_url FROM wrapper_github_drafts WHERE request_id=?1",
        [&request_id],
        |r| Ok(Stored { repository: r.get(0)?, issue: r.get(1)?, login: r.get(2)?, account: r.get(3)?, text: r.get(4)?, print: r.get(5)?, state: r.get(6)?, url: r.get(7)? }),
    ).optional()?;
    let Some(Stored {
        repository,
        issue,
        login,
        account,
        text,
        print,
        state,
        url,
    }) = row
    else {
        return Err(invalid("Brouillon introuvable : rien n’a été publié."));
    };
    let issue = issue as u64;
    match state.as_str() {
        "draft" => {}
        // A replay answers with the first result and never calls GitHub.
        "sent" => {
            return Ok(published(
                &login,
                &repository,
                issue,
                url.as_deref().unwrap_or(""),
                &text,
            ))
        }
        _ => return Err(uncertain()),
    }
    let held = match held(ctx, &login, &account) {
        Ok(held) if fingerprint(&repository, issue, &login, &account, &text) == print => held,
        other => {
            settle(ctx, &request_id, "failed", None)?;
            return Err(other.err().unwrap_or_else(|| {
                invalid(
                    "Le compte GitHub a changé depuis l’affichage du brouillon : rien n’a été publié.",
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
    let endpoint = format!(
        "{}/repos/{repository}/issues/{issue}/comments",
        endpoint(m, "api")
    );
    let body = json!({"body": text});
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
                &body,
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
    let comment = reply.body["html_url"].as_str().filter(|u| {
        u.starts_with("https://") && u.len() <= 500 && !u.chars().any(char::is_control)
    });
    match (reply.status, comment) {
        (201, Some(url)) => {
            settle(ctx, &request_id, "sent", Some(url))?;
            Ok(published(&login, &repository, issue, url, &text))
        }
        // Created but unreadable: it exists on GitHub, so it must not be sent again.
        (201, None) => {
            settle(ctx, &request_id, "sent", None)?;
            Ok(published(&login, &repository, issue, "", &text))
        }
        (status, _) => {
            let error = refusal(&reply, status, &repository, issue, &login);
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
fn refusal(reply: &Reply, status: u16, repository: &str, issue: u64, login: &str) -> ApiError {
    match status {
        404 => invalid(&format!("L’issue {repository}#{issue} est introuvable pour le compte {login}, ou l’application GitHub d’Ezerbot n’y est pas installée. Rien n’a été publié.")),
        403 | 429 if limited(reply) => limit_error(reply, login),
        403 => invalid(&format!("Accès refusé à {repository} pour le compte {login} : l’application GitHub d’Ezerbot doit avoir le droit d’écrire dans les issues de ce dépôt. Rien n’a été publié.")),
        410 => invalid(&format!("Les issues sont désactivées sur {repository}. Rien n’a été publié.")),
        422 => invalid(&format!("GitHub a refusé ce commentaire sur {repository}#{issue} (issue verrouillée ou texte refusé). Rien n’a été publié.")),
        500..=599 => uncertain(),
        code => invalid(&format!("GitHub a répondu de façon inattendue (code {code}). Rien n’a été publié.")),
    }
}
