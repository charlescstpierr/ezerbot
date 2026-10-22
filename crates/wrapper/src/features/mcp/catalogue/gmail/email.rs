//! Email as a draft card, on the table and the rules of the GitHub drafts: preparing the
//! card calls no provider, the send is bound to one fingerprint of the validated subject,
//! body, recipients and account, and the row is the single claim on that send, so a
//! replayed Envoyer or a replayed run never reaches Gmail twice. A send that Google
//! refused, rate limited or did not acknowledge is never reported as a mail that left.
use super::*;
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};

/// Write tool of the closed list. The local policy keeps it on a confirmation card that no
/// standing rule can cover, whatever its name says.
pub(in super::super::super) const TOOL: &str = "gmail_send_email";
const MAX_SUBJECT: usize = 255;
const MAX_BODY: usize = 65_536;
const MAX_RECIPIENTS: usize = 20;
const ACTION: &str = "Envoyer un courriel";
/// What the model writes for the connected account, which it may not know the address of.
const MYSELF: &str = "moi";

pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_gmail_drafts (request_id TEXT PRIMARY KEY,login TEXT NOT NULL,to_json TEXT NOT NULL,cc_json TEXT NOT NULL,subject TEXT NOT NULL,body TEXT NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'draft',message_id TEXT,call_id TEXT NOT NULL DEFAULT '');")?;
    // A send that was in flight when the app stopped may or may not have reached Gmail.
    conn.execute(
        "UPDATE wrapper_gmail_drafts SET state='unconfirmed' WHERE state='sending'",
        [],
    )?;
    Ok(())
}

fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "to":{"type":"array","minItems":1,"items":{"type":"string"},"description":"Adresses des destinataires (À), une adresse seule par entrée. Écrivez « moi » pour le compte Gmail connecté."},
        "cc":{"type":"array","items":{"type":"string"},"description":"Adresses en copie (Cc), si besoin."},
        "subject":{"type":"string","description":"Objet exact du courriel, que Charles pourra corriger avant l’envoi."},
        "body":{"type":"string","description":"Corps exact du courriel, en texte, que Charles pourra corriger avant l’envoi."}
    },"required":["to","subject"]})
}
pub(super) fn tool_definition() -> Value {
    json!({
        "name":"send_email",
        "description":"Préparer un courriel Gmail : une carte brouillon attend la validation de Charles, rien n’est envoyé avant son clic.",
        "inputSchema":schema(),
    })
}
pub(super) fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: TOOL.into(),
        description: "Gmail · préparer un courriel avec le compte Gmail connecté. Une carte brouillon montre De, À, Cc, Objet et Corps, que Charles peut corriger ; rien n’est envoyé avant son clic sur Envoyer, et Annuler n’envoie rien. Le résultat dit quel objet et quel corps sont partis. Si l’envoi est refusé, limité ou sans accusé de réception, le courriel n’est pas considéré comme parti : dites-le tel quel.".into(),
        schema: schema(),
        requires_approval: true,
    }
}

struct Draft {
    login: String,
    to: Vec<String>,
    cc: Vec<String>,
    subject: String,
    body: String,
}
fn fingerprint(login: &str, to: &[String], cc: &[String], subject: &str, body: &str) -> String {
    let bytes =
        serde_json::to_vec(&json!(["email", login, to, cc, subject, body])).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}
/// The card shows the stored text: a value masked there would not be what leaves.
fn unmasked(ctx: &Context<'_>, text: &str, what: &str) -> Result<(), ApiError> {
    let shown = redact_value(&crate::features::forms::vault(ctx)?, &json!(text))?;
    if shown != json!(text) {
        return Err(invalid(&format!(
            "{what} contient une valeur privée du coffre : le courriel n’a pas été préparé."
        )));
    }
    Ok(())
}
fn check_subject(ctx: &Context<'_>, subject: &str) -> Result<(), ApiError> {
    if subject.trim().is_empty() {
        return Err(invalid(
            "L’objet est vide : écrivez un objet avant d’envoyer.",
        ));
    }
    if subject.chars().count() > MAX_SUBJECT {
        return Err(invalid("L’objet dépasse 255 caractères."));
    }
    if subject.chars().any(char::is_control) {
        return Err(invalid(
            "L’objet tient sur une seule ligne, sans caractère de contrôle.",
        ));
    }
    unmasked(ctx, subject, "L’objet")
}
fn check_body(ctx: &Context<'_>, body: &str) -> Result<(), ApiError> {
    if body.chars().count() > MAX_BODY {
        return Err(invalid("Le corps dépasse 65 536 caractères."));
    }
    if body
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid("Le corps contient des caractères de contrôle."));
    }
    unmasked(ctx, body, "Le corps")
}
/// A bare address: no display name, no list, nothing a header could be split on.
fn address(text: &str, login: &str) -> Result<String, ApiError> {
    let text = text.trim();
    if text.eq_ignore_ascii_case(MYSELF) {
        return Ok(login.to_owned());
    }
    let valid = text.len() <= 254
        && text.is_ascii()
        && !text.bytes().any(|b| {
            b.is_ascii_control() || b.is_ascii_whitespace() || b"<>,;:\"'()[]\\".contains(&b)
        })
        && text.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && !domain.contains('@')
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains("..")
        });
    if valid {
        Ok(text.to_owned())
    } else {
        Err(invalid(&format!(
            "Adresse invalide : « {} ». Écrivez l’adresse seule, par exemple nom@exemple.com.",
            text.chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect::<String>()
        )))
    }
}
fn recipients(entries: &[String], login: &str) -> Result<Vec<String>, ApiError> {
    let mut found: Vec<String> = Vec::new();
    for entry in entries {
        let one = address(entry, login)?;
        if !found.iter().any(|seen| seen.eq_ignore_ascii_case(&one)) {
            found.push(one);
        }
    }
    Ok(found)
}
fn parse(ctx: &Context<'_>, arguments: &Value) -> Result<Draft, ApiError> {
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
    let account = usable(ctx)?;
    let to = recipients(&args.to, &account.email)?;
    let cc: Vec<String> = recipients(&args.cc, &account.email)?
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
        login: account.email,
        to,
        cc,
        subject: args.subject,
        body: args.body,
    })
}
fn changed() -> ApiError {
    invalid("Le compte Gmail a changé depuis l’affichage du brouillon : rien n’a été envoyé.")
}
/// The account the draft was made for, still connected and still given to the bot.
fn held(ctx: &Context<'_>, login: &str) -> Result<Account, ApiError> {
    let account = usable(ctx)?;
    if account.email.eq_ignore_ascii_case(login) {
        Ok(account)
    } else {
        Err(changed())
    }
}

/// Checked before any card: a bad address or a missing account never opens one.
pub(in super::super::super) fn validate(
    ctx: &Context<'_>,
    call: &ToolCall,
) -> Result<(), ApiError> {
    // A draft that already exists is bound to its account: resuming it does not read
    // the arguments of the original call again.
    if let Some(run) = ctx.run_id {
        if let Ok((request_id, _)) = crate::features::forms::approval_identity(run, call) {
            let bound: Option<String> = ctx
                .connection
                .query_row(
                    "SELECT login FROM wrapper_gmail_drafts WHERE request_id=?1",
                    [&request_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(login) = bound {
                return held(ctx, &login).map(|_| ());
            }
        }
    }
    parse(ctx, &call.arguments).map(|_| ())
}
fn display(draft: &Draft, fingerprint: &str) -> Value {
    json!({"brouillon":{
        "service":"Gmail","action":ACTION,"de":draft.login,"a":draft.to,"cc":draft.cc,
        "objet":draft.subject,"login":draft.login,"texte":draft.body,"empreinte":fingerprint,
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
        &draft.login,
        &draft.to,
        &draft.cc,
        &draft.subject,
        &draft.body,
    );
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_gmail_drafts(request_id,login,to_json,cc_json,subject,body,fingerprint,call_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![request_id, draft.login, json!(draft.to).to_string(), json!(draft.cc).to_string(), draft.subject, draft.body, print, call.call_id],
    )?;
    Ok(display(&draft, &print))
}
/// The tool call this draft came from: its result is the only one the card may show.
pub(in super::super::super) fn call_id(ctx: &Context<'_>, request_id: &str) -> Option<String> {
    ctx.connection
        .query_row(
            "SELECT call_id FROM wrapper_gmail_drafts WHERE request_id=?1",
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
    let row: Option<(String, String, String, String, String, String)> = ctx.connection.query_row(
        "SELECT login,to_json,cc_json,state,subject,body FROM wrapper_gmail_drafts WHERE request_id=?1",
        [request_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    ).optional()?;
    let Some((login, to, cc, state, kept_subject, kept_body)) = row else {
        return Err(invalid("Ce brouillon est introuvable."));
    };
    if state != "draft" {
        return Err(ApiError::new(409, "Ce brouillon a déjà été envoyé."));
    }
    let subject = subject.unwrap_or(&kept_subject);
    let body = body.unwrap_or(&kept_body);
    check_subject(ctx, subject)?;
    check_body(ctx, body)?;
    let print = fingerprint(&login, &list(&to), &list(&cc), subject, body);
    ctx.connection.execute(
        "UPDATE wrapper_gmail_drafts SET subject=?2,body=?3,fingerprint=?4 WHERE request_id=?1 AND state='draft'",
        params![request_id, subject, body, print],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.objet',?2,'$.brouillon.texte',?3,'$.brouillon.empreinte',?4) WHERE request_id=?1",
        params![request_id, subject, body, print],
    )?;
    Ok(())
}

pub(in super::super::super) fn call(ctx: &Context<'_>, call: &ToolCall) -> ToolOutcome {
    match send(ctx, call) {
        Ok(output) => ToolOutcome::Done { output },
        Err(error) => ToolOutcome::Failed {
            reason: error.message,
        },
    }
}

/// Google refused the mail: it did not leave.
fn refused(login: &str) -> ApiError {
    invalid(&format!("Refusé · Google a refusé d’envoyer ce courriel depuis le compte {login}. Aucun courriel n’est parti. Si cela se répète, reconnectez Gmail et accordez les trois autorisations demandées."))
}
/// Google limits what this account may send: it did not leave.
fn rate_limited(login: &str) -> ApiError {
    ApiError::new(429, format!("Trop de messages · Google limite le nombre de courriels envoyés par le compte {login}. Aucun courriel n’est parti : réessayez plus tard."))
}
/// No receipt: the mail may or may not have left, and it is not reported as sent.
pub(super) fn unconfirmed() -> ApiError {
    ApiError::new(
        502,
        "Envoi non confirmé · l’accusé de réception de Google manque : le courriel est peut-être parti. Vérifiez les messages envoyés du compte Gmail avant d’en préparer un autre.",
    )
}
fn unsent() -> ApiError {
    invalid("Ce courriel n’a pas été envoyé : la carte est close. Demandez au bot d’en préparer un nouveau.")
}

/// Where the card shows how the send ended, beside the stored state.
fn settle(
    ctx: &Context<'_>,
    request_id: &str,
    state: &str,
    message_id: Option<&str>,
) -> Result<(), ApiError> {
    ctx.connection.execute(
        "UPDATE wrapper_gmail_drafts SET state=?2,message_id=?3 WHERE request_id=?1",
        params![request_id, state, message_id],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.etat',?2) WHERE request_id=?1",
        params![request_id, state],
    )?;
    Ok(())
}
fn sent(draft: &Stored) -> Value {
    let recipients = draft.to.join(", ");
    json!({"publie":true,"compte":draft.login,"a":draft.to,"cc":draft.cc,"objet":draft.subject,"texte":draft.body,"id":draft.message_id,
        "message":format!("Courriel envoyé à {recipients} sous le compte {} : « {} ».", draft.login, draft.subject)})
}
struct Stored {
    login: String,
    to: Vec<String>,
    cc: Vec<String>,
    subject: String,
    body: String,
    print: String,
    state: String,
    message_id: String,
}

/// RFC 2047 encoded words for a subject that is not plain ASCII, none past 75 characters.
fn header_subject(subject: &str) -> String {
    if subject.is_ascii() {
        return subject.to_owned();
    }
    let mut words: Vec<String> = Vec::new();
    let mut chunk = String::new();
    for c in subject.chars() {
        if chunk.len() + c.len_utf8() > 45 {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    words.push(chunk);
    words
        .iter()
        .map(|word| {
            format!(
                "=?UTF-8?B?{}?=",
                base64::engine::general_purpose::STANDARD.encode(word)
            )
        })
        .collect::<Vec<_>>()
        .join("\r\n ")
}
/// The message Gmail is given, in the base64url form its `raw` field takes. The body travels
/// base64 encoded, so any text survives and no line of it can be taken for a header.
fn raw(draft: &Stored) -> String {
    let std = base64::engine::general_purpose::STANDARD;
    let body = draft.body.replace("\r\n", "\n").replace('\n', "\r\n");
    let encoded = std.encode(body);
    let wrapped = encoded
        .as_bytes()
        .chunks(76)
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect::<Vec<_>>()
        .join("\r\n");
    let mut message = format!("From: {}\r\nTo: {}\r\n", draft.login, draft.to.join(", "));
    if !draft.cc.is_empty() {
        message.push_str(&format!("Cc: {}\r\n", draft.cc.join(", ")));
    }
    message.push_str(&format!(
        "Subject: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{wrapped}\r\n",
        header_subject(&draft.subject)
    ));
    URL_SAFE_NO_PAD.encode(message)
}

fn send(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let run = ctx.run_id.ok_or_else(|| {
        invalid("Rien n’a été envoyé : cette demande n’appartient à aucune exécution.")
    })?;
    let (request_id, _) = crate::features::forms::approval_identity(run, call)?;
    let row = ctx.connection.query_row(
        "SELECT login,to_json,cc_json,subject,body,fingerprint,state,message_id FROM wrapper_gmail_drafts WHERE request_id=?1",
        [&request_id],
        |r| Ok(Stored { login: r.get(0)?, to: list(&r.get::<_, String>(1)?), cc: list(&r.get::<_, String>(2)?), subject: r.get(3)?, body: r.get(4)?, print: r.get(5)?, state: r.get(6)?, message_id: r.get::<_, Option<String>>(7)?.unwrap_or_default() }),
    ).optional()?;
    let Some(draft) = row else {
        return Err(invalid("Brouillon introuvable : rien n’a été envoyé."));
    };
    match draft.state.as_str() {
        "draft" => {}
        // A replay answers with the first result and never calls Gmail.
        "sent" => return Ok(sent(&draft)),
        "refused" => return Err(refused(&draft.login)),
        "rate_limited" => return Err(rate_limited(&draft.login)),
        "failed" => return Err(unsent()),
        _ => return Err(unconfirmed()),
    }
    let account = match held(ctx, &draft.login) {
        Ok(account)
            if fingerprint(
                &draft.login,
                &draft.to,
                &draft.cc,
                &draft.subject,
                &draft.body,
            ) == draft.print =>
        {
            account
        }
        other => {
            settle(ctx, &request_id, "failed", None)?;
            return Err(other.err().unwrap_or_else(changed));
        }
    };
    // One claim before the call: only the run that wins it may reach Gmail.
    let claimed = ctx.connection.execute(
        "UPDATE wrapper_gmail_drafts SET state='sending' WHERE request_id=?1 AND state='draft'",
        [&request_id],
    )?;
    if claimed != 1 {
        return Err(unconfirmed());
    }
    let reply = match exchange(
        ctx,
        &account,
        "gmail/v1/users/me/messages/send",
        Some(&json!({"raw": raw(&draft)})),
    ) {
        Ok(reply) => reply,
        Err(error) => {
            settle(
                ctx,
                &request_id,
                if error.status == 502 {
                    "unconfirmed"
                } else {
                    "failed"
                },
                None,
            )?;
            return Err(error);
        }
    };
    match reply.status {
        // Accepted, even if the reply is unreadable: it left, so it must not be sent again.
        200 => {
            let id = reply.body["id"].as_str().and_then(message_id).unwrap_or("");
            settle(ctx, &request_id, "sent", Some(id))?;
            Ok(sent(&Stored {
                message_id: id.to_owned(),
                ..draft
            }))
        }
        status => {
            let limited = status == 429
                || (status == 403
                    && reply.body["error"]["errors"].as_array().is_some_and(|all| {
                        all.iter().any(|e| {
                            e["reason"]
                                .as_str()
                                .is_some_and(|r| r.contains("ateLimit") || r.contains("dailyLimit"))
                        })
                    }));
            let (state, error) = if limited {
                ("rate_limited", rate_limited(&draft.login))
            } else if status >= 500 {
                ("unconfirmed", unconfirmed())
            } else {
                ("refused", refused(&draft.login))
            };
            settle(ctx, &request_id, state, None)?;
            Err(error)
        }
    }
}
