//! Model/tool loop boundary, implemented by gb-engine-loop.
//! The wrapper supplies context, tool dispatch, approval storage and event hooks.
use crate::contracts::{
    pending, ModelGateway, ModelRequest, Result, RunEvent, SuspendCause, ToolCall, ToolOutcome,
    ToolSpec, TurnEnd,
};
use crate::model::Run;
use rusqlite::Connection;

pub trait RunServices {
    fn context(&mut self, run: &Run) -> Result<ModelRequest>;
    fn tools(&self) -> Vec<ToolSpec>;
    fn call_tool(&mut self, conn: &Connection, run: &Run, call: &ToolCall) -> Result<ToolOutcome>;
    fn request_approval(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &ToolCall,
    ) -> Result<ToolOutcome>;
    fn is_cancelled(&self, run_id: &str) -> bool;
    fn on_run_event(&mut self, event: &RunEvent) -> Result<()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunProgress {
    Ended(TurnEnd),
    Suspended(SuspendCause),
}

pub fn run(
    _conn: &Connection,
    _gateway: &dyn ModelGateway,
    _services: &mut dyn RunServices,
    _run_id: &str,
) -> Result<RunProgress> {
    pending("engine.run_loop")
}
