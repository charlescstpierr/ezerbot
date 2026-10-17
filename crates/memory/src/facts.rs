//! Durable facts scoped to a bot or the local workspace (team).
//! CM09–CM12 in `reports/goal-context-memory-learning.md` establish listing and
//! sharing, but not correction/forgetting: supersession and tombstones are local
//! design choices. Run-trace redaction belongs to gb-memory-forget.
//!
//! The caller owns transactions and component schema versions. In particular,
//! wrap `correct` in the caller's transaction to commit both writes together.
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Private,
    Team,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    UserStated,
    ModelInferred,
    Imported,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub conversation_id: Option<String>,
    pub entry_id: Option<String>,
    pub source: Source,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FactStatus {
    Active,
    Superseded { by: String },
    Forgotten,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub fact_id: String,
    pub bot_id: String,
    pub scope: Scope,
    pub text: String,
    pub provenance: Provenance,
    pub learned_at: i64,
    pub status: FactStatus,
    pub sensitive: bool,
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_facts (
            fact_id TEXT PRIMARY KEY NOT NULL,
            bot_id TEXT NOT NULL,
            scope TEXT NOT NULL CHECK (scope IN ('private', 'team')),
            text TEXT NOT NULL,
            provenance TEXT NOT NULL,
            learned_at INTEGER NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('active', 'superseded', 'forgotten')),
            superseded_by TEXT,
            sensitive INTEGER NOT NULL CHECK (sensitive IN (0, 1)),
            CHECK (sensitive = 0 OR scope = 'private'),
            CHECK (status != 'forgotten' OR text = ''),
            CHECK ((status = 'superseded' AND superseded_by IS NOT NULL)
                OR (status != 'superseded' AND superseded_by IS NULL))
        );
        CREATE INDEX IF NOT EXISTS memory_facts_bot_active
            ON memory_facts(bot_id, status, learned_at DESC);
        CREATE INDEX IF NOT EXISTS memory_facts_team_active
            ON memory_facts(scope, status, learned_at DESC);",
    )?;
    Ok(())
}

/// Insert a new active fact without ever overwriting an existing ID. An empty
/// `fact_id` allocates a random `fact_<hex>` ID; all other metadata is preserved.
pub fn remember(conn: &Connection, fact: &Fact) -> Result<Fact> {
    if fact.bot_id.trim().is_empty() || fact.text.trim().is_empty() {
        return Err(invalid("a fact needs a bot and nonempty text"));
    }
    if fact.status != FactStatus::Active || (fact.sensitive && fact.scope == Scope::Team) {
        return Err(invalid(
            "new facts must be active; sensitive facts must be private",
        ));
    }
    let mut fact = fact.clone();
    if fact.fact_id.is_empty() {
        fact.fact_id = new_id()?;
    }
    if !fact
        .fact_id
        .strip_prefix("fact_")
        .is_some_and(|hex| !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(invalid("fact IDs must have the form fact_<hex>"));
    }
    conn.execute(
        "INSERT INTO memory_facts
            (fact_id, bot_id, scope, text, provenance, learned_at, status, sensitive)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7)",
        params![
            fact.fact_id,
            fact.bot_id,
            if fact.scope == Scope::Team {
                "team"
            } else {
                "private"
            },
            fact.text,
            serde_json::to_string(&fact.provenance)?,
            fact.learned_at,
            fact.sensitive,
        ],
    )?;
    Ok(fact)
}

pub fn get(conn: &Connection, fact_id: &str) -> Result<Option<Fact>> {
    Ok(conn
        .query_row(
            "SELECT * FROM memory_facts WHERE fact_id = ?1",
            [fact_id],
            read_fact,
        )
        .optional()?)
}

/// Active private facts of this bot plus active team facts from every bot,
/// newest first. Superseded/forgotten facts never enter model context.
pub fn list_active(conn: &Connection, bot_id: &str) -> Result<Vec<Fact>> {
    let mut statement = conn.prepare(
        "SELECT * FROM memory_facts
         WHERE status = 'active' AND (bot_id = ?1 OR scope = 'team')
         ORDER BY learned_at DESC, fact_id ASC",
    )?;
    let rows = statement.query_map([bot_id], read_fact)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Create a replacement with a fresh ID/date, preserving provenance, owner,
/// scope and sensitivity. The old text remains in its superseded record.
pub fn correct(conn: &Connection, fact_id: &str, new_text: &str) -> Result<Fact> {
    let mut replacement = get(conn, fact_id)?.ok_or_else(|| invalid("unknown fact"))?;
    if replacement.status != FactStatus::Active {
        return Err(invalid("only active facts can be corrected"));
    }
    replacement.fact_id.clear();
    replacement.text = new_text.into();
    replacement.learned_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .ok_or_else(|| invalid("system clock is outside the supported time range"))?;
    let replacement = remember(conn, &replacement)?;
    let changed = conn.execute(
        "UPDATE memory_facts SET status = 'superseded', superseded_by = ?1
         WHERE fact_id = ?2 AND status = 'active'",
        params![replacement.fact_id, fact_id],
    )?;
    if changed != 1 {
        return Err(invalid(
            "fact changed during correction; roll back the transaction",
        ));
    }
    Ok(replacement)
}

/// Wipe the stored text, retaining the stable ID, date and provenance needed by
/// the wrapper to redact run traces. Repeated forgetting is harmless.
pub fn forget(conn: &Connection, fact_id: &str) -> Result<()> {
    if conn.execute(
        "UPDATE memory_facts SET text = '', status = 'forgotten', superseded_by = NULL
         WHERE fact_id = ?1",
        [fact_id],
    )? == 0
    {
        return Err(invalid("unknown fact"));
    }
    Ok(())
}

fn new_id() -> Result<String> {
    let mut bytes = [0; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!("fact_{hex}"))
}

fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}

fn read_fact(row: &Row<'_>) -> rusqlite::Result<Fact> {
    // CHECK constraints restrict these columns to the enum's known variants.
    let scope: String = row.get("scope")?;
    let status: String = row.get("status")?;
    let provenance: String = row.get("provenance")?;
    Ok(Fact {
        fact_id: row.get("fact_id")?,
        bot_id: row.get("bot_id")?,
        scope: if scope == "team" {
            Scope::Team
        } else {
            Scope::Private
        },
        text: row.get("text")?,
        provenance: serde_json::from_str(&provenance).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                row.as_ref().column_index("provenance").unwrap_or(4),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        learned_at: row.get("learned_at")?,
        status: match status.as_str() {
            "active" => FactStatus::Active,
            "forgotten" => FactStatus::Forgotten,
            _ => FactStatus::Superseded {
                by: row.get("superseded_by")?,
            },
        },
        sensitive: row.get("sensitive")?,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use grokbot_engine::contracts::with_tx;

    pub(crate) fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        conn
    }

    pub(crate) fn fact(id: usize, bot: &str) -> Fact {
        Fact {
            fact_id: format!("fact_{id:x}"),
            bot_id: bot.into(),
            scope: Scope::Private,
            text: format!("Fait {id} — français 🦀"),
            provenance: Provenance {
                conversation_id: Some("conversation-1".into()),
                entry_id: Some("entry-1".into()),
                source: Source::UserStated,
            },
            learned_at: id as i64,
            status: FactStatus::Active,
            sensitive: false,
        }
    }

    #[test]
    fn correct_supersedes_without_overwrite() {
        let conn = database();
        let old = remember(&conn, &fact(1, "a")).unwrap();
        let new = with_tx(&conn, |conn| correct(conn, &old.fact_id, "Texte corrigé")).unwrap();
        let stored = get(&conn, &old.fact_id).unwrap().unwrap();
        assert_eq!(stored.text, old.text);
        assert_eq!(
            stored.status,
            FactStatus::Superseded {
                by: new.fact_id.clone()
            }
        );
        assert_ne!(new.fact_id, old.fact_id);
        assert_eq!(new.text, "Texte corrigé");
        assert_eq!(new.provenance, old.provenance);
        assert_eq!(new.scope, old.scope);
        assert!(new.learned_at > old.learned_at);
        assert_eq!(list_active(&conn, "a").unwrap(), vec![new]);
        assert!(correct(&conn, &old.fact_id, "stale correction").is_err());
    }

    #[test]
    fn forget_wipes_text_keeps_id() {
        let conn = database();
        let old = remember(&conn, &fact(1, "a")).unwrap();
        forget(&conn, &old.fact_id).unwrap();
        forget(&conn, &old.fact_id).unwrap();
        let forgotten = get(&conn, &old.fact_id).unwrap().unwrap();
        assert_eq!(forgotten.fact_id, old.fact_id);
        assert_eq!(forgotten.learned_at, old.learned_at);
        assert_eq!(forgotten.provenance, old.provenance);
        assert_eq!(forgotten.text, "");
        assert_eq!(forgotten.status, FactStatus::Forgotten);
        assert!(correct(&conn, &old.fact_id, "cannot revive").is_err());
        assert!(remember(&conn, &old).is_err());
    }

    #[test]
    fn forgotten_fact_never_listed_as_active() {
        let conn = database();
        let mut shared = fact(1, "a");
        shared.scope = Scope::Team;
        remember(&conn, &shared).unwrap();
        forget(&conn, &shared.fact_id).unwrap();
        assert!(list_active(&conn, "a").unwrap().is_empty());
        assert!(list_active(&conn, "b").unwrap().is_empty());
    }

    #[test]
    fn private_is_bot_scoped_team_is_workspace_scoped_and_newest_first() {
        let conn = database();
        let private = remember(&conn, &fact(1, "a")).unwrap();
        remember(&conn, &fact(3, "b")).unwrap();
        let mut shared = fact(2, "b");
        shared.scope = Scope::Team;
        remember(&conn, &shared).unwrap();
        assert_eq!(
            list_active(&conn, "a").unwrap(),
            vec![shared.clone(), private]
        );
        assert_eq!(list_active(&conn, "c").unwrap(), vec![shared]);
    }

    #[test]
    fn migrations_and_writes_respect_caller_transaction_and_user_version() {
        let conn = database();
        conn.pragma_update(None, "user_version", 42).unwrap();
        crate::migrate(&conn).unwrap();
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
        let failed: Result<()> = with_tx(&conn, |conn| {
            remember(conn, &fact(1, "a"))?;
            correct(conn, "fact_1", "replacement")?;
            Err(invalid("roll back"))
        });
        assert!(failed.is_err());
        assert!(list_active(&conn, "a").unwrap().is_empty());
        assert!(get(&conn, "fact_1").unwrap().is_none());
        assert!(conn.is_autocommit());
    }

    #[test]
    fn remember_allocates_ids_and_rejects_invalid_or_duplicate_facts() {
        let conn = database();
        let mut value = fact(1, "a");
        value.fact_id.clear();
        let saved = remember(&conn, &value).unwrap();
        assert_eq!(saved.fact_id.len(), 37);
        assert_eq!(get(&conn, &saved.fact_id).unwrap(), Some(saved.clone()));
        assert!(remember(&conn, &saved).is_err());
        value.text = " \n\t".into();
        assert!(remember(&conn, &value).is_err());
        assert!(get(&conn, "absent").unwrap().is_none());
        assert!(forget(&conn, "absent").is_err());
    }
}
