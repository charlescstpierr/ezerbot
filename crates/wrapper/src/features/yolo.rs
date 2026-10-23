//! Mode YOLO : les approbations sont levées, les garde-fous restent.
//!
//! Un interrupteur global (activé par défaut, aussi pour les bots existants) et
//! une surcharge par bot décident si les outils s'exécutent sans carte. Le mode
//! recouvre les réglages sans jamais les écraser : repasser à « Demander avant
//! d'agir » rend aussitôt les réglages d'avant (Terminal, Fichiers, Écran,
//! connecteurs, règles « Toujours »).
//!
//! Garde-fous qui survivent au mode : les données d'Ezerbot (dossier de la base,
//! coffre, jetons) restent hors de portée des outils, un refus permanent écrit
//! par l'utilisateur gagne encore, et Arrêter (ou Échap) interrompt toujours le
//! bot. Les cartes de réglage (`propose_skill`, connecteurs à démarrer ou
//! retirer, consentement Recall, publication de webhooks, agent de routines) et
//! les brouillons à relire avant envoi restent des cartes : ce ne sont pas des
//! approbations d'outil mais des décisions ou des relectures.
//!
//! Chaque action admise est inscrite dans le journal d'activité, dans la même
//! transaction que l'admission, puis rendue dans le fil par la carte « Fait
//! automatiquement ».
use super::{write_transaction, Context};
use crate::{parse, ApiError, ApiResponse, App, ACCOUNT, DEFAULT_BOT};
use grokbot_engine::{
    contracts::{ApprovalCheck, AuthorizedBy, ToolCall},
    model::Run,
};
use grokbot_forms::{approvals, policy, redact::redact_value};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

empty_hooks!(
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

pub const YOLO: &str = "yolo";
pub const ASK: &str = "ask";
const INHERIT: &str = "inherit";
const LOG_LIMIT: usize = 100;
const LOG_KEEP: i64 = 2000;
const DETAIL_CHARS: usize = 20_000;
pub const PROTECTED: &str =
    "Mode YOLO : les données et les identifiants d’Ezerbot restent hors de portée des outils.";

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    // Une base neuve ou existante reçoit le mode YOLO une seule fois ; le choix
    // « Demander avant d'agir » est ensuite conservé tel quel.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_yolo_settings(
            singleton INTEGER PRIMARY KEY CHECK(singleton=1),
            mode TEXT NOT NULL CHECK(mode IN ('yolo','ask')),
            changed_at INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS wrapper_yolo_bots(
            bot_id TEXT PRIMARY KEY,
            mode TEXT NOT NULL CHECK(mode IN ('yolo','ask')));
        CREATE TABLE IF NOT EXISTS wrapper_yolo_log(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            bot_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            call_id TEXT NOT NULL,
            tool TEXT NOT NULL,
            summary TEXT NOT NULL,
            detail TEXT NOT NULL DEFAULT '',
            source TEXT NOT NULL,
            UNIQUE(run_id, call_id));
        INSERT OR IGNORE INTO wrapper_yolo_settings(singleton, mode) VALUES (1, 'yolo');",
    )?;
    // Table créée par une version d'avant la colonne `detail` : on l'ajoute.
    let has_detail: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('wrapper_yolo_log') WHERE name='detail')",
        [],
        |row| row.get(0),
    )?;
    if !has_detail {
        conn.execute_batch(
            "ALTER TABLE wrapper_yolo_log ADD COLUMN detail TEXT NOT NULL DEFAULT ''",
        )?;
    }
    Ok(())
}

/// Mode du réglage global. Une lecture impossible vaut « Demander » : ne jamais
/// lever une approbation sur une erreur.
pub fn global_mode(conn: &Connection) -> &'static str {
    conn.query_row(
        "SELECT mode FROM wrapper_yolo_settings WHERE singleton=1",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .map_or(ASK, |mode| if mode == YOLO { YOLO } else { ASK })
}

fn bot_override(conn: &Connection, bot: &str) -> Option<&'static str> {
    conn.query_row(
        "SELECT mode FROM wrapper_yolo_bots WHERE bot_id=?1",
        [bot],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(|mode| if mode == YOLO { YOLO } else { ASK })
}

/// `Some("bot" | "global")` quand ce bot agit sans demander, d'après le réglage
/// qui s'applique ; `None` quand il demande.
pub fn source(conn: &Connection, bot: &str) -> Option<&'static str> {
    match bot_override(conn, bot) {
        Some(YOLO) => Some("bot"),
        Some(_) => None,
        None => (global_mode(conn) == YOLO).then_some("global"),
    }
}

pub fn active(conn: &Connection, bot: &str) -> bool {
    source(conn, bot).is_some()
}

/// Outils dont l'approbation est levée. Le reste garde sa carte : une relecture
/// de skill, un réglage système ou un consentement n'est pas une permission d'outil.
pub fn covered(tool: &str) -> bool {
    !(tool == "propose_skill"
        || super::mcp::is_lifecycle(tool)
        || super::mcp::is_draft(tool)
        || super::recall::is_consent(tool)
        || tool == super::routine_webhooks::ACTION
        || tool == super::routines_background::ACTION)
}

/// Un refus permanent écrit par l'utilisateur gagne, même sur une commande qu'un
/// « Toujours autoriser » ne pourrait jamais couvrir (composée, destructive) : seule
/// la lecture des règles de refus compte ici, jamais leur filtre de mémorisation.
fn standing_refusal(ctx: &Context<'_>, run: &Run, call: &ToolCall) -> Result<bool, ApiError> {
    let (tool, arguments) = if policy::describe(&call.name, &call.arguments, None).scope == "action"
    {
        (policy::BACKGROUND_TOOL, json!({}))
    } else {
        (call.name.as_str(), call.arguments.clone())
    };
    let digest = approvals::args_digest(tool, &arguments)?;
    let pattern = policy::command_pattern(tool, &arguments).unwrap_or_default();
    Ok(ctx.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM forms_approval_rules
         WHERE requesting_bot_id=?1 AND target_bot_id=?2 AND tool=?3 AND allow=0
           AND (scope='\"global\"' OR args_digest=?4 OR (scope='\"pattern\"' AND args_digest=?5)))",
        params![run.bot_id, DEFAULT_BOT.id, tool, digest, pattern],
        |row| row.get(0),
    )?)
}

/// Décision d'admission du mode YOLO, ou `None` pour laisser la voie habituelle.
/// Appelée dans la transaction de l'admission : le journal et l'autorisation
/// sont validés ou annulés ensemble.
pub(super) fn check(
    ctx: &Context<'_>,
    run: &Run,
    call: &ToolCall,
) -> Result<Option<ApprovalCheck>, ApiError> {
    let Some(source) = source(ctx.connection, &run.bot_id) else {
        return Ok(None);
    };
    if !covered(&call.name) {
        return Ok(None);
    }
    if standing_refusal(ctx, run, call)? {
        return Ok(Some(ApprovalCheck::Denied {
            reason: "Une autorisation permanente refuse cette action.".into(),
        }));
    }
    if super::local::is_tool(&call.name) && super::local::touches_app_data(ctx, call)? {
        return Ok(Some(ApprovalCheck::Denied {
            reason: PROTECTED.into(),
        }));
    }
    record(ctx, run, call, source)?;
    // Un geste d'écran reste lié à la session vivante à l'admission : si elle est
    // révoquée puis remplacée, le geste en attente n'agit pas dans la nouvelle.
    let session = super::screen::is_tool(&call.name)
        .then(|| super::screen::approval_session(ctx).as_u64())
        .flatten();
    Ok(Some(ApprovalCheck::Authorized {
        by: AuthorizedBy::Yolo {
            source: source.into(),
            session,
        },
    }))
}

fn text(arguments: &Value, key: &str) -> String {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Le texte complet de ce qui a été fait, tel que l'appel l'a écrit. Le coffre le
/// masque avant l'écriture : un secret saisi dans une commande n'entre pas dans le
/// journal. Ce texte exact est ce que l'oubli d'un souvenir efface ; la ligne
/// affichée n'en est qu'un résumé recalculé (voir [`summary`]).
fn detail(ctx: &Context<'_>, call: &ToolCall) -> String {
    let args = &call.arguments;
    let raw = match call.name.as_str() {
        "run_shell" => format!(
            "Commande « {} » dans {}",
            text(args, "command"),
            text(args, "cwd")
        ),
        "read_file" => format!("Lecture de {}", text(args, "path")),
        "list_directory" => format!("Contenu du dossier {}", text(args, "path")),
        "write_file" => format!(
            "Écriture de {} ({} octets)",
            text(args, "path"),
            text(args, "content").len()
        ),
        "delegate" => format!("Délégation : {}", text(args, "objective")),
        name if super::screen::is_tool(name) => format!("Écran du Mac : {name}"),
        name if name.starts_with("mcp_") => super::mcp::display(ctx, name)
            .map(|label| {
                format!(
                    "Connecteur {} · {}",
                    label["server"].as_str().unwrap_or("MCP"),
                    label["tool"].as_str().unwrap_or(name)
                )
            })
            .unwrap_or_else(|| format!("Connecteur : {name}")),
        name => format!("Outil {name}"),
    };
    let raw: String = raw.chars().take(DETAIL_CHARS).collect();
    super::forms::vault(ctx)
        .ok()
        .and_then(|vault| redact_value(&vault, &json!(raw)).ok())
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or(raw)
}

/// La ligne affichée : le texte complet, espaces ramenés à un seul et raccourci.
pub fn summary(detail: &str) -> String {
    policy::shorten_summary(detail)
}

/// Efface d'un run, dans le texte complet comme dans la ligne affichée, ce que
/// `redact` retire : l'oubli d'un souvenir passe ici avec les mêmes règles que pour
/// les arguments d'outil. La ligne est recalculée depuis le texte déjà effacé, donc
/// ni la normalisation des espaces ni le raccourcissement ne gardent une trace.
pub fn redact_log(
    conn: &Connection,
    run_id: &str,
    redact: &dyn Fn(&str) -> String,
) -> Result<(), ApiError> {
    let mut statement =
        conn.prepare("SELECT id,summary,detail FROM wrapper_yolo_log WHERE run_id=?1")?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, shown, detail) in rows {
        // Une ligne sans texte complet (base d'avant cette colonne) s'efface comme elle est.
        let (new_detail, new_shown) = if detail.is_empty() {
            (String::new(), redact(&shown))
        } else {
            let cleaned = redact(&detail);
            let line = summary(&cleaned);
            (cleaned, line)
        };
        if new_detail != detail || new_shown != shown {
            conn.execute(
                "UPDATE wrapper_yolo_log SET detail=?2, summary=?3 WHERE id=?1",
                params![id, new_detail, new_shown],
            )?;
        }
    }
    Ok(())
}

fn record(ctx: &Context<'_>, run: &Run, call: &ToolCall, source: &str) -> Result<(), ApiError> {
    let detail = detail(ctx, call);
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_yolo_log(at,bot_id,conversation_id,run_id,call_id,tool,summary,detail,source)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            ctx.now_ms as i64,
            run.bot_id,
            run.conversation_id,
            run.run_id,
            call.call_id,
            call.name,
            summary(&detail),
            detail,
            source
        ],
    )?;
    ctx.connection.execute(
        "DELETE FROM wrapper_yolo_log WHERE id <= (SELECT max(id) FROM wrapper_yolo_log) - ?1",
        [LOG_KEEP],
    )?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Global {
    mode: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bot {
    bot_id: String,
    mode: String,
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if method != "POST" || !matches!(path, "/api/yolo/mode" | "/api/yolo/bot") {
        return None;
    }
    Some((|| {
        let now = crate::now() as i64;
        let tx = write_transaction(&app.connection)?;
        if path == "/api/yolo/mode" {
            let input: Global = parse(body)?;
            if !matches!(input.mode.as_str(), YOLO | ASK) {
                return Err(ApiError::new(400, "Choisissez « yolo » ou « ask »."));
            }
            tx.execute(
                "UPDATE wrapper_yolo_settings SET mode=?1, changed_at=?2 WHERE singleton=1",
                params![input.mode, now],
            )?;
        } else {
            let input: Bot = parse(body)?;
            let live: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM wrapper_bots WHERE account_id=?1 AND id=?2 AND deleted=0)",
                params![ACCOUNT, input.bot_id],
                |row| row.get(0),
            )?;
            if !live {
                return Err(ApiError::new(404, "Bot introuvable."));
            }
            match input.mode.as_str() {
                INHERIT => {
                    tx.execute(
                        "DELETE FROM wrapper_yolo_bots WHERE bot_id=?1",
                        [&input.bot_id],
                    )?;
                }
                YOLO | ASK => {
                    tx.execute(
                        "INSERT INTO wrapper_yolo_bots(bot_id,mode) VALUES (?1,?2)
                         ON CONFLICT(bot_id) DO UPDATE SET mode=excluded.mode",
                        params![input.bot_id, input.mode],
                    )?;
                }
                _ => {
                    return Err(ApiError::new(
                        400,
                        "Choisissez « yolo », « ask » ou « inherit ».",
                    ))
                }
            }
        }
        tx.commit()?;
        Ok(ApiResponse::Json(state(&app.feature_context())))
    })())
}

pub fn state(ctx: &Context<'_>) -> Value {
    let load = || -> Result<Value, ApiError> {
        let conn = ctx.connection;
        let mut statement = conn
            .prepare("SELECT id FROM wrapper_bots WHERE account_id=?1 AND deleted=0 ORDER BY id")?;
        let ids = statement
            .query_map([ACCOUNT], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let bots: Vec<Value> = ids
            .iter()
            .map(|id| {
                json!({
                    "bot_id": id,
                    "mode": bot_override(conn, id).unwrap_or(INHERIT),
                    "effective": if active(conn, id) { YOLO } else { ASK },
                })
            })
            .collect();
        let mut statement = conn.prepare(
            "SELECT id,at,bot_id,conversation_id,tool,summary,source FROM wrapper_yolo_log ORDER BY id DESC LIMIT ?1",
        )?;
        let log = statement
            .query_map([LOG_LIMIT as i64], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "at": row.get::<_, i64>(1)?,
                    "bot_id": row.get::<_, String>(2)?,
                    "conversation_id": row.get::<_, String>(3)?,
                    "tool": row.get::<_, String>(4)?,
                    "summary": row.get::<_, String>(5)?,
                    "source": row.get::<_, String>(6)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({
            "mode": global_mode(conn),
            "bots": bots,
            "log": log,
            "home": std::env::var("HOME").ok(),
        }))
    };
    load().unwrap_or_else(|_| json!({"error": "Mode YOLO indisponible.", "mode": ASK}))
}
