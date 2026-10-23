//! An Outlook mail as a draft card. Nothing leaves before the click: preparing a card calls
//! no provider, and the send is bound to one fingerprint of the validated subject, body,
//! recipients and account. The row below is the single claim on that send, so a replayed
//! Envoyer or a replayed run never reaches Microsoft twice. A send that Microsoft refused,
//! limited or did not acknowledge is never reported as a mail that left.
use super::super::gmail::email::{check_body, check_subject, recipients, MAX_RECIPIENTS};
use super::*;
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};

/// Write tool of the closed list. The local policy keeps it on a confirmation card that no
/// standing rule can cover, whatever its name says.
pub(in super::super::super) const SEND_TOOL: &str = "outlook_send_mail";
const ACTION: &str = "Envoyer un courriel";
const CHANGED: &str =
    "Le compte Microsoft 365 a changé depuis l’affichage du brouillon : rien n’a été envoyé.";

pub(in super::super::super) fn is_tool(name: &str) -> bool {
    name == SEND_TOOL
}

pub(in super::super::super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_outlook_drafts (request_id TEXT PRIMARY KEY,server_id TEXT NOT NULL,account TEXT NOT NULL,generation INTEGER NOT NULL,to_json TEXT NOT NULL,cc_json TEXT NOT NULL,subject TEXT NOT NULL,body TEXT NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'draft',call_id TEXT NOT NULL DEFAULT '',result TEXT);")?;
    // A send that was in flight when the app stopped may or may not have reached Microsoft.
    conn.execute(
        "UPDATE wrapper_outlook_drafts SET state='uncertain' WHERE state='sending'",
        [],
    )?;
    Ok(())
}

fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "to":{"type":"array","minItems":1,"items":{"type":"string"},"description":"Adresses des destinataires (À), une adresse seule par entrée. Écrivez « moi » pour le compte Microsoft 365 connecté."},
        "cc":{"type":"array","items":{"type":"string"},"description":"Adresses en copie (Cc), si besoin."},
        "subject":{"type":"string","description":"Objet exact du courriel, que Charles pourra corriger avant l’envoi."},
        "body":{"type":"string","description":"Corps exact du courriel, en texte, que Charles pourra corriger avant l’envoi."}
    },"required":["to","subject"]})
}
pub(super) fn tool_definitions() -> Vec<Value> {
    vec![json!({
        "name":"send_mail",
        "description":"Préparer un courriel Outlook : une carte brouillon attend la validation de Charles, rien n’est envoyé avant son clic.",
        "inputSchema":schema(),
    })]
}
pub(super) fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: SEND_TOOL.into(),
        description: "Outlook · préparer un courriel avec le compte Microsoft 365 connecté. Une carte brouillon montre De, À, Cc, Objet et Corps, que Charles peut corriger ; rien n’est envoyé avant son clic sur Envoyer, et Annuler n’envoie rien. Le résultat dit quel objet et quel corps sont partis. Si l’envoi est refusé, limité ou sans accusé de réception, le courriel n’est pas considéré comme parti : dites-le tel quel.".into(),
        schema: schema(),
        requires_approval: true,
    }]
}

struct Draft {
    account: String,
    to: Vec<String>,
    cc: Vec<String>,
    subject: String,
    body: String,
}
fn fingerprint(account: &str, to: &[String], cc: &[String], subject: &str, body: &str) -> String {
    let bytes =
        serde_json::to_vec(&json!(["outlook", account, to, cc, subject, body])).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}
fn parse(ctx: &Context<'_>, arguments: &Value) -> Result<Draft, ApiError> {
    let account = match connected(ctx)? {
        Some(account) if account.enabled => account.name,
        _ => return Err(invalid(&unavailable(ctx))),
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        to: Vec<String>,
        #[serde(default)]
        cc: Vec<String>,
        subject: String,
        #[serde(default)]
        body: String,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
        invalid("Arguments invalides : indiquez les destinataires (une adresse par entrée), l’objet et le corps du courriel.")
    })?;
    check_subject(ctx, &args.subject)?;
    check_body(ctx, &args.body)?;
    let to = recipients(ctx, &args.to, &account)?;
    let cc: Vec<String> = recipients(ctx, &args.cc, &account)?
        .into_iter()
        .filter(|one| !to.iter().any(|seen| seen.eq_ignore_ascii_case(one)))
        .collect();
    if to.is_empty() {
        return Err(invalid("Indiquez au moins un destinataire."));
    }
    if to.len() + cc.len() > MAX_RECIPIENTS {
        return Err(invalid("Vingt destinataires au plus par courriel."));
    }
    Ok(Draft {
        account,
        to,
        cc,
        subject: args.subject,
        body: args.body,
    })
}
/// The account a draft card was prepared for, while it is still the connection it was
/// prepared with: a new sign-in (another account) is not.
fn held(ctx: &Context<'_>, name: &str, generation: i64) -> Result<Account, ApiError> {
    match connected(ctx)? {
        Some(account) if account.name == name && account.generation == generation => {
            // Another bot's activation does not authorize this one.
            if account.enabled {
                Ok(account)
            } else {
                Err(invalid(
                    "Les outils Microsoft 365 ne sont plus activés pour ce bot : rien n’a été envoyé.",
                ))
            }
        }
        _ => Err(invalid(CHANGED)),
    }
}

/// Checked before any card: a bad address or a missing account never opens one.
pub(in super::super::super) fn validate(
    ctx: &Context<'_>,
    call: &ToolCall,
) -> Result<(), ApiError> {
    // A draft that already exists is bound to its account: resuming it must not parse the
    // original call again.
    if let Some(run) = ctx.run_id {
        if let Ok((request_id, _)) =
            crate::features::forms::approval_identity_in(ctx.connection, run, call)
        {
            let bound: Option<(String, i64)> = ctx
                .connection
                .query_row(
                    "SELECT account,generation FROM wrapper_outlook_drafts WHERE request_id=?1",
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
        "service":"Microsoft 365","action":ACTION,"de":draft.account,"a":draft.to,"cc":draft.cc,
        "objet":draft.subject,"compte":draft.account,"texte":draft.body,"empreinte":fingerprint,
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
        &draft.account,
        &draft.to,
        &draft.cc,
        &draft.subject,
        &draft.body,
    );
    let (server, generation) = connected(ctx)?
        .map(|account| (account.server.id, account.generation))
        .ok_or_else(|| invalid(&unavailable(ctx)))?;
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_outlook_drafts(request_id,server_id,account,generation,to_json,cc_json,subject,body,fingerprint,call_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![request_id, server, draft.account, generation, json!(draft.to).to_string(), json!(draft.cc).to_string(), draft.subject, draft.body, print, call.call_id],
    )?;
    Ok(display(&draft, &print))
}
/// The tool call this draft came from: its result is the only one the card may show.
pub(in super::super::super) fn call_id(ctx: &Context<'_>, request_id: &str) -> Option<String> {
    ctx.connection
        .query_row(
            "SELECT call_id FROM wrapper_outlook_drafts WHERE request_id=?1",
            [request_id],
            |r| r.get(0),
        )
        .ok()
}
fn list(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}
/// The subject and the body Charles validated replace the proposal, and the fingerprint
/// follows. The recipients and the account are the ones of the card.
pub(in super::super::super) fn edit(
    ctx: &Context<'_>,
    request_id: &str,
    subject: Option<&str>,
    body: Option<&str>,
) -> Result<(), ApiError> {
    let row: Option<(String, String, String, String, String, String)> = ctx
        .connection
        .query_row(
            "SELECT account,to_json,cc_json,state,subject,body FROM wrapper_outlook_drafts WHERE request_id=?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let Some((account, to, cc, state, kept_subject, kept_body)) = row else {
        return Err(invalid("Ce brouillon est introuvable."));
    };
    if state != "draft" {
        return Err(ApiError::new(409, "Ce brouillon a déjà été envoyé."));
    }
    let subject = subject.unwrap_or(&kept_subject);
    let body = body.unwrap_or(&kept_body);
    check_subject(ctx, subject)?;
    check_body(ctx, body)?;
    let print = fingerprint(&account, &list(&to), &list(&cc), subject, body);
    ctx.connection.execute(
        "UPDATE wrapper_outlook_drafts SET subject=?2,body=?3,fingerprint=?4 WHERE request_id=?1 AND state='draft'",
        params![request_id, subject, body, print],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.objet',?2,'$.brouillon.texte',?3,'$.brouillon.empreinte',?4) WHERE request_id=?1",
        params![request_id, subject, body, print],
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
        "Envoi non confirmé · l’accusé de réception de Microsoft manque : le courriel est peut-être parti. Vérifiez les éléments envoyés du compte Outlook avant d’en préparer un autre.",
    )
}
/// A refusal that is certain: Microsoft sent nothing. Its status says which one.
fn not_sent(error: ApiError) -> (Failure, ApiError) {
    let failure = if error.status == 429 {
        Failure::RateLimited
    } else {
        Failure::Refused
    };
    (
        failure,
        ApiError::new(
            error.status,
            format!("{} Aucun courriel n’est parti.", error.message),
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
        "UPDATE wrapper_outlook_drafts SET state=?2,result=?3 WHERE request_id=?1",
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
    server: String,
    account: String,
    generation: i64,
    to: Vec<String>,
    cc: Vec<String>,
    subject: String,
    body: String,
    print: String,
    state: String,
    result: Option<String>,
}
fn publish(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let run = ctx.run_id.ok_or_else(|| {
        invalid("Rien n’a été envoyé : cette demande n’appartient à aucune exécution.")
    })?;
    let (request_id, _) = crate::features::forms::approval_identity_in(ctx.connection, run, call)?;
    let row: Option<Stored> = ctx.connection.query_row(
        "SELECT server_id,account,generation,to_json,cc_json,subject,body,fingerprint,state,result FROM wrapper_outlook_drafts WHERE request_id=?1",
        [&request_id],
        |r| Ok(Stored { server: r.get(0)?, account: r.get(1)?, generation: r.get(2)?, to: list(&r.get::<_, String>(3)?), cc: list(&r.get::<_, String>(4)?), subject: r.get(5)?, body: r.get(6)?, print: r.get(7)?, state: r.get(8)?, result: r.get(9)? }),
    ).optional()?;
    let Some(stored) = row else {
        return Err(invalid("Brouillon introuvable : rien n’a été envoyé."));
    };
    match stored.state.as_str() {
        "draft" => {}
        // A replay answers with the first result and never calls Microsoft.
        "sent" => {
            return stored
                .result
                .and_then(|result| serde_json::from_str(&result).ok())
                .ok_or_else(unconfirmed)
        }
        "failed" => {
            return Err(invalid(
                "Ce courriel n’a pas été envoyé : la carte est close. Demandez au bot d’en préparer un nouveau.",
            ))
        }
        _ => return Err(unconfirmed()),
    }
    let account = match held(ctx, &stored.account, stored.generation) {
        Ok(account)
            if account.server.id == stored.server
                && fingerprint(
                    &stored.account,
                    &stored.to,
                    &stored.cc,
                    &stored.subject,
                    &stored.body,
                ) == stored.print =>
        {
            account
        }
        other => {
            let error = other.err().unwrap_or_else(|| invalid(CHANGED));
            return fail(ctx, &request_id, Failure::Refused, error);
        }
    };
    // One claim before the call: only the run that wins it may reach Microsoft.
    let claimed = ctx.connection.execute(
        "UPDATE wrapper_outlook_drafts SET state='sending' WHERE request_id=?1 AND state='draft'",
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
/// Whether Microsoft certainly rejected the send, as opposed to an answer that says nothing:
/// a refusal of the request is not a timeout, and a 5xx may hide a mail that left.
fn rejected(reply: &Reply) -> bool {
    (400..500).contains(&reply.status) && reply.status != 408
}
fn recipient_list(addresses: &[String]) -> Vec<Value> {
    addresses
        .iter()
        .map(|address| json!({"emailAddress":{"address":address}}))
        .collect()
}
fn send(
    ctx: &Context<'_>,
    account: &Account,
    stored: &Stored,
) -> Result<Value, (Failure, ApiError)> {
    super::super::ensure(ctx, &account.server).map_err(not_sent)?;
    // A sign-in replaces the token under this lock: the send below keeps the one it took.
    let access = {
        let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
        bound(ctx, account, stored).map_err(not_sent)?
    };
    let message = json!({
        "message":{
            "subject":stored.subject,
            "body":{"contentType":"Text","content":stored.body},
            "toRecipients":recipient_list(&stored.to),
            "ccRecipients":recipient_list(&stored.cc),
        },
        "saveToSentItems":true,
    });
    // Past this point the mail may have left: only an explicit answer says it did not.
    let reply = post(&access, "/me/sendMail", message)
        .map_err(|_| (Failure::Unconfirmed, unconfirmed()))?;
    // Accepted: it left, so it must not be sent again.
    if reply.status == 202 {
        let recipients = stored.to.join(", ");
        return Ok(
            json!({"publie":true,"compte":stored.account,"a":stored.to,"cc":stored.cc,
            "objet":stored.subject,"texte":stored.body,
            "message":format!("Courriel envoyé à {recipients} sous le compte {} : « {} ».", stored.account, stored.subject)}),
        );
    }
    if rejected(&reply) {
        return Err(not_sent(refusal(
            ctx,
            account,
            &reply,
            "l’envoi du courriel",
        )));
    }
    // An outage, or an answer that says nothing: the mail may exist.
    Err((Failure::Unconfirmed, unconfirmed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_send_tool_is_told_from_the_reads() {
        assert!(is_tool(SEND_TOOL));
        assert!(!is_tool("outlook_list_events"));
    }
    #[test]
    fn a_fingerprint_follows_every_validated_field() {
        let to = vec!["a@exemple.com".to_owned()];
        let cc = vec![];
        let base = fingerprint("me@x.com", &to, &cc, "s", "b");
        for other in [
            fingerprint("you@x.com", &to, &cc, "s", "b"),
            fingerprint("me@x.com", &["c@exemple.com".to_owned()], &cc, "s", "b"),
            fingerprint("me@x.com", &to, &["d@exemple.com".to_owned()], "s", "b"),
            fingerprint("me@x.com", &to, &cc, "t", "b"),
            fingerprint("me@x.com", &to, &cc, "s", "c"),
        ] {
            assert_ne!(base, other);
        }
    }
    #[test]
    fn only_a_certain_client_refusal_proves_nothing_left() {
        let reply = |status| Reply {
            status,
            retry_after: None,
            body: Value::Null,
        };
        for status in [400, 401, 403, 404, 429] {
            assert!(rejected(&reply(status)), "{status}");
        }
        for status in [408, 500, 502, 503, 504] {
            assert!(!rejected(&reply(status)), "{status}");
        }
    }
}
