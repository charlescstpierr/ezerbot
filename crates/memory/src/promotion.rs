//! Local-team promotion (CM09 in `reports/goal-context-memory-learning.md`,
//! RC02–RC04 in `reports/server-context-residual.md`). These establish IDs,
//! 100-item batches, distinct counts and preservation of earlier batch successes.
//! Team visibility, sensitive-fact protection and durable batch replay are local
//! design choices, not inferred server behavior.
//!
//! The caller must wrap each batch in its transaction, including its tool-result
//! event. No transaction is opened here; earlier committed batches are untouched.
use crate::facts::{self, FactStatus, Scope};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const MAX_PROMOTION_BATCH: usize = 100;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionReport {
    pub created: Vec<String>,
    pub already_in_team: Vec<String>,
    pub kept_private_count: usize,
}

pub fn migrate(conn: &Connection) -> Result<()> {
    // Receipts contain IDs/counts only, never fact text that forgetting must wipe.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_promotions (
            bot_id TEXT NOT NULL,
            batch_key TEXT NOT NULL,
            report TEXT NOT NULL,
            PRIMARY KEY (bot_id, batch_key)
        );",
    )?;
    Ok(())
}

/// Promote at most 100 owned private or workspace-visible team IDs. Duplicate
/// IDs count once; a sorted unique selection identifies a durable batch receipt.
/// Replays return the original report even after correction/forgetting: receipts
/// contain only IDs/counts and never mutate facts. New batches reject unknown,
/// foreign-private or inactive IDs before writes, while foreign team facts count
/// as already in team.
pub fn promote(conn: &Connection, bot_id: &str, fact_ids: &[String]) -> Result<PromotionReport> {
    if bot_id.trim().is_empty() || fact_ids.len() > MAX_PROMOTION_BATCH {
        return Err(Error::InvalidInput(
            "promotion needs a bot and at most 100 IDs".into(),
        ));
    }
    let mut ids = fact_ids.to_vec();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(PromotionReport::default());
    }
    let key = serde_json::to_string(&ids)?;
    let receipt: Option<String> = conn
        .query_row(
            "SELECT report FROM memory_promotions WHERE bot_id = ?1 AND batch_key = ?2",
            params![bot_id, key],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(receipt) = receipt {
        return Ok(serde_json::from_str(&receipt)?);
    }

    let mut selected = Vec::with_capacity(ids.len());
    for id in &ids {
        let fact = facts::get(conn, id)?
            .filter(|fact| {
                fact.status == FactStatus::Active
                    && (fact.bot_id == bot_id || fact.scope == Scope::Team)
            })
            .ok_or_else(|| {
                Error::InvalidInput("unknown or unavailable fact for promotion".into())
            })?;
        selected.push(fact);
    }
    let mut report = PromotionReport::default();
    for fact in selected {
        if fact.sensitive {
            report.kept_private_count += 1;
        } else if fact.scope == Scope::Team {
            report.already_in_team.push(fact.fact_id);
        } else {
            let changed = conn.execute(
                "UPDATE memory_facts SET scope = 'team'
                 WHERE fact_id = ?1 AND bot_id = ?2 AND status = 'active'
                    AND scope = 'private' AND sensitive = 0",
                params![fact.fact_id, bot_id],
            )?;
            if changed != 1 {
                return Err(Error::InvalidInput(
                    "fact changed during promotion; roll back the transaction".into(),
                ));
            }
            report.created.push(fact.fact_id);
        }
    }
    conn.execute(
        "INSERT INTO memory_promotions (bot_id, batch_key, report) VALUES (?1, ?2, ?3)",
        params![bot_id, key, serde_json::to_string(&report)?],
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::tests::{correction_provenance, database, fact, NOW_MS};
    use grokbot_engine::contracts::with_tx;

    #[test]
    fn promote_batches_of_100_report_created_already_kept() {
        let conn = database();
        let mut ids = Vec::new();
        for id in 0..100 {
            let mut value = fact(id, "a");
            value.sensitive = id >= 80;
            if id < 10 {
                value.scope = Scope::Team;
            }
            ids.push(facts::remember(&conn, &value).unwrap().fact_id);
        }
        let report = with_tx(&conn, |conn| promote(conn, "a", &ids)).unwrap();
        assert_eq!(report.created.len(), 70);
        assert_eq!(report.already_in_team.len(), 10);
        assert_eq!(report.kept_private_count, 20);
        assert_eq!(facts::list_active(&conn, "b").unwrap().len(), 80);
        let receipt_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_promotions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(receipt_count, 1);
    }

    #[test]
    fn replayed_promotion_is_idempotent() {
        let conn = database();
        let first = facts::remember(&conn, &fact(1, "a")).unwrap();
        let second = facts::remember(&conn, &fact(2, "a")).unwrap();
        let ids = vec![first.fact_id.clone(), second.fact_id.clone()];
        let report = with_tx(&conn, |conn| promote(conn, "a", &ids)).unwrap();
        assert_eq!(report.created.len(), 2);
        let replay = vec![
            second.fact_id.clone(),
            first.fact_id.clone(),
            first.fact_id.clone(),
        ];
        assert_eq!(
            with_tx(&conn, |conn| promote(conn, "a", &replay)).unwrap(),
            report
        );
        let overlap = promote(&conn, "a", std::slice::from_ref(&first.fact_id)).unwrap();
        assert_eq!(overlap.already_in_team, vec![first.fact_id]);
        assert!(overlap.created.is_empty());
        assert_eq!(facts::list_active(&conn, "b").unwrap().len(), 2);
    }

    #[test]
    fn sensitive_fact_stays_private() {
        let conn = database();
        let mut sensitive = fact(1, "a");
        sensitive.sensitive = true;
        facts::remember(&conn, &sensitive).unwrap();
        let ids = vec![sensitive.fact_id.clone()];
        let report = promote(&conn, "a", &ids).unwrap();
        assert_eq!(
            report,
            PromotionReport {
                kept_private_count: 1,
                ..Default::default()
            }
        );
        assert_eq!(promote(&conn, "a", &ids).unwrap(), report);
        assert_eq!(facts::get(&conn, &ids[0]).unwrap(), Some(sensitive.clone()));
        assert!(facts::list_active(&conn, "b").unwrap().is_empty());
        let corrected = with_tx(&conn, |conn| {
            facts::correct(
                conn,
                "a",
                &ids[0],
                "still private",
                &correction_provenance(),
                NOW_MS,
            )
        })
        .unwrap();
        assert!(corrected.sensitive);
        assert_eq!(corrected.scope, Scope::Private);
        sensitive.fact_id.clear();
        sensitive.scope = Scope::Team;
        assert!(facts::remember(&conn, &sensitive).is_err());
    }

    #[test]
    fn failed_later_batch_keeps_previous_success_and_validates_before_writes() {
        let conn = database();
        let first = facts::remember(&conn, &fact(1, "a")).unwrap();
        let second = facts::remember(&conn, &fact(2, "a")).unwrap();
        let foreign = facts::remember(&conn, &fact(3, "b")).unwrap();
        let first_report = with_tx(&conn, |conn| {
            promote(conn, "a", std::slice::from_ref(&first.fact_id))
        })
        .unwrap();
        for invalid_ids in [
            vec![second.fact_id.clone(), "missing".into()],
            vec![second.fact_id.clone(), foreign.fact_id.clone()],
            vec![second.fact_id.clone(); 101],
        ] {
            assert!(with_tx(&conn, |conn| promote(conn, "a", &invalid_ids)).is_err());
        }
        assert_eq!(
            facts::get(&conn, &first.fact_id).unwrap().unwrap().scope,
            Scope::Team
        );
        assert_eq!(
            facts::get(&conn, &second.fact_id).unwrap().unwrap().scope,
            Scope::Private
        );
        facts::forget(&conn, "a", &first.fact_id).unwrap();
        assert_eq!(
            promote(&conn, "a", std::slice::from_ref(&first.fact_id)).unwrap(),
            first_report
        );
        let corrected = with_tx(&conn, |conn| {
            facts::correct(
                conn,
                "a",
                &second.fact_id,
                "new",
                &correction_provenance(),
                NOW_MS,
            )
        })
        .unwrap();
        assert!(promote(&conn, "a", &[second.fact_id]).is_err());
        assert_eq!(
            facts::get(&conn, &corrected.fact_id)
                .unwrap()
                .unwrap()
                .scope,
            Scope::Private
        );
    }

    #[test]
    fn batch_rollback_includes_receipt() {
        let conn = database();
        let saved = facts::remember(&conn, &fact(1, "a")).unwrap();
        let ids = vec![saved.fact_id];
        let failed: Result<()> = with_tx(&conn, |conn| {
            promote(conn, "a", &ids)?;
            Err(Error::InvalidInput("tool event failed".into()))
        });
        assert!(failed.is_err());
        assert_eq!(
            facts::get(&conn, &ids[0]).unwrap().unwrap().scope,
            Scope::Private
        );
        let report = with_tx(&conn, |conn| promote(conn, "a", &ids)).unwrap();
        assert_eq!(report.created, ids);
        assert!(promote(&conn, "a", &[]).unwrap().created.is_empty());
    }

    #[test]
    fn selecting_all_visible_facts_accepts_foreign_team_facts() {
        let conn = database();
        let owned = facts::remember(&conn, &fact(1, "a")).unwrap();
        let mut shared = fact(2, "b");
        shared.scope = Scope::Team;
        facts::remember(&conn, &shared).unwrap();
        let foreign_private = facts::remember(&conn, &fact(3, "b")).unwrap();
        let ids = facts::list_active(&conn, "a")
            .unwrap()
            .into_iter()
            .map(|fact| fact.fact_id)
            .collect::<Vec<_>>();
        let report = with_tx(&conn, |conn| promote(conn, "a", &ids)).unwrap();
        assert_eq!(report.created, vec![owned.fact_id]);
        assert_eq!(report.already_in_team, vec![shared.fact_id]);
        assert_eq!(report.kept_private_count, 0);
        assert!(promote(&conn, "a", &[foreign_private.fact_id]).is_err());
    }

    #[test]
    fn promotion_replay_after_correction_and_forgetting_never_resurrects_text() {
        let conn = database();
        let first = facts::remember(&conn, &fact(1, "a")).unwrap();
        let second = facts::remember(&conn, &fact(2, "a")).unwrap();
        let ids = vec![first.fact_id.clone(), second.fact_id.clone()];
        let report = with_tx(&conn, |conn| promote(conn, "a", &ids)).unwrap();
        let corrected = with_tx(&conn, |conn| {
            facts::correct(
                conn,
                "a",
                &first.fact_id,
                "new shared text",
                &correction_provenance(),
                NOW_MS,
            )
        })
        .unwrap();
        with_tx(&conn, |conn| facts::forget(conn, "a", &second.fact_id)).unwrap();
        assert_eq!(promote(&conn, "a", &ids).unwrap(), report);
        assert_eq!(
            facts::list_active(&conn, "b").unwrap(),
            vec![corrected.clone()]
        );
        assert_eq!(
            facts::get(&conn, &first.fact_id).unwrap().unwrap().status,
            FactStatus::Superseded {
                by: corrected.fact_id
            }
        );
        let forgotten = facts::get(&conn, &second.fact_id).unwrap().unwrap();
        assert_eq!(forgotten.status, FactStatus::Forgotten);
        assert!(forgotten.text.is_empty());
        assert!(promote(&conn, "a", std::slice::from_ref(&second.fact_id)).is_err());
    }

    #[test]
    fn facts_and_replay_survive_reopening_database() {
        struct TempDatabase(std::path::PathBuf);
        impl Drop for TempDatabase {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let path = TempDatabase(std::env::temp_dir().join(format!(
            "grokbot-memory-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        )));
        let ids = vec![fact(1, "a").fact_id];
        let report = {
            let conn = Connection::open(&path.0).unwrap();
            crate::migrate(&conn).unwrap();
            facts::remember(&conn, &fact(1, "a")).unwrap();
            with_tx(&conn, |conn| promote(conn, "a", &ids)).unwrap()
        };
        {
            let conn = Connection::open(&path.0).unwrap();
            crate::migrate(&conn).unwrap();
            assert_eq!(promote(&conn, "a", &ids).unwrap(), report);
            assert_eq!(facts::list_active(&conn, "b").unwrap().len(), 1);
            facts::forget(&conn, "a", &ids[0]).unwrap();
        }
        {
            let conn = Connection::open(&path.0).unwrap();
            assert_eq!(facts::get(&conn, &ids[0]).unwrap().unwrap().text, "");
            assert!(facts::list_active(&conn, "b").unwrap().is_empty());
            assert_eq!(promote(&conn, "a", &ids).unwrap(), report);
        }
    }
}
