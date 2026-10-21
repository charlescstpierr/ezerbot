//! Provider quota and local, measured engine telemetry are deliberately separate:
//! token counts cannot be converted to a percentage of a ChatGPT subscription.
use super::Context;
use crate::{ApiError, ApiResponse, App, ACCOUNT};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

empty_hooks!(
    migrate,
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

#[derive(Default, Serialize)]
struct Totals {
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    measured_runs: u64,
    unknown_runs: u64,
}
impl Totals {
    fn add(&mut self, input: Option<u64>, output: u64, cached: u64) {
        if let Some(input) = input {
            self.input_tokens += input;
            self.output_tokens += output;
            self.cached_input_tokens += cached;
            self.measured_runs += 1;
        } else {
            self.unknown_runs += 1;
        }
    }
}

fn local_usage(conn: &Connection) -> Result<Value, ApiError> {
    // Aggregate each response first, then each run. Neither the bot nor source
    // roll-up duplicates a child run into its parent's usage. Deleted bots and
    // conversations retain their measured history.
    let mut query = conn.prepare(
        "SELECT r.bot_id, COALESCE(b.name,'Bot supprimé'), COALESCE(b.deleted,1),
         CASE WHEN r.trigger_kind='delegation' THEN 'delegation'
              WHEN r.trigger_kind='occurrence' THEN 'routine'
              WHEN EXISTS(SELECT 1 FROM wrapper_group_turns g WHERE g.run_id=r.run_id AND g.parent_id IS NOT NULL) THEN 'bot_to_bot'
              WHEN c.kind='group' OR EXISTS(SELECT 1 FROM wrapper_group_turns g WHERE g.run_id=r.run_id) THEN 'group'
              ELSE 'chat' END,
         u.input_tokens, COALESCE(u.output_tokens,0), COALESCE(u.cached_input_tokens,0)
         FROM engine_runs r
         LEFT JOIN wrapper_bots b ON b.account_id=?1 AND b.id=r.bot_id
         LEFT JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=r.conversation_id
         LEFT JOIN (SELECT run_id,SUM(input_tokens) input_tokens,SUM(output_tokens) output_tokens,
                    SUM(cached_input_tokens) cached_input_tokens FROM engine_response_usage GROUP BY run_id) u
                    ON u.run_id=r.run_id",
    )?;
    let mut total = Totals::default();
    let mut bots: BTreeMap<String, (String, bool, Totals)> = BTreeMap::new();
    let mut sources: BTreeMap<String, Totals> =
        ["chat", "routine", "group", "delegation", "bot_to_bot"]
            .into_iter()
            .map(|s| (s.into(), Totals::default()))
            .collect();
    let rows = query.query_map([ACCOUNT], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, bool>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<u64>>(4)?,
            r.get::<_, u64>(5)?,
            r.get::<_, u64>(6)?,
        ))
    })?;
    for row in rows {
        let (id, name, deleted, source, input, output, cached) = row?;
        total.add(input, output, cached);
        bots.entry(id)
            .or_insert_with(|| (name, deleted, Totals::default()))
            .2
            .add(input, output, cached);
        sources
            .entry(source)
            .or_default()
            .add(input, output, cached);
    }
    let by_bot: Vec<_> = bots.into_iter().map(|(id, (name, deleted, tokens))|
        json!({"id":id,"name":name,"deleted":deleted,"tokens":tokens})).collect();
    let by_source: Vec<_> = sources
        .into_iter()
        .map(|(source, tokens)| json!({"source":source,"tokens":tokens}))
        .collect();
    Ok(json!({"total":total,"by_bot":by_bot,"by_source":by_source}))
}

pub fn state(ctx: &Context<'_>) -> Value {
    json!({"codex":ctx.provider.codex_usage(false)})
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    _: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !matches!(
        (method, path),
        ("GET", "/api/usage") | ("POST", "/api/usage/refresh")
    ) {
        return None;
    }
    Some((|| {
        let local = local_usage(&app.connection)?;
        Ok(ApiResponse::Json(
            json!({"codex":app.provider.codex_usage(method == "POST"),"local":local}),
        ))
    })())
}
