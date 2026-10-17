//! Typed human input and durable, idempotent resolutions.
//!
//! SR01/SR02 in `reports/server-orchestration-residual.md` show empty retries of
//! already settled forms. Replay is looked up *before* validating new values.
//! The wrapper owns the transaction: submit, enqueue an engine wake referencing
//! `resolution_id`, then [`attach_wake`]. Polling [`receipt`] reads the engine's
//! current delivery status, never treating a queued wake as a resumed run.
use grokbot_engine::contracts::{Error, Result, ResumeReceipt, SecretRef, WakeCause, WakeStatus};
use grokbot_engine::store::Wake;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Number,
    Choice { options: Vec<String> },
    Boolean,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub label: String,
    pub field_type: FieldType,
    pub required: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputKind {
    Form {
        fields: Vec<Field>,
    },
    Approval {
        tool: String,
        args_digest: String,
        expires_at: i64,
    },
    Secret {
        name: String,
        purpose: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Resolved { resolution_id: String },
    Denied,
    Expired,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanInputRequest {
    pub id: String,
    pub run_id: String,
    pub entry_id: String,
    /// Trusted principal-bot display destination, independent of the originating run.
    pub route: grokbot_engine::contracts::ApprovalRoute,
    pub kind: InputKind,
    pub status: RequestStatus,
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS forms_requests (
            id TEXT PRIMARY KEY, run_id TEXT NOT NULL, entry_id TEXT NOT NULL UNIQUE,
            kind_json TEXT NOT NULL, route_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            resolution_id TEXT
        );
        CREATE INDEX IF NOT EXISTS forms_requests_run ON forms_requests(run_id);
        CREATE TABLE IF NOT EXISTS forms_resolutions (
            resolution_id TEXT PRIMARY KEY, entry_id TEXT NOT NULL UNIQUE,
            submission_id TEXT NOT NULL, values_json TEXT NOT NULL, wake_id TEXT UNIQUE,
            UNIQUE(entry_id, submission_id)
        );",
    )?;
    Ok(())
}

pub(crate) fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}

pub(crate) fn require_transaction(conn: &Connection) -> Result<()> {
    if conn.is_autocommit() {
        return Err(invalid(
            "human-input resolution requires a caller-owned transaction",
        ));
    }
    Ok(())
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

pub(crate) fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_kind(kind: &InputKind) -> Result<()> {
    match kind {
        InputKind::Form { fields } => {
            let mut names = HashSet::new();
            for field in fields {
                if !nonempty(&field.name) || !names.insert(&field.name) {
                    return Err(invalid("form field names must be nonempty and unique"));
                }
                if let FieldType::Choice { options } = &field.field_type {
                    let unique: HashSet<_> = options.iter().collect();
                    if options.is_empty()
                        || unique.len() != options.len()
                        || options.iter().any(|option| !nonempty(option))
                    {
                        return Err(invalid("choice options must be nonempty and unique"));
                    }
                }
            }
        }
        InputKind::Approval {
            tool,
            args_digest,
            expires_at,
        } => {
            if !nonempty(tool) || !valid_digest(args_digest) || *expires_at <= 0 {
                return Err(invalid(
                    "invalid approval binding or principal conversation route",
                ));
            }
        }
        InputKind::Secret { name, purpose } => {
            if !nonempty(name) || !nonempty(purpose) {
                return Err(invalid("secret name and purpose are required"));
            }
        }
    }
    Ok(())
}

pub fn create(conn: &Connection, request: &HumanInputRequest) -> Result<HumanInputRequest> {
    if !nonempty(&request.id)
        || !nonempty(&request.run_id)
        || !nonempty(&request.entry_id)
        || !nonempty(&request.route.requesting_bot_id)
        || !nonempty(&request.route.target_bot_id)
        || !nonempty(&request.route.target_conversation_id)
        || request.status != RequestStatus::Pending
    {
        return Err(invalid(
            "a new request requires identifiers and pending status",
        ));
    }
    validate_kind(&request.kind)?;
    if let Some(existing) = get(conn, &request.id)? {
        if existing.run_id == request.run_id
            && existing.entry_id == request.entry_id
            && existing.kind == request.kind
            && existing.route == request.route
        {
            return Ok(existing);
        }
        return Err(invalid(
            "request identifier already has a different binding",
        ));
    }
    conn.execute(
        "INSERT INTO forms_requests (id, run_id, entry_id, kind_json, route_json) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            request.id,
            request.run_id,
            request.entry_id,
            serde_json::to_string(&request.kind)?,
            serde_json::to_string(&request.route)?
        ],
    )?;
    Ok(request.clone())
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

type RequestRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
);

fn decode(
    (id, run_id, entry_id, kind, status, resolution_id, route): RequestRow,
) -> Result<HumanInputRequest> {
    let status = match status.as_str() {
        "pending" => RequestStatus::Pending,
        "resolved" => RequestStatus::Resolved {
            resolution_id: resolution_id.ok_or_else(|| invalid("missing resolution identifier"))?,
        },
        "denied" => RequestStatus::Denied,
        "expired" => RequestStatus::Expired,
        "cancelled" => RequestStatus::Cancelled,
        _ => return Err(invalid("unknown human-input status")),
    };
    Ok(HumanInputRequest {
        id,
        run_id,
        entry_id,
        kind: serde_json::from_str(&kind)?,
        route: serde_json::from_str(&route)?,
        status,
    })
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<HumanInputRequest>> {
    conn.query_row(
        "SELECT id, run_id, entry_id, kind_json, status, resolution_id, route_json FROM forms_requests WHERE id=?1",
        [id], from_row,
    ).optional()?.map(decode).transpose()
}

fn for_entry(conn: &Connection, entry_id: &str) -> Result<HumanInputRequest> {
    let id: String = conn
        .query_row(
            "SELECT id FROM forms_requests WHERE entry_id=?1",
            [entry_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| invalid("unknown human-input entry"))?;
    get(conn, &id)?.ok_or_else(|| invalid("unknown human-input request"))
}

pub fn list_for_run(conn: &Connection, run_id: &str) -> Result<Vec<HumanInputRequest>> {
    let mut statement = conn.prepare(
        "SELECT id, run_id, entry_id, kind_json, status, resolution_id, route_json
         FROM forms_requests WHERE run_id=?1 ORDER BY rowid",
    )?;
    let rows = statement.query_map([run_id], from_row)?;
    rows.map(|row| decode(row?)).collect()
}

pub(crate) fn replay(
    conn: &Connection,
    entry_id: &str,
    submission_id: &str,
) -> Result<Option<ResumeReceipt>> {
    if !nonempty(submission_id) {
        return Err(invalid("submission identifier is required"));
    }
    let existing: Option<String> = conn
        .query_row(
            "SELECT submission_id FROM forms_resolutions WHERE entry_id=?1",
            [entry_id],
            |row| row.get(0),
        )
        .optional()?;
    match existing {
        Some(existing) if existing == submission_id => {
            receipt(conn, entry_id, submission_id).map(Some)
        }
        Some(_) => Err(invalid("request already has a different submission")),
        None => Ok(None),
    }
}

fn validate_values(fields: &[Field], values: &Value) -> Result<()> {
    let values = values
        .as_object()
        .ok_or_else(|| invalid("form values must be an object"))?;
    if values
        .keys()
        .any(|key| !fields.iter().any(|field| &field.name == key))
    {
        return Err(invalid("unknown form field"));
    }
    for field in fields {
        let value = values.get(&field.name).filter(|value| !value.is_null());
        let Some(value) = value else {
            if field.required {
                return Err(invalid("required form field is missing"));
            }
            continue;
        };
        let valid = match &field.field_type {
            FieldType::Text => value
                .as_str()
                .is_some_and(|text| !field.required || nonempty(text)),
            FieldType::Number => value.is_number(),
            FieldType::Boolean => value.is_boolean(),
            FieldType::Choice { options } => value
                .as_str()
                .is_some_and(|text| options.iter().any(|option| option == text)),
        };
        if !valid {
            return Err(invalid("form value does not match its schema"));
        }
    }
    Ok(())
}

/// Resolve only; enqueue and attach the wake in the same caller-owned transaction.
/// Retries with the same submission ID return the original, current receipt.
pub fn submit(
    conn: &Connection,
    entry_id: &str,
    submission_id: &str,
    values: &Value,
) -> Result<ResumeReceipt> {
    if let Some(receipt) = replay(conn, entry_id, submission_id)? {
        return Ok(receipt);
    }
    let request = for_entry(conn, entry_id)?;
    let InputKind::Form { fields } = &request.kind else {
        return Err(invalid("use the typed approval or secret resolution API"));
    };
    validate_values(fields, values)?;
    resolve(conn, &request, submission_id, values, false)
}

/// Only a vault reference is persisted. Raw secrets are never accepted here.
pub fn submit_secret_ref(
    conn: &Connection,
    entry_id: &str,
    submission_id: &str,
    secret: &SecretRef,
) -> Result<ResumeReceipt> {
    if let Some(receipt) = replay(conn, entry_id, submission_id)? {
        return Ok(receipt);
    }
    let request = for_entry(conn, entry_id)?;
    if !matches!(&request.kind, InputKind::Secret { name, .. } if name == &secret.name) {
        return Err(invalid("secret reference does not match this request"));
    }
    resolve(
        conn,
        &request,
        submission_id,
        &serde_json::to_value(secret)?,
        false,
    )
}

pub(crate) fn resolve(
    conn: &Connection,
    request: &HumanInputRequest,
    submission_id: &str,
    values: &Value,
    denied: bool,
) -> Result<ResumeReceipt> {
    require_transaction(conn)?;
    if request.status != RequestStatus::Pending {
        return Err(invalid("human-input request is no longer pending"));
    }
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| invalid("cannot generate resolution identifier"))?;
    let resolution_id = format!(
        "resolution_{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    conn.execute(
        "INSERT INTO forms_resolutions (resolution_id, entry_id, submission_id, values_json) VALUES (?1, ?2, ?3, ?4)",
        params![resolution_id, request.entry_id, submission_id, serde_json::to_string(values)?],
    )?;
    let changed = conn.execute(
        "UPDATE forms_requests SET status=?1, resolution_id=?2 WHERE id=?3 AND status='pending'",
        params![
            if denied { "denied" } else { "resolved" },
            resolution_id,
            request.id
        ],
    )?;
    if changed != 1 {
        return Err(invalid("human-input request is no longer pending"));
    }
    Ok(ResumeReceipt {
        resolution_id,
        wake: WakeStatus::Pending,
    })
}

pub fn receipt(conn: &Connection, entry_id: &str, submission_id: &str) -> Result<ResumeReceipt> {
    let (resolution_id, wake_id): (String, Option<String>) = conn.query_row(
        "SELECT resolution_id, wake_id FROM forms_resolutions WHERE entry_id=?1 AND submission_id=?2",
        params![entry_id, submission_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?.ok_or_else(|| invalid("unknown human-input submission"))?;
    let wake = match wake_id {
        Some(id) => grokbot_engine::store::wake_status(conn, &id)?,
        None => WakeStatus::Pending,
    };
    Ok(ResumeReceipt {
        resolution_id,
        wake,
    })
}

/// Bind the engine wake (whose reference is the resolution ID) before committing.
/// Its destination is always the originating run, not the principal UI conversation.
pub fn attach_wake(conn: &Connection, resolution_id: &str, wake: &Wake) -> Result<()> {
    require_transaction(conn)?;
    let (entry_id, existing): (String, Option<String>) = conn
        .query_row(
            "SELECT entry_id, wake_id FROM forms_resolutions WHERE resolution_id=?1",
            [resolution_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| invalid("unknown human-input resolution"))?;
    let request = for_entry(conn, &entry_id)?;
    let cause = match request.kind {
        InputKind::Form { .. } => WakeCause::Form,
        InputKind::Approval { .. } => WakeCause::Approval,
        InputKind::Secret { .. } => WakeCause::Secret,
    };
    if wake.run_id != request.run_id
        || wake.reference != resolution_id
        || wake.cause != cause
        || !nonempty(&wake.wake_id)
        || existing.as_ref().is_some_and(|id| id != &wake.wake_id)
    {
        return Err(invalid("wake does not match this resolution"));
    }
    conn.execute(
        "UPDATE forms_resolutions SET wake_id=?1 WHERE resolution_id=?2",
        params![wake.wake_id, resolution_id],
    )?;
    Ok(())
}

/// Read validated form values or SecretRef metadata; never a secret value.
pub fn resolved_values(conn: &Connection, resolution_id: &str) -> Result<Value> {
    let json: String = conn
        .query_row(
            "SELECT values_json FROM forms_resolutions WHERE resolution_id=?1",
            [resolution_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| invalid("unknown human-input resolution"))?;
    Ok(serde_json::from_str(&json)?)
}

/// Cancels waiting input and invalidates approved-but-not-yet-dispatched tools.
/// Like PS03, this does not undo an effect already dispatched by the caller.
pub fn cancel_for_run(conn: &Connection, run_id: &str) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE forms_requests SET status='cancelled' WHERE run_id=?1 AND
         (status='pending' OR (status='resolved' AND json_extract(kind_json, '$.kind')='approval'))",
        [run_id],
    )?)
}

#[cfg(test)]
pub(crate) mod requests_tests {
    use super::*;
    use grokbot_engine::contracts::with_tx;
    use serde_json::json;

    pub(crate) fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        conn
    }

    pub(crate) fn form() -> HumanInputRequest {
        HumanInputRequest {
            id: "request-1".into(),
            run_id: "run-1".into(),
            entry_id: "entry-1".into(),
            route: grokbot_engine::contracts::ApprovalRoute {
                requesting_bot_id: "child-bot".into(),
                target_bot_id: "main-bot".into(),
                target_conversation_id: "main-conversation".into(),
            },
            kind: InputKind::Form {
                fields: vec![
                    Field {
                        name: "text".into(),
                        label: "Text".into(),
                        field_type: FieldType::Text,
                        required: true,
                    },
                    Field {
                        name: "number".into(),
                        label: "Number".into(),
                        field_type: FieldType::Number,
                        required: false,
                    },
                    Field {
                        name: "choice".into(),
                        label: "Choice".into(),
                        field_type: FieldType::Choice {
                            options: vec!["a".into(), "b".into()],
                        },
                        required: false,
                    },
                    Field {
                        name: "boolean".into(),
                        label: "Boolean".into(),
                        field_type: FieldType::Boolean,
                        required: false,
                    },
                ],
            },
            status: RequestStatus::Pending,
        }
    }

    #[test]
    fn submission_validates_against_schema() {
        let conn = database();
        create(&conn, &form()).unwrap();
        for values in [
            json!({}),
            json!({"text":" "}),
            json!({"text":false}),
            json!({"text":"ok","number":"1"}),
            json!({"text":"ok","choice":"c"}),
            json!({"text":"ok","boolean":1}),
            json!({"text":"ok","unknown":1}),
            json!([]),
        ] {
            assert!(with_tx(&conn, |conn| submit(
                conn,
                "entry-1",
                "submission-1",
                &values
            ))
            .is_err());
        }
        let values = json!({"text":"ok","number":1.5,"choice":"a","boolean":false});
        let result = with_tx(&conn, |conn| {
            submit(conn, "entry-1", "submission-1", &values)
        })
        .unwrap();
        assert_eq!(
            resolved_values(&conn, &result.resolution_id).unwrap(),
            values
        );
    }

    #[test]
    fn same_submission_returns_same_receipt() {
        let conn = database();
        create(&conn, &form()).unwrap();
        let first = with_tx(&conn, |conn| {
            submit(conn, "entry-1", "s", &json!({"text":"original"}))
        })
        .unwrap();
        assert_eq!(
            submit(&conn, "entry-1", "s", &json!({"text":"changed"})).unwrap(),
            first
        );
        assert_eq!(receipt(&conn, "entry-1", "s").unwrap(), first);
        assert!(submit(&conn, "entry-1", "different", &json!({"text":"other"})).is_err());
        assert_eq!(list_for_run(&conn, "run-1").unwrap().len(), 1);
        assert!(matches!(
            get(&conn, "request-1").unwrap().unwrap().status,
            RequestStatus::Resolved { .. }
        ));
    }

    #[test]
    fn empty_resubmission_does_not_overwrite_values() {
        let conn = database();
        create(&conn, &form()).unwrap();
        let original = json!({"text":"keep"});
        let first = with_tx(&conn, |conn| submit(conn, "entry-1", "s", &original)).unwrap();
        assert_eq!(submit(&conn, "entry-1", "s", &json!({})).unwrap(), first);
        assert_eq!(
            resolved_values(&conn, &first.resolution_id).unwrap(),
            original
        );
    }

    #[test]
    fn requests_preserve_binding_and_reject_bad_schema() {
        let conn = database();
        let mut request = form();
        create(&conn, &request).unwrap();
        assert_eq!(create(&conn, &request).unwrap(), request);
        request.run_id = "different".into();
        assert!(create(&conn, &request).is_err());
        request.id = "new".into();
        if let InputKind::Form { fields } = &mut request.kind {
            fields.push(fields[0].clone());
        }
        assert!(create(&conn, &request).is_err());
    }

    #[test]
    fn requests_all_kinds_preserve_principal_display_route() {
        let conn = database();
        for (index, kind) in [
            form().kind,
            InputKind::Secret {
                name: "API_TOKEN".into(),
                purpose: "connector".into(),
            },
            InputKind::Approval {
                tool: "write".into(),
                args_digest: "a".repeat(64),
                expires_at: 100,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let mut request = form();
            request.id = format!("request-{index}");
            request.entry_id = format!("entry-{index}");
            request.kind = kind;
            create(&conn, &request).unwrap();
            assert_eq!(
                get(&conn, &request.id).unwrap().unwrap().route,
                request.route
            );
            let mut changed = request.clone();
            changed.route.target_conversation_id = "other-conversation".into();
            assert!(create(&conn, &changed).is_err());
            let decoded: HumanInputRequest =
                serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
            assert_eq!(decoded, request);
        }
        assert!(list_for_run(&conn, "run-1")
            .unwrap()
            .iter()
            .all(|request| request.route.target_conversation_id == "main-conversation"));
    }

    #[test]
    fn requests_resolution_rolls_back_with_caller_effect() {
        let conn = database();
        create(&conn, &form()).unwrap();
        assert!(submit(&conn, "entry-1", "s", &json!({"text":"ok"})).is_err());
        let result: Result<()> = with_tx(&conn, |conn| {
            submit(conn, "entry-1", "s", &json!({"text":"ok"}))?;
            Err(invalid("wake enqueue failed"))
        });
        assert!(result.is_err());
        assert_eq!(
            get(&conn, "request-1").unwrap().unwrap().status,
            RequestStatus::Pending
        );
        assert!(receipt(&conn, "entry-1", "s").is_err());
    }

    #[test]
    fn requests_secret_accepts_only_matching_reference() {
        let conn = database();
        let mut request = form();
        request.kind = InputKind::Secret {
            name: "API_TOKEN".into(),
            purpose: "connector".into(),
        };
        create(&conn, &request).unwrap();
        assert!(submit(&conn, "entry-1", "s", &json!({"value":"secret-value"})).is_err());
        assert!(submit_secret_ref(
            &conn,
            "entry-1",
            "s",
            &SecretRef {
                name: "OTHER".into()
            }
        )
        .is_err());
        let secret = SecretRef {
            name: "API_TOKEN".into(),
        };
        let result = with_tx(&conn, |conn| {
            submit_secret_ref(conn, "entry-1", "s", &secret)
        })
        .unwrap();
        assert_eq!(
            resolved_values(&conn, &result.resolution_id).unwrap(),
            json!({"name":"API_TOKEN"})
        );
    }

    #[test]
    fn requests_cancel_prevents_new_submission() {
        let conn = database();
        create(&conn, &form()).unwrap();
        assert_eq!(cancel_for_run(&conn, "run-1").unwrap(), 1);
        assert_eq!(cancel_for_run(&conn, "run-1").unwrap(), 0);
        assert!(with_tx(&conn, |conn| submit(
            conn,
            "entry-1",
            "s",
            &json!({"text":"ok"})
        ))
        .is_err());
    }

    #[test]
    fn requests_wake_must_target_originating_run_and_resolution() {
        let conn = database();
        create(&conn, &form()).unwrap();
        with_tx(&conn, |conn| -> Result<()> {
            let receipt = submit(conn, "entry-1", "s", &json!({"text":"ok"}))?;
            let mut wake = Wake {
                wake_id: "wake-1".into(),
                run_id: "principal-run".into(),
                reference: receipt.resolution_id.clone(),
                cause: WakeCause::Form,
                status: WakeStatus::Pending,
            };
            assert!(attach_wake(conn, &receipt.resolution_id, &wake).is_err());
            wake.run_id = "run-1".into();
            attach_wake(conn, &receipt.resolution_id, &wake)?;
            attach_wake(conn, &receipt.resolution_id, &wake)?;
            wake.wake_id = "wake-2".into();
            assert!(attach_wake(conn, &receipt.resolution_id, &wake).is_err());
            Ok(())
        })
        .unwrap();
    }
}
