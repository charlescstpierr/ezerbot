//! Ancre de passage : un message vise un passage affiché d'une Page. L'ancre
//! suit le message, reste « ouverte » tant que la Page n'a pas été écrite avec
//! succès dans la réponse à ce message, puis passe à « traitée ».
use super::{source_version, Context};
use crate::features::multibot::Mention;
use crate::{ApiError, ACCOUNT};
use grokbot_engine::contracts::{ContextBlock, ContextRequest, ContextSource};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const PASSAGE_LIMIT: usize = 500;
const LINES_LIMIT: usize = 20;
const LINE_LIMIT: usize = 240;
const BLOCK_BUDGET: usize = 16_384;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageAnchor {
    page_id: String,
    source_version: String,
    passage: String,
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_page_anchors (
            conversation_id TEXT NOT NULL,
            message_id TEXT NOT NULL,
            page_id TEXT NOT NULL,
            passage TEXT NOT NULL,
            source_version TEXT NOT NULL,
            state TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (conversation_id, message_id)
        );",
    )?;
    Ok(())
}

/// L'ancre appartient au premier envoi : un renvoi doit porter la même ancre
/// (même Page, même passage rogné), ou aucune si le premier n'en avait pas.
pub fn check_resend(
    conn: &Connection,
    conversation: &str,
    message: &str,
    anchor: Option<&PageAnchor>,
) -> Result<(), ApiError> {
    let stored: Option<(String, String)> = conn
        .query_row(
            "SELECT page_id, passage FROM wrapper_page_anchors WHERE conversation_id = ?1 AND message_id = ?2",
            params![conversation, message],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let sent = anchor.map(|a| (a.page_id.as_str(), a.passage.trim()));
    if stored
        .as_ref()
        .map(|(page, passage)| (page.as_str(), passage.as_str()))
        != sent
    {
        return Err(ApiError::new(
            409,
            "Ce message a déjà été envoyé avec une autre ancre de Page : renvoyez-le tel quel.",
        ));
    }
    Ok(())
}

/// Une sélection sur plusieurs cellules ou paragraphes porte des tabulations et des retours à la ligne,
/// l'aperçu un espace à chaque frontière de bloc (`compile::BREAK`) : on compare les deux avec chaque suite
/// de blancs (propriété Unicode White_Space) réduite à un seul espace, comme `collapsed` dans
/// app/ui/panels/pages.js. Un blanc au milieu d'un mot, ou un blanc absent entre deux cellules, ne passe donc
/// pas. Le passage reste stocké tel qu'envoyé.
fn collapsed(text: &str) -> String {
    text.split(char::is_whitespace)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Refuse sans rien écrire ; l'appelant garde le message dans la même transaction.
pub fn record(
    conn: &Connection,
    conversation: &str,
    message: &str,
    anchor: &PageAnchor,
    mentions: &[Mention],
    now_ms: u64,
) -> Result<(), ApiError> {
    let passage = anchor.passage.trim();
    if passage.is_empty() || passage.chars().count() > PASSAGE_LIMIT {
        return Err(ApiError::new(
            400,
            "Le passage cité doit contenir entre 1 et 500 caractères.",
        ));
    }
    if anchor.source_version.len() != 64
        || !anchor.source_version.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ApiError::new(400, "Version de Page invalide."));
    }
    let (page_bot, shown): (String, String) = conn
        .query_row(
            "SELECT bot_id, preview FROM bot_pages WHERE id = ?1",
            [&anchor.page_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| ApiError::new(404, "Cette Page est introuvable."))?;
    let thread_bot: String = conn.query_row(
        "SELECT bot_id FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2",
        params![ACCOUNT, conversation],
        |row| row.get(0),
    )?;
    if page_bot != thread_bot {
        return Err(ApiError::new(
            409,
            "Cette Page appartient à un autre bot que celui de ce fil.",
        ));
    }
    if mentions.iter().any(|mention| mention.id != page_bot) {
        return Err(ApiError::new(
            409,
            "Une ancre de Page ne peut viser que le bot propriétaire de la Page : ne mentionnez pas d’autre bot.",
        ));
    }
    if !collapsed(&shown).contains(&collapsed(passage)) {
        return Err(ApiError::new(
            400,
            "Ce passage n’apparaît pas dans le texte affiché de la Page.",
        ));
    }
    conn.execute(
        "INSERT INTO wrapper_page_anchors (conversation_id, message_id, page_id, passage, source_version, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'open', ?6)",
        params![
            conversation,
            message,
            anchor.page_id,
            passage,
            anchor.source_version,
            now_ms
        ],
    )?;
    Ok(())
}

/// À appeler après toute écriture réussie de la Page : l'ancre du message auquel
/// répond ce tour passe à « traitée » avec la nouvelle version de la source.
pub fn settle(ctx: &Context<'_>, page_id: &str, source: &str) -> Result<(), ApiError> {
    let (Some(run), Some(conversation)) = (ctx.run_id, ctx.conversation_id) else {
        return Ok(());
    };
    let Some(entry) = ctx
        .connection
        .query_row(
            "SELECT json_extract(input_json, '$.entry_id') FROM wrapper_engine_runs WHERE run_id = ?1",
            [run],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(());
    };
    let message = crate::features::multibot::source_message(ctx.connection, run, &entry)?;
    ctx.connection.execute(
        "UPDATE wrapper_page_anchors SET state = 'done', source_version = ?4
         WHERE conversation_id = ?1 AND message_id = ?2 AND page_id = ?3",
        params![conversation, message, page_id, source_version(source)],
    )?;
    Ok(())
}

pub fn list(conn: &Connection, page_id: &str) -> Result<Vec<Value>, ApiError> {
    let mut statement = conn.prepare(
        "SELECT message_id, passage, state, source_version FROM wrapper_page_anchors
         WHERE page_id = ?1 ORDER BY created_at, message_id",
    )?;
    let rows = statement.query_map([page_id], |row| {
        Ok(json!({
            "message_id": row.get::<_, String>(0)?,
            "passage": row.get::<_, String>(1)?,
            "state": row.get::<_, String>(2)?,
            "source_version": row.get::<_, String>(3)?,
        }))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Ce que la bulle du fil montre de l'ancre d'un message : la Page (avec son titre actuel) et le passage cité.
/// `source_version` permet au client de rejouer le même envoi après un redémarrage.
/// Une Page qui n'existe plus ne laisse pas de puce : elle n'aurait rien à ouvrir.
pub fn for_message(conn: &Connection, conversation: &str, message: &str) -> Value {
    conn.query_row(
        "SELECT a.page_id, a.passage, p.title, a.source_version FROM wrapper_page_anchors a
         JOIN bot_pages p ON p.id = a.page_id
         WHERE a.conversation_id = ?1 AND a.message_id = ?2",
        params![conversation, message],
        |row| {
            Ok(json!({
                "page_id": row.get::<_, String>(0)?,
                "passage": row.get::<_, String>(1)?,
                "title": row.get::<_, String>(2)?,
                "source_version": row.get::<_, String>(3)?,
            }))
        },
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(Value::Null)
}

pub fn discard_conversation(conn: &Connection, conversation: &str) -> Result<(), ApiError> {
    conn.execute(
        "DELETE FROM wrapper_page_anchors WHERE conversation_id = ?1",
        [conversation],
    )?;
    Ok(())
}

pub fn discard_message(
    conn: &Connection,
    conversation: &str,
    message: &str,
) -> Result<(), ApiError> {
    conn.execute(
        "DELETE FROM wrapper_page_anchors WHERE conversation_id = ?1 AND message_id = ?2",
        params![conversation, message],
    )?;
    Ok(())
}

pub fn context(ctx: &Context<'_>, request: &ContextRequest) -> Vec<ContextBlock> {
    let Ok(message) = crate::features::multibot::source_message(
        ctx.connection,
        &request.run_id,
        &request.current_message.entry_id,
    ) else {
        return Vec::new();
    };
    let anchored: rusqlite::Result<Option<(String, String, String, String)>> = ctx
        .connection
        .query_row(
            "SELECT p.title, p.filename, p.source, a.passage
             FROM wrapper_page_anchors a JOIN bot_pages p ON p.id = a.page_id
             WHERE a.conversation_id = ?1 AND a.message_id = ?2 AND p.bot_id = ?3",
            params![request.conversation_id, message, request.bot_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional();
    let Ok(Some((title, filename, source, passage))) = anchored else {
        return Vec::new();
    };
    let lines = source_lines(&source, &passage);
    let found = if lines.is_empty() {
        "Le passage n’a pas été retrouvé tel quel dans la source : appuie-toi sur la citation seule."
            .to_string()
    } else {
        let numbered: Vec<_> = lines
            .iter()
            .map(|(n, line)| format!("{n}: {line}"))
            .collect();
        format!(
            "Lignes de la source où il apparaît :\n{}",
            numbered.join("\n")
        )
    };
    let text = format!(
        "Charles vise un passage précis de la Page « {title} » ({filename}).\n\
         Passage cité : « {passage} »\n{found}\n\
         Change seulement cet endroit, garde le reste de la Page tel quel."
    );
    vec![ContextBlock {
        source: ContextSource::DelegatedContext,
        priority: crate::features::priority::PAGE_ANCHOR,
        budget: BLOCK_BUDGET,
        text,
    }]
}

/// Numéros (à partir de 1) et texte coupé des lignes qui portent le passage.
fn source_lines(source: &str, passage: &str) -> Vec<(usize, String)> {
    let mut numbers = BTreeSet::new();
    for (start, _) in source.match_indices(passage).take(LINES_LIMIT) {
        let first = source[..start].matches('\n').count();
        numbers.extend(first..=first + passage.matches('\n').count());
    }
    let lines: Vec<_> = source.lines().collect();
    numbers
        .into_iter()
        .take(LINES_LIMIT)
        .filter_map(|n| Some((n + 1, lines.get(n)?.chars().take(LINE_LIMIT).collect())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{collapsed, for_message, migrate, source_lines};
    use rusqlite::Connection;

    fn bench() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE bot_pages (id TEXT PRIMARY KEY, title TEXT NOT NULL);")
            .unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn anchor(conn: &Connection, conversation: &str, message: &str, page: &str, passage: &str) {
        conn.execute(
            "INSERT INTO wrapper_page_anchors VALUES (?1, ?2, ?3, ?4, 'v', 'open', 1)",
            [conversation, message, page, passage],
        )
        .unwrap();
    }

    #[test]
    fn the_thread_anchor_of_a_deleted_page_is_not_exposed() {
        let conn = bench();
        conn.execute("INSERT INTO bot_pages VALUES ('p1', 'Trois forfaits')", [])
            .unwrap();
        anchor(&conn, "c1", "m1", "p1", "Confort");
        assert_eq!(
            for_message(&conn, "c1", "m1"),
            serde_json::json!({"page_id": "p1", "passage": "Confort", "title": "Trois forfaits", "source_version": "v"})
        );
        conn.execute("DELETE FROM bot_pages", []).unwrap();
        assert_eq!(for_message(&conn, "c1", "m1"), serde_json::Value::Null);
    }

    #[test]
    fn two_conversations_sharing_a_message_id_each_see_their_own_anchor() {
        let conn = bench();
        conn.execute(
            "INSERT INTO bot_pages VALUES ('p1', 'Un'), ('p2', 'Deux')",
            [],
        )
        .unwrap();
        anchor(&conn, "c1", "m", "p1", "gauche");
        anchor(&conn, "c2", "m", "p2", "droite");
        assert_eq!(for_message(&conn, "c1", "m")["passage"], "gauche");
        assert_eq!(for_message(&conn, "c2", "m")["passage"], "droite");
        assert_eq!(for_message(&conn, "c3", "m"), serde_json::Value::Null);
    }

    #[test]
    fn collapsing_reduces_every_run_of_blanks_to_one_space() {
        assert_eq!(
            collapsed("Forfait B\t19 €\n\u{a0}Famille"),
            "Forfait B 19 € Famille"
        );
        assert_eq!(collapsed("  a \r\n\t\u{2003}b  "), "a b");
        assert_eq!(collapsed("Con fort"), "Con fort");
        assert_ne!(collapsed("Con fort"), collapsed("Confort"));
        assert_eq!(collapsed(" \t"), "");
    }

    #[test]
    fn lines_carry_their_one_based_number() {
        let source = "a\nb Confort c\nd\nConfort";
        let lines = source_lines(source, "Confort");
        assert_eq!(
            lines,
            vec![(2, "b Confort c".to_string()), (4, "Confort".to_string())]
        );
        assert!(source_lines(source, "Absent").is_empty());
    }

    #[test]
    fn a_passage_over_several_lines_lists_each_of_them() {
        let lines = source_lines("x\nun\ndeux\ny", "un\ndeux");
        assert_eq!(lines, vec![(2, "un".to_string()), (3, "deux".to_string())]);
    }
}
