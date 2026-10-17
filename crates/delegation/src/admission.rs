//! Admission boundary with the Q3 defaults: 4 active workspace-wide, 3 per root,
//! depth 3, backlog 16. No human approval is required to delegate. These bounds
//! are reconstruction policy, not a recovered Grok server algorithm (see
//! `reports/delegation-lifecycle-deep.md` and `server-context-residual.md` RC05).
//!
//! `admit` is a decision, not a reservation. Compose it with `graph::create`
//! or `graph::promote` in a caller-owned IMMEDIATE transaction, then start the
//! child after commit. `submit` is a standalone IMMEDIATE transaction helper;
//! do not call it inside another transaction. Queue retries keep their FIFO slot.
use crate::graph::{self, Delegation, DelegationStatus};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    Existing(#[serde(with = "existing_outcome")] DelegationStatus),
}

// Nest the terminal status to avoid colliding with Admission's own status tag,
// while preserving the existing Running/Queued/Refused wire representation.
mod existing_outcome {
    use super::DelegationStatus;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        outcome: &DelegationStatus,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Payload<'a> {
            outcome: &'a DelegationStatus,
        }
        Payload { outcome }.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<DelegationStatus, D::Error> {
        #[derive(Deserialize)]
        struct Payload {
            outcome: DelegationStatus,
        }
        Ok(Payload::deserialize(deserializer)?.outcome)
    }
}

fn refused(reason: impl Into<String>) -> Admission {
    Admission::Refused {
        reason: reason.into(),
    }
}

fn request_refusal(
    conn: &Connection,
    delegation: &Delegation,
    limits: &AdmissionLimits,
) -> Result<Option<String>> {
    // Validate lineage first: corruption is an error even for an over-depth request.
    if let Err(reason) = graph::validate_request(conn, delegation)? {
        return Ok(Some(reason));
    }
    Ok((delegation.depth > limits.max_depth)
        .then(|| format!("delegation depth exceeds {}", limits.max_depth)))
}

#[derive(Default)]
struct Capacity {
    active: u64,
    roots: HashMap<String, u64>,
}
impl Capacity {
    fn available(&self, root: &str, limits: &AdmissionLimits) -> bool {
        self.active < limits.active_workspace as u64
            && self.roots.get(root).copied().unwrap_or(0) < limits.active_per_root as u64
    }

    fn reserve(&mut self, root: &str) {
        self.active += 1;
        *self.roots.entry(root.into()).or_default() += 1;
    }
}

struct QueuePlan {
    ready: Vec<Delegation>,
    refused: Vec<(Delegation, String)>,
    capacity: Capacity,
    queued: usize,
}

// Simulate FIFO reservations without mutating the graph. A full root does not
// prevent an unrelated root from using otherwise idle workspace capacity.
fn queue_plan(conn: &Connection, limits: &AdmissionLimits) -> Result<QueuePlan> {
    let mut capacity = Capacity::default();
    let mut statement = conn.prepare(
        "SELECT root_run_id, COUNT(*) FROM delegation_graph
         WHERE json_extract(status, '$.status') = 'running' GROUP BY root_run_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
    })?;
    for row in rows {
        let (root, count) = row?;
        capacity.active += count;
        capacity.roots.insert(root, count);
    }
    let queued = graph::queued(conn)?;
    let mut plan = QueuePlan {
        ready: Vec::new(),
        refused: Vec::new(),
        capacity,
        queued: queued.len(),
    };
    for edge in queued {
        if let Some(reason) = request_refusal(conn, &edge, limits)? {
            plan.refused.push((edge, reason));
        } else if plan.capacity.available(&edge.root_run_id, limits) {
            plan.capacity.reserve(&edge.root_run_id);
            plan.ready.push(edge);
        }
    }
    Ok(plan)
}

/// Refusals are readable tool outcomes; storage/corrupt-lineage errors remain
/// errors. Active-per-root counts every running descendant of that run root.
/// Eligible queued requests reserve free slots in insertion order before any
/// newcomer; retries never jump ahead of earlier eligible queued requests.
pub fn admit(
    conn: &Connection,
    delegation: &Delegation,
    limits: &AdmissionLimits,
) -> Result<Admission> {
    let existing = graph::get(conn, &delegation.id)?;
    if let Some(existing) = &existing {
        if !graph::same_request(existing, delegation)
            || (delegation.child_run_id.is_some()
                && existing.child_run_id.is_some()
                && delegation.child_run_id != existing.child_run_id)
            || (delegation.conversation_id.is_some()
                && existing.conversation_id.is_some()
                && delegation.conversation_id != existing.conversation_id)
        {
            return Ok(refused(
                "delegation identifier already belongs to another request",
            ));
        }
        match &existing.status {
            DelegationStatus::Running => return Ok(Admission::Running),
            DelegationStatus::Queued => {}
            DelegationStatus::Refused { reason } => return Ok(refused(reason)),
            status => return Ok(Admission::Existing(status.clone())),
        }
    }
    if let Some(reason) = request_refusal(conn, delegation, limits)? {
        return Ok(refused(reason));
    }
    let plan = queue_plan(conn, limits)?;
    if existing.is_some() {
        return Ok(if plan.ready.iter().any(|edge| edge.id == delegation.id) {
            Admission::Running
        } else {
            Admission::Queued
        });
    }
    if plan.capacity.available(&delegation.root_run_id, limits) {
        return Ok(Admission::Running);
    }
    if plan.queued < limits.backlog {
        return Ok(Admission::Queued);
    }
    Ok(refused(format!(
        "delegation backlog is full (limit {})",
        limits.backlog
    )))
}

/// Select all currently eligible queued requests in FIFO order, simulating both
/// workspace and per-root capacity as each slot is assigned. Invalid/orphaned
/// requests become Refused instead of occupying backlog forever. In the same
/// caller-owned IMMEDIATE transaction, call `graph::promote` for these requests;
/// neither this list nor `admit` alone reserves capacity. Start children after commit.
pub fn promotable(conn: &Connection, limits: &AdmissionLimits) -> Result<Vec<Delegation>> {
    let plan = queue_plan(conn, limits)?;
    for (edge, reason) in plan.refused {
        graph::set_status(conn, &edge.id, &DelegationStatus::Refused { reason })?;
    }
    Ok(plan.ready)
}

/// Standalone atomic admission + reservation. BEGIN IMMEDIATE takes the write
/// lock before reading, so SQLite's busy_timeout can serialize concurrent bots
/// instead of failing a DEFERRED read-to-write upgrade. For wrapper-composed
/// effects, use admit/create/promote in its own IMMEDIATE transaction instead.
/// Running requests need generated child run/conversation IDs, but the child
/// must not be startable until this function returns successfully.
pub fn submit(
    conn: &Connection,
    delegation: &Delegation,
    limits: &AdmissionLimits,
) -> Result<Admission> {
    let transaction = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let decision = admit(&transaction, delegation, limits)?;
    match &decision {
        Admission::Running => {
            if let Some(existing) = graph::get(&transaction, &delegation.id)?
                .filter(|edge| edge.status == DelegationStatus::Queued)
            {
                let run = delegation
                    .child_run_id
                    .as_deref()
                    .or(existing.child_run_id.as_deref())
                    .ok_or_else(|| {
                        Error::InvalidInput(
                            "running delegation must be bound to its child run".into(),
                        )
                    })?;
                let conversation = delegation
                    .conversation_id
                    .as_deref()
                    .or(existing.conversation_id.as_deref())
                    .ok_or_else(|| {
                        Error::InvalidInput(
                            "running delegation must be bound to its child run".into(),
                        )
                    })?;
                graph::promote(&transaction, &delegation.id, run, conversation)?;
            } else {
                let mut edge = delegation.clone();
                edge.status = DelegationStatus::Running;
                graph::create(&transaction, &edge)?;
            }
        }
        Admission::Queued => {
            if graph::get(&transaction, &delegation.id)?.is_none() {
                let mut edge = delegation.clone();
                edge.status = DelegationStatus::Queued;
                graph::create(&transaction, &edge)?;
            }
        }
        Admission::Refused { .. } | Admission::Existing(_) => {}
    }
    transaction.commit()?;
    Ok(decision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{database, delegation, nested, running};
    use grokbot_engine::contracts::with_tx;

    fn submit(conn: &Connection, edge: Delegation) -> Admission {
        super::submit(conn, &edge, &AdmissionLimits::default()).unwrap()
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
    fn queued_runs_before_newcomer_fifo() {
        let conn = database();
        let limits = AdmissionLimits::default();
        for (id, root) in [("a", "r1"), ("b", "r1"), ("c", "r2"), ("d", "r2")] {
            submit(&conn, delegation(id, root));
        }
        // IDs deliberately sort opposite to insertion order.
        assert_eq!(submit(&conn, delegation("z-old", "r3")), Admission::Queued);
        graph::set_status(&conn, "a", &DelegationStatus::Cancelled).unwrap();
        let newcomer = delegation("a-new", "r4");
        assert_eq!(submit(&conn, newcomer.clone()), Admission::Queued);
        assert_eq!(admit(&conn, &newcomer, &limits).unwrap(), Admission::Queued);
        let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).unwrap();
        let ready = promotable(&tx, &limits).unwrap();
        assert_eq!(
            ready.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["z-old"]
        );
        graph::promote(&tx, "z-old", "run-z-old", "task:z-old:r3").unwrap();
        assert!(promotable(&tx, &limits).unwrap().is_empty());
        tx.commit().unwrap();
        graph::set_status(&conn, "b", &DelegationStatus::Cancelled).unwrap();
        assert_eq!(promotable(&conn, &limits).unwrap()[0].id, "a-new");
        assert_eq!(submit(&conn, newcomer), Admission::Running);
    }

    #[test]
    fn promotion_skips_root_at_capacity() {
        let conn = database();
        let limits = AdmissionLimits::default();
        for (id, root) in [("a", "r1"), ("b", "r1"), ("c", "r1"), ("d", "r2")] {
            submit(&conn, delegation(id, root));
        }
        submit(&conn, delegation("z-blocked", "r1"));
        submit(&conn, delegation("y-eligible", "r2"));
        submit(&conn, delegation("x-later", "r3"));
        graph::set_status(&conn, "d", &DelegationStatus::Cancelled).unwrap();
        let ready = promotable(&conn, &limits).unwrap();
        assert_eq!(
            ready.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["y-eligible"]
        );
        assert_eq!(
            submit(&conn, delegation("y-eligible", "r2")),
            Admission::Running
        );
        graph::set_status(&conn, "a", &DelegationStatus::Cancelled).unwrap();
        assert_eq!(promotable(&conn, &limits).unwrap()[0].id, "z-blocked");
    }

    #[test]
    fn promotion_reserves_each_planned_slot() {
        let conn = database();
        let limits = AdmissionLimits {
            active_workspace: 3,
            active_per_root: 1,
            ..Default::default()
        };
        for (id, root) in [
            ("z-first", "r1"),
            ("y-same-root", "r1"),
            ("x-second", "r2"),
            ("w-third", "r3"),
            ("v-spill", "r4"),
        ] {
            let mut edge = delegation(id, root);
            edge.status = DelegationStatus::Queued;
            graph::create(&conn, &edge).unwrap();
        }
        let ready = promotable(&conn, &limits).unwrap();
        assert_eq!(
            ready.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["z-first", "x-second", "w-third"]
        );
        assert_eq!(
            admit(&conn, &delegation("new", "r5"), &limits).unwrap(),
            Admission::Queued
        );
        assert_eq!(
            admit(&conn, &delegation("y-same-root", "r1"), &limits).unwrap(),
            Admission::Queued
        );
        // A plan alone does not mutate or reserve the selected rows.
        assert_eq!(graph::queued(&conn).unwrap().len(), 5);
        let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).unwrap();
        for edge in promotable(&tx, &limits).unwrap() {
            graph::promote(
                &tx,
                &edge.id,
                edge.child_run_id.as_deref().unwrap(),
                edge.conversation_id.as_deref().unwrap(),
            )
            .unwrap();
        }
        assert!(promotable(&tx, &limits).unwrap().is_empty());
        tx.commit().unwrap();
        assert_eq!(graph::queued(&conn).unwrap().len(), 2);
    }

    #[test]
    fn orphaned_queue_is_refused_and_releases_backlog() {
        let conn = database();
        let parent = running(&conn, delegation("a", "root"));
        let mut orphan = nested("orphan", &parent);
        orphan.status = DelegationStatus::Queued;
        graph::create(&conn, &orphan).unwrap();
        let mut valid = delegation("valid", "other-root");
        valid.status = DelegationStatus::Queued;
        graph::create(&conn, &valid).unwrap();
        // Legacy data: a parent completed without cancelling its queue.
        conn.execute(
            "UPDATE delegation_graph SET status = ?1 WHERE id = 'a'",
            [serde_json::to_string(&DelegationStatus::Done {
                result: "done".into(),
            })
            .unwrap()],
        )
        .unwrap();
        let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).unwrap();
        assert_eq!(promotable(&tx, &Default::default()).unwrap(), [valid]);
        assert!(matches!(graph::get(&tx, "orphan").unwrap().unwrap().status,
            DelegationStatus::Refused { reason } if reason.contains("not running")));
        tx.commit().unwrap();
        assert_eq!(graph::queued(&conn).unwrap().len(), 1);
    }

    #[test]
    fn corrupt_lineage_is_an_error_not_a_refusal() {
        let conn = database();
        let parent = running(&conn, delegation("a", "root"));
        let mut child = nested("b", &parent);
        child.status = DelegationStatus::Queued;
        graph::create(&conn, &child).unwrap();
        conn.execute(
            "UPDATE delegation_graph SET parent_run_id = child_run_id WHERE id = 'a'",
            [],
        )
        .unwrap();
        assert!(matches!(admit(&conn, &child, &Default::default()),
            Err(Error::InvalidInput(reason)) if reason.contains("cycle in delegation run lineage")));
        assert!(promotable(&conn, &Default::default()).is_err());
        assert_eq!(
            graph::get(&conn, "b").unwrap().unwrap().status,
            DelegationStatus::Queued
        );
    }

    #[test]
    fn done_replay_returns_result() {
        for status in [
            DelegationStatus::Done {
                result: "child summary 🖼".into(),
            },
            DelegationStatus::Failed {
                reason: "failure".into(),
            },
            DelegationStatus::Cancelled,
        ] {
            let conn = database();
            let edge = delegation("a", "root");
            assert_eq!(submit(&conn, edge.clone()), Admission::Running);
            graph::set_status(&conn, "a", &status).unwrap();
            let replay = submit(&conn, edge);
            assert_eq!(replay, Admission::Existing(status.clone()));
            assert_eq!(
                serde_json::to_value(&replay).unwrap(),
                serde_json::json!({"status": "existing", "outcome": status})
            );
            let json = serde_json::to_string(&replay).unwrap();
            assert_eq!(serde_json::from_str::<Admission>(&json).unwrap(), replay);
            assert_eq!(graph::get(&conn, "a").unwrap().unwrap().status, status);
        }
    }

    #[test]
    fn admission_wire_keeps_existing_decision_shapes() {
        for (decision, value) in [
            (Admission::Running, serde_json::json!({"status": "running"})),
            (Admission::Queued, serde_json::json!({"status": "queued"})),
            (
                refused("full"),
                serde_json::json!({"status": "refused", "reason": "full"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&decision).unwrap(), value);
            assert_eq!(
                serde_json::from_value::<Admission>(value).unwrap(),
                decision
            );
        }
    }

    #[test]
    fn submit_rolls_back_missing_running_link_and_promotes_bound_retry() {
        let conn = database();
        let mut edge = delegation("a", "root");
        edge.child_run_id = None;
        edge.conversation_id = None;
        assert!(super::submit(&conn, &edge, &Default::default()).is_err());
        assert!(graph::get(&conn, "a").unwrap().is_none());
        let no_slots = AdmissionLimits {
            active_workspace: 0,
            ..Default::default()
        };
        assert_eq!(
            super::submit(&conn, &edge, &no_slots).unwrap(),
            Admission::Queued
        );
        edge.child_run_id = Some("run-a".into());
        edge.conversation_id = Some("task:a:root".into());
        // Generating IDs during a queued retry cannot bind a runnable child early.
        assert_eq!(
            super::submit(&conn, &edge, &no_slots).unwrap(),
            Admission::Queued
        );
        assert_eq!(graph::get(&conn, "a").unwrap().unwrap().child_run_id, None);
        assert_eq!(submit(&conn, edge), Admission::Running);
        graph::bind_child(&conn, "a", "run-a", "task:a:root").unwrap();
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
            let promoted = graph::promote(conn, "d", "run-d", "task:d:root")?;
            assert_eq!(promoted.status, DelegationStatus::Running);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            admit(&conn, &queued, &AdmissionLimits::default()).unwrap(),
            Admission::Running
        );
        assert_eq!(
            admit(&conn, &delegation("a", "root"), &AdmissionLimits::default()).unwrap(),
            Admission::Existing(DelegationStatus::Done {
                result: "done".into()
            })
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
                    conn.busy_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    barrier.wait();
                    super::submit(&conn, &delegation(id, root), &AdmissionLimits::default())
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();
        assert_eq!(
            results.iter().filter(|r| **r == Admission::Running).count(),
            1
        );
        assert_eq!(
            results.iter().filter(|r| **r == Admission::Queued).count(),
            1
        );
        drop(conn);
        let reopened = Connection::open(path).unwrap();
        let count: u32 = reopened
            .query_row("SELECT COUNT(*) FROM delegation_graph", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 5);
        let queued = graph::queued(&reopened).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(submit(&reopened, queued[0].clone()), Admission::Queued);
        assert_eq!(graph::descendants(&reopened, "root1").unwrap().len(), 2);
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
