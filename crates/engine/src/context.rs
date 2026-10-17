//! Pure context assembly boundary, implemented by gb-engine-context.
//! Sources follow the plan's fixed order; external content is fenced as data.
use crate::contracts::{
    pending, ContextBlock, ContextBudget, ContextRequest, HistoryItem, Result, Summary,
};

pub fn assemble(
    _request: &ContextRequest,
    _blocks: &[ContextBlock],
    _history: &[HistoryItem],
    _summary: Option<&Summary>,
    _last_reset_entry: Option<&str>,
    _budget: &ContextBudget,
) -> Result<Vec<ContextBlock>> {
    pending("engine.context")
}
