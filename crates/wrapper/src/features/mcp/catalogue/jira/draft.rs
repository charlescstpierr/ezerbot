//! Jira comment as a draft card. Nothing leaves before the click: preparing a card calls no
//! provider, and the send is bound to one fingerprint of the validated text, the account (which
//! names the site) and the issue. The row below is the single claim on that send, so a replayed
//! Envoyer or a replayed run never reaches Jira twice.
use super::*;
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};

/// Write tool of the closed list. The local policy keeps it on a confirmation card that
/// no standing rule can cover, whatever its name says.
pub(in super::super::super) const COMMENT_TOOL: &str = "jira_comment_issue";
/// Jira refuses a comment beyond 32 767 characters.
const MAX_TEXT: usize = 30_000;
const CHANGED: &str =
    "Le compte Jira a changé depuis l’affichage du brouillon : rien n’a été publié.";

pub(in super::super::super) fn is_tool(name: &str) -> bool {
    name == COMMENT_TOOL
}

pub(in super::super::super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_jira_drafts (request_id TEXT PRIMARY KEY,tool TEXT NOT NULL,server_id TEXT NOT NULL,account TEXT NOT NULL,generation INTEGER NOT NULL,target TEXT NOT NULL,text TEXT NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'draft',call_id TEXT NOT NULL DEFAULT '',result TEXT);")?;
    // A send that was in flight when the app stopped may or may not have reached Jira.
    conn.execute(
        "UPDATE wrapper_jira_drafts SET state='uncertain' WHERE state='sending'",
        [],
    )?;
    Ok(())
}

fn comment_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "issue":{"type":"string","description":"Clé de l’issue à commenter, par exemple ENG-12."},
        "text":{"type":"string","description":"Texte exact du commentaire (texte brut, une ligne par paragraphe), que Charles pourra corriger avant l’envoi."}
    },"required":["issue","text"]})
}
pub(super) fn tool_definitions() -> Vec<Value> {
    vec![
        json!({"name":"comment_issue","description":"Préparer un commentaire sur une issue Jira : une carte brouillon attend la validation de Charles, rien n’est publié avant son clic.","inputSchema":comment_schema()}),
    ]
}
pub(super) fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: COMMENT_TOOL.into(),
        description: "Jira · commenter une issue (par exemple ENG-12) avec le compte Jira connecté. Une carte brouillon montre le compte et le site, l’issue et le texte, que Charles peut corriger ; rien n’est publié avant son clic sur Envoyer, et un refus ne publie rien. Le résultat dit quel commentaire a été publié.".into(),
        schema: comment_schema(),
        requires_approval: true,
    }]
}

struct Draft {
    account: String,
    /// The key of the issue to comment.
    target: String,
    text: String,
}
fn fingerprint(tool: &str, account: &str, target: &str, text: &str) -> String {
    let bytes = serde_json::to_vec(&json!([tool, account, target, text])).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}
fn check_text(ctx: &Context<'_>, text: &str) -> Result<(), ApiError> {
    if text.trim().is_empty() {
        return Err(invalid(
            "Le commentaire est vide : écrivez un texte avant d’envoyer.",
        ));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(invalid("Le texte dépasse 30 000 caractères."));
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid("Le texte contient des caractères de contrôle."));
    }
    // The card shows the stored text: a value masked there would not be what leaves.
    let shown = redact_value(&crate::features::forms::vault(ctx)?, &json!(text))?;
    if shown != json!(text) {
        return Err(invalid(
            "Le texte contient une valeur privée du coffre : il n’a pas été préparé.",
        ));
    }
    Ok(())
}
fn parse(ctx: &Context<'_>, arguments: &Value) -> Result<Draft, ApiError> {
    let account = match connected(ctx)? {
        Some(account) if account.enabled => account.name,
        _ => return Err(invalid(&unavailable(ctx))),
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        issue: String,
        text: String,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
        invalid("Arguments invalides : indiquez la clé de l’issue Jira et le texte du commentaire.")
    })?;
    let issue = args.issue.trim().to_uppercase();
    if !is_issue_key(&issue) {
        return Err(invalid(
            "Issue invalide : indiquez sa clé Jira, par exemple ENG-12.",
        ));
    }
    check_text(ctx, &args.text)?;
    Ok(Draft {
        account,
        target: issue,
        text: args.text,
    })
}
/// The account a draft card was prepared for, while it is still the connection it was
/// prepared with: a new sign-in (another account, another site) is not.
fn held(ctx: &Context<'_>, name: &str, generation: i64) -> Result<Account, ApiError> {
    match connected(ctx)? {
        Some(account) if account.name == name && account.generation == generation => {
            // Another bot's activation does not authorize this one.
            if account.enabled {
                Ok(account)
            } else {
                Err(invalid(
                    "Les outils Jira ne sont plus activés pour ce bot : rien n’a été publié.",
                ))
            }
        }
        _ => Err(invalid(CHANGED)),
    }
}

/// Checked before any card: a bad target or a missing account never opens one.
pub(in super::super::super) fn validate(
    ctx: &Context<'_>,
    call: &ToolCall,
) -> Result<(), ApiError> {
    // A draft that already exists is bound to its account: resuming it must not parse the
    // original call again.
    if let Some(run) = ctx.run_id {
        if let Ok((request_id, _)) = crate::features::forms::approval_identity(run, call) {
            let bound: Option<(String, i64)> = ctx
                .connection
                .query_row(
                    "SELECT account,generation FROM wrapper_jira_drafts WHERE request_id=?1",
                    [&request_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((account, generation)) = bound {
                return held(ctx, &account, generation).map(|_| ());
            }
        }
    }
    parse(ctx, &call.arguments).map(|_| ())
}
fn display(draft: &Draft, fingerprint: &str) -> Value {
    json!({"brouillon":{
        "service":"Jira Cloud","action":"Commenter une issue Jira","compte":draft.account,
        "issue":draft.target,"texte":draft.text,"empreinte":fingerprint,
    }})
}
/// Called while the approval request is created, in the same transaction: no network.
pub(in super::super::super) fn prepare(
    ctx: &Context<'_>,
    request_id: &str,
    call: &ToolCall,
) -> Result<Value, ApiError> {
    let draft = parse(ctx, &call.arguments)?;
    let print = fingerprint(&call.name, &draft.account, &draft.target, &draft.text);
    let (server, generation) = connected(ctx)?
        .map(|account| (account.server.id, account.generation))
        .ok_or_else(|| invalid(&unavailable(ctx)))?;
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_jira_drafts(request_id,tool,server_id,account,generation,target,text,fingerprint,call_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![request_id, call.name, server, draft.account, generation, draft.target, draft.text, print, call.call_id],
    )?;
    Ok(display(&draft, &print))
}
/// The tool call this draft came from: its result is the only one the card may show.
pub(in super::super::super) fn call_id(ctx: &Context<'_>, request_id: &str) -> Option<String> {
    ctx.connection
        .query_row(
            "SELECT call_id FROM wrapper_jira_drafts WHERE request_id=?1",
            [request_id],
            |r| r.get(0),
        )
        .ok()
}
/// What Charles validated replaces the proposal, and the fingerprint follows it.
pub(in super::super::super) fn edit(
    ctx: &Context<'_>,
    request_id: &str,
    text: Option<&str>,
) -> Result<(), ApiError> {
    let row: Option<(String, String, String, String, String)> = ctx
        .connection
        .query_row(
            "SELECT tool,account,target,text,state FROM wrapper_jira_drafts WHERE request_id=?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((tool, account, target, old_text, state)) = row else {
        return Err(invalid("Ce brouillon est introuvable."));
    };
    if state != "draft" {
        return Err(ApiError::new(409, "Ce brouillon a déjà été envoyé."));
    }
    let new_text = text.map_or(old_text, str::to_owned);
    check_text(ctx, &new_text)?;
    let print = fingerprint(&tool, &account, &target, &new_text);
    ctx.connection.execute(
        "UPDATE wrapper_jira_drafts SET text=?2,fingerprint=?3 WHERE request_id=?1 AND state='draft'",
        params![request_id, new_text, print],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.texte',?2,'$.brouillon.empreinte',?3) WHERE request_id=?1",
        params![request_id, new_text, print],
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
/// What the card says of a send that did not go out: Refusé, Trop de messages, Envoi non confirmé.
#[derive(Clone, Copy, PartialEq)]
enum Failure {
    Refused,
    RateLimited,
    Unconfirmed,
}
impl Failure {
    fn state(self) -> &'static str {
        match self {
            Failure::Unconfirmed => "uncertain",
            _ => "failed",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Failure::Refused => "refused",
            Failure::RateLimited => "rate_limited",
            Failure::Unconfirmed => "unconfirmed",
        }
    }
}
fn unconfirmed() -> ApiError {
    ApiError::new(
        502,
        "Jira n’a pas confirmé la réception : le commentaire a peut-être été publié. Vérifiez dans Jira avant d’en préparer un autre.",
    )
}
/// A refusal that is certain: Jira published nothing. Its status says which one.
fn not_published(error: ApiError) -> (Failure, ApiError) {
    let failure = if error.status == 429 {
        Failure::RateLimited
    } else {
        Failure::Refused
    };
    (
        failure,
        ApiError::new(
            error.status,
            format!("{} Rien n’a été publié.", error.message),
        ),
    )
}
/// The row, and the card with it, record how the send ended.
fn settle(
    ctx: &Context<'_>,
    request_id: &str,
    state: &str,
    etat: &str,
    result: Option<&Value>,
) -> Result<(), ApiError> {
    ctx.connection.execute(
        "UPDATE wrapper_jira_drafts SET state=?2,result=?3 WHERE request_id=?1",
        params![request_id, state, result.map(Value::to_string)],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.etat',?2) WHERE request_id=?1",
        params![request_id, etat],
    )?;
    Ok(())
}
fn fail(
    ctx: &Context<'_>,
    request_id: &str,
    failure: Failure,
    error: ApiError,
) -> Result<Value, ApiError> {
    settle(ctx, request_id, failure.state(), failure.label(), None)?;
    Err(error)
}
struct Stored {
    tool: String,
    server: String,
    account: String,
    generation: i64,
    target: String,
    text: String,
    print: String,
    state: String,
    result: Option<String>,
}
fn publish(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let run = ctx.run_id.ok_or_else(|| {
        invalid("Rien n’a été publié : cette demande n’appartient à aucune exécution.")
    })?;
    let (request_id, _) = crate::features::forms::approval_identity(run, call)?;
    let row: Option<Stored> = ctx.connection.query_row(
        "SELECT tool,server_id,account,generation,target,text,fingerprint,state,result FROM wrapper_jira_drafts WHERE request_id=?1",
        [&request_id],
        |r| Ok(Stored { tool: r.get(0)?, server: r.get(1)?, account: r.get(2)?, generation: r.get(3)?, target: r.get(4)?, text: r.get(5)?, print: r.get(6)?, state: r.get(7)?, result: r.get(8)? }),
    ).optional()?;
    let Some(stored) = row else {
        return Err(invalid("Brouillon introuvable : rien n’a été publié."));
    };
    match stored.state.as_str() {
        "draft" => {}
        // A replay answers with the first result and never calls Jira.
        "sent" => {
            return stored
                .result
                .and_then(|result| serde_json::from_str(&result).ok())
                .ok_or_else(unconfirmed)
        }
        "failed" => {
            return Err(invalid(
                "Ce brouillon n’a pas été envoyé : rien n’a été publié. Demandez au bot de le préparer à nouveau.",
            ))
        }
        _ => return Err(unconfirmed()),
    }
    let account = match held(ctx, &stored.account, stored.generation) {
        Ok(account)
            if account.server.id == stored.server
                && fingerprint(&stored.tool, &stored.account, &stored.target, &stored.text)
                    == stored.print =>
        {
            account
        }
        other => {
            let error = other.err().unwrap_or_else(|| invalid(CHANGED));
            return fail(ctx, &request_id, Failure::Refused, error);
        }
    };
    // One claim before the call: only the run that wins it may reach Jira.
    let claimed = ctx.connection.execute(
        "UPDATE wrapper_jira_drafts SET state='sending' WHERE request_id=?1 AND state='draft'",
        [&request_id],
    )?;
    if claimed != 1 {
        return Err(unconfirmed());
    }
    match send(ctx, &account, &stored) {
        Ok(output) => {
            settle(ctx, &request_id, "sent", "sent", Some(&output))?;
            Ok(output)
        }
        Err((failure, error)) => fail(ctx, &request_id, failure, error),
    }
}
/// Under the sign-in lock, the credentials in use are still those of the approved account.
fn bound(ctx: &Context<'_>, account: &Account, stored: &Stored) -> Result<Access, ApiError> {
    let same: Option<(Option<String>, i64)> = ctx
        .connection
        .query_row(
            "SELECT account_email,generation FROM wrapper_connector_catalogue WHERE server_id=?1 AND phase='connected'",
            [&account.server.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if !same.is_some_and(|(name, current)| {
        name.as_deref() == Some(stored.account.as_str()) && current == stored.generation
    }) {
        return Err(invalid(CHANGED));
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
/// Whether Jira certainly rejected the comment, as opposed to an answer that says nothing.
fn rejected(reply: &Reply) -> bool {
    matches!(reply.status, 400 | 401 | 403 | 404 | 429)
}
fn send(
    ctx: &Context<'_>,
    account: &Account,
    stored: &Stored,
) -> Result<Value, (Failure, ApiError)> {
    super::super::ensure(ctx, &account.server).map_err(not_published)?;
    // A sign-in replaces the token under this lock: the send below keeps the one it took.
    let access = {
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        bound(ctx, account, stored).map_err(not_published)?
    };
    let url = rest(&access, &format!("/issue/{}/comment", stored.target)).map_err(not_published)?;
    // Past this point the effect may have reached Jira: only an explicit answer says it did not.
    let reply = request(
        "POST",
        &url,
        &access.token,
        Some(json!({"body":comment_document(&stored.text)})),
    )
    .map_err(|_| (Failure::Unconfirmed, unconfirmed()))?;
    let id = name_of(&reply.body["id"]);
    if reply.status == 201 && !id.is_empty() {
        let account_name = &account.name;
        return Ok(
            json!({"publie":true,"compte":account_name,"issue":stored.target,
            "url":format!("{}/browse/{}?focusedCommentId={id}", access.site.url, stored.target),
            "texte":stored.text,
            "message":format!("Commentaire publié sur l’issue {} dans Jira sous le compte {account_name}.", stored.target)}),
        );
    }
    if rejected(&reply) {
        return Err(not_published(refusal(
            ctx,
            account,
            &reply,
            &format!("l’issue {}", stored.target),
        )));
    }
    // An outage, or an answer that says nothing: the effect may exist.
    Err((Failure::Unconfirmed, unconfirmed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_write_tool_is_told_from_the_reads() {
        assert!(is_tool(COMMENT_TOOL));
        assert!(!is_tool("jira_get_issue") && !is_tool("jira_list_issues"));
    }
    #[test]
    fn a_fingerprint_follows_every_validated_field() {
        let base = fingerprint(COMMENT_TOOL, "a · b", "ENG-1", "x");
        for other in [
            fingerprint("autre", "a · b", "ENG-1", "x"),
            fingerprint(COMMENT_TOOL, "c · d", "ENG-1", "x"),
            fingerprint(COMMENT_TOOL, "a · b", "ENG-2", "x"),
            fingerprint(COMMENT_TOOL, "a · b", "ENG-1", "y"),
        ] {
            assert_ne!(base, other);
        }
    }
}
