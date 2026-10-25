//! Local, durable notification outbox. REA desktop.md §8: baseline, identity,
//! focus suppression and click destination are separate from transcript reads.
//! Stable source keys survive restart/replay. Claim BEFORE native delivery gives
//! at-most-once attempts (a crash in that gap may lose a banner, never the inbox).
use super::{write_transaction, Context};
use crate::{ApiError, ApiResponse, App, ACCOUNT};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

empty_hooks!(
    tools,
    call_tool,
    context,
    on_run_event,
    link_attachments,
    tick,
    entry
);

// No model text, tool arguments or secret names go to the OS notification centre.
// SQLite CROSS JOIN fixes r→m loop order, so both reply branches use the full
// message primary key even on a new database without ANALYZE statistics.
// A routine pass the bot noted with `rien_a_signaler` is done without a notification (#286), and
// so are the sub-tasks anywhere below it (`root_run_id` is the run of the occurrence). Only the
// "done" kinds go quiet: the same pass failing afterwards still notifies, and a request waiting on
// Charles (approval, form, secret) is not a result, so it stays.
const SOURCES: &str = "
SELECT 'reply:'||r.run_id AS id, 'reply' AS kind, r.conversation_id AS conversation_id,
 'reply-'||m.message_id AS card_id, 'Réponse terminée' AS title
 FROM engine_runs r CROSS JOIN wrapper_messages m ON m.account_id=?1
 AND m.conversation_id=r.conversation_id AND m.kind='submission'
 AND m.message_id=substr(r.trigger_ref,length(r.conversation_id)+2)
 WHERE r.trigger_kind='user_message' AND r.status='finished'
 AND substr(r.trigger_ref,1,length(r.conversation_id)+1)=r.conversation_id||'/'
 AND NOT EXISTS (SELECT 1 FROM wrapper_notifications n WHERE n.account_id=?1 AND n.id='reply:'||r.run_id)
UNION ALL
SELECT 'reply:'||r.run_id, 'reply', r.conversation_id, 'reply-'||r.run_id, 'Réponse terminée'
 FROM engine_runs r CROSS JOIN wrapper_messages m ON m.account_id=?1
 AND m.conversation_id=r.conversation_id AND m.message_id=r.run_id AND m.kind='engine_reply'
 WHERE r.trigger_kind='user_message' AND r.status='finished'
 AND NOT EXISTS (SELECT 1 FROM wrapper_notifications n WHERE n.account_id=?1 AND n.id='reply:'||r.run_id)
UNION ALL
SELECT 'input:'||q.id, json_extract(q.kind_json,'$.kind'),
 json_extract(q.route_json,'$.target_conversation_id'), q.entry_id,
 CASE json_extract(q.kind_json,'$.kind') WHEN 'approval' THEN 'Autorisation demandée'
 WHEN 'secret' THEN 'Secret demandé' ELSE 'Informations demandées' END
 FROM forms_requests q WHERE q.status='pending'
UNION ALL
SELECT 'routine:'||o.occurrence_id, 'routine', s.conversation_id, COALESCE('reply-'||o.run_id,''),
 CASE o.status WHEN 'done' THEN 'Routine terminée' ELSE 'Échec de la routine' END
 FROM scheduler_occurrences o JOIN scheduler_routines s ON s.routine_id=o.routine_id
 WHERE o.status IN ('done','failed') AND NOT (o.status='done' AND o.quiet=1)
UNION ALL
SELECT 'delegation:'||d.id, 'delegation', p.conversation_id, d.id, 'Sous-tâche terminée'
 FROM delegation_graph d JOIN engine_runs p ON p.run_id=d.parent_run_id
 WHERE json_extract(d.status,'$.status')='done'
 AND NOT EXISTS (SELECT 1 FROM scheduler_occurrences q WHERE q.run_id=d.root_run_id AND q.status='done' AND q.quiet=1)
";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    enabled: bool,
    dnd: bool,
    reply: bool,
    approval: bool,
    form: bool,
    secret: bool,
    routine: bool,
    delegation: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            dnd: false,
            reply: true,
            approval: true,
            form: true,
            secret: true,
            routine: true,
            delegation: true,
        }
    }
}
impl Settings {
    pub(super) fn permits(&self, kind: &str) -> bool {
        self.enabled
            && !self.dnd
            && match kind {
                "reply" => self.reply,
                "approval" => self.approval,
                "form" => self.form,
                "secret" => self.secret,
                "routine" => self.routine,
                "delegation" => self.delegation,
                _ => false,
            }
    }
}
/// Pure policy. OS permission/delivery stays in Cocoa; transcript reading is separate.
fn should_deliver(
    settings: &Settings,
    kind: &str,
    visible: bool,
    last: Option<u64>,
    now: u64,
) -> bool {
    !visible && settings.permits(kind) && last.is_none_or(|last| now.saturating_sub(last) >= 5_000)
}

pub(super) fn settings(conn: &Connection) -> Result<Settings, ApiError> {
    let raw: String = conn.query_row(
        "SELECT settings FROM wrapper_notification_settings WHERE account_id=?1",
        [ACCOUNT],
        |r| r.get(0),
    )?;
    serde_json::from_str(&raw)
        .map_err(|_| ApiError::new(500, "Réglages de notifications illisibles."))
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_notification_settings (
        account_id TEXT PRIMARY KEY, settings TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS wrapper_notifications (
        account_id TEXT NOT NULL, id TEXT NOT NULL, kind TEXT NOT NULL,
        conversation_id TEXT NOT NULL, card_id TEXT NOT NULL, title TEXT NOT NULL,
        created_at INTEGER NOT NULL, unread INTEGER NOT NULL, attempted INTEGER NOT NULL,
        PRIMARY KEY(account_id,id));
        CREATE INDEX IF NOT EXISTS wrapper_notifications_unread ON wrapper_notifications(account_id,unread,created_at);
        CREATE TABLE IF NOT EXISTS wrapper_notification_throttle (
            account_id TEXT NOT NULL, conversation_id TEXT NOT NULL, kind TEXT NOT NULL,
            title TEXT NOT NULL, attempted_at INTEGER NOT NULL,
            PRIMARY KEY(account_id,conversation_id,kind,title));")?;
    // Repair the first implementation's notification-authored transcript rows.
    // Only this dedicated kind is removed; genuine user/bot messages are untouched.
    let migrated: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM component_versions WHERE component='wrapper.notifications' AND version>=2)", [], |r|r.get(0))?;
    if !migrated {
        conn.execute(
            "DELETE FROM wrapper_messages WHERE account_id=?1 AND kind='notification_event'",
            [ACCOUNT],
        )?;
        conn.execute("UPDATE wrapper_notifications SET card_id=COALESCE((SELECT 'reply-'||o.run_id FROM scheduler_occurrences o WHERE o.occurrence_id=substr(wrapper_notifications.id,9)),'') WHERE account_id=?1 AND kind='routine'", [ACCOUNT])?;
        conn.execute("INSERT INTO component_versions(component,version) VALUES ('wrapper.notifications',2) ON CONFLICT(component) DO UPDATE SET version=2", [])?;
    }
    let first = conn.execute(
        "INSERT OR IGNORE INTO wrapper_notification_settings VALUES (?1,?2)",
        params![ACCOUNT, json!(Settings::default()).to_string()],
    )? > 0;
    if first {
        project(conn, crate::now(), true)?;
    }
    Ok(())
}

pub(super) fn project(conn: &Connection, now: u64, baseline: bool) -> Result<(), ApiError> {
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO wrapper_notifications
        SELECT ?1,src.id,src.kind,src.conversation_id,src.card_id,src.title,?2,?3,?4
        FROM ({SOURCES}) src JOIN wrapper_conversations c
        ON c.account_id=?1 AND c.id=src.conversation_id AND c.deleted=0
        WHERE NOT EXISTS (SELECT 1 FROM wrapper_notifications n WHERE n.account_id=?1 AND n.id=src.id)"
        ),
        params![ACCOUNT, now, !baseline, baseline],
    )?;
    // Deleting a conversation removes it from the inbox without resetting source deduplication.
    conn.execute("UPDATE wrapper_notifications SET unread=0,attempted=1 WHERE account_id=?1
        AND conversation_id IN (SELECT id FROM wrapper_conversations WHERE account_id=?1 AND deleted=1)", [ACCOUNT])?;
    remind(conn, now, baseline)?;
    // A fast auto-review or a resolved request is no longer waiting: never post a stale question.
    // The reminder of an approval also ends with its own expiry.
    conn.execute("UPDATE wrapper_notifications SET attempted=1 WHERE account_id=?1 AND attempted=0
        AND kind IN ('approval','form','secret') AND NOT EXISTS
        (SELECT 1 FROM forms_requests q WHERE q.status='pending'
         AND q.id=substr(wrapper_notifications.id,CASE WHEN wrapper_notifications.id LIKE 'remind:%' THEN 8 ELSE 7 END)
         AND (wrapper_notifications.id NOT LIKE 'remind:%' OR json_extract(q.kind_json,'$.expires_at')>?2))", params![ACCOUNT, now])?;
    Ok(())
}

/// One reminder per approval still waiting after the delay chosen in Settings (#217).
/// The source key `remind:<request>` is claimed once: no second reminder, whatever the
/// polls, the restarts or a changed delay. Kind `approval` means the same switches and
/// Ne pas déranger as the first notice. The title carries no tool, argument or secret.
fn remind(conn: &Connection, now: u64, baseline: bool) -> Result<(), ApiError> {
    let Some((delay, phrase)) = super::forms::reminder_after(conn)? else {
        return Ok(());
    };
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_notifications
        SELECT ?1,'remind:'||q.id,'approval',m.conversation_id,q.entry_id,?5,?2,?3,?4
        FROM forms_requests q JOIN wrapper_messages m ON m.account_id=?1
        AND m.conversation_id=json_extract(q.route_json,'$.target_conversation_id') AND m.message_id=q.entry_id
        JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=m.conversation_id AND c.deleted=0
        WHERE q.status='pending' AND json_extract(q.kind_json,'$.kind')='approval'
        AND json_extract(q.kind_json,'$.expires_at')>?2 AND m.created_at+?6<=?2",
        params![
            ACCOUNT,
            now,
            !baseline,
            baseline,
            format!("Une demande attend votre décision depuis {phrase}"),
            delay
        ],
    )?;
    Ok(())
}

fn snapshot(conn: &Connection) -> Result<Value, ApiError> {
    let mut stmt = conn.prepare("SELECT n.id,n.kind,n.conversation_id,n.card_id,n.title,n.created_at,n.unread,c.title
        FROM wrapper_notifications n JOIN wrapper_conversations c ON c.account_id=n.account_id AND c.id=n.conversation_id
        WHERE n.account_id=?1 AND c.deleted=0 AND n.unread=1 ORDER BY n.created_at DESC,n.id DESC LIMIT 100")?;
    let items = stmt.query_map([ACCOUNT], |r| Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,
        "conversation_id":r.get::<_,String>(2)?,"card_id":r.get::<_,String>(3)?,"title":r.get::<_,String>(4)?,
        "created_at":r.get::<_,u64>(5)?,"unread":r.get::<_,bool>(6)?,"conversation_title":r.get::<_,String>(7)?})))?
        .collect::<Result<Vec<_>,_>>()?;
    let unread: u64 = conn.query_row(
        "SELECT COUNT(*) FROM wrapper_notifications WHERE account_id=?1 AND unread=1",
        [ACCOUNT],
        |r| r.get(0),
    )?;
    Ok(json!({"settings":settings(conn)?,"items":items,"unread":unread}))
}
pub fn state(_: &Context<'_>) -> Value {
    json!({})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Poll {
    focused: bool,
    conversation_id: Option<String>,
    visible: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    ids: Vec<String>,
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !path.starts_with("/api/notifications") {
        return None;
    }
    Some((|| {
        let tx = write_transaction(&app.connection)?;
        project(&tx, crate::now(), false)?;
        let response = match (method, path) {
            ("GET", "/api/notifications") => snapshot(&tx)?,
            ("POST", "/api/notifications/settings") => {
                let input: Settings = crate::parse(body)?;
                let previous = settings(&tx)?;
                tx.execute(
                    "UPDATE wrapper_notification_settings SET settings=?2 WHERE account_id=?1",
                    params![ACCOUNT, json!(input).to_string()],
                )?;
                // Turning a kind on isn't permission to replay its muted backlog.
                // Unrelated, already enabled kinds keep their pending deliveries.
                for kind in [
                    "reply",
                    "approval",
                    "form",
                    "secret",
                    "routine",
                    "delegation",
                ] {
                    if !previous.permits(kind) {
                        tx.execute("UPDATE wrapper_notifications SET attempted=1 WHERE account_id=?1 AND kind=?2",params![ACCOUNT,kind])?;
                    }
                }
                snapshot(&tx)?
            }
            ("POST", "/api/notifications/open") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Open {
                    id: String,
                }
                let input: Open = crate::parse(body)?;
                let target: Option<(String,String,String)> = tx.query_row("SELECT n.conversation_id,n.card_id,n.id FROM wrapper_notifications n JOIN wrapper_conversations c ON c.account_id=n.account_id AND c.id=n.conversation_id WHERE n.account_id=?1 AND n.id=?2 AND c.deleted=0", params![ACCOUNT,input.id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                let (conversation, card, id) = target.ok_or_else(|| {
                    ApiError::new(404, "Cette conversation n’est plus disponible.")
                })?;
                tx.execute("UPDATE wrapper_notifications SET unread=0,attempted=1 WHERE account_id=?1 AND id=?2",params![ACCOUNT,id])?;
                json!({"id":id,"conversation_id":conversation,"card_id":card})
            }
            ("POST", "/api/notifications/read") => {
                let input: Read = crate::parse(body)?;
                if input.ids.len() > 100 {
                    return Err(ApiError::new(
                        400,
                        "Trop de notifications à marquer comme lues.",
                    ));
                }
                for id in input.ids {
                    tx.execute("UPDATE wrapper_notifications SET unread=0,attempted=1 WHERE account_id=?1 AND id=?2",params![ACCOUNT,id])?;
                }
                snapshot(&tx)?
            }
            ("POST", "/api/notifications/poll") => {
                let input: Poll = crate::parse(body)?;
                let prefs = settings(&tx)?;
                let mut stmt = tx.prepare("SELECT id,kind,conversation_id,card_id,title FROM wrapper_notifications WHERE account_id=?1 AND attempted=0 ORDER BY created_at,id LIMIT 100")?;
                let pending = stmt
                    .query_map([ACCOUNT], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                let mut deliveries = vec![];
                for (id, kind, conversation, card, title) in pending {
                    let visible = input.focused
                        && input.visible
                        && input.conversation_id.as_deref() == Some(&conversation);
                    if visible || !prefs.permits(&kind) {
                        tx.execute("UPDATE wrapper_notifications SET attempted=1,unread=CASE WHEN ?3 THEN 0 ELSE unread END WHERE account_id=?1 AND id=?2",params![ACCOUNT,id,visible])?;
                        continue;
                    }
                    let now = crate::now();
                    let last: Option<u64> = tx.query_row("SELECT attempted_at FROM wrapper_notification_throttle WHERE account_id=?1 AND conversation_id=?2 AND kind=?3 AND title=?4",params![ACCOUNT,conversation,kind,title],|r|r.get(0)).optional()?;
                    if should_deliver(&prefs, &kind, visible, last, now) {
                        // Claim only when delivery is due. A distinct throttled event
                        // remains pending across polls/restarts, unless read or resolved.
                        tx.execute("UPDATE wrapper_notifications SET attempted=1 WHERE account_id=?1 AND id=?2", params![ACCOUNT,id])?;
                        tx.execute("INSERT INTO wrapper_notification_throttle VALUES (?1,?2,?3,?4,?5) ON CONFLICT(account_id,conversation_id,kind,title) DO UPDATE SET attempted_at=excluded.attempted_at",params![ACCOUNT,conversation,kind,title,now])?;
                        deliveries.push(json!({"id":id,"kind":kind,"conversation_id":conversation,"card_id":card,"title":title}));
                    }
                }
                let mut result = snapshot(&tx)?;
                result["deliveries"] = json!(deliveries);
                result
            }
            _ => {
                return Err(ApiError::new(
                    404,
                    "Cette route de notifications n’existe pas.",
                ))
            }
        };
        tx.commit()?;
        Ok(ApiResponse::Json(response))
    })())
}

/// Reuse the exact settings, source identities, claims and throttles used by
/// the WebView. A headless run never prompts for OS permission.
pub(crate) fn background(app: &mut App) -> Result<(), ApiError> {
    let response = routes(
        app,
        "POST",
        "/api/notifications/poll",
        br#"{"focused":false,"visible":false,"conversation_id":null}"#,
    )
    .expect("notification route")?;
    let ApiResponse::Json(value) = response else {
        return Ok(());
    };
    for event in value["deliveries"].as_array().into_iter().flatten() {
        #[cfg(debug_assertions)]
        {
            use std::io::Write;
            // Agent installation already validated this throwaway environment.
            // No debug build can execute the native delivery helper.
            let root = super::routines_background::test_root()?;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join("notifications-recording.jsonl"))
                .map_err(|_| {
                    ApiError::new(500, "Impossible d’enregistrer la notification de test.")
                })?;
            writeln!(file, "{event}")
                .map_err(|_| ApiError::new(500, "Notification de test non enregistrée."))?;
        }
        #[cfg(not(debug_assertions))]
        {
            let helper = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join("Ezerbot")))
                .ok_or_else(|| {
                    ApiError::new(500, "Service de notifications Ezerbot introuvable.")
                })?;
            let status = std::process::Command::new(helper)
                .arg("--notify-background")
                .arg(event.to_string())
                .status()
                .map_err(|_| ApiError::new(500, "Le service de notifications n’a pas démarré."))?;
            if !status.success() {
                return Err(ApiError::new(
                    500,
                    "L’alerte macOS n’a pas pu être envoyée. L’événement reste dans Notifications.",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reply_projection_uses_both_full_message_primary_key_lookups() {
        let guard = grokbot_test_support::TestDir::new("notification-plan");
        let directory = guard.to_path_buf();
        {
            let app = App::open(&directory.join("workspace.sqlite")).unwrap();
            let ctx = app.feature_context();
            let mut statement=ctx.connection.prepare(&format!("EXPLAIN QUERY PLAN SELECT * FROM ({SOURCES}) src WHERE NOT EXISTS (SELECT 1 FROM wrapper_notifications n WHERE n.account_id=?1 AND n.id=src.id)")).unwrap();
            let plan = statement
                .query_map([ACCOUNT], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let lookups: Vec<_> = plan
                .iter()
                .filter(|line| line.contains("SEARCH m USING"))
                .collect();
            assert_eq!(lookups.len(), 2, "{plan:#?}");
            assert!(
                lookups.iter().all(
                    |line| line.contains("account_id=? AND conversation_id=? AND message_id=?")
                ),
                "{plan:#?}"
            );
            assert!(
                !plan.iter().any(|line| line.contains("SCAN m")),
                "{plan:#?}"
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    const HOUR: u64 = 3_600_000;

    fn reminders(conn: &Connection) -> Vec<(String, String, String)> {
        let mut statement = conn
            .prepare("SELECT id,kind,title FROM wrapper_notifications WHERE id LIKE 'remind:%' ORDER BY id")
            .unwrap();
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn a_waiting_approval_is_reminded_once_after_the_chosen_delay_on_a_simulated_clock() {
        let guard = grokbot_test_support::TestDir::new("notification-reminder");
        let directory = guard.to_path_buf();
        {
            let mut app = App::open(&directory.join("workspace.sqlite")).unwrap();
            let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()
                ["conversation"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
            let (created, expires) = (1_000_000u64, 1_000_000 + 24 * HOUR);
            app.connection.execute("INSERT INTO forms_requests(id,run_id,entry_id,kind_json,route_json) VALUES ('approval-1','source','approval-1',?1,?2)",
                params![json!({"kind":"approval","tool":"NE_JAMAIS_NOTIFIER_CET_OUTIL","expires_at":expires}).to_string(), json!({"target_conversation_id":conversation}).to_string()]).unwrap();
            app.connection.execute("INSERT INTO wrapper_messages(account_id,conversation_id,message_id,created_at,kind) VALUES (?1,?2,'approval-1',?3,'approval')", params![ACCOUNT, conversation, created]).unwrap();
            let set = |app: &mut App, reminder: &str| {
                let body = json!({"task":true,"chat":false,"validity":"24h","reminder":reminder});
                app.api(
                    "POST",
                    "/api/approvals/waiting",
                    body.to_string().as_bytes(),
                )
                .unwrap();
            };
            // The default is one hour: nothing a second before, exactly one at 1 h 00, still one after.
            project(&app.connection, created + HOUR - 1, false).unwrap();
            assert!(reminders(&app.connection).is_empty());
            project(&app.connection, created + HOUR + 60_000, false).unwrap();
            project(&app.connection, created + 2 * HOUR, false).unwrap();
            let rows = reminders(&app.connection);
            assert_eq!(rows.len(), 1, "{rows:?}");
            assert_eq!(rows[0].0, "remind:approval-1");
            assert_eq!(rows[0].1, "approval");
            assert_eq!(rows[0].2, "Une demande attend votre décision depuis 1 h");
            assert!(!rows[0].2.contains("NE_JAMAIS"));
            // A reminder already sent is not sent again when the delay changes afterwards.
            set(&mut app, "30min");
            project(&app.connection, created + 3 * HOUR, false).unwrap();
            assert_eq!(reminders(&app.connection).len(), 1);

            app.connection
                .execute(
                    "DELETE FROM wrapper_notifications WHERE id LIKE 'remind:%'",
                    [],
                )
                .unwrap();
            set(&mut app, "30min");
            project(&app.connection, created + HOUR / 2 - 1, false).unwrap();
            assert!(reminders(&app.connection).is_empty());
            project(&app.connection, created + HOUR / 2, false).unwrap();
            assert_eq!(
                reminders(&app.connection)[0].2,
                "Une demande attend votre décision depuis 30 min"
            );

            app.connection
                .execute(
                    "DELETE FROM wrapper_notifications WHERE id LIKE 'remind:%'",
                    [],
                )
                .unwrap();
            set(&mut app, "3h");
            project(&app.connection, created + 3 * HOUR - 1, false).unwrap();
            assert!(reminders(&app.connection).is_empty());
            project(&app.connection, created + 3 * HOUR, false).unwrap();
            assert_eq!(
                reminders(&app.connection)[0].2,
                "Une demande attend votre décision depuis 3 h"
            );

            // « Jamais » remembers nothing and never reminds.
            app.connection
                .execute(
                    "DELETE FROM wrapper_notifications WHERE id LIKE 'remind:%'",
                    [],
                )
                .unwrap();
            set(&mut app, "never");
            project(&app.connection, created + 10 * HOUR, false).unwrap();
            assert!(reminders(&app.connection).is_empty());

            // Decided before the delay, or expired: no reminder.
            set(&mut app, "1h");
            app.connection
                .execute("UPDATE forms_requests SET status='resolved'", [])
                .unwrap();
            project(&app.connection, created + 2 * HOUR, false).unwrap();
            assert!(reminders(&app.connection).is_empty());
            app.connection
                .execute("UPDATE forms_requests SET status='pending'", [])
                .unwrap();
            project(&app.connection, expires, false).unwrap();
            assert!(reminders(&app.connection).is_empty());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn every_kind_switch_dnd_focus_and_throttle_are_independent() {
        let prefs = Settings {
            enabled: true,
            ..Settings::default()
        };
        for kind in [
            "reply",
            "approval",
            "form",
            "secret",
            "routine",
            "delegation",
        ] {
            assert!(should_deliver(&prefs, kind, false, None, 10_000));
            let mut value = json!(prefs);
            value[kind] = json!(false);
            let muted: Settings = serde_json::from_value(value).unwrap();
            assert!(!should_deliver(&muted, kind, false, None, 10_000));
            assert!(!should_deliver(&prefs, kind, true, None, 10_000));
            assert!(!should_deliver(&prefs, kind, false, Some(5_001), 10_000));
            assert!(should_deliver(&prefs, kind, false, Some(5_000), 10_000));
            assert!(!should_deliver(&prefs, kind, false, Some(11_000), 10_000));
            assert!(!should_deliver(
                &Settings {
                    dnd: true,
                    ..prefs.clone()
                },
                kind,
                false,
                None,
                10_000
            ));
        }
        assert!(!should_deliver(&prefs, "unknown", false, None, 10_000));
        assert!(!should_deliver(
            &Settings::default(),
            "reply",
            false,
            None,
            10_000
        ));
    }
}
