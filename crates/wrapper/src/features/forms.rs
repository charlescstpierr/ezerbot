//! Human-input requests, principal-conversation approvals and metadata-only secrets.
//!
//! REA `reports/server-orchestration-residual.md` SR01/SR02 motivates empty
//! idempotent retries; our receipt additionally proves actual engine delivery.
//! `reports/goal-permissions-secrets.md` PS01/PS05 motivates save-before-resolve
//! standing permissions, PS07/PS08 the separate secret submission boundary.
//! Q5 routine activation is conversation confirmation, not an approval policy here.
//! The caller owns tool transactions; HTTP writes/ticks acquire IMMEDIATE locks.
//! Wake delivery/approved-tool dispatch belong to gb-forms-resume/engine-cutover.
use super::{write_transaction, Context};
use crate::{parse, ApiError, ApiResponse, App, Message, ACCOUNT, DEFAULT_BOT};
use grokbot_engine::{
    contracts::{
        ApprovalRoute, RunEvent, RunEventKind, SuspendCause, ToolCall, ToolOutcome, ToolSpec,
        WakeCause,
    },
    model::RunStatus,
    store,
};
use grokbot_forms::{
    approvals::{self, ApprovalDecision},
    redact::redact_value,
    requests::{self, Field, HumanInputRequest, InputKind, RequestStatus},
    vault::{canonical_secret_name, Vault, MAX_SECRET_PURPOSE_CHARACTERS},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

empty_hooks!(context, link_attachments);

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    // Needed before cutover too: the forms HTTP receipt already reads engine wakes.
    store::migrate(conn)?;
    grokbot_forms::migrate(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_forms_calls (
        request_id TEXT PRIMARY KEY REFERENCES forms_requests(id), digest TEXT NOT NULL
    ); CREATE INDEX IF NOT EXISTS forms_requests_status ON forms_requests(status);",
    )?;
    Ok(())
}

/// Same database companion convention as chatgpt-auth.json. Values never enter SQLite.
pub fn vault(ctx: &Context<'_>) -> Result<Vault, ApiError> {
    let mut path = ctx.db_path.as_os_str().to_os_string();
    path.push(".secrets.json");
    Ok(Vault::open(std::path::Path::new(&path))?)
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalArgs {
    tool: String,
    arguments: Value,
    expires_at: Option<i64>,
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
        ("request_approval", "Ask the user in the principal bot conversation to approve this exact tool/action. Do not use for routine activation confirmation.", object(json!({
            "tool":{"type":"string"},"arguments":{"type":"object"},"expires_at":{"type":["integer","null"]}
        }), json!(["tool","arguments","expires_at"]))),
    ].into_iter().map(|(name, description, schema)| ToolSpec {
        name: name.into(), description: description.into(), schema, requires_approval: false,
    }).collect()
}

/// Route is trusted host state, never model-supplied IDs. Prefer the originating
/// principal chat or its ancestor; routines/other bots fall back to the most recent
/// principal chat. If none exists, create a visible principal approval conversation.
fn principal_conversation(ctx: &Context<'_>, origin: &str) -> Result<String, ApiError> {
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
        if !deleted && bot == DEFAULT_BOT.id && kind == "chat" {
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
    if !matches!(
        call.name.as_str(),
        "request_form" | "request_secret" | "request_approval"
    ) {
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

fn request(ctx: &Context<'_>, call: &ToolCall) -> Result<HumanInputRequest, ApiError> {
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
    if call.call_id.trim().is_empty() || !call.arguments.is_object() {
        return Err(ApiError::new(400, "Invalid tool arguments"));
    }
    let live: bool = ctx.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations c JOIN wrapper_bots b ON b.account_id=c.account_id AND b.id=c.bot_id WHERE c.account_id=?1 AND c.id=?2 AND c.bot_id=?3 AND c.deleted=0 AND b.deleted=0)", params![ACCOUNT,run.conversation_id,run.bot_id], |r| r.get(0))?;
    if !live {
        return Err(ApiError::new(
            409,
            "The requesting conversation is unavailable",
        ));
    }
    let id = format!(
        "input_{}",
        approvals::args_digest("human_input", &json!([run_id, call.call_id]))?
    );
    let digest = approvals::args_digest(&call.name, &call.arguments)?;
    if let Some(existing) = requests::get(ctx.connection, &id)? {
        let binding: String = ctx.connection.query_row(
            "SELECT digest FROM wrapper_forms_calls WHERE request_id=?1",
            [&id],
            |r| r.get(0),
        )?;
        if binding != digest {
            return Err(ApiError::new(
                409,
                "Tool call already has a different binding",
            ));
        }
        return Ok(existing);
    }
    let kind = match call.name.as_str() {
        "request_form" => {
            let args: FormArgs = parse_args(&call.arguments)?;
            InputKind::Form {
                fields: args.fields,
            }
        }
        "request_secret" => {
            let args: SecretArgs = parse_args(&call.arguments)?;
            let name = canonical_secret_name(&args.name)?;
            let purpose = args.purpose.trim().to_owned();
            if purpose.is_empty() || purpose.encode_utf16().count() > MAX_SECRET_PURPOSE_CHARACTERS
            {
                return Err(ApiError::new(
                    400,
                    "Secret purpose requires 1 to 500 UTF-16 units",
                ));
            }
            InputKind::Secret { name, purpose }
        }
        _ => {
            let args: ApprovalArgs = parse_args(&call.arguments)?;
            if !args.arguments.is_object() {
                return Err(ApiError::new(400, "Approval arguments must be an object"));
            }
            let expires_at = args
                .expires_at
                .unwrap_or_else(|| clock(ctx).saturating_add(600_000));
            if expires_at <= clock(ctx) {
                return Err(ApiError::new(400, "Approval expiry must be in the future"));
            }
            InputKind::Approval {
                args_digest: approvals::args_digest(&args.tool, &args.arguments)?,
                tool: args.tool,
                expires_at,
            }
        }
    };
    // Sanitize before persistence, not just emission: rotation/removal must not
    // reveal old values in a later transcript (vault review r1, finding 6).
    let sanitized = redact_value(&vault(ctx)?, &json!(kind))?;
    // An approval's executable identity must not be rewritten by redaction.
    if matches!(kind, InputKind::Approval { .. }) && sanitized != json!(kind) {
        return Err(ApiError::new(400, "Approval metadata contains a secret"));
    }
    let kind: InputKind = serde_json::from_value(sanitized)
        .map_err(|_| ApiError::new(400, "Invalid human input schema"))?;
    if let InputKind::Secret { name, .. } = &kind {
        // Redaction must not turn a valid name into an unsatisfiable request.
        if canonical_secret_name(name)? != *name {
            return Err(ApiError::new(400, "Invalid secret request name"));
        }
    }
    let approval = matches!(kind, InputKind::Approval { .. });
    let target = if approval {
        principal_conversation(ctx, &run.conversation_id)?
    } else {
        run.conversation_id.clone()
    };
    let request = requests::create(
        ctx.connection,
        &HumanInputRequest {
            id: id.clone(),
            entry_id: id.clone(),
            run_id: run_id.into(),
            kind,
            route: ApprovalRoute {
                requesting_bot_id: run.bot_id.clone(),
                target_bot_id: if approval {
                    DEFAULT_BOT.id.into()
                } else {
                    run.bot_id
                },
                target_conversation_id: target.clone(),
            },
            status: RequestStatus::Pending,
        },
    )?;
    ctx.connection.execute(
        "INSERT INTO wrapper_forms_calls(request_id,digest) VALUES (?1,?2)",
        params![id, digest],
    )?;
    super::conversations::append_entry(
        ctx.connection,
        &target,
        card_kind(&request.kind),
        &id,
        ctx.now_ms,
    )?;
    ctx.connection.execute("UPDATE wrapper_conversations SET updated_at=MAX(updated_at,?3) WHERE account_id=?1 AND id=?2",params![ACCOUNT,target,ctx.now_ms])?;
    if approval {
        if let Some(receipt) =
            approvals::auto_review(ctx.connection, &id, &format!("auto:{id}"), clock(ctx))?
        {
            attach(ctx.connection, &request, receipt)?;
        }
    }
    requests::get(ctx.connection, &id)?
        .ok_or_else(|| ApiError::new(500, "Human input request was not persisted"))
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
    let live: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations WHERE account_id=?1 AND id=?2 AND bot_id=?3 AND deleted=0)",params![ACCOUNT,conversation,request.route.target_bot_id],|r| r.get(0))?;
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
                | "/api/approvals/resolve"
                | "/api/secrets/delete"
        ) | ("GET" | "DELETE", "/api/secrets")
    ) {
        return None;
    }
    Some(route(app, method, path, body).map(ApiResponse::Json))
}
fn route(app: &App, method: &str, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    let ctx = app.feature_context();
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
                if request.status != RequestStatus::Pending {
                    return Err(ApiError::new(409, "Request is no longer pending"));
                }
                // Core cancel_for_run invalidates all waiting grants on stop. This
                // route instead cancels just this card and creates its wake receipt.
                let resolution: String = tx.query_row(
                    "SELECT 'resolution_' || lower(hex(randomblob(16)))",
                    [],
                    |r| r.get(0),
                )?;
                tx.execute("INSERT INTO forms_resolutions(resolution_id,entry_id,submission_id,values_json) VALUES (?1,?2,?3,'\"cancelled\"')",params![resolution,input.entry_id,submission])?;
                tx.execute("UPDATE forms_requests SET status='cancelled', resolution_id=?2 WHERE id=?1 AND status='pending'",params![request.id,resolution])?;
                attach(
                    &tx,
                    &request,
                    requests::receipt(&tx, &input.entry_id, &submission)?,
                )?
            }
        }
        _ => unreachable!(),
    };
    tx.commit()?;
    Ok(json!({"accepted":true,"receipt":receipt}))
}

fn secret_metadata(ctx: &Context<'_>) -> Result<Value, ApiError> {
    let vault = vault(ctx)?;
    Ok(redact_value(&vault, &json!(vault.list()?))?)
}
fn request_value(ctx: &Context<'_>, request: &HumanInputRequest) -> Result<Value, ApiError> {
    let receipt: Option<(String, String)> = ctx
        .connection
        .query_row(
            "SELECT entry_id,submission_id FROM forms_resolutions WHERE entry_id=?1",
            [&request.entry_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let mut value = json!(request);
    value["receipt"] = match receipt {
        Some((entry, submission)) => json!(requests::receipt(ctx.connection, &entry, &submission)?),
        None => Value::Null,
    };
    Ok(redact_value(&vault(ctx)?, &value)?)
}
pub fn state(ctx: &Context<'_>) -> Value {
    fn load(ctx: &Context<'_>) -> Result<Value, ApiError> {
        let mut statement = ctx
            .connection
            .prepare("SELECT id FROM forms_requests ORDER BY rowid")?;
        let ids = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut requests = Vec::new();
        for id in ids {
            if let Some(request) = requests::get(ctx.connection, &id)? {
                requests.push(request_value(ctx, &request)?);
            }
        }
        Ok(
            json!({"requests":requests,"secrets":secret_metadata(ctx)?,"rules":redact_value(&vault(ctx)?,&json!(approvals::list_rules(ctx.connection)?))?}),
        )
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
    let payload = request_value(ctx, &request)
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
        let tx = write_transaction(ctx.connection)?;
        for (request, receipt) in approvals::expire_pending(&tx, clock(ctx))? {
            attach(&tx, &request, receipt)?;
        }
        tx.commit()?;
        Ok(())
    };
    // Roll back the entire expiry batch on error; the next host tick retries it.
    let _ = expire();
}

pub fn on_run_event(ctx: &Context<'_>, event: &RunEvent) {
    if matches!(
        event.kind,
        RunEventKind::Cancelled | RunEventKind::TurnEnded
    ) {
        let _ = requests::cancel_for_run(ctx.connection, &event.run_id);
    } else if event.kind == RunEventKind::Suspended {
        // Auto-review may resolve before the engine writes its suspension event.
        let reconcile = || -> Result<(), ApiError> {
            for request in requests::list_for_run(ctx.connection, &event.run_id)? {
                let resolution: Option<String> = ctx
                    .connection
                    .query_row(
                        "SELECT resolution_id FROM forms_resolutions WHERE entry_id=?1",
                        [&request.entry_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(resolution) = resolution {
                    retarget_wait(ctx.connection, &request, &resolution)?;
                }
            }
            Ok(())
        };
        let _ = reconcile();
    }
}
