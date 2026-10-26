use super::*;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::AtomicU64;

pub const SOFT_MS: u64 = 180_000;
pub const HARD_MS: u64 = 480_000;
pub const SCAN_MS: u64 = 15_000;

#[derive(Clone, Copy)]
pub struct Policy {
    pub soft_ms: u64,
    pub hard_ms: u64,
    pub scan_ms: u64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            soft_ms: SOFT_MS,
            hard_ms: HARD_MS,
            scan_ms: SCAN_MS,
        }
    }
}
#[derive(Clone)]
enum Phase {
    Watching,
    Silent,
    Stopping,
}
#[derive(Clone)]
pub(crate) struct Watch {
    last_ms: u64,
    seq: u64,
    version: u64,
    internal_active: bool,
    step: String,
    phase: Phase,
}
#[derive(Default)]
struct State {
    watches: HashMap<String, Watch>,
    parents: HashMap<String, String>,
    revision: u64,
    next_scan: u64,
    policy: Policy,
}
pub(crate) struct Registry {
    state: Mutex<State>,
    clock: AtomicU64,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            clock: AtomicU64::new(u64::MAX),
        }
    }
}
impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn now(&self) -> u64 {
        let fixed = self.clock.load(Ordering::SeqCst);
        if fixed == u64::MAX {
            now()
        } else {
            fixed
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn set_clock(&self, value: u64) {
        self.clock.store(value, Ordering::SeqCst);
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn set_policy(&self, policy: Policy) {
        self.lock().policy = policy;
    }
    pub(crate) fn revision(&self) -> u64 {
        self.lock().revision
    }
    pub(crate) fn silence(&self) -> BTreeMap<String, u64> {
        self.lock()
            .watches
            .iter()
            .filter(|(_, w)| !matches!(w.phase, Phase::Watching))
            .map(|(id, w)| (id.clone(), w.last_ms))
            .collect()
    }
    pub(crate) fn start(&self, run_id: &str, seq: u64) {
        let time = self.now();
        self.lock().watches.entry(run_id.into()).or_insert(Watch {
            last_ms: time,
            seq,
            version: 0,
            internal_active: false,
            step: "réponse du modèle".into(),
            phase: Phase::Watching,
        });
    }
    fn touch(state: &mut State, id: &str, time: u64, step: Option<&str>) {
        if let Some(watch) = state.watches.get_mut(id) {
            if matches!(watch.phase, Phase::Stopping) {
                return;
            }
            if matches!(watch.phase, Phase::Silent) {
                state.revision += 1;
            }
            watch.phase = Phase::Watching;
            watch.last_ms = time.max(watch.last_ms);
            watch.version += 1;
            if let Some(step) = step {
                watch.step = step.into();
            }
        }
    }
    pub(crate) fn pulse(&self, id: &str) {
        self.activity(id, None);
    }
    pub(crate) fn activity(&self, id: &str, step: Option<&str>) {
        self.activity_kind(id, step, None);
    }
    pub(crate) fn activity_kind(&self, id: &str, step: Option<&str>, internal: Option<bool>) {
        let time = self.now();
        let mut state = self.lock();
        if !state.watches.contains_key(id) {
            return;
        }
        if let Some(active) = internal {
            if let Some(watch) = state.watches.get_mut(id) {
                watch.internal_active = active;
            }
        }
        let mut current = Some(id.to_owned());
        let mut seen = HashSet::new();
        while let Some(id) = current.filter(|id| seen.insert(id.clone())) {
            Self::touch(&mut state, &id, time, step);
            current = state.parents.get(&id).cloned();
        }
    }
    pub(crate) fn committed(&self, run: &Run, seq: u64) {
        let time = self.now();
        let mut state = self.lock();
        if run.status.is_terminal()
            || matches!(
                run.status,
                RunStatus::AwaitingApproval
                    | RunStatus::AwaitingHumanInput
                    | RunStatus::Interrupted
            )
        {
            let mut id = Some(run.run_id.clone());
            let mut seen = HashSet::new();
            while let Some(current) = id.filter(|id| seen.insert(id.clone())) {
                if state
                    .watches
                    .remove(&current)
                    .is_some_and(|w| !matches!(w.phase, Phase::Watching))
                {
                    state.revision += 1;
                }
                id = if run.status.is_terminal() {
                    None
                } else {
                    state.parents.get(&current).cloned()
                };
            }
            if run.status.is_terminal() {
                let mut parent = state.parents.get(&run.run_id).cloned();
                let mut seen = HashSet::new();
                while let Some(id) = parent.filter(|id| seen.insert(id.clone())) {
                    Self::touch(&mut state, &id, time, None);
                    parent = state.parents.get(&id).cloned();
                }
            }
            return;
        }
        let Some(watch) = state.watches.get_mut(&run.run_id) else {
            return;
        };
        if seq <= watch.seq {
            return;
        }
        watch.seq = seq;
        let mut current = Some(run.run_id.clone());
        let mut seen = HashSet::new();
        while let Some(id) = current.filter(|id| seen.insert(id.clone())) {
            Self::touch(&mut state, &id, time, None);
            current = state.parents.get(&id).cloned();
        }
    }
    fn due(&self, time: u64) -> bool {
        let mut state = self.lock();
        if time < state.next_scan {
            return false;
        }
        state.next_scan = time.saturating_add(state.policy.scan_ms);
        true
    }
    fn reconcile(
        &self,
        runs: &[(Run, u64)],
        parents: HashMap<String, String>,
        time: u64,
    ) -> Vec<(Run, Watch)> {
        let mut state = self.lock();
        let live: HashSet<_> = runs.iter().map(|(r, _)| r.run_id.as_str()).collect();
        let old_silence = state
            .watches
            .iter()
            .filter(|(_, w)| !matches!(w.phase, Phase::Watching))
            .count();
        state.watches.retain(|id, _| live.contains(id.as_str()));
        if old_silence
            != state
                .watches
                .iter()
                .filter(|(_, w)| !matches!(w.phase, Phase::Watching))
                .count()
        {
            state.revision += 1;
        }
        state.parents = parents;
        for (run, seq) in runs {
            let watch = state.watches.entry(run.run_id.clone()).or_insert(Watch {
                last_ms: time,
                seq: *seq,
                version: 0,
                internal_active: false,
                step: "réponse du modèle".into(),
                phase: Phase::Watching,
            });
            if *seq > watch.seq {
                watch.seq = *seq;
                Self::touch(&mut state, &run.run_id, time, None);
            }
        }
        for _ in 0..4 {
            let signs: Vec<_> = state
                .parents
                .iter()
                .filter_map(|(child, parent)| {
                    let last = state.watches.get(child)?.last_ms;
                    (last > state.watches.get(parent)?.last_ms).then(|| (parent.clone(), last))
                })
                .collect();
            for (parent, last) in signs {
                Self::touch(&mut state, &parent, last, None);
            }
        }
        let soft = state.policy.soft_ms;
        for watch in state.watches.values_mut() {
            if matches!(watch.phase, Phase::Watching) && time.saturating_sub(watch.last_ms) >= soft
            {
                watch.phase = Phase::Silent;
            }
        }
        let new_silence = state
            .watches
            .iter()
            .filter(|(_, w)| !matches!(w.phase, Phase::Watching))
            .count();
        if new_silence != old_silence {
            state.revision += 1;
        }
        runs.iter()
            .filter_map(|(run, _)| {
                let watch = state.watches.get(&run.run_id)?;
                (run.status == RunStatus::Running
                    && time.saturating_sub(watch.last_ms) >= state.policy.hard_ms)
                    .then(|| (run.clone(), watch.clone()))
            })
            .collect()
    }
    fn claim(&self, id: &str, candidate: &Watch, seq: u64, time: u64) -> bool {
        let mut state = self.lock();
        let hard = state.policy.hard_ms;
        let Some(watch) = state.watches.get_mut(id) else {
            return false;
        };
        if watch.version != candidate.version
            || watch.seq != seq
            || time.saturating_sub(watch.last_ms) < hard
        {
            return false;
        }
        watch.phase = Phase::Stopping;
        true
    }
    fn stopping(&self, id: &str) -> Option<String> {
        self.lock()
            .watches
            .get(id)
            .filter(|w| matches!(w.phase, Phase::Stopping))
            .map(|w| w.step.clone())
    }
}

#[doc(hidden)]
pub const LATEST_SEQ_SQL: &str =
    "SELECT seq FROM engine_run_events WHERE run_id=?1 ORDER BY seq DESC LIMIT 1";
#[doc(hidden)]
pub const STALLED_ENTRY_SQL: &str =
    "SELECT step,created_at,wrapper_late_approvals.run_id IS NOT NULL FROM wrapper_run_stalls LEFT JOIN wrapper_late_approvals USING(run_id) WHERE wrapper_run_stalls.run_id=?1";

pub(crate) fn latest_seq(conn: &Connection, id: &str) -> grokbot_engine::contracts::Result<u64> {
    Ok(conn
        .query_row(LATEST_SEQ_SQL, [id], |r| r.get(0))
        .optional()?
        .unwrap_or(0))
}

pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_run_stalls (
        run_id TEXT PRIMARY KEY REFERENCES engine_runs(run_id), step TEXT NOT NULL, created_at INTEGER NOT NULL
    );")?;
    Ok(())
}

fn eligible(conn: &Connection, run: &Run) -> grokbot_engine::contracts::Result<bool> {
    if !matches!(run.status, RunStatus::Running | RunStatus::AwaitingChildren) {
        return Ok(false);
    }
    let background: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_shell_tasks WHERE run_id=?1 AND status='en cours')",
        [&run.run_id],
        |r| r.get(0),
    )?;
    if background {
        return Ok(false);
    }
    let mut statement = conn.prepare("WITH RECURSIVE children(id) AS (
        SELECT child_run_id FROM delegation_graph WHERE parent_run_id=?1 AND json_extract(status,'$.status')='running'
        UNION SELECT d.child_run_id FROM delegation_graph d JOIN children c ON d.parent_run_id=c.id WHERE json_extract(d.status,'$.status')='running')
        SELECT r.status FROM children c JOIN engine_runs r ON r.run_id=c.id")?;
    let statuses = statement
        .query_map([&run.run_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        (run.status == RunStatus::Running || statuses.iter().any(|s| s == "running"))
            && !statuses
                .iter()
                .any(|s| matches!(s.as_str(), "awaiting_approval" | "awaiting_human_input")),
    )
}

fn has_active_child(conn: &Connection, id: &str) -> grokbot_engine::contracts::Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM delegation_graph d JOIN engine_runs c ON c.run_id=d.child_run_id WHERE d.parent_run_id=?1 AND json_extract(d.status,'$.status')='running' AND c.status IN ('running','awaiting_children'))", [id], |r| r.get(0))?)
}

pub(super) fn scan(ctx: &super::super::Context<'_>) -> Result<(), ApiError> {
    let registry = ctx.provider.watchdog();
    if !registry.due(ctx.now_ms) {
        return Ok(());
    }
    let mut runs = Vec::new();
    for status in [RunStatus::Running, RunStatus::AwaitingChildren] {
        for run in store::runs_with_status(ctx.connection, status)? {
            if eligible(ctx.connection, &run)? {
                let seq = latest_seq(ctx.connection, &run.run_id)?;
                runs.push((run, seq));
            }
        }
    }
    let mut statement = ctx.connection.prepare("SELECT d.child_run_id,d.parent_run_id FROM delegation_graph d JOIN engine_runs p ON p.run_id=d.parent_run_id WHERE p.status IN ('running','awaiting_children') AND json_extract(d.status,'$.status')='running'")?;
    let parents = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<HashMap<String, String>>>()?;
    for (run, candidate) in registry.reconcile(&runs, parents, ctx.now_ms) {
        ctx.connection.busy_timeout(Duration::ZERO)?;
        let result = stop(ctx, &run, &candidate);
        ctx.connection.busy_timeout(Duration::from_secs(3))?;
        if result.as_ref().is_err_and(|e| matches!(e, Error::Sqlite(rusqlite::Error::SqliteFailure(e,_)) if matches!(e.code,rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))) {
            let current = store::required(ctx.connection, &run.run_id)?;
            if candidate.internal_active && eligible(ctx.connection, &current)? && !has_active_child(ctx.connection, &run.run_id)? && registry.claim(&run.run_id, &candidate, latest_seq(ctx.connection, &run.run_id)?, ctx.now_ms) {
                ctx.provider.cancel_run(&run.run_id);
                stop(ctx, &run, &candidate)?;
            }
        } else { result?; }
    }
    Ok(())
}

fn stop(
    ctx: &super::super::Context<'_>,
    run: &Run,
    candidate: &Watch,
) -> grokbot_engine::contracts::Result<()> {
    let mut services = Services::new(ctx)?;
    run_loop::cancel_if(
        ctx.connection,
        &mut services,
        &run.run_id,
        TurnEnd::Stopped,
        |conn, current| {
            if current.status != RunStatus::Running
                || !eligible(conn, current)?
                || has_active_child(conn, &run.run_id)?
            {
                return Ok(false);
            }
            let seq = latest_seq(conn, &run.run_id)?;
            if seq != candidate.seq {
                ctx.provider.watchdog().committed(current, seq);
                return Ok(false);
            }
            if !ctx
                .provider
                .watchdog()
                .claim(&run.run_id, candidate, seq, ctx.now_ms)
            {
                return Ok(false);
            }
            ctx.provider.cancel_run(&run.run_id);
            Ok(true)
        },
    )?;
    let _ = super::super::turns::record(ctx.connection, &run.run_id);
    Ok(())
}

pub fn record(
    ctx: &super::super::Context<'_>,
    event: &RunEvent,
    run: &Run,
) -> Result<(), ApiError> {
    if event.kind != RunEventKind::Cancelled {
        return Ok(());
    }
    let Some(step) = ctx.provider.watchdog().stopping(&run.run_id) else {
        return Ok(());
    };
    let id = format!("stalled:{}", run.run_id);
    let time = ctx.provider.watchdog().now();
    ctx.connection.execute(
        "INSERT OR IGNORE INTO wrapper_run_stalls VALUES (?1,?2,?3)",
        params![run.run_id, step, time],
    )?;
    super::super::conversations::append_entry(
        ctx.connection,
        &run.conversation_id,
        "stalled",
        &id,
        time,
    )?;
    ctx.connection.execute("INSERT OR IGNORE INTO wrapper_notifications VALUES (?1,?2,'stalled',?3,?2,'Tâche arrêtée après un silence prolongé',?4,1,0)",params![ACCOUNT,id,run.conversation_id,time])?;
    Ok(())
}

pub(super) fn entry(ctx: &super::super::Context<'_>, id: &str) -> Option<Message> {
    let run_id = id.strip_prefix("stalled:")?;
    let run = store::get(ctx.connection, run_id).ok()??;
    if ctx
        .conversation_id
        .is_some_and(|id| id != run.conversation_id)
    {
        return None;
    }
    let (step, time, late): (String, u64, bool) = ctx
        .connection
        .query_row(STALLED_ENTRY_SQL, [run_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()
        .ok()??;
    let message_id = match &run.trigger {
        Trigger::UserMessage { message_id, .. } => {
            message_id.strip_prefix(&format!("{}/", run.conversation_id))
        }
        _ => None,
    };
    let can_regenerate = message_id.is_some()
        && !late
        && !super::super::multibot::is_group(ctx.connection, &run.conversation_id);
    Some(card(
        id,
        "stalled",
        format!("Arrêté : rien ne bougeait depuis 8 min (dernière étape : {step})."),
        json!({"run_id":run_id,"conversation_id":run.conversation_id,"message_id":message_id,"can_regenerate":can_regenerate}),
        time,
    ))
}

pub(crate) fn activity(
    provider: &Provider,
    id: &str,
    activity: run_loop::RunActivity<'_>,
    internal_tool: bool,
) {
    let step = match activity {
        run_loop::RunActivity::Model => "réponse du modèle",
        run_loop::RunActivity::ToolStarted(name) | run_loop::RunActivity::ToolFinished(name) => {
            if name.len() <= 80
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            {
                name
            } else {
                "outil"
            }
        }
    };
    let internal = match activity {
        run_loop::RunActivity::ToolStarted(_) => Some(internal_tool),
        run_loop::RunActivity::ToolFinished(_) => Some(false),
        _ => None,
    };
    provider.watchdog().activity_kind(id, Some(step), internal);
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub struct TestRunLease {
    pub(crate) provider: Provider,
    pub(crate) id: String,
    pub cancel: Arc<AtomicBool>,
}
#[cfg(any(test, feature = "test-support"))]
impl Drop for TestRunLease {
    fn drop(&mut self) {
        self.provider.release_run(&self.id);
    }
}
