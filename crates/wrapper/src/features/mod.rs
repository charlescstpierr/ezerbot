//! Fixed-order capability hooks. Each implementation owns only its module.
//!
//! Routes receive the mutable app (the submission session has mutable state). Other
//! hooks use a borrowed context, also constructible by run threads with their own
//! SQLite connection. Capability writes participate in the caller's transaction;
//! they must not BEGIN/COMMIT. `run_id` is absent for the hidden preview/tool probes.

use crate::{ApiError, ApiResponse, App, Message, Provider};
use grokbot_engine::contracts::{
    ContextBlock, ContextRequest, RunEvent, ToolCall, ToolOutcome, ToolSpec,
};
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

pub struct Context<'a> {
    pub connection: &'a Connection,
    pub provider: &'a Provider,
    pub db_path: &'a Path,
    pub now_ms: u64,
    pub bot_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub run_id: Option<&'a str>,
}

// Replace the relevant default with a free function when implementing a capability.
macro_rules! empty_hooks {
    () => { empty_hooks!(migrate, routes, state, tick, tools, call_tool, context, entry, on_run_event, link_attachments); };
    ($($hook:ident),+ $(,)?) => { $(empty_hooks!(@one $hook);)+ };
    (@one migrate) => {
        pub fn migrate(_: &rusqlite::Connection) -> Result<(), crate::ApiError> { Ok(()) }
    };
    (@one routes) => {
        pub fn routes(_: &mut crate::App, _: &str, _: &str, _: &[u8]) -> Option<Result<crate::ApiResponse, crate::ApiError>> { None }
    };
    (@one state) => {
        pub fn state(_: &super::Context<'_>) -> serde_json::Value { serde_json::json!({}) }
    };
    (@one tick) => { pub fn tick(_: &super::Context<'_>) {} };
    (@one tools) => {
        pub fn tools() -> Vec<grokbot_engine::contracts::ToolSpec> { Vec::new() }
    };
    (@one call_tool) => {
        pub fn call_tool(_: &super::Context<'_>, _: &grokbot_engine::contracts::ToolCall) -> Option<grokbot_engine::contracts::ToolOutcome> { None }
    };
    (@one context) => {
        pub fn context(_: &super::Context<'_>, _: &grokbot_engine::contracts::ContextRequest) -> Vec<grokbot_engine::contracts::ContextBlock> { Vec::new() }
    };
    (@one entry) => {
        pub fn entry(_: &super::Context<'_>, _: &str, _: &str) -> Option<crate::Message> { None }
    };
    (@one on_run_event) => {
        pub fn on_run_event(_: &super::Context<'_>, _: &grokbot_engine::contracts::RunEvent) {}
    };
    (@one link_attachments) => {
        pub fn link_attachments(_: &super::Context<'_>, _: &str, _: &str, _: &[String]) -> Result<(), crate::ApiError> { Ok(()) }
    };
}

pub mod attachments;
pub mod conversations;
pub mod delegation;
pub mod engine;
pub mod forms;
pub mod memory;
pub mod routines;
pub mod skills;

type Route = fn(&mut App, &str, &str, &[u8]) -> Option<Result<ApiResponse, ApiError>>;
struct Feature {
    name: &'static str,
    migrate: fn(&Connection) -> Result<(), ApiError>,
    routes: Route,
    state: fn(&Context<'_>) -> Value,
    tick: fn(&Context<'_>),
    tools: fn() -> Vec<ToolSpec>,
    call_tool: fn(&Context<'_>, &ToolCall) -> Option<ToolOutcome>,
    context: fn(&Context<'_>, &ContextRequest) -> Vec<ContextBlock>,
    entry: fn(&Context<'_>, &str, &str) -> Option<Message>,
    on_run_event: fn(&Context<'_>, &RunEvent),
    link_attachments: fn(&Context<'_>, &str, &str, &[String]) -> Result<(), ApiError>,
}
macro_rules! registry {
    ($($module:ident),+ $(,)?) => {
        const FEATURES: &[Feature] = &[$(Feature {
            name: stringify!($module), migrate: $module::migrate, routes: $module::routes,
            state: $module::state, tick: $module::tick, tools: $module::tools,
            call_tool: $module::call_tool, context: $module::context, entry: $module::entry,
            on_run_event: $module::on_run_event, link_attachments: $module::link_attachments,
        }),+];
    };
}
registry!(
    conversations,
    engine,
    skills,
    memory,
    forms,
    routines,
    delegation,
    attachments
);

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS component_versions (component TEXT PRIMARY KEY, version INTEGER NOT NULL)")?;
    let tx = conn.unchecked_transaction()?;
    for feature in FEATURES {
        (feature.migrate)(&tx)?;
    }
    tx.execute("INSERT INTO component_versions (component, version) VALUES ('wrapper', 1) ON CONFLICT(component) DO UPDATE SET version = MAX(version, excluded.version)", [])?;
    tx.commit()?;
    Ok(())
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    FEATURES
        .iter()
        .find_map(|feature| (feature.routes)(app, method, path, body))
}

/// Namespaced: a capability cannot overwrite account, provider or conversation state.
pub fn state(ctx: &Context<'_>) -> Value {
    Value::Object(
        FEATURES
            .iter()
            .map(|feature| (feature.name.into(), (feature.state)(ctx)))
            .collect(),
    )
}

pub fn tick(ctx: &Context<'_>) {
    for feature in FEATURES {
        (feature.tick)(ctx);
    }
}
pub fn tools() -> Vec<ToolSpec> {
    FEATURES
        .iter()
        .flat_map(|feature| (feature.tools)())
        .collect()
}
pub fn call_tool(ctx: &Context<'_>, call: &ToolCall) -> Option<ToolOutcome> {
    FEATURES
        .iter()
        .find_map(|feature| (feature.call_tool)(ctx, call))
}
pub fn context(ctx: &Context<'_>, request: &ContextRequest) -> Vec<ContextBlock> {
    let mut blocks: Vec<_> = FEATURES
        .iter()
        .flat_map(|feature| (feature.context)(ctx, request))
        .collect();
    blocks.sort_by_key(|block| block.priority);
    blocks
}
pub fn entry(ctx: &Context<'_>, kind: &str, id: &str) -> Option<Message> {
    FEATURES
        .iter()
        .find_map(|feature| (feature.entry)(ctx, kind, id))
}
pub fn on_run_event(ctx: &Context<'_>, event: &RunEvent) {
    for feature in FEATURES {
        (feature.on_run_event)(ctx, event);
    }
}
pub fn link_attachments(
    ctx: &Context<'_>,
    conversation: &str,
    message: &str,
    ids: &[String],
) -> Result<(), ApiError> {
    for feature in FEATURES {
        (feature.link_attachments)(ctx, conversation, message, ids)?;
    }
    Ok(())
}
