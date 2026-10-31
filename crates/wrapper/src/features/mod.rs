//! Fixed-order capability hooks. Each implementation owns only its module.
//!
//! Routes receive the mutable app (the submission session has mutable state). Other
//! hooks use a borrowed context, also constructible by run threads with their own
//! SQLite connection. Capability writes participate in the caller's IMMEDIATE
//! transaction (use `write_transaction`); they must not BEGIN/COMMIT. Taking the
//! write lock before reading prevents WAL's non-retryable BUSY_SNAPSHOT upgrade.
//! `run_id` is absent for the hidden preview/tool probes.
//!
//! Attachment linking validates a new message before journal preparation. On a
//! retry, an empty ID list means "keep the existing links" and is not dispatched.
//! Hooks MUST return 409 if a nonempty retry list differs from the existing set;
//! silently accumulating attachments would violate message nonce immutability.

use crate::{ApiError, ApiResponse, App, Message, Provider};
use grokbot_engine::contracts::{
    ContextBlock, ContextRequest, RunEvent, ToolCall, ToolOutcome, ToolSpec,
};
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

/// Reserved assembly slots: app, bot, skills, memory, summary, history, current.
/// Tool items belong in history; external data never outranks the app contract.
pub mod priority {
    pub const APP: u32 = 0;
    pub const IDENTITY: u32 = 10;
    pub const INSTRUCTIONS: u32 = 20;
    pub const SKILLS: u32 = 30;
    pub const MEMORY: u32 = 40;
    pub const LIBRARY: u32 = 45;
    pub const THREAD: u32 = 48;
    pub const PAGE_ANCHOR: u32 = 49;
    pub const SUMMARY: u32 = 50;
    pub const HISTORY: u32 = 90;
    pub const CURRENT: u32 = 100;
}

/// Every wrapper read-then-write transaction must acquire its writer lock first.
pub fn write_transaction(conn: &Connection) -> rusqlite::Result<rusqlite::Transaction<'_>> {
    rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
}

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
pub mod demonstrations;
pub mod engine;
pub mod floating;
pub mod forms;
pub mod generated;
#[cfg(test)]
mod idle_writes_tests;
pub mod images;
pub mod iphone;
pub mod library;
pub mod local;
pub mod mcp;
pub mod meetings;
pub mod memory;
pub mod memory_auto;
pub mod multibot;
pub mod notifications;
pub mod page_snapshot;
pub mod pages;
pub mod recall;
pub mod routine_calendar;
mod routine_cost;
pub mod routine_github;
pub mod routine_mail;
pub mod routine_webhooks;
pub mod routines;
pub mod routines_background;
pub mod screen;
mod shell_tasks;
pub mod skills;
pub mod sources;
pub mod storage;
pub mod tasks;
pub mod templates;
pub mod threads;
pub mod topic_memory;
pub mod topics;
pub mod turns;
pub mod updates;
pub mod usage;
pub mod voice;
pub mod voice_call;
pub mod voice_session;
pub mod yolo;

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
    threads,
    topics,
    topic_memory,
    engine,
    multibot,
    skills,
    memory,
    memory_auto,
    forms,
    yolo,
    routines,
    routines_background,
    routine_webhooks,
    routine_github,
    routine_mail,
    routine_calendar,
    delegation,
    attachments,
    library,
    sources,
    generated,
    pages,
    images,
    mcp,
    storage,
    usage,
    turns,
    notifications,
    iphone,
    floating,
    local,
    tasks,
    voice,
    voice_call,
    meetings,
    recall,
    screen,
    page_snapshot,
    templates,
    demonstrations,
    updates
);

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    let tx = write_transaction(conn)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS component_versions (component TEXT PRIMARY KEY, version INTEGER NOT NULL)")?;
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
    #[cfg(test)]
    if let Some(result) = test_hooks::link_attachments(ctx, conversation, message, ids) {
        result?;
    }
    for feature in FEATURES {
        (feature.link_attachments)(ctx, conversation, message, ids)?;
    }
    Ok(())
}

/// Thread-local hook injection keeps negative-path unit tests isolated from each
/// other, without turning the production capability stubs into implementations.
#[cfg(test)]
pub(crate) mod test_hooks {
    use super::*;
    type Link = fn(&Context<'_>, &str, &str, &[String]) -> Result<(), ApiError>;
    std::thread_local! {
        static LINK: std::cell::Cell<Option<Link>> = const { std::cell::Cell::new(None) };
    }
    pub fn with_link<T>(hook: Link, operation: impl FnOnce() -> T) -> T {
        struct Restore(Option<Link>);
        impl Drop for Restore {
            fn drop(&mut self) {
                LINK.set(self.0);
            }
        }
        let _restore = Restore(LINK.replace(Some(hook)));
        operation()
    }
    pub(super) fn link_attachments(
        ctx: &Context<'_>,
        conversation: &str,
        message: &str,
        ids: &[String],
    ) -> Option<Result<(), ApiError>> {
        LINK.get().map(|hook| hook(ctx, conversation, message, ids))
    }
}

#[cfg(test)]
mod schema_coverage {
    /// Ignored by the validator on purpose: the engine does not check these `anyOf`
    /// branches. Only the tool's own deserialization of `field_type` does, and it does
    /// not reject extra fields.
    const TOLERATED: &[(&str, &str)] = &[
        (
            "request_form",
            "/properties/fields/items/properties/field_type/anyOf",
        ),
        (
            "request_site_form",
            "/properties/fields/items/properties/field_type/anyOf",
        ),
    ];

    #[test]
    fn every_static_tool_schema_uses_only_keywords_the_engine_validates() {
        let mut unexpected = Vec::new();
        let mut tolerated = Vec::new();
        for spec in super::tools() {
            for keyword in grokbot_engine::schema::unsupported_keywords(&spec.schema) {
                match TOLERATED.iter().find(|t| t == &&(&*spec.name, &*keyword)) {
                    Some(known) => tolerated.push(*known),
                    None => unexpected.push(format!("{}{keyword}", spec.name)),
                }
            }
        }
        assert!(
            unexpected.is_empty(),
            "keywords the argument validator ignores: {unexpected:?}"
        );
        tolerated.sort();
        assert_eq!(tolerated, TOLERATED, "stale entry in TOLERATED");
    }
}
