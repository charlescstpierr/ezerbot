//! Versioned, derived profile and metadata-only change log. No second copy of
//! fact text: correction/forgetting immediately removes it from every summary.
use crate::{facts, recall};
use grokbot_engine::contracts::Result;
use rusqlite::Connection;
use serde_json::{json, Value};

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS memory_profile_log (
        version INTEGER PRIMARY KEY AUTOINCREMENT,
        bot_id TEXT NOT NULL, fact_id TEXT NOT NULL, action TEXT NOT NULL,
        summary_ids TEXT NOT NULL DEFAULT '[]',
        changed_at INTEGER NOT NULL DEFAULT (unixepoch()*1000));
        CREATE INDEX IF NOT EXISTS memory_profile_owner ON memory_profile_log(bot_id,version);
        CREATE TRIGGER IF NOT EXISTS memory_profile_snapshot AFTER INSERT ON memory_profile_log BEGIN
            UPDATE memory_profile_log SET summary_ids=(SELECT json_group_array(fact_id) FROM
                (SELECT fact_id FROM memory_facts WHERE bot_id=NEW.bot_id AND status='active'
                ORDER BY learned_at DESC,fact_id LIMIT 9)) WHERE version=NEW.version; END;
        CREATE TRIGGER IF NOT EXISTS memory_profile_insert AFTER INSERT ON memory_facts BEGIN
            INSERT INTO memory_profile_log(bot_id,fact_id,action) VALUES (NEW.bot_id,NEW.fact_id,'learned'); END;
        CREATE TRIGGER IF NOT EXISTS memory_profile_update AFTER UPDATE OF text,status,scope ON memory_facts BEGIN
            INSERT INTO memory_profile_log(bot_id,fact_id,action) VALUES (NEW.bot_id,NEW.fact_id,
                CASE WHEN NEW.status='forgotten' THEN 'forgotten' WHEN NEW.status='superseded' THEN 'corrected'
                WHEN NEW.scope!=OLD.scope THEN 'shared' ELSE 'updated' END); END;
        INSERT INTO memory_profile_log(bot_id,fact_id,action)
            SELECT bot_id,MIN(fact_id),'baseline' FROM memory_facts f WHERE status='active'
            AND NOT EXISTS (SELECT 1 FROM memory_profile_log l WHERE l.bot_id=f.bot_id)
            GROUP BY bot_id;")?;
    Ok(())
}

pub fn read(conn: &Connection, bot: &str) -> Result<Value> {
    let facts = facts::list_active(conn, bot)?;
    let mut stmt = conn.prepare("SELECT version,action,changed_at,summary_ids FROM memory_profile_log WHERE bot_id=?1 ORDER BY version DESC LIMIT 8")?;
    let rows = stmt
        .query_map([bot], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let log: Vec<_> = rows
        .into_iter()
        .map(|(version, action, changed_at, ids)| {
            let ids: Vec<String> = serde_json::from_str(&ids).unwrap_or_default();
            // Resolve through today's active scope, never expose a forgotten,
            // superseded or no-longer-visible fact in a historical summary.
            let summary: Vec<_> = ids
                .iter()
                .filter_map(|id| facts.iter().find(|f| f.fact_id == *id))
                .collect();
            json!({"version":version,"action":action,"changed_at":changed_at,"summary":summary})
        })
        .collect();
    let sections: Vec<_> = [("work", "Projets et travail"), ("personal", "Préférences et vie personnelle"), ("review", "Autres faits à vérifier")].into_iter().map(|(class, title)| {
        let members: Vec<_> = facts.iter().filter(|f| recall::sharing_class(f)==class).collect();
        json!({"title":title,"count":members.len(),"facts":members.into_iter().take(3).collect::<Vec<_>>()})
    }).collect();
    Ok(
        json!({"version":log.first().map(|v|v["version"].clone()).unwrap_or(json!(0)),"total":facts.len(),"sections":sections,"log":log}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::tests::{correction_provenance, database, fact, NOW_MS};
    #[test]
    fn existing_catalogue_gets_one_baseline_and_migration_is_idempotent() {
        let conn = database();
        facts::remember(&conn, &fact(1, "a")).unwrap();
        conn.execute("DELETE FROM memory_profile_log", []).unwrap();
        migrate(&conn).unwrap();
        let first = read(&conn, "a").unwrap();
        assert_eq!(first["log"][0]["action"], "baseline");
        assert_eq!(first["log"][0]["summary"].as_array().unwrap().len(), 1);
        migrate(&conn).unwrap();
        assert_eq!(read(&conn, "a").unwrap(), first);
    }
    #[test]
    fn versioned_summaries_never_resurrect_corrected_or_forgotten_text() {
        let conn = database();
        let old = facts::remember(&conn, &fact(1, "a")).unwrap();
        let first = read(&conn, "a").unwrap();
        assert_eq!(first["log"][0]["summary"][0]["text"], old.text);
        let new = facts::correct(
            &conn,
            "a",
            &old.fact_id,
            "Nouveau projet",
            &correction_provenance(),
            NOW_MS,
        )
        .unwrap();
        let updated = read(&conn, "a").unwrap();
        assert!(updated["version"].as_i64().unwrap() > first["version"].as_i64().unwrap());
        assert!(!updated.to_string().contains(&old.text));
        facts::forget(&conn, "a", &new.fact_id).unwrap();
        let forgotten = read(&conn, "a").unwrap();
        assert!(!forgotten.to_string().contains("Nouveau projet"));
        assert_eq!(forgotten["total"], 0);
        assert_eq!(read(&conn, "b").unwrap()["log"], json!([]));
    }
}
