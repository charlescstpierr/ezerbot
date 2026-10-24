//! Durable facts scoped to a bot or the local workspace (team).
//! CM09–CM12 in `reports/goal-context-memory-learning.md` establish listing and
//! sharing, but not correction/forgetting: supersession and tombstones are local
//! design choices. Run-trace redaction belongs to gb-memory-forget.
//!
//! The caller owns transactions and component schema versions. Wrap `correct`
//! and `forget` with their tool events/redaction in the caller's transaction.
//! Forgetting returns pre-wipe snapshots, including predecessor provenance.
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

/// Local safety limit, in Unicode characters, applied to new and corrected facts.
pub const MAX_FACT_CHARS: usize = 2_000;

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
    /// Background extraction from an explicit, directly authored user turn.
    Automatic,
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

/// Un souvenir est « à revoir » au-delà de 49 jours (7 semaines) sans reconfirmation.
pub const REVIEW_AFTER_MS: i64 = 49 * 24 * 60 * 60 * 1000;

/// « À revoir » est dérivé à la lecture, jamais stocké. Les dates sont en millisecondes
/// UTC : aucun fuseau ni changement d'heure ne décale la borne. Strictement plus de
/// 49 jours : à 49 jours pile, le souvenir n'est pas encore à revoir.
pub fn needs_review(learned_at: i64, reconfirmed_at: Option<i64>, now_ms: i64) -> bool {
    let last_confirmed = learned_at.max(reconfirmed_at.unwrap_or(learned_at));
    now_ms.saturating_sub(last_confirmed) > REVIEW_AFTER_MS
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
    let has_column: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('memory_facts') WHERE name = 'reconfirmed_at')",
        [],
        |row| row.get(0),
    )?;
    if !has_column {
        conn.execute(
            "ALTER TABLE memory_facts ADD COLUMN reconfirmed_at INTEGER",
            [],
        )?;
    }
    Ok(())
}

/// « Encore vrai » : remet le compteur à zéro sans toucher au texte, à l'ID ni à
/// `learned_at`. Réservé aux faits actifs : un fait oublié ou remplacé ne revit pas
/// (`Ok(false)`). La date ne recule jamais, donc un rappel est sans effet nuisible.
pub fn reconfirm(conn: &Connection, fact_id: &str, now_ms: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE memory_facts SET reconfirmed_at = MAX(?2, COALESCE(reconfirmed_at, ?2))
         WHERE fact_id = ?1 AND status = 'active'",
        params![fact_id, now_ms],
    )? > 0)
}

/// Insert a new active fact without ever overwriting an existing ID. An empty
/// `fact_id` allocates a random `fact_<hex>` ID; all other metadata is preserved.
pub fn remember(conn: &Connection, fact: &Fact) -> Result<Fact> {
    if fact.bot_id.trim().is_empty() || fact.text.trim().is_empty() {
        return Err(invalid("a fact needs a bot and nonempty text"));
    }
    if fact.text.chars().take(MAX_FACT_CHARS + 1).count() > MAX_FACT_CHARS {
        return Err(invalid("fact text exceeds 2000 characters"));
    }
    if fact.status != FactStatus::Active || (fact.sensitive && fact.scope == Scope::Team) {
        return Err(invalid(
            "new facts must be active; sensitive facts must be private",
        ));
    }
    if !fact.fact_id.is_empty()
        && (fact.fact_id.len() != 37
            || !fact.fact_id.strip_prefix("fact_").is_some_and(|hex| {
                hex.bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            }))
    {
        return Err(invalid(
            "fact IDs must be fact_ followed by 32 lowercase hex digits",
        ));
    }
    let mut fact = fact.clone();
    if fact.fact_id.is_empty() {
        fact.fact_id = new_id()?;
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

/// Only the owning bot may correct a fact, including team facts. The replacement
/// keeps owner/scope/sensitivity but records the correction's own provenance.
/// `now_ms` is injected UTC Unix milliseconds; dates remain monotone even when
/// an imported predecessor has a future date. The old text is not overwritten.
pub fn correct(
    conn: &Connection,
    bot_id: &str,
    fact_id: &str,
    new_text: &str,
    provenance: &Provenance,
    now_ms: i64,
) -> Result<Fact> {
    let mut replacement = get(conn, fact_id)?
        .filter(|fact| fact.bot_id == bot_id)
        .ok_or_else(|| invalid("unknown or unavailable fact for correction"))?;
    if replacement.status != FactStatus::Active {
        return Err(invalid("only active facts can be corrected"));
    }
    replacement.learned_at = now_ms.max(
        replacement
            .learned_at
            .checked_add(1)
            .ok_or_else(|| invalid("fact timestamp is exhausted"))?,
    );
    replacement.fact_id.clear();
    replacement.text = new_text.into();
    replacement.provenance = provenance.clone();
    let replacement = remember(conn, &replacement)?;
    let changed = conn.execute(
        "UPDATE memory_facts SET status = 'superseded', superseded_by = ?1
         WHERE fact_id = ?2 AND bot_id = ?3 AND status = 'active'",
        params![replacement.fact_id, fact_id, bot_id],
    )?;
    if changed != 1 {
        return Err(invalid(
            "fact changed during correction; roll back the transaction",
        ));
    }
    Ok(replacement)
}

/// Wipe this owned fact and all superseded predecessors. Return their snapshots
/// BEFORE erasure so the caller can redact every text and invalidate summaries
/// covering every provenance, in the same transaction. IDs/dates remain as
/// tombstones. Repeated forgetting is harmless and returns empty-text snapshots.
pub fn forget(conn: &Connection, bot_id: &str, fact_id: &str) -> Result<Vec<Fact>> {
    // Fully collect the chain before clearing links; an UPDATE with a live
    // recursive selection could lose predecessors as superseded_by becomes NULL.
    let mut statement = conn.prepare(
        "WITH RECURSIVE lineage(id) AS (
            SELECT fact_id FROM memory_facts WHERE fact_id = ?1 AND bot_id = ?2
            UNION
            SELECT f.fact_id FROM memory_facts f JOIN lineage l ON f.superseded_by = l.id
                WHERE f.bot_id = ?2
         )
         SELECT f.* FROM memory_facts f JOIN lineage l ON f.fact_id = l.id
         ORDER BY f.learned_at ASC, f.fact_id ASC",
    )?;
    let snapshots = statement
        .query_map(params![fact_id, bot_id], read_fact)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if snapshots.is_empty() {
        return Err(invalid("unknown or unavailable fact for forgetting"));
    }
    for fact in &snapshots {
        conn.execute(
            "UPDATE memory_facts SET text = '', status = 'forgotten', superseded_by = NULL
             WHERE fact_id = ?1 AND bot_id = ?2",
            params![fact.fact_id, bot_id],
        )?;
    }
    Ok(snapshots)
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

    pub(crate) const NOW_MS: i64 = 1_800_000_000_000;

    pub(crate) fn correction_provenance() -> Provenance {
        Provenance {
            conversation_id: Some("conversation-correction".into()),
            entry_id: Some("entry-correction".into()),
            source: Source::UserStated,
        }
    }

    pub(crate) fn fact(id: usize, bot: &str) -> Fact {
        Fact {
            fact_id: format!("fact_{id:032x}"),
            bot_id: bot.into(),
            scope: Scope::Private,
            text: format!("Fait {id} — français 🦀"),
            provenance: Provenance {
                conversation_id: Some("conversation-1".into()),
                entry_id: Some("entry-1".into()),
                source: Source::UserStated,
            },
            learned_at: 1_700_000_000_000 + id as i64,
            status: FactStatus::Active,
            sensitive: false,
        }
    }

    #[test]
    fn correct_supersedes_without_overwrite() {
        let conn = database();
        let mut inferred = fact(1, "a");
        inferred.provenance.source = Source::ModelInferred;
        let old = remember(&conn, &inferred).unwrap();
        let provenance = correction_provenance();
        let new = with_tx(&conn, |conn| {
            correct(
                conn,
                "a",
                &old.fact_id,
                "Texte corrigé",
                &provenance,
                NOW_MS,
            )
        })
        .unwrap();
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
        assert_eq!(new.provenance, provenance);
        assert_ne!(new.provenance, old.provenance);
        assert_eq!(stored.provenance, old.provenance);
        assert_eq!(new.scope, old.scope);
        assert_eq!(new.learned_at, NOW_MS);
        assert_eq!(list_active(&conn, "a").unwrap(), vec![new]);
        assert!(correct(
            &conn,
            "a",
            &old.fact_id,
            "stale correction",
            &provenance,
            NOW_MS
        )
        .is_err());
    }

    #[test]
    fn forget_wipes_text_keeps_id() {
        let conn = database();
        let old = remember(&conn, &fact(1, "a")).unwrap();
        let snapshots = with_tx(&conn, |conn| forget(conn, "a", &old.fact_id)).unwrap();
        assert_eq!(snapshots, vec![old.clone()]);
        assert!(forget(&conn, "a", &old.fact_id).unwrap()[0].text.is_empty());
        let forgotten = get(&conn, &old.fact_id).unwrap().unwrap();
        assert_eq!(forgotten.fact_id, old.fact_id);
        assert_eq!(forgotten.learned_at, old.learned_at);
        assert_eq!(forgotten.provenance, old.provenance);
        assert_eq!(forgotten.text, "");
        assert_eq!(forgotten.status, FactStatus::Forgotten);
        assert!(correct(
            &conn,
            "a",
            &old.fact_id,
            "cannot revive",
            &correction_provenance(),
            NOW_MS
        )
        .is_err());
        assert!(remember(&conn, &old).is_err());
    }

    #[test]
    fn forgotten_fact_never_listed_as_active() {
        let conn = database();
        let mut shared = fact(1, "a");
        shared.scope = Scope::Team;
        remember(&conn, &shared).unwrap();
        forget(&conn, "a", &shared.fact_id).unwrap();
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
            let original = remember(conn, &fact(1, "a"))?;
            correct(
                conn,
                "a",
                &original.fact_id,
                "replacement",
                &correction_provenance(),
                NOW_MS,
            )?;
            Err(invalid("roll back"))
        });
        assert!(failed.is_err());
        assert!(list_active(&conn, "a").unwrap().is_empty());
        assert!(get(&conn, &fact(1, "a").fact_id).unwrap().is_none());
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
        assert!(forget(&conn, "a", "absent").is_err());
    }

    #[test]
    fn forget_wipes_superseded_predecessors() {
        let conn = database();
        let mut original = fact(1, "a");
        original.scope = Scope::Team;
        original.text = "Mon adresse est 12 rue Secrète".into();
        let old = remember(&conn, &original).unwrap();
        let provenance = correction_provenance();
        let middle = with_tx(&conn, |conn| {
            correct(
                conn,
                "a",
                &old.fact_id,
                "14 rue Secrète",
                &provenance,
                NOW_MS,
            )
        })
        .unwrap();
        let last_provenance = Provenance {
            conversation_id: Some("conversation-3".into()),
            entry_id: Some("entry-3".into()),
            source: Source::Imported,
        };
        let last = with_tx(&conn, |conn| {
            correct(
                conn,
                "a",
                &middle.fact_id,
                "16 rue Secrète",
                &last_provenance,
                NOW_MS + 1,
            )
        })
        .unwrap();
        let snapshots = with_tx(&conn, |conn| forget(conn, "a", &last.fact_id)).unwrap();
        assert_eq!(
            snapshots
                .iter()
                .map(|fact| fact.text.as_str())
                .collect::<Vec<_>>(),
            vec![old.text.as_str(), middle.text.as_str(), last.text.as_str()]
        );
        assert_eq!(snapshots[0].provenance, old.provenance);
        assert_eq!(snapshots[1].provenance, provenance);
        assert_eq!(snapshots[2].provenance, last_provenance);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM memory_facts WHERE text != ''",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        for snapshot in snapshots {
            let erased = get(&conn, &snapshot.fact_id).unwrap().unwrap();
            assert!(erased.text.is_empty());
            assert_eq!(erased.status, FactStatus::Forgotten);
            assert_eq!(erased.learned_at, snapshot.learned_at);
        }
        assert!(list_active(&conn, "a").unwrap().is_empty());
        assert!(list_active(&conn, "b").unwrap().is_empty());
    }

    #[test]
    fn mutation_requires_owner_even_for_team_facts() {
        let conn = database();
        let mut shared = fact(1, "a");
        shared.scope = Scope::Team;
        let original = remember(&conn, &shared).unwrap();
        assert!(correct(
            &conn,
            "b",
            &shared.fact_id,
            "injected edit",
            &correction_provenance(),
            NOW_MS
        )
        .is_err());
        assert!(forget(&conn, "b", &shared.fact_id).is_err());
        assert_eq!(get(&conn, &shared.fact_id).unwrap(), Some(original));
        assert_eq!(list_active(&conn, "b").unwrap().len(), 1);
    }

    #[test]
    fn forget_snapshot_and_wipe_rollback_together() {
        let conn = database();
        let old = remember(&conn, &fact(1, "a")).unwrap();
        let replacement = with_tx(&conn, |conn| {
            correct(
                conn,
                "a",
                &old.fact_id,
                "replacement",
                &correction_provenance(),
                NOW_MS,
            )
        })
        .unwrap();
        let failed: Result<()> = with_tx(&conn, |conn| {
            assert_eq!(forget(conn, "a", &replacement.fact_id)?.len(), 2);
            Err(invalid("trace redaction failed"))
        });
        assert!(failed.is_err());
        assert_eq!(get(&conn, &old.fact_id).unwrap().unwrap().text, old.text);
        assert_eq!(get(&conn, &replacement.fact_id).unwrap(), Some(replacement));
    }

    #[test]
    fn text_and_supplied_id_are_bounded() {
        let conn = database();
        let mut value = fact(1, "a");
        value.text = "🦀".repeat(MAX_FACT_CHARS);
        remember(&conn, &value).unwrap();
        value.fact_id = fact(2, "a").fact_id;
        value.text.push('é');
        assert!(remember(&conn, &value).is_err());
        value.text = "valid".into();
        for id in [
            "fact_1".into(),
            format!("fact_{}", "1".repeat(100_000)),
            format!("fact_{}", "A".repeat(32)),
            format!("fact_{}", "g".repeat(32)),
        ] {
            value.fact_id = id;
            assert!(remember(&conn, &value).is_err());
        }
        assert!(correct(
            &conn,
            "a",
            &fact(1, "a").fact_id,
            &"x".repeat(MAX_FACT_CHARS + 1),
            &correction_provenance(),
            NOW_MS
        )
        .is_err());
        assert_eq!(list_active(&conn, "a").unwrap().len(), 1);
    }

    #[test]
    fn correction_dates_use_injected_clock_and_surpass_future_imports() {
        let conn = database();
        let mut future = fact(1, "a");
        future.learned_at = NOW_MS + 60_000;
        remember(&conn, &future).unwrap();
        let replacement = with_tx(&conn, |conn| {
            correct(
                conn,
                "a",
                &future.fact_id,
                "corrected",
                &correction_provenance(),
                NOW_MS,
            )
        })
        .unwrap();
        assert_eq!(replacement.learned_at, future.learned_at + 1);
        let mut exhausted = fact(2, "a");
        exhausted.learned_at = i64::MAX;
        remember(&conn, &exhausted).unwrap();
        assert!(correct(
            &conn,
            "a",
            &exhausted.fact_id,
            "overflow",
            &correction_provenance(),
            NOW_MS
        )
        .is_err());
        assert_eq!(get(&conn, &exhausted.fact_id).unwrap(), Some(exhausted));
    }

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    #[test]
    fn review_boundary_is_strictly_more_than_49_days() {
        let age = |days_ms: i64, again: Option<i64>| needs_review(NOW_MS - days_ms, again, NOW_MS);
        assert!(!age(48 * DAY_MS, None));
        assert!(!age(49 * DAY_MS, None), "49 j pile : pas encore");
        assert!(age(49 * DAY_MS + 1, None), "49 j + 1 ms : à revoir");
        assert!(age(50 * DAY_MS, None));
        assert!(!age(90 * DAY_MS, Some(NOW_MS - DAY_MS)));
        assert!(age(90 * DAY_MS, Some(NOW_MS - 60 * DAY_MS)));
        assert!(!age(-DAY_MS, None), "date future : jamais à revoir");
    }

    #[test]
    fn reconfirm_keeps_fact_identical_and_refuses_forgotten_or_superseded() {
        let conn = database();
        let old = remember(&conn, &fact(1, "a")).unwrap();
        let stamp = |conn: &Connection| -> Option<i64> {
            let sql = "SELECT reconfirmed_at FROM memory_facts WHERE fact_id = ?1";
            conn.query_row(sql, [&old.fact_id], |r| r.get(0)).unwrap()
        };
        for _ in 0..2 {
            assert!(reconfirm(&conn, &old.fact_id, NOW_MS).unwrap());
            assert_eq!(get(&conn, &old.fact_id).unwrap().unwrap(), old);
            assert_eq!(stamp(&conn), Some(NOW_MS));
        }
        assert!(reconfirm(&conn, &old.fact_id, NOW_MS - DAY_MS).unwrap());
        assert_eq!(stamp(&conn), Some(NOW_MS), "la date ne recule pas");
        let new = correct(
            &conn,
            "a",
            &old.fact_id,
            "Neuf",
            &correction_provenance(),
            NOW_MS,
        );
        assert!(!reconfirm(&conn, &old.fact_id, NOW_MS).unwrap(), "remplacé");
        assert!(reconfirm(&conn, &new.unwrap().fact_id, NOW_MS).unwrap());
        forget(&conn, "a", &old.fact_id).unwrap();
        assert!(!reconfirm(&conn, &old.fact_id, NOW_MS).unwrap(), "oublié");
        assert!(!reconfirm(&conn, "fact_absent", NOW_MS).unwrap());
        assert_eq!(get(&conn, &old.fact_id).unwrap().unwrap().text, "");
    }

    #[test]
    fn migration_adds_the_column_to_an_existing_table_once() {
        let conn = database();
        conn.execute("ALTER TABLE memory_facts DROP COLUMN reconfirmed_at", [])
            .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        conn.prepare("SELECT reconfirmed_at FROM memory_facts")
            .unwrap();
    }
}
