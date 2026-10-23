//! Linear issue and Linear comment as draft cards. Nothing leaves before the click:
//! preparing a card calls no provider, and the send is bound to one fingerprint of the
//! validated text, the account and the target. The row below is the single claim on that
//! send, so a replayed Envoyer or a replayed run never reaches Linear twice.
use super::*;
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};

/// Write tools of the closed list. The local policy keeps them on a confirmation card that
/// no standing rule can cover, whatever their names say.
pub(in super::super::super) const CREATE_TOOL: &str = "linear_create_issue";
pub(in super::super::super) const COMMENT_TOOL: &str = "linear_comment_issue";
const MAX_TITLE: usize = 256;
const MAX_TEXT: usize = 65_536;
const CHANGED: &str =
    "Le compte Linear a changé depuis l’affichage du brouillon : rien n’a été publié.";

pub(in super::super::super) fn is_create(name: &str) -> bool {
    name == CREATE_TOOL
}
pub(in super::super::super) fn is_tool(name: &str) -> bool {
    is_create(name) || name == COMMENT_TOOL
}

pub(in super::super::super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_linear_drafts (request_id TEXT PRIMARY KEY,tool TEXT NOT NULL,server_id TEXT NOT NULL,account TEXT NOT NULL,generation INTEGER NOT NULL,target TEXT NOT NULL,title TEXT,text TEXT NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'draft',call_id TEXT NOT NULL DEFAULT '',result TEXT);")?;
    // A send that was in flight when the app stopped may or may not have reached Linear.
    conn.execute(
        "UPDATE wrapper_linear_drafts SET state='uncertain' WHERE state='sending'",
        [],
    )?;
    Ok(())
}

fn create_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "team":{"type":"string","description":"Clé de l’équipe (par exemple ENG) ou son nom."},
        "title":{"type":"string","description":"Titre de l’issue, que Charles pourra corriger avant l’envoi."},
        "description":{"type":"string","description":"Description de l’issue (Markdown), que Charles pourra corriger avant l’envoi ; peut être vide."}
    },"required":["team","title"]})
}
fn comment_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "issue":{"type":"string","description":"Identifiant de l’issue à commenter, par exemple ENG-12."},
        "text":{"type":"string","description":"Texte exact du commentaire (Markdown), que Charles pourra corriger avant l’envoi."}
    },"required":["issue","text"]})
}
pub(super) fn tool_definitions() -> Vec<Value> {
    vec![
        json!({"name":"create_issue","description":"Préparer une issue Linear : une carte brouillon attend la validation de Charles, rien n’est créé avant son clic.","inputSchema":create_schema()}),
        json!({"name":"comment_issue","description":"Préparer un commentaire sur une issue Linear : une carte brouillon attend la validation de Charles, rien n’est publié avant son clic.","inputSchema":comment_schema()}),
    ]
}
pub(super) fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: CREATE_TOOL.into(),
            description: "Linear · créer une issue dans une équipe avec le compte Linear connecté. Une carte brouillon montre le compte, l’équipe, le titre et la description, que Charles peut corriger ; rien n’est créé avant son clic sur Envoyer, et un refus ne crée rien. Le résultat dit quelle issue a été créée.".into(),
            schema: create_schema(),
            requires_approval: true,
        },
        ToolSpec {
            name: COMMENT_TOOL.into(),
            description: "Linear · commenter une issue (par exemple ENG-12) avec le compte Linear connecté. Une carte brouillon montre le compte, l’issue et le texte, que Charles peut corriger ; rien n’est publié avant son clic sur Envoyer, et un refus ne publie rien. Le résultat dit quel commentaire a été publié.".into(),
            schema: comment_schema(),
            requires_approval: true,
        },
    ]
}

struct Draft {
    account: String,
    /// The team as asked (a key or a name), or the identifier of the issue to comment.
    target: String,
    title: Option<String>,
    text: String,
}
fn fingerprint(tool: &str, account: &str, target: &str, title: Option<&str>, text: &str) -> String {
    let bytes =
        serde_json::to_vec(&json!([tool, account, target, title, text])).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}
fn check_title(title: &str) -> Result<(), ApiError> {
    if title.trim().is_empty() {
        return Err(invalid(
            "Le titre est vide : écrivez un titre avant d’envoyer.",
        ));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(invalid("Le titre dépasse 256 caractères."));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid("Le titre contient des caractères de contrôle."));
    }
    Ok(())
}
/// A description may be empty; a comment may not.
fn check_text(ctx: &Context<'_>, text: &str, required: bool) -> Result<(), ApiError> {
    if required && text.trim().is_empty() {
        return Err(invalid(
            "Le commentaire est vide : écrivez un texte avant d’envoyer.",
        ));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(invalid("Le texte dépasse 65 536 caractères."));
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
fn parse(ctx: &Context<'_>, tool: &str, arguments: &Value) -> Result<Draft, ApiError> {
    let account = match connected(ctx)? {
        Some(account) if account.enabled => account.name,
        _ => return Err(invalid(&unavailable(ctx))),
    };
    if is_create(tool) {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            team: String,
            title: String,
            #[serde(default)]
            description: String,
        }
        let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
            invalid("Arguments invalides : indiquez l’équipe Linear et le titre de l’issue.")
        })?;
        let team = args.team.trim();
        if team.is_empty() || team.len() > 100 || team.chars().any(char::is_control) {
            return Err(invalid("Indiquez la clé ou le nom de l’équipe Linear."));
        }
        let title = args.title.trim();
        check_title(title)?;
        check_text(ctx, title, false)?;
        check_text(ctx, &args.description, false)?;
        return Ok(Draft {
            account,
            target: team.to_owned(),
            title: Some(title.to_owned()),
            text: args.description,
        });
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        issue: String,
        text: String,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
        invalid("Arguments invalides : indiquez l’identifiant de l’issue Linear et le texte du commentaire.")
    })?;
    let issue = args.issue.trim().to_uppercase();
    if !is_issue_identifier(&issue) {
        return Err(invalid(
            "Issue invalide : indiquez son identifiant Linear, par exemple ENG-12.",
        ));
    }
    check_text(ctx, &args.text, true)?;
    Ok(Draft {
        account,
        target: issue,
        title: None,
        text: args.text,
    })
}
/// The account a draft card was prepared for, while it is still the connection it was
/// prepared with: a new sign-in (another account, another organization) is not.
fn held(ctx: &Context<'_>, name: &str, generation: i64) -> Result<Account, ApiError> {
    match connected(ctx)? {
        Some(account) if account.name == name && account.generation == generation => {
            // Another bot's activation does not authorize this one.
            if account.enabled {
                Ok(account)
            } else {
                Err(invalid(
                    "Les outils Linear ne sont plus activés pour ce bot : rien n’a été publié.",
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
                    "SELECT account,generation FROM wrapper_linear_drafts WHERE request_id=?1",
                    [&request_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((account, generation)) = bound {
                return held(ctx, &account, generation).map(|_| ());
            }
        }
    }
    parse(ctx, &call.name, &call.arguments).map(|_| ())
}
fn display(tool: &str, draft: &Draft, fingerprint: &str) -> Value {
    if is_create(tool) {
        json!({"brouillon":{
            "service":"Linear","action":"Créer une issue Linear","compte":draft.account,
            "equipe":draft.target,"titre":draft.title,"texte":draft.text,"empreinte":fingerprint,
        }})
    } else {
        json!({"brouillon":{
            "service":"Linear","action":"Commenter une issue Linear","compte":draft.account,
            "issue":draft.target,"texte":draft.text,"empreinte":fingerprint,
        }})
    }
}
/// Called while the approval request is created, in the same transaction: no network.
pub(in super::super::super) fn prepare(
    ctx: &Context<'_>,
    request_id: &str,
    call: &ToolCall,
) -> Result<Value, ApiError> {
    let draft = parse(ctx, &call.name, &call.arguments)?;
    let print = fingerprint(
        &call.name,
        &draft.account,
        &draft.target,
        draft.title.as_deref(),
        &draft.text,
    );
    let (server, generation) = connected(ctx)?
        .map(|account| (account.server.id, account.generation))
        .ok_or_else(|| invalid(&unavailable(ctx)))?;
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_linear_drafts(request_id,tool,server_id,account,generation,target,title,text,fingerprint,call_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![request_id, call.name, server, draft.account, generation, draft.target, draft.title, draft.text, print, call.call_id],
    )?;
    Ok(display(&call.name, &draft, &print))
}
/// The tool call this draft came from: its result is the only one the card may show.
pub(in super::super::super) fn call_id(ctx: &Context<'_>, request_id: &str) -> Option<String> {
    ctx.connection
        .query_row(
            "SELECT call_id FROM wrapper_linear_drafts WHERE request_id=?1",
            [request_id],
            |r| r.get(0),
        )
        .ok()
}
/// What Charles validated replaces the proposal, and the fingerprint follows it.
pub(in super::super::super) fn edit(
    ctx: &Context<'_>,
    request_id: &str,
    title: Option<&str>,
    text: Option<&str>,
) -> Result<(), ApiError> {
    let row: Option<(String, String, String, Option<String>, String, String)> = ctx
        .connection
        .query_row(
            "SELECT tool,account,target,title,text,state FROM wrapper_linear_drafts WHERE request_id=?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let Some((tool, account, target, old_title, old_text, state)) = row else {
        return Err(invalid("Ce brouillon est introuvable."));
    };
    if state != "draft" {
        return Err(ApiError::new(409, "Ce brouillon a déjà été envoyé."));
    }
    let new_title = match (title, is_create(&tool)) {
        (Some(title), true) => Some(title.trim().to_owned()),
        (None, true) => old_title,
        (_, false) => None,
    };
    if let Some(title) = &new_title {
        check_title(title)?;
        check_text(ctx, title, false)?;
    }
    let new_text = text.map_or(old_text, str::to_owned);
    check_text(ctx, &new_text, !is_create(&tool))?;
    let print = fingerprint(&tool, &account, &target, new_title.as_deref(), &new_text);
    ctx.connection.execute(
        "UPDATE wrapper_linear_drafts SET title=?2,text=?3,fingerprint=?4 WHERE request_id=?1 AND state='draft'",
        params![request_id, new_title, new_text, print],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.texte',?2,'$.brouillon.empreinte',?3) WHERE request_id=?1",
        params![request_id, new_text, print],
    )?;
    if let Some(title) = new_title {
        ctx.connection.execute(
            "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.titre',?2) WHERE request_id=?1",
            params![request_id, title],
        )?;
    }
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
        "Linear n’a pas confirmé la réception : l’issue ou le commentaire a peut-être été publié. Vérifiez dans Linear avant d’en préparer un autre.",
    )
}
/// A refusal that is certain: Linear published nothing. Its status says which one.
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
        "UPDATE wrapper_linear_drafts SET state=?2,result=?3 WHERE request_id=?1",
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
    title: Option<String>,
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
        "SELECT tool,server_id,account,generation,target,title,text,fingerprint,state,result FROM wrapper_linear_drafts WHERE request_id=?1",
        [&request_id],
        |r| Ok(Stored { tool: r.get(0)?, server: r.get(1)?, account: r.get(2)?, generation: r.get(3)?, target: r.get(4)?, title: r.get(5)?, text: r.get(6)?, print: r.get(7)?, state: r.get(8)?, result: r.get(9)? }),
    ).optional()?;
    let Some(stored) = row else {
        return Err(invalid("Brouillon introuvable : rien n’a été publié."));
    };
    match stored.state.as_str() {
        "draft" => {}
        // A replay answers with the first result and never calls Linear.
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
                && fingerprint(
                    &stored.tool,
                    &stored.account,
                    &stored.target,
                    stored.title.as_deref(),
                    &stored.text,
                ) == stored.print =>
        {
            account
        }
        other => {
            let error = other.err().unwrap_or_else(|| invalid(CHANGED));
            return fail(ctx, &request_id, Failure::Refused, error);
        }
    };
    // One claim before the call: only the run that wins it may reach Linear.
    let claimed = ctx.connection.execute(
        "UPDATE wrapper_linear_drafts SET state='sending' WHERE request_id=?1 AND state='draft'",
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
fn bound(ctx: &Context<'_>, account: &Account, stored: &Stored) -> Result<String, ApiError> {
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
    credentials(ctx, &account.server.id)?
        .and_then(|env| env.get("ACCESS_TOKEN").cloned())
        .ok_or_else(|| invalid(RECONNECT))
}
/// Whether Linear certainly rejected a mutation, as opposed to an answer that says nothing.
fn rejected(reply: &Reply) -> bool {
    matches!(reply.status, 400 | 401 | 403 | 429)
        || matches!(
            error_code(reply),
            "AUTHENTICATION_ERROR"
                | "FORBIDDEN"
                | "RATELIMITED"
                | "INVALID_INPUT"
                | "BAD_USER_INPUT"
        )
        || reply.body["errors"][0]["extensions"]["userError"] == true
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
    let (document, input, key, what) = if is_create(&stored.tool) {
        // Looking a team up creates nothing: a failure here is a certain "not published".
        let team = find_team(ctx, account, &access, &stored.target).map_err(not_published)?;
        let mut input = json!({"teamId":team.id,"title":stored.title});
        if !stored.text.trim().is_empty() {
            input["description"] = json!(stored.text);
        }
        (
            "mutation($input:IssueCreateInput!){issueCreate(input:$input){success issue{identifier title url team{key}}}}",
            input,
            "issueCreate",
            format!("l’équipe {}", team.key),
        )
    } else {
        let id = issue_id(ctx, account, &access, &stored.target).map_err(not_published)?;
        (
            "mutation($input:CommentCreateInput!){commentCreate(input:$input){success comment{id url}}}",
            json!({"issueId":id,"body":stored.text}),
            "commentCreate",
            format!("l’issue {}", stored.target),
        )
    };
    // Past this point the effect may have reached Linear: only an explicit answer says it did not.
    let reply = graphql(&access, document, json!({"input":input}))
        .map_err(|_| (Failure::Unconfirmed, unconfirmed()))?;
    let created = &reply.body["data"][key];
    if reply.status == 200 && created["success"] == true && reply.body["errors"].is_null() {
        let account_name = &account.name;
        return Ok(if is_create(&stored.tool) {
            let issue = &created["issue"];
            let identifier = name_of(&issue["identifier"]);
            json!({"publie":true,"compte":account_name,"equipe":name_of(&issue["team"]["key"]),
                "identifiant":identifier,"titre":name_of(&issue["title"]),"url":name_of(&issue["url"]),
                "message":format!("Issue {identifier} créée dans Linear sous le compte {account_name}.")})
        } else {
            json!({"publie":true,"compte":account_name,"issue":stored.target,
                "url":name_of(&created["comment"]["url"]),"texte":stored.text,
                "message":format!("Commentaire publié sur l’issue {} dans Linear sous le compte {account_name}.", stored.target)})
        });
    }
    if rejected(&reply) {
        return Err(not_published(refusal(ctx, &account.server, &reply, &what)));
    }
    // An outage, or an answer that says nothing: the effect may exist.
    Err((Failure::Unconfirmed, unconfirmed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_two_write_tools_are_told_from_the_reads() {
        assert!(is_tool(CREATE_TOOL) && is_tool(COMMENT_TOOL));
        assert!(!is_tool("linear_get_issue"));
        assert!(is_create(CREATE_TOOL) && !is_create(COMMENT_TOOL));
    }
    #[test]
    fn a_fingerprint_follows_every_validated_field() {
        let base = fingerprint(CREATE_TOOL, "a · b", "ENG", Some("t"), "x");
        for other in [
            fingerprint(COMMENT_TOOL, "a · b", "ENG", Some("t"), "x"),
            fingerprint(CREATE_TOOL, "c · d", "ENG", Some("t"), "x"),
            fingerprint(CREATE_TOOL, "a · b", "OPS", Some("t"), "x"),
            fingerprint(CREATE_TOOL, "a · b", "ENG", Some("u"), "x"),
            fingerprint(CREATE_TOOL, "a · b", "ENG", Some("t"), "y"),
        ] {
            assert_ne!(base, other);
        }
    }
}
