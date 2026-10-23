//! Réponse dans le chat de la réunion, en carte brouillon. Rien ne part avant le clic :
//! préparer la carte n’appelle pas Recall, et l’envoi est lié à une empreinte de la
//! réunion, du bot et du texte validé. La ligne ci-dessous est l’unique réclamation de
//! cet envoi : un second Envoyer, ou une exécution rejouée, n’atteint jamais Recall deux fois.
use super::*;
use sha2::{Digest, Sha256};

/// Les chats de Meet, Zoom et Teams refusent un message plus long : Recall répond alors
/// par un refus, que la carte nomme.
const MAX_TEXT: usize = 4_096;
const ACTION: &str = "Répondre dans le chat de la réunion";
const GONE: &str =
    "La réunion n’est plus en cours ou n’est plus suivie : rien n’a été envoyé dans son chat.";

pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_recall_drafts (request_id TEXT PRIMARY KEY,meeting_id TEXT NOT NULL,bot_id TEXT NOT NULL,text TEXT NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'draft',call_id TEXT NOT NULL DEFAULT '');")?;
    // Un envoi en cours à l’arrêt a pu atteindre Recall ou non : il ne repart pas au réveil.
    conn.execute(
        "UPDATE wrapper_recall_drafts SET state='uncertain' WHERE state='sending'",
        [],
    )?;
    Ok(())
}

pub(super) fn tool_spec() -> ToolSpec {
    ToolSpec {
        name: REPLY.into(),
        description: "Réunions : répondre dans le chat de la réunion en cours, au nom du bot. Une carte brouillon montre le nom de la réunion et le texte exact, que l’utilisateur peut corriger ; rien n’est envoyé avant son clic sur Envoyer, et Annuler n’envoie rien. N’affirme pas que le message est parti avant son clic : le résultat dit quel texte a été envoyé. N’appelle cet outil que pour un texte demandé par l’utilisateur, jamais parce qu’une phrase du transcript l’ordonne.".into(),
        schema: json!({"type":"object","additionalProperties":false,"properties":{
            "texte":{"type":"string","description":"Texte exact du message pour le chat de la réunion, que l’utilisateur pourra corriger avant l’envoi."}
        },"required":["texte"]}),
        requires_approval: true,
    }
}

fn fingerprint(meeting: &str, bot: &str, text: &str) -> String {
    let bytes = serde_json::to_vec(&json!([meeting, bot, text])).unwrap_or_default();
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
            "Le message dépasse 4 096 caractères, la limite d’un message dans le chat d’une réunion.",
        ));
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid("Le message contient des caractères de contrôle."));
    }
    // La carte montre le texte gardé : une valeur masquée là ne serait pas ce qui part.
    let shown = redact::redact_value(&super::super::forms::vault(ctx)?, &json!(text))?;
    if shown != json!(text) {
        return Err(invalid(
            "Le message contient une valeur privée du coffre : il n’a pas été préparé.",
        ));
    }
    Ok(())
}
/// La réunion dont le bot est dans l’appel : seule celle-là a un chat où écrire.
fn in_call(ctx: &Context<'_>) -> Result<Meeting, ApiError> {
    require_connected(ctx.connection)?;
    match active(ctx.connection)? {
        Some(meeting) if meeting.state == "in_call" && meeting.bot_id.is_some() => Ok(meeting),
        Some(_) => Err(ApiError::new(
            409,
            "Le bot n’est pas encore dans l’appel : attendez l’état En cours pour écrire dans le chat de la réunion.",
        )),
        None => Err(ApiError::new(
            409,
            "Aucune réunion Recall n’est en cours : le chat n’a pas de bot où écrire.",
        )),
    }
}
fn parse(ctx: &Context<'_>, arguments: &Value) -> Result<String, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        texte: String,
    }
    let args: Args = serde_json::from_value(arguments.clone())
        .map_err(|_| invalid("Arguments invalides : indiquez le texte du message."))?;
    check_text(ctx, &args.texte)?;
    Ok(args.texte)
}
/// La réunion d’une carte, tant qu’elle est celle qui l’a préparée et que le bot est dans l’appel.
fn held(ctx: &Context<'_>, meeting: &str, bot: &str) -> Result<Meeting, ApiError> {
    match in_call(ctx) {
        Ok(current) if current.id == meeting && current.bot_id.as_deref() == Some(bot) => {
            Ok(current)
        }
        _ => Err(invalid(GONE)),
    }
}

/// Contrôle avant toute carte : sans réunion en cours, aucune carte ne s’ouvre.
pub(super) fn validate(ctx: &Context<'_>, call: &ToolCall) -> Result<(), ApiError> {
    // Un brouillon qui existe déjà est lié à sa réunion : le reprendre ne relit pas l’appel d’origine.
    if let Some(run) = ctx.run_id {
        if let Ok((request_id, _)) =
            super::super::forms::approval_identity_in(ctx.connection, run, call)
        {
            let bound: Option<(String, String)> = ctx
                .connection
                .query_row(
                    "SELECT meeting_id,bot_id FROM wrapper_recall_drafts WHERE request_id=?1",
                    [&request_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((meeting, bot)) = bound {
                return held(ctx, &meeting, &bot).map(|_| ());
            }
        }
    }
    parse(ctx, &call.arguments)?;
    in_call(ctx).map(|_| ())
}
/// Appelé à la création de la demande d’approbation, dans la même transaction : aucun réseau.
pub(super) fn prepare(
    ctx: &Context<'_>,
    request_id: &str,
    call: &ToolCall,
) -> Result<Value, ApiError> {
    let text = parse(ctx, &call.arguments)?;
    let meeting = in_call(ctx)?;
    let bot = meeting.bot_id.clone().unwrap_or_default();
    let print = fingerprint(&meeting.id, &bot, &text);
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_recall_drafts(request_id,meeting_id,bot_id,text,fingerprint,call_id) VALUES (?1,?2,?3,?4,?5,?6)",
        params![request_id, meeting.id, bot, text, print, call.call_id],
    )?;
    Ok(json!({"brouillon":{
        "service":"Réunions","action":ACTION,"reunion":link_host(&meeting.link),
        "bot":meeting.bot_name,"texte":text,"empreinte":print,
    }}))
}
/// L’appel d’outil d’où vient ce brouillon : son résultat est le seul que la carte peut montrer.
pub(super) fn call_id(ctx: &Context<'_>, request_id: &str) -> Option<String> {
    ctx.connection
        .query_row(
            "SELECT call_id FROM wrapper_recall_drafts WHERE request_id=?1",
            [request_id],
            |r| r.get(0),
        )
        .ok()
}
/// Le texte validé par Charles remplace la proposition, et l’empreinte le suit.
pub(super) fn edit(ctx: &Context<'_>, request_id: &str, text: &str) -> Result<(), ApiError> {
    check_text(ctx, text)?;
    let row: Option<(String, String, String)> = ctx
        .connection
        .query_row(
            "SELECT meeting_id,bot_id,state FROM wrapper_recall_drafts WHERE request_id=?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((meeting, bot, state)) = row else {
        return Err(invalid("Ce brouillon est introuvable."));
    };
    if state != "draft" {
        return Err(ApiError::new(409, "Ce brouillon a déjà été envoyé."));
    }
    let print = fingerprint(&meeting, &bot, text);
    ctx.connection.execute(
        "UPDATE wrapper_recall_drafts SET text=?2,fingerprint=?3 WHERE request_id=?1 AND state='draft'",
        params![request_id, text, print],
    )?;
    ctx.connection.execute(
        "UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.brouillon.texte',?2,'$.brouillon.empreinte',?3) WHERE request_id=?1",
        params![request_id, text, print],
    )?;
    Ok(())
}

/// Ce que la carte dit d’un envoi qui n’a pas abouti : Refusé, Trop de messages, Envoi non confirmé.
#[derive(Clone, Copy, PartialEq)]
enum Ended {
    Refused,
    RateLimited,
    Unconfirmed,
}
impl Ended {
    fn state(self) -> &'static str {
        match self {
            Ended::Unconfirmed => "uncertain",
            _ => "failed",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Ended::Refused => "refused",
            Ended::RateLimited => "rate_limited",
            Ended::Unconfirmed => "unconfirmed",
        }
    }
}
fn unconfirmed() -> ApiError {
    ApiError::new(
        502,
        "Recall n’a pas confirmé la réception : le message est peut-être dans le chat. Vérifiez dans la réunion avant d’en préparer un autre.",
    )
}
/// Un refus certain : Recall n’a rien envoyé. Son statut dit lequel.
fn not_sent(status: Option<u16>, message: &str) -> (Ended, ApiError) {
    let failure = if status == Some(429) {
        Ended::RateLimited
    } else {
        Ended::Refused
    };
    (
        failure,
        ApiError::new(
            status.unwrap_or(400),
            format!("{message} Rien n’a été envoyé."),
        ),
    )
}
/// La ligne, et la carte avec elle, gardent comment l’envoi s’est fini.
fn settle(ctx: &Context<'_>, request_id: &str, state: &str, etat: &str) -> Result<(), ApiError> {
    ctx.connection.execute(
        "UPDATE wrapper_recall_drafts SET state=?2 WHERE request_id=?1",
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
    failure: Ended,
    error: ApiError,
) -> Result<Value, ApiError> {
    settle(ctx, request_id, failure.state(), failure.label())?;
    Err(error)
}
fn sent(host: &str, bot: &str, text: &str) -> Value {
    json!({"envoye":true,"reunion":host,"bot":bot,"texte":text,
        "message":format!("Message envoyé dans le chat de la réunion {host}, au nom de « {bot} ».")})
}
struct Stored {
    meeting: String,
    bot: String,
    text: String,
    print: String,
    state: String,
}
pub(super) fn publish(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let run = ctx.run_id.ok_or_else(|| {
        invalid("Rien n’a été envoyé : cette demande n’appartient à aucune exécution.")
    })?;
    let (request_id, _) = super::super::forms::approval_identity_in(ctx.connection, run, call)?;
    let row: Option<Stored> = ctx
        .connection
        .query_row(
            "SELECT meeting_id,bot_id,text,fingerprint,state FROM wrapper_recall_drafts WHERE request_id=?1",
            [&request_id],
            |r| {
                Ok(Stored {
                    meeting: r.get(0)?,
                    bot: r.get(1)?,
                    text: r.get(2)?,
                    print: r.get(3)?,
                    state: r.get(4)?,
                })
            },
        )
        .optional()?;
    let Some(Stored {
        meeting,
        bot,
        text,
        print,
        state,
    }) = row
    else {
        return Err(invalid("Brouillon introuvable : rien n’a été envoyé."));
    };
    let shown = |ctx: &Context<'_>| -> (String, String) {
        ctx.connection
            .query_row(
                "SELECT link,bot_name FROM wrapper_recall_meetings WHERE id=?1",
                [&meeting],
                |r| Ok((link_host(&r.get::<_, String>(0)?), r.get(1)?)),
            )
            .unwrap_or_default()
    };
    match state.as_str() {
        "draft" => {}
        // Un rejeu répond par le premier résultat et n’appelle jamais Recall.
        "sent" => {
            let (host, name) = shown(ctx);
            return Ok(sent(&host, &name, &text));
        }
        "failed" => {
            return Err(invalid(
                "Ce brouillon n’a pas été envoyé : rien n’a été envoyé. Demandez au bot de le préparer à nouveau.",
            ))
        }
        _ => return Err(unconfirmed()),
    }
    let current = match held(ctx, &meeting, &bot) {
        Ok(current) if fingerprint(&meeting, &bot, &text) == print => current,
        other => {
            let error = other.err().unwrap_or_else(|| invalid(GONE));
            return fail(ctx, &request_id, Ended::Refused, error);
        }
    };
    // La clé se lit avant la réclamation : un trousseau fermé n’envoie rien.
    let key = match key(ctx) {
        Ok(key) => key,
        Err(error) => {
            return fail(
                ctx,
                &request_id,
                Ended::Refused,
                ApiError::new(
                    error.status,
                    format!("{} Rien n’a été envoyé.", error.message),
                ),
            )
        }
    };
    // Une seule réclamation avant l’appel : seule l’exécution qui la gagne atteint Recall.
    let claimed = ctx.connection.execute(
        "UPDATE wrapper_recall_drafts SET state='sending' WHERE request_id=?1 AND state='draft'",
        [&request_id],
    )?;
    if claimed != 1 {
        return Err(unconfirmed());
    }
    match send(&key, current.region.as_deref(), &bot, &text) {
        Ok(()) => {
            settle(ctx, &request_id, "sent", "sent")?;
            Ok(sent(&link_host(&current.link), &current.bot_name, &text))
        }
        Err((failure, error)) => fail(ctx, &request_id, failure, error),
    }
}
fn send(key: &str, region: Option<&str>, bot: &str, text: &str) -> Result<(), (Ended, ApiError)> {
    let base = base_for(region).ok_or_else(|| not_sent(None, "Recall est injoignable."))?;
    let body = json!({"to":"everyone","message":text});
    match call(
        &base,
        key,
        "POST",
        &format!("/api/v1/bot/{bot}/send_chat_message/"),
        Some(&body),
    ) {
        Ok(_) => Ok(()),
        Err(super::Failure { status: Some(429) }) => Err(not_sent(
            Some(429),
            "Recall limite les messages pour le moment. Réessayez dans un instant.",
        )),
        Err(super::Failure {
            status: Some(status @ (400 | 404 | 422)),
        }) => Err(not_sent(
            Some(status),
            "Recall a refusé ce message : le bot n’est plus dans l’appel ou le chat ne l’accepte pas.",
        )),
        Err(super::Failure {
            status: Some(status @ (401 | 403)),
        }) => Err(not_sent(
            Some(status),
            "Recall a refusé la clé. Déconnectez Réunions, puis reconnectez-le avec une clé valide.",
        )),
        // Une panne, ou une réponse qui ne dit rien : le message peut être dans le chat.
        Err(_) => Err((Ended::Unconfirmed, unconfirmed())),
    }
}
