//! Admission boundary with the Q3 defaults: 4 active workspace-wide, 3 per root,
//! depth 3, backlog 16. No human approval is required to delegate. These bounds
//! are reconstruction policy, not a recovered Grok server algorithm (see
//! `reports/delegation-lifecycle-deep.md` and `server-context-residual.md` RC05).
//!
//! `admit` is a decision, not a reservation. The caller must run admission and
//! `graph::create` (or queued-to-running `graph::set_status`) together in one
//! write transaction. A queued retry does not consume a second backlog slot.
use crate::graph::{self, Delegation, DelegationStatus};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionLimits {
    pub active_workspace: usize,
    pub active_per_root: usize,
    pub max_depth: u32,
    pub backlog: usize,
}
impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            active_workspace: 4,
            active_per_root: 3,
            max_depth: 3,
            backlog: 16,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Admission {
    Running,
    Queued,
    Refused { reason: String },
}

fn refused(reason: impl Into<String>) -> Admission {
    Admission::Refused {
        reason: reason.into(),
    }
}

/// Refusals are readable tool outcomes; storage/resolver errors remain errors,
/// never silently become admissions. Queue capacity counts the whole workspace,
/// whereas active-per-root counts every running descendant of that run root.
pub fn admit(
    conn: &Connection,
    delegation: &Delegation,
    limits: &AdmissionLimits,
) -> Result<Admission> {
    let existing = graph::get(conn, &delegation.id)?;
    if let Some(existing) = &existing {
        if !graph::same_request(existing, delegation) {
            return Ok(refused(
                "delegation identifier already belongs to another request",
            ));
        }
        match &existing.status {
            DelegationStatus::Running => return Ok(Admission::Running),
            DelegationStatus::Queued => {}
            DelegationStatus::Refused { reason } => return Ok(refused(reason)),
            _ => return Ok(refused("delegation is already terminal")),
        }
    }
    if delegation.depth > limits.max_depth {
        return Ok(refused(format!(
            "delegation depth exceeds {}",
            limits.max_depth
        )));
    }
    match graph::validate_request(conn, delegation) {
        Ok(()) => {}
        Err(Error::InvalidInput(reason)) => return Ok(refused(reason)),
        Err(error) => return Err(error),
    }
    let (active, root_active, queued): (u64, u64, u64) = conn.query_row(
        "SELECT
            COUNT(CASE WHEN json_extract(status, '$.status') = 'running' THEN 1 END),
            COUNT(CASE WHEN json_extract(status, '$.status') = 'running'
                AND root_run_id = ?2 THEN 1 END),
            COUNT(CASE WHEN json_extract(status, '$.status') = 'queued' THEN 1 END)
         FROM delegation_graph WHERE id != ?1",
        [&delegation.id, &delegation.root_run_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if active < limits.active_workspace as u64 && root_active < limits.active_per_root as u64 {
        return Ok(Admission::Running);
    }
    if existing.is_some() || queued < limits.backlog as u64 {
        return Ok(Admission::Queued);
    }
    Ok(refused(format!(
        "delegation backlog is full (limit {})",
        limits.backlog
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{database, delegation, nested, running};
    use grokbot_engine::contracts::with_tx;

    // This is also the wrapper's composition contract: no capacity decision is
    // committed separately from the corresponding graph reservation.
    fn submit(conn: &Connection, mut edge: Delegation) -> Admission {
        with_tx(conn, |conn| -> Result<Admission> {
            let admission = admit(conn, &edge, &AdmissionLimits::default())?;
            edge.status = match &admission {
                Admission::Running => DelegationStatus::Running,
                Admission::Queued => DelegationStatus::Queued,
                Admission::Refused { reason } => DelegationStatus::Refused {
                    reason: reason.clone(),
                },
            };
            if !matches!(admission, Admission::Refused { .. }) {
                graph::create(conn, &edge)?;
            }
            Ok(admission)
        })
        .unwrap()
    }

    #[test]
    fn ancestor_bot_is_refused_as_cycle() {
        let conn = database();
        let a = running(&conn, delegation("a", "root"));
        let b = running(&conn, nested("b", &a));
        assert_eq!(
            graph::ancestor_bots(&conn, b.child_run_id.as_ref().unwrap()).unwrap(),
            ["bot-root", "bot-a", "bot-b"]
        );
        let mut cycle = nested("c", &b);
        cycle.child_bot = "bot-root".into();
        assert!(
            matches!(submit(&conn, cycle), Admission::Refused { reason } if reason.contains("cycle"))
        );
        let mut direct = delegation("direct", "another-root");
        direct.child_bot = direct.parent_bot.clone();
        assert!(
            matches!(submit(&conn, direct), Admission::Refused { reason } if reason.contains("cycle"))
        );
    }

    #[test]
    fn depth_over_3_refused() {
        let conn = database();
        let a = running(&conn, delegation("a", "root"));
        let b = running(&conn, nested("b", &a));
        let c = running(&conn, nested("c", &b));
        assert!(matches!(submit(&conn, nested("d", &c)),
            Admission::Refused { reason } if reason.contains("depth exceeds 3")));
    }

    #[test]
    fn fifth_running_child_is_queued() {
        let conn = database();
        for (id, root) in [
            ("a", "root1"),
            ("b", "root1"),
            ("c", "root2"),
            ("d", "root2"),
        ] {
            assert_eq!(submit(&conn, delegation(id, root)), Admission::Running);
        }
        assert_eq!(
            submit(&conn, delegation("fifth", "root3")),
            Admission::Queued
        );
    }

    #[test]
    fn fourth_child_of_same_root_is_queued() {
        let conn = database();
        for id in ["a", "b", "c"] {
            assert_eq!(submit(&conn, delegation(id, "root")), Admission::Running);
        }
        assert_eq!(
            submit(&conn, delegation("fourth", "root")),
            Admission::Queued
        );
        assert_eq!(
            submit(&conn, delegation("other", "other-root")),
            Admission::Running
        );
    }

    #[test]
    fn backlog_over_16_refused_explicitly() {
        let conn = database();
        for id in ["a", "b", "c"] {
            submit(&conn, delegation(id, "root"));
        }
        for i in 0..16 {
            assert_eq!(
                submit(&conn, delegation(&format!("queue-{i}"), "root")),
                Admission::Queued
            );
        }
        assert!(matches!(submit(&conn, delegation("overflow", "root")),
            Admission::Refused { reason } if reason.contains("backlog is full (limit 16)")));
        assert_eq!(
            submit(&conn, delegation("queue-0", "root")),
            Admission::Queued
        );
    }

    #[test]
    fn queued_child_is_promoted_when_capacity_returns() {
        let conn = database();
        for id in ["a", "b", "c", "d"] {
            submit(&conn, delegation(id, "root"));
        }
        graph::set_status(
            &conn,
            "a",
            &DelegationStatus::Done {
                result: "done".into(),
            },
        )
        .unwrap();
        let queued = graph::get(&conn, "d").unwrap().unwrap();
        with_tx(&conn, |conn| -> Result<()> {
            assert_eq!(
                admit(conn, &queued, &AdmissionLimits::default())?,
                Admission::Running
            );
            graph::set_status(conn, "d", &DelegationStatus::Running)
        })
        .unwrap();
        assert_eq!(
            admit(&conn, &queued, &AdmissionLimits::default()).unwrap(),
            Admission::Running
        );
        assert!(
            matches!(admit(&conn, &delegation("a", "root"), &AdmissionLimits::default()).unwrap(),
            Admission::Refused { reason } if reason.contains("terminal"))
        );
    }

    #[test]
    fn forged_lineage_and_reused_id_are_explicit_refusals() {
        let conn = database();
        let a = running(&conn, delegation("a", "root"));
        let mut forged = nested("b", &a);
        forged.root_run_id = "another-root".into();
        assert!(matches!(submit(&conn, forged), Admission::Refused { .. }));
        assert!(matches!(
            submit(&conn, delegation("a", "another-root")),
            Admission::Refused { .. }
        ));
        let child = nested("c", &a);
        graph::cancel_descendants(&conn, "root").unwrap();
        assert!(
            matches!(submit(&conn, child), Admission::Refused { reason } if reason.contains("not running"))
        );
    }

    #[test]
    fn concurrent_reservations_cannot_over_admit_and_survive_reopen() {
        use std::sync::{Arc, Barrier};
        // Own a unique scratch directory, never a worktree or shared database.
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let mut random = [0; 16];
        getrandom::getrandom(&mut random).unwrap();
        let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let scratch = Scratch(std::env::temp_dir().join(format!("grokbot-delegation-{nonce}")));
        std::fs::create_dir(&scratch.0).unwrap();
        let path = scratch.0.join("delegation.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        graph::migrate(&conn).unwrap();
        for (id, root) in [("a", "root1"), ("b", "root1"), ("c", "root2")] {
            assert_eq!(submit(&conn, delegation(id, root)), Admission::Running);
        }
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = [("d", "root2"), ("e", "root3")]
            .into_iter()
            .map(|(id, root)| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let conn = Connection::open(path).unwrap();
                    with_tx(&conn, |conn| -> Result<()> {
                        let edge = delegation(id, root);
                        assert_eq!(
                            admit(conn, &edge, &AdmissionLimits::default())?,
                            Admission::Running
                        );
                        // Both decisions see the last free slot. SQLite must reject
                        // one stale/concurrent write; it cannot commit five children.
                        barrier.wait();
                        graph::create(conn, &edge)?;
                        Ok(())
                    })
                })
            })
            .collect();
        let successes = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(Result::is_ok)
            .count();
        assert_eq!(successes, 1);
        drop(conn);
        let reopened = Connection::open(path).unwrap();
        let count: u32 = reopened
            .query_row("SELECT COUNT(*) FROM delegation_graph", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 4);
        let loser = if graph::get(&reopened, "d").unwrap().is_none() {
            delegation("d", "root2")
        } else {
            delegation("e", "root3")
        };
        assert_eq!(submit(&reopened, loser), Admission::Queued);
    }

    #[test]
    fn zero_backlog_refuses_instead_of_dropping_request() {
        let conn = database();
        let limits = AdmissionLimits {
            active_workspace: 0,
            backlog: 0,
            ..Default::default()
        };
        assert!(matches!(
            admit(&conn, &delegation("a", "root"), &limits).unwrap(),
            Admission::Refused { .. }
        ));
    }
}
