//! Human-input requests, principal-conversation approvals and metadata-only secrets.
//!
//! REA `reports/server-orchestration-residual.md` SR01/SR02 motivates empty
//! idempotent retries; our receipt additionally proves actual engine delivery.
//! `reports/goal-permissions-secrets.md` PS01/PS05 motivates save-before-resolve
//! standing permissions, PS07/PS08 the separate secret submission boundary.
//! Q5 routine activation is conversation confirmation, not an approval policy here.
//! The caller owns tool transactions; HTTP writes/ticks acquire IMMEDIATE locks.
//! The engine commits wake delivery with its first Resumed event, never on submit.
//! Deleted deciding conversations explicitly cancel pending cards on the next tick.
//! Approval requests are host gate helpers, not an ordinary model tool: ordinary
//! HumanInput tools suspend as AwaitingHumanInput and cannot consume Approval wakes.
//! The gate binds the request/grant to the actual protected (run, call, digest).
use super::{write_transaction, Context};
use crate::{parse, ApiError, ApiResponse, App, Message, ACCOUNT, DEFAULT_BOT};
use grokbot_engine::{
    contracts::{
        ApprovalCheck, ApprovalRoute, AuthorizedBy, RunEvent, RunEventKind, SuspendCause, ToolCall,
        ToolOutcome, ToolSpec, WakeCause,
    },
    model::{Run, RunStatus},
    store,
};
use grokbot_forms::{
    approvals::{self, ApprovalDecision, ApprovalRule, RuleScope},
    redact::redact_value,
    requests::{self, Field, HumanInputRequest, InputKind, RequestStatus},
    rules,
    vault::{canonical_secret_name, Vault, MAX_SECRET_PURPOSE_CHARACTERS},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

empty_hooks!(context, link_attachments);
mod reviewer;

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    // Needed before cutover too: the forms HTTP receipt already reads engine wakes.
    store::migrate(conn)?;
    grokbot_forms::migrate(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_forms_calls (
            request_id TEXT PRIMARY KEY REFERENCES forms_requests(id), digest TEXT NOT NULL,
            display_json TEXT NOT NULL DEFAULT 'null'
        );",
    )?;
    let has_display: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('wrapper_forms_calls') WHERE name='display_json')", [], |row| row.get(0))?;
    if !has_display {
        conn.execute_batch(
            "ALTER TABLE wrapper_forms_calls ADD COLUMN display_json TEXT NOT NULL DEFAULT 'null'",
        )?;
    }
    Ok(())
}

/// Match the provider's <directory>/<database-stem> companion-file convention.
pub fn vault(ctx: &Context<'_>) -> Result<Vault, ApiError> {
    let directory = ctx
        .db_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let stem = ctx
        .db_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("grokbot");
    Ok(Vault::open(
        &directory.join(format!("{stem}.secrets.json")),
    )?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FormArgs {
    fields: Vec<Field>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretArgs {
    name: String,
    purpose: String,
}
pub fn tools() -> Vec<ToolSpec> {
    let object = |properties: Value, required: Value| {
        json!({
            "type":"object", "additionalProperties":false, "properties":properties, "required":required
        })
    };
    let field_type = json!({"anyOf":[
        object(json!({"kind":{"type":"string","enum":["text","number","boolean"]}}), json!(["kind"])),
        object(json!({"kind":{"type":"string","enum":["choice"]},"options":{"type":"array","items":{"type":"string"}}}), json!(["kind","options"]))
    ]});
    vec![
        ("request_form", "Ask the user for typed form input; suspend this run.", object(json!({"fields":{"type":"array","items":object(json!({
            "name":{"type":"string"},"label":{"type":"string"},"field_type":field_type,"required":{"type":"boolean"}
        }),json!(["name","label","field_type","required"]))}}), json!(["fields"]))),
        ("request_secret", "Ask for a secret through the vault card, never through chat text.", object(json!({
            "name":{"type":"string"},"purpose":{"type":"string"}
        }), json!(["name","purpose"]))),
    ].into_iter().map(|(name, description, schema)| ToolSpec {
        name: name.into(), description: description.into(), schema, requires_approval: false,
    }).collect()
}

/// Route is trusted host state, never model-supplied IDs. Prefer the originating
/// principal chat or its ancestor; routines/other bots fall back to the most recent
/// principal chat. If none exists, create a visible principal approval conversation.
pub(crate) fn principal_conversation(ctx: &Context<'_>, origin: &str) -> Result<String, ApiError> {
    let mut candidate = Some(origin.to_owned());
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = candidate {
        if !seen.insert(id.clone()) {
            return Err(ApiError::new(409, "Cyclic conversation ancestry"));
        }
        let row: Option<(String, String, Option<String>, bool)> = ctx.connection.query_row(
            "SELECT bot_id, kind, parent_id, deleted FROM wrapper_conversations WHERE account_id=?1 AND id=?2",
            params![ACCOUNT, id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional()?;
        let Some((bot, kind, parent, deleted)) = row else {
            break;
        };
        if !deleted && (kind == "group" || (bot == DEFAULT_BOT.id && kind == "chat")) {
            return Ok(id);
        }
        candidate = parent;
    }
    let existing: Option<String> = ctx.connection.query_row(
        "SELECT id FROM wrapper_conversations WHERE account_id=?1 AND bot_id=?2 AND kind='chat' AND deleted=0 ORDER BY updated_at DESC, id LIMIT 1",
        params![ACCOUNT, DEFAULT_BOT.id], |r| r.get(0),
    ).optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id: String = ctx
        .connection
        .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
    ctx.connection.execute("INSERT INTO wrapper_conversations(account_id,id,title,bot_id,created_at,updated_at) VALUES (?1,?2,'Approbations',?3,?4,?4)", params![ACCOUNT,id,DEFAULT_BOT.id,ctx.now_ms])?;
    Ok(id)
}

fn clock(ctx: &Context<'_>) -> i64 {
    ctx.now_ms.min(i64::MAX as u64) as i64
}
fn card_kind(kind: &InputKind) -> &'static str {
    match kind {
        InputKind::Form { .. } => "form",
        InputKind::Approval { .. } => "approval",
        InputKind::Secret { .. } => "secret",
    }
}
fn cause(kind: &InputKind) -> WakeCause {
    match kind {
        InputKind::Form { .. } => WakeCause::Form,
        InputKind::Approval { .. } => WakeCause::Approval,
        InputKind::Secret { .. } => WakeCause::Secret,
    }
}

pub fn call_tool(ctx: &Context<'_>, call: &ToolCall) -> Option<ToolOutcome> {
    if !matches!(call.name.as_str(), "request_form" | "request_secret") {
        return None;
    }
    // Failed outcomes are still journalled by the engine. A savepoint ensures a
    // late SQL failure cannot leave half a request when that outer tx commits.
    let result = (|| -> Result<HumanInputRequest, ApiError> {
        if ctx.connection.is_autocommit() {
            return Err(ApiError::new(
                409,
                "Human input requires a caller-owned transaction",
            ));
        }
        ctx.connection
            .execute_batch("SAVEPOINT wrapper_forms_tool")?;
        let result = request(ctx, call);
        if result.is_err() {
            ctx.connection
                .execute_batch("ROLLBACK TO wrapper_forms_tool")?;
        }
        ctx.connection.execute_batch("RELEASE wrapper_forms_tool")?;
        result
    })();
    Some(match result {
        Ok(request) => ToolOutcome::Suspend {
            cause: SuspendCause::HumanInput { id: request.id },
        },
        Err(error) => ToolOutcome::Failed {
            reason: error.message,
        },
    })
}

// A group's owner is only its coordinator. Both transactional approval and
// post-admission execution must accept the same live, active participants.
fn requesting_conversation_live(ctx: &Context<'_>, run: &Run) -> Result<bool, ApiError> {
    Ok(ctx.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations c JOIN wrapper_bots b ON b.account_id=c.account_id AND b.id=?3 WHERE c.account_id=?1 AND c.id=?2 AND c.deleted=0 AND b.deleted=0 AND (c.bot_id=?3 OR (c.kind='group' AND EXISTS(SELECT 1 FROM wrapper_members m WHERE m.account_id=c.account_id AND m.conversation_id=c.id AND m.bot_id=?3 AND m.active=1))))", params![ACCOUNT,run.conversation_id,run.bot_id], |r| r.get(0))?)
}

fn active_run(ctx: &Context<'_>, call: &ToolCall) -> Result<Run, ApiError> {
    if ctx.connection.is_autocommit() {
        return Err(ApiError::new(
            409,
            "Human input requires a caller-owned transaction",
        ));
    }
    let run_id = ctx
        .run_id
        .ok_or_else(|| ApiError::new(400, "Human input requires an active run"))?;
    let run = store::required(ctx.connection, run_id)?;
    if ctx.bot_id != Some(run.bot_id.as_str())
        || ctx.conversation_id != Some(run.conversation_id.as_str())
        || run.status != RunStatus::Running
    {
        return Err(ApiError::new(
            409,
            "Human input does not match the active run",
        ));
    }
    if call.call_id.trim().is_empty() || call.name.trim().is_empty() || !call.arguments.is_object()
    {
        return Err(ApiError::new(400, "Invalid tool arguments"));
    }
    let live = requesting_conversation_live(ctx, &run)?;
    if !live {
        return Err(ApiError::new(
            409,
            "The requesting conversation is unavailable",
        ));
    }
    Ok(run)
}

fn existing_request(
    ctx: &Context<'_>,
    id: &str,
    digest: &str,
) -> Result<Option<HumanInputRequest>, ApiError> {
    let Some(existing) = requests::get(ctx.connection, id)? else {
        return Ok(None);
    };
    let binding: String = ctx.connection.query_row(
        "SELECT digest FROM wrapper_forms_calls WHERE request_id=?1",
        [id],
        |row| row.get(0),
    )?;
    if binding != digest {
        return Err(ApiError::new(
            409,
            "Tool call already has a different binding",
        ));
    }
    Ok(Some(existing))
}

fn request(ctx: &Context<'_>, call: &ToolCall) -> Result<HumanInputRequest, ApiError> {
    let run = active_run(ctx, call)?;
    let id = format!(
        "input_{}",
        approvals::args_digest("human_input", &json!([run.run_id, call.call_id]))?
    );
    let digest = approvals::args_digest(&call.name, &call.arguments)?;
    if let Some(existing) = existing_request(ctx, &id, &digest)? {
        return Ok(existing);
    }
    let kind = if call.name == "request_form" {
        let args: FormArgs = parse_args(&call.arguments)?;
        InputKind::Form {
            fields: args.fields,
        }
    } else {
        let args: SecretArgs = parse_args(&call.arguments)?;
        let name = canonical_secret_name(&args.name)?;
        if name.starts_with("MCP_") && name.ends_with("_ENV") {
            return Err(ApiError::new(
                400,
                "Ce nom est réservé aux variables privées des connecteurs MCP.",
            ));
        }
        let purpose = args.purpose.trim().to_owned();
        if purpose.is_empty() || purpose.encode_utf16().count() > MAX_SECRET_PURPOSE_CHARACTERS {
            return Err(ApiError::new(
                400,
                "Secret purpose requires 1 to 500 UTF-16 units",
            ));
        }
        InputKind::Secret { name, purpose }
    };
    create_request(ctx, &run, &id, &digest, kind, Value::Null)
}

fn request_route(ctx: &Context<'_>, run: &Run, approval: bool) -> Result<ApprovalRoute, ApiError> {
    let hidden: bool = ctx.connection.query_row("SELECT kind IN ('task','routine') FROM wrapper_conversations WHERE account_id=?1 AND id=?2", params![ACCOUNT,run.conversation_id], |row| row.get(0))?;
    let group = super::multibot::is_group(ctx.connection, &run.conversation_id);
    let principal = approval || hidden;
    Ok(ApprovalRoute {
        requesting_bot_id: run.bot_id.clone(),
        target_bot_id: if principal {
            DEFAULT_BOT.id.into()
        } else {
            run.bot_id.clone()
        },
        target_conversation_id: if group {
            run.conversation_id.clone()
        } else if principal {
            principal_conversation(ctx, &run.conversation_id)?
        } else {
            run.conversation_id.clone()
        },
    })
}

fn create_request(
    ctx: &Context<'_>,
    run: &Run,
    id: &str,
    digest: &str,
    kind: InputKind,
    display: Value,
) -> Result<HumanInputRequest, ApiError> {
    // Sanitize before persistence so rotation/removal cannot reveal old values.
    let vault = vault(ctx)?;
    let sanitized = redact_value(&vault, &json!(kind))?;
    let approval = matches!(kind, InputKind::Approval { .. });
    if approval && sanitized != json!(kind) {
        return Err(ApiError::new(400, "Approval metadata contains a secret"));
    }
    let kind: InputKind = serde_json::from_value(sanitized)
        .map_err(|_| ApiError::new(400, "Invalid human input schema"))?;
    if let InputKind::Secret { name, .. } = &kind {
        if canonical_secret_name(name)? != *name {
            return Err(ApiError::new(400, "Invalid secret request name"));
        }
    }
    let sanitized_display = redact_value(&vault, &display)?;
    if matches!(&kind, InputKind::Approval { tool, .. } if tool == "propose_skill")
        && sanitized_display != display
    {
        return Err(ApiError::new(
            400,
            "Une proposition de skill ne doit contenir aucun secret enregistré.",
        ));
    }
    let display = sanitized_display;
    let request = requests::create(
        ctx.connection,
        &HumanInputRequest {
            id: id.into(),
            entry_id: id.into(),
            run_id: run.run_id.clone(),
            kind,
            route: request_route(ctx, run, approval)?,
            status: RequestStatus::Pending,
        },
    )?;
    ctx.connection.execute(
        "INSERT INTO wrapper_forms_calls(request_id,digest,display_json) VALUES (?1,?2,?3)",
        params![id, digest, display.to_string()],
    )?;
    let target = &request.route.target_conversation_id;
    super::conversations::append_entry(
        ctx.connection,
        target,
        card_kind(&request.kind),
        id,
        ctx.now_ms,
    )?;
    ctx.connection.execute("UPDATE wrapper_conversations SET updated_at=MAX(updated_at,?3) WHERE account_id=?1 AND id=?2", params![ACCOUNT,target,ctx.now_ms])?;
    // Defense in depth if a standing rule appears between gate check and request.
    if approval
        && !matches!(&request.kind, InputKind::Approval { tool, .. } if tool == "propose_skill" || super::mcp::is_lifecycle(tool))
    {
        if let Some(receipt) =
            approvals::auto_review(ctx.connection, id, &format!("auto:{id}"), clock(ctx))?
        {
            attach(ctx.connection, &request, receipt)?;
        }
    }
    requests::get(ctx.connection, id)?
        .ok_or_else(|| ApiError::new(500, "Human input request was not persisted"))
}

fn guarded_identity(
    ctx: &Context<'_>,
    run: &Run,
    call: &ToolCall,
) -> Result<(String, String), ApiError> {
    if active_run(ctx, call)? != *run {
        return Err(ApiError::new(409, "Approval does not match the active run"));
    }
    approval_identity(&run.run_id, call)
}
fn approval_identity(run_id: &str, call: &ToolCall) -> Result<(String, String), ApiError> {
    let digest = approvals::args_digest(&call.name, &call.arguments)?;
    let bytes = serde_json::to_vec(&(run_id, &call.call_id, &digest))
        .map_err(|_| ApiError::new(400, "Invalid approval binding"))?;
    Ok((format!("approval:{:x}", Sha256::digest(bytes)), digest))
}
pub(crate) fn tool_approval_status(
    ctx: &Context<'_>,
    run_id: &str,
    call: &ToolCall,
) -> Option<Value> {
    let (id, _) = approval_identity(run_id, call).ok()?;
    requests::get(ctx.connection, &id)
        .ok()?
        .map(|r| json!(r.status)["status"].clone())
}

/// RunServices::request_approval adapter. The caller propagates errors and owns
/// the transaction. This is not a model tool: the engine suspends approval=true
/// before executing the original protected call, retaining its exact call ID.
pub fn request_tool_approval(
    ctx: &Context<'_>,
    run: &Run,
    call: &ToolCall,
) -> Result<SuspendCause, ApiError> {
    let (id, digest) = guarded_identity(ctx, run, call)?;
    if existing_request(ctx, &id, &digest)?.is_none() {
        let display = if call.name == "propose_skill" {
            super::skills::proposal_display(ctx, call)?
        } else {
            call.arguments.clone()
        };
        create_request(
            ctx,
            run,
            &id,
            &digest,
            InputKind::Approval {
                tool: call.name.clone(),
                args_digest: digest.clone(),
                expires_at: clock(ctx).saturating_add(if call.name == "propose_skill" {
                    86_400_000
                } else {
                    600_000
                }),
            },
            display,
        )?;
    }
    Ok(SuspendCause::HumanInput { id })
}

/// RunServices::check_approval adapter. Existing per-call terminal decisions take
/// precedence over standing rules. Admission/consumption, protected effect and
/// ToolResult must commit in the SAME caller-owned IMMEDIATE transaction.
pub fn check_tool_approval(
    ctx: &Context<'_>,
    run: &Run,
    call: &ToolCall,
) -> Result<ApprovalCheck, ApiError> {
    let (id, digest) = guarded_identity(ctx, run, call)?;
    if let Some(request) = existing_request(ctx, &id, &digest)? {
        return match request.status {
            RequestStatus::Pending => Ok(ApprovalCheck::Ask),
            RequestStatus::Resolved { .. } => {
                match approvals::consume_allow_once(
                    ctx.connection,
                    &id,
                    &run.run_id,
                    &call.name,
                    &call.arguments,
                    clock(ctx),
                ) {
                    Ok(()) => Ok(ApprovalCheck::Authorized {
                        by: AuthorizedBy::AllowOnce { request_id: id },
                    }),
                    Err(grokbot_engine::contracts::Error::InvalidInput(_)) => {
                        Ok(ApprovalCheck::Denied {
                            reason: "L’autorisation est indisponible ou a déjà été utilisée."
                                .into(),
                        })
                    }
                    Err(error) => Err(error.into()),
                }
            }
            RequestStatus::Denied => Ok(ApprovalCheck::Denied {
                reason: grokbot_forms::reviewer::get(ctx.connection, &id)?
                    .filter(|review| {
                        !review.pending
                            && review.answer.decision
                                == grokbot_forms::reviewer::ReviewDecision::Deny
                    })
                    .map(|review| review.answer.reason)
                    .unwrap_or_else(|| "L’action a été refusée.".into()),
            }),
            RequestStatus::Cancelled { .. } => Ok(ApprovalCheck::Denied {
                reason: "L’action a été annulée.".into(),
            }),
            RequestStatus::Expired { .. } => Ok(ApprovalCheck::Denied {
                reason: "Le délai d’approbation est écoulé.".into(),
            }),
        };
    }
    if super::mcp::is_lifecycle(&call.name) {
        return Ok(ApprovalCheck::Ask);
    }
    // Skill revisions always require the user's explicit click, even if an
    // older client or imported DB contains a standing permission for this tool.
    if call.name == "propose_skill" {
        return Ok(match super::skills::proposal_display(ctx, call) {
            Ok(_) => ApprovalCheck::Ask,
            Err(error) => ApprovalCheck::Denied {
                reason: error.message,
            },
        });
    }
    // Standing rules are bot-scoped, not conversation-scoped. A successful
    // auto-admission must not create an otherwise unused "Approbations" chat.
    let route = ApprovalRoute {
        requesting_bot_id: run.bot_id.clone(),
        target_bot_id: DEFAULT_BOT.id.into(),
        target_conversation_id: run.conversation_id.clone(),
    };
    Ok(
        match rules::evaluate(ctx.connection, &route, &call.name, &call.arguments)? {
            Some((true, rule_id)) => ApprovalCheck::Authorized {
                by: AuthorizedBy::Rule { rule_id },
            },
            Some((false, _)) => ApprovalCheck::Denied {
                reason: "Une autorisation permanente refuse cette action.".into(),
            },
            None => ApprovalCheck::Ask,
        },
    )
}

/// Defense in depth for MCP's external effects. Only a grant consumed for this
/// exact protected call, or the same bot/tool standing rule, admits execution.
pub(crate) fn verify_tool_admitted(ctx: &Context<'_>, call: &ToolCall) -> Result<(), ApiError> {
    let run = store::required(
        ctx.connection,
        ctx.run_id
            .ok_or_else(|| ApiError::new(403, "Approbation MCP requise."))?,
    )?;
    if ctx.connection.is_autocommit() {
        let live = requesting_conversation_live(ctx, &run)?;
        if live
            && run.status == RunStatus::Running
            && ctx.bot_id == Some(run.bot_id.as_str())
            && ctx.conversation_id == Some(run.conversation_id.as_str())
            && grokbot_engine::run_loop::external_authorization(ctx.connection, &run.run_id, call)?
                .is_some()
        {
            return Ok(());
        }
        return Err(ApiError::new(
            403,
            "Cette action MCP n’a pas été autorisée.",
        ));
    }
    let (id, digest) = guarded_identity(ctx, &run, call)?;
    if let Some(request) = existing_request(ctx, &id, &digest)? {
        let admitted: bool = ctx.connection.query_row("SELECT EXISTS(SELECT 1 FROM forms_approval_grants WHERE request_id=?1 AND tool=?2 AND args_digest=?3 AND consumed_at IS NOT NULL)",params![id,call.name,digest],|r|r.get(0))?;
        if admitted && matches!(request.status, RequestStatus::Resolved { .. }) {
            return Ok(());
        }
    } else if !super::mcp::is_lifecycle(&call.name) {
        let route = ApprovalRoute {
            requesting_bot_id: run.bot_id,
            target_bot_id: DEFAULT_BOT.id.into(),
            target_conversation_id: run.conversation_id,
        };
        if matches!(
            rules::evaluate(ctx.connection, &route, &call.name, &call.arguments)?,
            Some((true, _))
        ) {
            return Ok(());
        }
    }
    Err(ApiError::new(
        403,
        "Cette action MCP n’a pas été autorisée.",
    ))
}

pub(super) fn consumed_tool_approval(
    ctx: &Context<'_>,
    call: &ToolCall,
) -> Result<String, ApiError> {
    let run = active_run(ctx, call)?;
    let (id, digest) = guarded_identity(ctx, &run, call)?;
    let authorized: bool = ctx.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM forms_requests r JOIN forms_approval_grants g ON g.request_id=r.id
         WHERE r.id=?1 AND r.run_id=?2 AND r.status='resolved' AND g.tool=?3 AND g.args_digest=?4 AND g.consumed_at IS NOT NULL)",
        params![id, run.run_id, call.name, digest], |row| row.get(0))?;
    if !authorized {
        return Err(ApiError::new(
            403,
            "Enregistrez d’abord cette proposition dans sa carte d’approbation.",
        ));
    }
    Ok(id)
}

fn parse_args<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, ApiError> {
    serde_json::from_value(value.clone()).map_err(|_| ApiError::new(400, "Invalid tool arguments"))
}

fn attach(
    conn: &Connection,
    request: &HumanInputRequest,
    receipt: grokbot_engine::contracts::ResumeReceipt,
) -> Result<grokbot_engine::contracts::ResumeReceipt, ApiError> {
    let wake = store::enqueue_wake(
        conn,
        &request.run_id,
        cause(&request.kind),
        &receipt.resolution_id,
    )?;
    requests::attach_wake(conn, &receipt.resolution_id, &wake)?;
    retarget_wait(conn, request, &receipt.resolution_id)?;
    Ok(grokbot_engine::contracts::ResumeReceipt {
        wake: wake.status,
        ..receipt
    })
}

// The engine initially suspends on the request ID; wakes reference the durable
// resolution instead. Change only that exact wait, in the resolution transaction.
fn retarget_wait(
    conn: &Connection,
    request: &HumanInputRequest,
    resolution: &str,
) -> Result<(), ApiError> {
    let run = store::required(conn, &request.run_id)?;
    if matches!(
        run.status,
        RunStatus::AwaitingApproval | RunStatus::AwaitingHumanInput
    ) && store::awaiting_references(conn, &request.run_id)? == vec![request.id.clone()]
    {
        store::set_awaiting(
            conn,
            &request.run_id,
            &SuspendCause::HumanInput {
                id: resolution.into(),
            },
        )?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    entry_id: String,
    submission_id: String,
    conversation_id: String,
    #[serde(default = "empty_values")]
    values: Value,
    // No Debug/Serialize: this is the only raw-secret DTO, never an event/message.
    value: Option<String>,
}
fn empty_values() -> Value {
    json!({})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    entry_id: String,
    submission_id: String,
    conversation_id: String,
    decision: ApprovalDecision,
    tool: Option<String>,
    arguments: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancellation {
    entry_id: String,
    submission_id: String,
    conversation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretName {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeRule {
    requesting_bot_id: String,
    target_bot_id: String,
    tool: String,
    args_digest: String,
    scope: RuleScope,
}

// Prefix user IDs so retries can never alias the expiry/auto-review system IDs.
fn user_submission(id: &str) -> Result<String, ApiError> {
    if id.trim().is_empty() || id == approvals::EXPIRY_SUBMISSION_ID || id.starts_with("auto:") {
        return Err(ApiError::new(
            400,
            "Invalid or reserved submission identifier",
        ));
    }
    Ok(format!("user:{id}"))
}
fn for_entry(
    conn: &Connection,
    entry: &str,
    conversation: &str,
) -> Result<HumanInputRequest, ApiError> {
    let id: String = conn
        .query_row(
            "SELECT id FROM forms_requests WHERE entry_id=?1",
            [entry],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| ApiError::new(404, "Unknown human input entry"))?;
    let request = requests::get(conn, &id)?
        .ok_or_else(|| ApiError::new(404, "Unknown human input request"))?;
    if conversation != request.route.target_conversation_id {
        return Err(ApiError::new(
            403,
            "Human input must be resolved in its displayed conversation",
        ));
    }
    let live: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations c WHERE account_id=?1 AND id=?2 AND deleted=0 AND (bot_id=?3 OR (kind='group' AND (?3='grokbot' OR EXISTS(SELECT 1 FROM wrapper_members m WHERE m.account_id=c.account_id AND m.conversation_id=c.id AND m.bot_id=?3 AND m.active=1)))))",params![ACCOUNT,conversation,request.route.target_bot_id],|r| r.get(0))?;
    if !live {
        return Err(ApiError::new(
            409,
            "The deciding conversation is unavailable",
        ));
    }
    Ok(request)
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !matches!(
        (method, path),
        (
            "POST",
            "/api/forms/submit"
                | "/api/forms/cancel"
                | "/api/forms/skill-content"
                | "/api/approvals/resolve"
                | "/api/approvals/rules/revoke"
                | "/api/secrets/delete"
        ) | ("GET" | "DELETE", "/api/secrets")
            | ("GET" | "POST", "/api/approvals/reviewer")
    ) {
        return None;
    }
    Some(route(app, method, path, body).map(ApiResponse::Json))
}
fn route(app: &App, method: &str, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    let ctx = app.feature_context();
    if path == "/api/approvals/reviewer" {
        return reviewer::settings(&ctx, method, body);
    }
    if path == "/api/forms/skill-content" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Preview {
            entry_id: String,
            conversation_id: String,
        }
        let input: Preview = parse(body)?;
        let request = for_entry(ctx.connection, &input.entry_id, &input.conversation_id)?;
        if !matches!(&request.kind, InputKind::Approval { tool, .. } if tool == "propose_skill") {
            return Err(ApiError::new(404, "Proposition de skill introuvable."));
        }
        return Ok(
            json!({"request":redact_value(&vault(&ctx)?, &request_value(&ctx, &request, true)?)?}),
        );
    }
    if path == "/api/approvals/rules/revoke" {
        let input: RevokeRule = parse(body)?;
        if input.target_bot_id != DEFAULT_BOT.id {
            return Err(ApiError::new(
                400,
                "Permission target must be the principal bot",
            ));
        }
        let tx = write_transaction(ctx.connection)?;
        approvals::revoke_rule(
            &tx,
            &ApprovalRule {
                route: ApprovalRoute {
                    requesting_bot_id: input.requesting_bot_id,
                    target_bot_id: input.target_bot_id,
                    target_conversation_id: String::new(),
                },
                tool: input.tool,
                args_digest: input.args_digest,
                scope: input.scope,
                allow: false,
            },
        )?;
        let rules = redact_value(&vault(&ctx)?, &json!(approvals::list_rules(&tx)?))?;
        tx.commit()?;
        return Ok(json!({"rules":rules}));
    }
    if path == "/api/secrets" && method == "GET" {
        return Ok(json!({"secrets":secret_metadata(&ctx)?}));
    }
    if path == "/api/secrets/delete" || method == "DELETE" {
        let input: SecretName = parse(body)?;
        vault(&ctx)?.remove(&input.name)?;
        return Ok(json!({"secrets":secret_metadata(&ctx)?}));
    }
    let tx = write_transaction(ctx.connection)?;
    let ctx = Context {
        connection: &tx,
        ..ctx
    };
    // All mutating routes are reached only through the local HTTP host's Origin
    // and X-Grokbot-Request guards. No model tool can dispatch a decision route.
    let receipt = match path {
        "/api/forms/submit" => {
            let input: Submission = parse(body)?;
            let submission = user_submission(&input.submission_id)?;
            let request = for_entry(&tx, &input.entry_id, &input.conversation_id)?;
            if matches!(request.kind, InputKind::Approval { .. }) {
                return Err(ApiError::new(400, "Use the approval decision API"));
            }
            // Replay before reading/validating the value or touching the vault.
            let existing: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM forms_resolutions WHERE entry_id=?1)",
                [&input.entry_id],
                |r| r.get(0),
            )?;
            if existing {
                requests::receipt(&tx, &input.entry_id, &submission)?
            } else {
                let receipt = match &request.kind {
                    InputKind::Form { .. } => {
                        if input.value.is_some() {
                            return Err(ApiError::new(
                                400,
                                "Secret values require a secret request",
                            ));
                        }
                        requests::submit(
                            &tx,
                            &input.entry_id,
                            &submission,
                            &redact_value(&vault(&ctx)?, &input.values)?,
                        )?
                    }
                    InputKind::Secret { name, .. } => {
                        if input.values != json!({}) {
                            return Err(ApiError::new(
                                400,
                                "Secret submissions use the value field",
                            ));
                        }
                        requests::submit_secret_ref(
                            &tx,
                            &input.entry_id,
                            &submission,
                            &grokbot_engine::contracts::SecretRef { name: name.clone() },
                        )?
                    }
                    _ => unreachable!(),
                };
                let receipt = attach(&tx, &request, receipt)?;
                if let InputKind::Secret { name, purpose } = &request.kind {
                    // Do SQL work first, so a wake failure cannot mutate the vault.
                    // File + SQLite cannot be one transaction: a failed commit can
                    // leave a vault value, but never a falsely accepted resolution.
                    let value = input
                        .value
                        .as_deref()
                        .ok_or_else(|| ApiError::new(400, "Secret value is required"))?;
                    vault(&ctx)?.set(name, purpose, value)?;
                }
                receipt
            }
        }
        "/api/approvals/resolve" => {
            let input: Decision = parse(body)?;
            let submission = user_submission(&input.submission_id)?;
            let request = for_entry(&tx, &input.entry_id, &input.conversation_id)?;
            let InputKind::Approval {
                tool, args_digest, ..
            } = &request.kind
            else {
                return Err(ApiError::new(400, "Request is not an approval"));
            };
            if super::mcp::is_lifecycle(tool)
                && matches!(
                    input.decision,
                    ApprovalDecision::AlwaysAllow { .. } | ApprovalDecision::Never { .. }
                )
            {
                return Err(ApiError::new(400, "Le démarrage et la suppression d’un connecteur exigent une décision ponctuelle."));
            }
            if tool == "propose_skill"
                && !matches!(
                    input.decision,
                    ApprovalDecision::AllowOnce | ApprovalDecision::Deny
                )
            {
                return Err(ApiError::new(
                    400,
                    "Chaque version de skill exige Enregistrer ou Ignorer, sans règle automatique.",
                ));
            }
            // Optional action echo may not rewrite the approved action, even on a retry.
            match (&input.tool, &input.arguments) {
                (None, None) => {}
                (Some(t), Some(args))
                    if args.is_object()
                        && t == tool
                        && approvals::args_digest(t, args)? == *args_digest => {}
                _ => {
                    return Err(ApiError::new(
                        409,
                        "Approval does not match tool and argument digest",
                    ))
                }
            }
            let receipt = approvals::decide(
                &tx,
                &request.id,
                &input.conversation_id,
                &submission,
                input.decision,
                clock(&ctx),
            )?;
            attach(&tx, &request, receipt)?
        }
        "/api/forms/cancel" => {
            let input: Cancellation = parse(body)?;
            let submission = user_submission(&input.submission_id)?;
            let request = for_entry(&tx, &input.entry_id, &input.conversation_id)?;
            let existing: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM forms_resolutions WHERE entry_id=?1)",
                [&input.entry_id],
                |r| r.get(0),
            )?;
            if existing {
                requests::receipt(&tx, &input.entry_id, &submission)?
            } else {
                cancel_request(&tx, &request, &submission)?
            }
        }
        _ => unreachable!(),
    };
    tx.commit()?;
    Ok(json!({"accepted":true,"receipt":receipt}))
}

// Compatibility shim until the core owns a single-entry cancel API. Both the
// user route and orphan reconciliation must resolve + wake in the same transaction.
fn cancel_request(
    conn: &Connection,
    request: &HumanInputRequest,
    submission: &str,
) -> Result<grokbot_engine::contracts::ResumeReceipt, ApiError> {
    if request.status != RequestStatus::Pending {
        return Err(ApiError::new(409, "Request is no longer pending"));
    }
    let resolution: String = conn.query_row(
        "SELECT 'resolution_' || lower(hex(randomblob(16)))",
        [],
        |r| r.get(0),
    )?;
    conn.execute("INSERT INTO forms_resolutions(resolution_id,entry_id,submission_id,values_json) VALUES (?1,?2,?3,'\"cancelled\"')",params![resolution,request.entry_id,submission])?;
    let changed = conn.execute("UPDATE forms_requests SET status='cancelled', resolution_id=?2 WHERE id=?1 AND status='pending'",params![request.id,resolution])?;
    if changed != 1 {
        return Err(ApiError::new(409, "Request is no longer pending"));
    }
    attach(
        conn,
        request,
        requests::receipt(conn, &request.entry_id, submission)?,
    )
}

fn orphaned_requests(conn: &Connection) -> Result<Vec<String>, ApiError> {
    let mut statement = conn.prepare(
        "SELECT r.id FROM forms_requests r LEFT JOIN wrapper_conversations c
         ON c.account_id=?1 AND c.id=json_extract(r.route_json,'$.target_conversation_id')
         AND (c.bot_id=json_extract(r.route_json,'$.target_bot_id') OR
             (c.kind='group' AND (json_extract(r.route_json,'$.target_bot_id')='grokbot' OR EXISTS(
                 SELECT 1 FROM wrapper_members m WHERE m.account_id=c.account_id AND m.conversation_id=c.id
                 AND m.bot_id=json_extract(r.route_json,'$.target_bot_id') AND m.active=1))))
         WHERE r.status='pending' AND (c.id IS NULL OR c.deleted=1)",
    )?;
    let ids = statement
        .query_map([ACCOUNT], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

fn secret_metadata(ctx: &Context<'_>) -> Result<Value, ApiError> {
    let vault = vault(ctx)?;
    Ok(redact_value(&vault, &json!(vault.list()?))?)
}
// Build metadata without reopening the vault; callers redact their whole public
// projection once, rather than reading/validating the file once per request.
fn request_value(
    ctx: &Context<'_>,
    request: &HumanInputRequest,
    full_content: bool,
) -> Result<Value, ApiError> {
    let submission: Option<String> = ctx
        .connection
        .query_row(
            "SELECT submission_id FROM forms_resolutions WHERE entry_id=?1",
            [&request.entry_id],
            |row| row.get(0),
        )
        .optional()?;
    let mut value = json!(request);
    value["receipt"] = match submission {
        Some(id) => json!(requests::receipt(ctx.connection, &request.entry_id, &id)?),
        None => Value::Null,
    };
    if matches!(request.kind, InputKind::Approval { .. }) {
        let compact = !full_content
            && request.status != RequestStatus::Pending
            && matches!(&request.kind, InputKind::Approval { tool, .. } if tool == "propose_skill");
        // Do not deserialize/transport historical bodies on every state poll.
        // The authenticated preview route loads a settled card only on expansion.
        let sql = if compact {
            "SELECT json_remove(json_set(display_json,
                '$.content_available',json('true'),
                '$.proposal.file_count',coalesce(json_array_length(json_extract(display_json,'$.proposal.files')),0),
                '$.before.content.file_count',coalesce(json_array_length(json_extract(display_json,'$.before.content.files')),0),
                '$.before.content.name',substr(json_extract(display_json,'$.before.content.name'),1,128),
                '$.before.content.display_name',substr(json_extract(display_json,'$.before.content.display_name'),1,128),
                '$.before.content.description',substr(json_extract(display_json,'$.before.content.description'),1,512)),
                '$.proposal.body','$.proposal.files','$.before.content.body','$.before.content.files','$.before.content.origin')
             FROM wrapper_forms_calls WHERE request_id=?1"
        } else {
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1"
        };
        let display: Option<String> = ctx
            .connection
            .query_row(sql, [&request.id], |row| row.get(0))
            .optional()?;
        value["review"] = json!(grokbot_forms::reviewer::get(ctx.connection, &request.id)?);
        value["arguments"] = display
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|_| ApiError::new(500, "Invalid approval display metadata"))?
            .unwrap_or(Value::Null);
    }
    if let InputKind::Approval { tool, .. } = &request.kind {
        if let Some(display) = super::mcp::call_display(ctx, tool, &value["arguments"]) {
            value["connector"] = display;
        }
    }
    Ok(value)
}
pub fn state(ctx: &Context<'_>) -> Value {
    fn load(ctx: &Context<'_>) -> Result<Value, ApiError> {
        let mut statement = ctx.connection.prepare("SELECT id FROM forms_requests WHERE status='pending' OR rowid IN (SELECT rowid FROM forms_requests ORDER BY rowid DESC LIMIT 100) ORDER BY rowid")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut requests = Vec::new();
        for id in ids {
            if let Some(request) = requests::get(ctx.connection, &id)? {
                requests.push(request_value(ctx, &request, false)?);
            }
        }
        let vault = vault(ctx)?;
        Ok(redact_value(
            &vault,
            &json!({"requests":requests,"secrets":vault.list()?,"rules":approvals::list_rules(ctx.connection)?,"reviewer":reviewer::config_value(ctx.connection)?}),
        )?)
    }
    load(ctx).unwrap_or_else(|_| json!({"error":"Human input state is unavailable"}))
}

pub fn entry(ctx: &Context<'_>, kind: &str, id: &str) -> Option<Message> {
    if !matches!(kind, "form" | "approval" | "secret") {
        return None;
    }
    let request = requests::get(ctx.connection, id).ok()??;
    if kind != card_kind(&request.kind)
        || ctx.conversation_id != Some(request.route.target_conversation_id.as_str())
    {
        return None;
    }
    let payload = (|| -> Result<Value, ApiError> {
        Ok(redact_value(
            &vault(ctx)?,
            &request_value(ctx, &request, false)?,
        )?)
    })()
    .unwrap_or_else(|_| json!({"error":"Human input state is unavailable"}));
    let created_at = ctx.connection.query_row("SELECT created_at FROM wrapper_messages WHERE account_id=?1 AND conversation_id=?2 AND message_id=?3",params![ACCOUNT,request.route.target_conversation_id,request.entry_id],|r| r.get(0)).unwrap_or(0);
    Some(Message {
        id: request.entry_id,
        kind: kind.into(),
        payload,
        attachments: Vec::new(),
        role: "assistant",
        body: String::new(),
        state: "echoed",
        created_at,
        error: None,
        error_kind: None,
        notice: None,
        proposals: Vec::new(),
    })
}

pub fn tick(ctx: &Context<'_>) {
    let expire = || -> Result<(), ApiError> {
        let due: bool = ctx.connection.query_row("SELECT EXISTS(SELECT 1 FROM forms_requests WHERE status='pending' AND json_extract(kind_json,'$.kind')='approval' AND json_extract(kind_json,'$.expires_at')<=?1)", [clock(ctx)], |row| row.get(0))?;
        if !due && orphaned_requests(ctx.connection)?.is_empty() {
            return Ok(());
        }
        let tx = write_transaction(ctx.connection)?;
        for (request, receipt) in approvals::expire_pending(&tx, clock(ctx))? {
            attach(&tx, &request, receipt)?;
        }
        // A task/routine's visible deciding chat can be deleted independently of
        // its origin. Without this resolution a form/secret has no expiry and
        // strands that run forever. Re-read under the writer lock to avoid racing
        // a submission; approved/executed actions are never rewritten (REA PS03).
        for id in orphaned_requests(&tx)? {
            if let Some(request) = requests::get(&tx, &id)? {
                cancel_request(&tx, &request, "unavailable")?;
            }
        }
        tx.commit()?;
        Ok(())
    };
    // Roll back the entire expiry batch on error; the next host tick retries it.
    let _ = expire();
    let _ = reviewer::tick(ctx);
}

/// Legacy registry hook cannot propagate failures (its signature is frozen in
/// features/mod.rs, outside this manifest). Cutover must wire the checked helper
/// below into RunServices so an invalidation failure rolls back its lifecycle event.
pub fn on_run_event(ctx: &Context<'_>, event: &RunEvent) {
    let _ = on_run_event_checked(ctx, event);
}

pub fn on_run_event_checked(ctx: &Context<'_>, event: &RunEvent) -> Result<(), ApiError> {
    if ctx.connection.is_autocommit() {
        return Err(ApiError::new(
            409,
            "Run event reactions require a caller-owned transaction",
        ));
    }
    if matches!(
        event.kind,
        RunEventKind::Cancelled | RunEventKind::TurnEnded
    ) {
        requests::cancel_for_run(ctx.connection, &event.run_id)?;
    } else if event.kind == RunEventKind::Suspended {
        // Auto-review may resolve before the engine writes its suspension event.
        for request in requests::list_for_run(ctx.connection, &event.run_id)? {
            let resolution: Option<String> = ctx
                .connection
                .query_row(
                    "SELECT resolution_id FROM forms_resolutions WHERE entry_id=?1",
                    [&request.entry_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(resolution) = resolution {
                retarget_wait(ctx.connection, &request, &resolution)?;
            }
        }
    }
    Ok(())
}
