//! Décision tardive d'une carte détachée (#216).
//!
//! Avec « Continuer sans attendre mes approbations » (#215), le tour qui a posé la carte
//! est fini quand Charles décide : son appel est déjà réglé comme non exécuté et rien ne
//! le relance. La décision ouvre donc un tour de suite dans la même conversation :
//! « Charles a approuvé » avec l'appel exact que la carte liait, journalisé par l'hôte,
//! ou « Charles a refusé » sans aucun appel.
//! L'appel ne part que par le portail habituel : la carte reste son identité, l'octroi
//! « une fois » se consomme une seule fois et le bot ne choisit aucun argument. Une autre
//! empreinte pour le même outil est refusée.
use super::*;
use sha2::{Digest, Sha256};

const EXPIRED: &str = "Cette demande a expiré. Aucune action n’a été faite.";
const MAX_ARGUMENTS_CHARS: usize = 600;

pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_late_approvals (
            request_id TEXT PRIMARY KEY REFERENCES forms_requests(id),
            run_id TEXT NOT NULL UNIQUE,
            allowed INTEGER NOT NULL,
            started INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    Ok(())
}

pub(super) fn detached(conn: &Connection, request_id: &str) -> Result<bool, ApiError> {
    Ok(conn
        .query_row(
            "SELECT detached FROM forms_requests WHERE id=?1",
            [request_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// The follow-up turn of a request has a fixed identity, so a replayed decision or a
/// retried start finds the same turn instead of opening a second one.
fn follow_up_run(request_id: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(request_id.as_bytes()));
    format!("late-{}", &digest[..32])
}

/// Called with every resolution: a detached card that was just allowed or refused gets
/// its follow-up turn recorded in the same transaction as the decision.
pub(super) fn record_decision(
    conn: &Connection,
    request: &HumanInputRequest,
) -> Result<(), ApiError> {
    let row: Option<(bool, String)> = conn
        .query_row(
            "SELECT detached, status FROM forms_requests WHERE id=?1",
            [&request.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((true, status)) = row else {
        return Ok(());
    };
    let allowed = match status.as_str() {
        "resolved" => true,
        "denied" => false,
        _ => return Ok(()),
    };
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_late_approvals(request_id,run_id,allowed) VALUES (?1,?2,?3)",
        params![request.id, follow_up_run(&request.id), allowed],
    )?;
    Ok(())
}

/// Why this card can no longer take a decision, in words for Charles. A replay of the
/// decision that settled it is not refused: it gets its original receipt back.
pub(super) fn unavailable(
    conn: &Connection,
    request: &HumanInputRequest,
    submission_id: &str,
    now_ms: i64,
) -> Result<Option<ApiError>, ApiError> {
    let replay: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM forms_resolutions WHERE entry_id=?1 AND submission_id=?2)",
        params![request.entry_id, submission_id],
        |row| row.get(0),
    )?;
    let InputKind::Approval { expires_at, .. } = &request.kind else {
        return Ok(None);
    };
    Ok(match &request.status {
        _ if replay => None,
        RequestStatus::Pending if now_ms < *expires_at => None,
        RequestStatus::Pending | RequestStatus::Expired { .. } => Some(ApiError::new(409, EXPIRED)),
        RequestStatus::Cancelled { .. } => Some(ApiError::new(
            409,
            "Cette demande a été annulée. Aucune action n’a été faite.",
        )),
        RequestStatus::Resolved { .. } | RequestStatus::Denied => {
            Some(ApiError::new(409, "Cette demande a déjà reçu une réponse."))
        }
    })
}

/// The approved card, bound to the call the follow-up turn of an allowed decision runs.
fn allowed_card(conn: &Connection, run_id: &str) -> Result<Option<HumanInputRequest>, ApiError> {
    let id: Option<String> = conn
        .query_row(
            "SELECT request_id FROM wrapper_late_approvals WHERE run_id=?1 AND allowed=1",
            [run_id],
            |row| row.get(0),
        )
        .optional()?;
    match id {
        Some(id) => Ok(requests::get(conn, &id)?),
        None => Ok(None),
    }
}

/// The card this call answers to when it is the approved action of a follow-up turn.
pub(super) fn approved_card(
    conn: &Connection,
    run_id: &str,
    call: &ToolCall,
    digest: &str,
) -> Result<Option<String>, ApiError> {
    let Some(card) = allowed_card(conn, run_id)? else {
        return Ok(None);
    };
    Ok(match &card.kind {
        InputKind::Approval {
            tool, args_digest, ..
        } if *tool == call.name && args_digest == digest => Some(card.id),
        _ => None,
    })
}

/// The approved tool called again with other arguments is not what Charles approved.
pub(super) fn refusal(
    conn: &Connection,
    run: &Run,
    call: &ToolCall,
    digest: &str,
) -> Result<Option<String>, ApiError> {
    let Some(card) = allowed_card(conn, &run.run_id)? else {
        return Ok(None);
    };
    Ok(match &card.kind {
        InputKind::Approval {
            tool, args_digest, ..
        } if *tool == call.name && args_digest != digest => Some(
            "Ces arguments ne sont pas ceux que Charles a approuvés : rien n’a été exécuté.".into(),
        ),
        _ => None,
    })
}

fn shown_arguments(ctx: &Context<'_>, request_id: &str) -> Result<String, ApiError> {
    let text = display_arguments(ctx, request_id)?.to_string();
    Ok(match text.char_indices().nth(MAX_ARGUMENTS_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    })
}

/// Opens the follow-up turn of every decision recorded and not yet started. Call
/// outside a transaction; a failure leaves the record for the next host tick.
pub(super) fn start_pending(ctx: &Context<'_>) -> Result<(), ApiError> {
    if !ctx.connection.is_autocommit() {
        return Ok(());
    }
    let pending = {
        let mut statement = ctx.connection.prepare(
            "SELECT request_id,run_id,allowed FROM wrapper_late_approvals WHERE started=0 ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<rusqlite::Result<Vec<(String, String, bool)>>>()?
    };
    for (request_id, run_id, allowed) in pending {
        if start(ctx, &request_id, &run_id, allowed).is_ok() {
            ctx.connection.execute(
                "UPDATE wrapper_late_approvals SET started=1 WHERE request_id=?1",
                [&request_id],
            )?;
        }
    }
    Ok(())
}

fn start(ctx: &Context<'_>, request_id: &str, run_id: &str, allowed: bool) -> Result<(), ApiError> {
    let Some(request) = requests::get(ctx.connection, request_id)? else {
        return Ok(());
    };
    let InputKind::Approval { tool, .. } = &request.kind else {
        return Ok(());
    };
    let origin = store::required(ctx.connection, &request.run_id)?;
    // A deleted conversation or bot has nobody to tell, and a group turn is not ours to open.
    if !requesting_conversation_live(ctx, &origin)?
        || super::super::multibot::is_group(ctx.connection, &origin.conversation_id)
    {
        return Ok(());
    }
    let call = if allowed {
        match approved_call(ctx, &request)? {
            Some(call) => Some(call),
            None => return Ok(()),
        }
    } else {
        None
    };
    let arguments = shown_arguments(ctx, request_id)?;
    let text = if allowed {
        format!("Charles a approuvé : {tool} {arguments}. L’action est lancée une seule fois, avec ces arguments exacts ; son résultat suit. Dis-lui brièvement, en français, ce qui a été fait.")
    } else {
        format!("Charles a refusé : {tool} {arguments}. Rien n’a été exécuté. Ne la refais pas sans qu’il le demande et dis-lui brièvement, en français, que tu n’as rien fait.")
    };
    let local = Context {
        bot_id: Some(&origin.bot_id),
        conversation_id: Some(&origin.conversation_id),
        run_id: None,
        ..*ctx
    };
    super::super::engine::start_late_run(&local, run_id, text, call.as_ref())?;
    Ok(())
}
