//! Slack message as a draft card. Nothing leaves before the click: preparing the card
//! calls no provider, and the send is bound to one fingerprint of the validated text,
//! the workspace, the channel and the thread. The row below is the single claim on that
//! send, so a replayed Envoyer or a replayed run never reaches Slack twice.
use super::*;
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};

/// Write tool of the closed list. The local policy keeps it on a confirmation card that no
/// standing rule can cover, whatever its name says.
pub(in super::super::super) const TOOL: &str = "slack_post_message";
const SHORT: &str = "post_message";
/// The limit of `chat.postMessage`: Slack truncates a longer text.
const MAX_TEXT: usize = 40_000;
const ACTION: &str = "Publier un message Slack";
const CHANGED: &str =
    "L’espace Slack a changé depuis l’affichage du brouillon : rien n’a été publié.";

pub(in super::super::super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_slack_drafts (request_id TEXT PRIMARY KEY,server_id TEXT NOT NULL,workspace TEXT NOT NULL,channel TEXT NOT NULL,thread TEXT,text TEXT NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'draft',call_id TEXT NOT NULL DEFAULT '');")?;
    // A send that was in flight when the app stopped may or may not have reached Slack.
    conn.execute(
        "UPDATE wrapper_slack_drafts SET state='uncertain' WHERE state='sending'",
        [],
    )?;
    Ok(())
}

fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "channel":{"type":"string","description":"Nom du canal, avec ou sans #, par exemple general ; ou son identifiant Slack (C…, G…)."},
        "text":{"type":"string","description":"Texte exact du message, que Charles pourra corriger avant l’envoi."},
        "thread":{"type":"string","description":"Horodatage du message parent (par exemple 1760000100.000200) pour répondre dans son fil ; absent pour publier dans le canal."}
    },"required":["channel","text"]})
}
pub(super) fn tool_definition() -> Value {
    json!({
        "name":SHORT,
        "description":"Préparer un message Slack : une carte brouillon attend la validation de Charles, rien n’est publié avant son clic.",
        "inputSchema":schema(),
    })
}
pub(in super::super::super) fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: TOOL.into(),
        description: "Slack · publier un message dans un canal (ou un fil) avec le compte Slack connecté. Une carte brouillon montre l’espace, la cible, le fil et le texte, que Charles peut corriger ; rien n’est publié avant son clic sur Envoyer, et un refus ne publie rien. Le résultat dit quel texte a été publié.".into(),
        schema: schema(),
        requires_approval: true,
    }
}

struct Draft {
    workspace: String,
    /// The channel as asked, without `#`: a name or an identifier.
    channel: String,
    thread: Option<String>,
    text: String,
}
impl Draft {
    /// How the card and the result name the target.
    fn target(&self) -> String {
        target(&self.channel)
    }
}
fn target(channel: &str) -> String {
    if is_identifier(channel) {
        channel.to_owned()
    } else {
        format!("#{}", clean(channel, 80))
    }
}
fn fingerprint(workspace: &str, channel: &str, thread: Option<&str>, text: &str) -> String {
    let bytes = serde_json::to_vec(&json!([workspace, channel, thread, text])).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}
fn check_text(ctx: &Context<'_>, text: &str) -> Result<(), ApiError> {
    if text.trim().is_empty() {
        return Err(invalid(
            "Le message est vide : écrivez un texte avant d’envoyer.",
        ));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(invalid(
            "Le message dépasse 40 000 caractères, la limite de Slack.",
        ));
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid("Le message contient des caractères de contrôle."));
    }
    // The card shows the stored text: a value masked there would not be what leaves.
    let shown = redact_value(&crate::features::forms::vault(ctx)?, &json!(text))?;
    if shown != json!(text) {
        return Err(invalid(
            "Le message contient une valeur privée du coffre : il n’a pas été préparé.",
        ));
    }
    Ok(())
}
/// A Slack timestamp (`1760000100.000200`): the message a reply hangs from.
fn is_timestamp(text: &str) -> bool {
    text.split_once('.').is_some_and(|(seconds, micros)| {
        (1..=12).contains(&seconds.len())
            && (1..=9).contains(&micros.len())
            && seconds
                .bytes()
                .chain(micros.bytes())
                .all(|b| b.is_ascii_digit())
    })
}
fn parse(ctx: &Context<'_>, arguments: &Value) -> Result<Draft, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        channel: String,
        text: String,
        thread: Option<String>,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
        invalid("Arguments invalides : indiquez le canal Slack et le texte du message.")
    })?;
    let channel = args.channel.trim().trim_start_matches('#').trim();
    if channel.is_empty() || channel.len() > 100 || channel.chars().any(char::is_control) {
        return Err(invalid("Indiquez le nom du canal Slack où publier."));
    }
    let thread = args
        .thread
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    if thread.is_some_and(|t| !is_timestamp(t)) {
        return Err(invalid(
            "Fil invalide : indiquez l’horodatage du message parent, par exemple 1760000100.000200.",
        ));
    }
    check_text(ctx, &args.text)?;
    let account = match connected(ctx)? {
        Some(account) if account.enabled => account,
        _ => return Err(invalid(&unavailable(ctx))),
    };
    Ok(Draft {
        workspace: account.workspace,
        channel: channel.to_owned(),
        thread: thread.map(str::to_owned),
        text: args.text,
    })
}
/// The workspace a draft card was prepared for, while it is still the connected one.
fn held(ctx: &Context<'_>, workspace: &str) -> Result<Account, ApiError> {
    match connected(ctx)? {
        Some(account) if account.workspace == workspace => {
            // Another bot's activation does not authorize this one.
            if account.enabled {
                Ok(account)
            } else {
                Err(invalid(
                    "Les outils Slack ne sont plus activés pour ce bot : rien n’a été publié.",
                ))
            }
        }
        _ => Err(invalid(CHANGED)),
    }
}

/// Checked before any card: a bad target or a missing workspace never opens one.
pub(in super::super::super) fn validate(
    ctx: &Context<'_>,
    call: &ToolCall,
) -> Result<(), ApiError> {
    // A draft that already exists is bound to its workspace: resuming it must not parse the
    // original call again.
    if let Some(run) = ctx.run_id {
        if let Ok((request_id, _)) = crate::features::forms::approval_identity(run, call) {
            let bound: Option<String> = ctx
                .connection
                .query_row(
                    "SELECT workspace FROM wrapper_slack_drafts WHERE request_id=?1",
                    [&request_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(workspace) = bound {
                return held(ctx, &workspace).map(|_| ());
            }
        }
    }
    parse(ctx, &call.arguments).map(|_| ())
}
fn display(draft: &Draft, fingerprint: &str) -> Value {
    json!({"brouillon":{
        "service":"Slack","action":ACTION,"espace":draft.workspace,"cible":draft.target(),
        "fil":draft.thread,"texte":draft.text,"empreinte":fingerprint,
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
        &draft.workspace,
        &draft.channel,
        draft.thread.as_deref(),
        &draft.text,
    );
    let server = connected(ctx)?
        .map(|account| account.server.id)
        .ok_or_else(|| invalid(&unavailable(ctx)))?;
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_slack_drafts(request_id,server_id,workspace,channel,thread,text,fingerprint,call_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![request_id, server, draft.workspace, draft.channel, draft.thread, draft.text, print, call.call_id],
    )?;
    Ok(display(&draft, &print))
}
/// The tool call this draft came from: its result is the only one the card may show.
pub(in super::super::super) fn call_id(ctx: &Context<'_>, request_id: &str) -> Option<String> {
    ctx.connection
        .query_row(
            "SELECT call_id FROM wrapper_slack_drafts WHERE request_id=?1",
            [request_id],
            |r| r.get(0),
        )
        .ok()
}
/// The text Charles validated replaces the proposal, and the fingerprint follows it.
pub(in super::super::super) fn edit(
    ctx: &Context<'_>,
    request_id: &str,
    text: &str,
) -> Result<(), ApiError> {
    check_text(ctx, text)?;
    let row: Option<(String, String, Option<String>, String)> = ctx
        .connection
        .query_row(
            "SELECT workspace,channel,thread,state FROM wrapper_slack_drafts WHERE request_id=?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((workspace, channel, thread, state)) = row else {
        return Err(invalid("Ce brouillon est introuvable."));
    };
    if state != "draft" {
        return Err(ApiError::new(409, "Ce brouillon a déjà été envoyé."));
    }
    let print = fingerprint(&workspace, &channel, thread.as_deref(), text);
    ctx.connection.execute(
        "UPDATE wrapper_slack_drafts SET text=?2,fingerprint=?3 WHERE request_id=?1 AND state='draft'",
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
        "Slack n’a pas confirmé la réception : le message a peut-être été publié. Vérifiez dans Slack avant d’en préparer un autre.",
    )
}
/// A refusal that is certain: Slack did not publish. Its status says which one.
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
fn settle(ctx: &Context<'_>, request_id: &str, state: &str, etat: &str) -> Result<(), ApiError> {
    ctx.connection.execute(
        "UPDATE wrapper_slack_drafts SET state=?2 WHERE request_id=?1",
        params![request_id, state],
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
    settle(ctx, request_id, failure.state(), failure.label())?;
    Err(error)
}
fn published(workspace: &str, channel: &str, thread: Option<&str>, text: &str) -> Value {
    let target = target(channel);
    let place = match thread {
        Some(thread) => format!("{target} (dans le fil {thread})"),
        None => target,
    };
    json!({"publie":true,"compte":workspace,"canal":channel,"fil":thread,"texte":text,
        "message":format!("Message publié dans {place} de l’espace Slack {workspace}.")})
}
struct Stored {
    server: String,
    workspace: String,
    channel: String,
    thread: Option<String>,
    text: String,
    print: String,
    state: String,
}
fn publish(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let run = ctx.run_id.ok_or_else(|| {
        invalid("Rien n’a été publié : cette demande n’appartient à aucune exécution.")
    })?;
    let (request_id, _) = crate::features::forms::approval_identity(run, call)?;
    let row: Option<Stored> = ctx.connection.query_row(
        "SELECT server_id,workspace,channel,thread,text,fingerprint,state FROM wrapper_slack_drafts WHERE request_id=?1",
        [&request_id],
        |r| Ok(Stored { server: r.get(0)?, workspace: r.get(1)?, channel: r.get(2)?, thread: r.get(3)?, text: r.get(4)?, print: r.get(5)?, state: r.get(6)? }),
    ).optional()?;
    let Some(Stored {
        server,
        workspace,
        channel,
        thread,
        text,
        print,
        state,
    }) = row
    else {
        return Err(invalid("Brouillon introuvable : rien n’a été publié."));
    };
    match state.as_str() {
        "draft" => {}
        // A replay answers with the first result and never calls Slack.
        "sent" => return Ok(published(&workspace, &channel, thread.as_deref(), &text)),
        "failed" => {
            return Err(invalid(
                "Ce brouillon n’a pas été envoyé : rien n’a été publié. Demandez au bot de le préparer à nouveau.",
            ))
        }
        _ => return Err(unconfirmed()),
    }
    let account = match held(ctx, &workspace) {
        Ok(account)
            if account.server.id == server
                && fingerprint(&workspace, &channel, thread.as_deref(), &text) == print =>
        {
            account
        }
        other => {
            let error = other.err().unwrap_or_else(|| invalid(CHANGED));
            return fail(ctx, &request_id, Failure::Refused, error);
        }
    };
    // One claim before the call: only the run that wins it may reach Slack.
    let claimed = ctx.connection.execute(
        "UPDATE wrapper_slack_drafts SET state='sending' WHERE request_id=?1 AND state='draft'",
        [&request_id],
    )?;
    if claimed != 1 {
        return Err(unconfirmed());
    }
    match send(
        ctx,
        &account,
        &workspace,
        &channel,
        thread.as_deref(),
        &text,
    ) {
        Ok(()) => {
            settle(ctx, &request_id, "sent", "sent")?;
            Ok(published(&workspace, &channel, thread.as_deref(), &text))
        }
        Err((failure, error)) => fail(ctx, &request_id, failure, error),
    }
}
/// Under the sign-in lock, the credentials in use are still those of the approved workspace.
fn bound(ctx: &Context<'_>, account: &Account, workspace: &str) -> Result<String, ApiError> {
    let same: Option<String> = ctx
        .connection
        .query_row(
            "SELECT account_email FROM wrapper_connector_catalogue WHERE server_id=?1 AND phase='connected'",
            [&account.server.id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    if same.as_deref() != Some(workspace) {
        return Err(invalid(CHANGED));
    }
    credentials(ctx, &account.server.id)?
        .and_then(|env| env.get("ACCESS_TOKEN").cloned())
        .ok_or_else(|| invalid(RECONNECT))
}
fn send(
    ctx: &Context<'_>,
    account: &Account,
    workspace: &str,
    channel: &str,
    thread: Option<&str>,
    text: &str,
) -> Result<(), (Failure, ApiError)> {
    let m = manifest("slack").map_err(not_published)?;
    super::super::ensure(ctx, &account.server).map_err(not_published)?;
    // A sign-in replaces the token under this lock: the send below keeps the one it took.
    let access = {
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        bound(ctx, account, workspace).map_err(not_published)?
    };
    // Looking a name up posts nothing: a failure here is a certain "not published".
    let id = channel_id(ctx, account, m, &access, channel).map_err(not_published)?;
    let mut body = json!({"channel":id,"text":text});
    if let Some(thread) = thread {
        body["thread_ts"] = json!(thread);
    }
    let label = target(channel);
    let url = format!("{}/chat.postMessage", endpoint(m, "api"));
    // Past this point the message may have reached Slack: only an explicit answer says it did not.
    let reply = reply_of(
        agent()
            .post(&url)
            .set("Authorization", &format!("Bearer {access}"))
            .set("Accept", "application/json")
            .send_json(&body),
    )
    .map_err(|_| (Failure::Unconfirmed, unconfirmed()))?;
    if reply.status == 200 && reply.body["ok"] == true {
        return Ok(());
    }
    let code = reply.body["error"].as_str().unwrap_or("");
    match code {
        _ if reply.status == 429 => {
            Err(not_published(refusal(ctx, &account.server, &reply, &label)))
        }
        "ratelimited" | "invalid_auth" | "not_authed" | "token_revoked" | "token_expired"
        | "account_inactive" | "missing_scope" => {
            Err(not_published(refusal(ctx, &account.server, &reply, &label)))
        }
        "channel_not_found" => Err(not_published(invalid(&format!(
            "Slack ne trouve pas le canal {label} avec ce compte."
        )))),
        "not_in_channel" => Err(not_published(invalid(&format!(
            "Vous n’êtes pas membre du canal {label}. Rejoignez-le dans Slack, puis réessayez."
        )))),
        "is_archived" => Err(not_published(invalid(&format!(
            "Le canal {label} est archivé : Slack n’y accepte plus de message."
        )))),
        "msg_too_long" | "no_text" | "restricted_action" | "access_denied" | "no_permission"
        | "thread_not_found" | "invalid_arguments" | "invalid_arg_name" | "invalid_form_data"
        | "invalid_charset" | "invalid_post_type" | "missing_post_type" => {
            let code: String = code
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(40)
                .collect();
            Err(not_published(invalid(&format!(
                "Slack a refusé ce message ({code})."
            ))))
        }
        // An outage, or an answer that says nothing: the message may exist.
        _ => Err((Failure::Unconfirmed, unconfirmed())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn threads_are_slack_timestamps() {
        assert!(is_timestamp("1760000100.000200"));
        assert!(!is_timestamp("1760000100"));
        assert!(!is_timestamp("pas-un-fil"));
        assert!(!is_timestamp(".000200"));
        assert!(!is_timestamp("1760000100."));
        assert!(!is_timestamp("17600001000000.000200"));
    }
    #[test]
    fn a_channel_is_named_by_hash_or_identifier() {
        assert_eq!(target("general"), "#general");
        assert_eq!(target("C0GENERAL"), "C0GENERAL");
    }
}
