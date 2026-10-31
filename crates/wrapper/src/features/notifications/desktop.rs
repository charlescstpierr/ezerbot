//! Linux delivery over `org.freedesktop.Notifications`, the counterpart of the macOS
//! `EzerbotNotifier`. Same rule as there: the title comes from the stored row and the body is
//! fixed, so no model text, tool argument or secret name reaches the desktop.
use super::*;
use grokbot_linux_desktop::{
    notifications::{Alert, Notifier, Shown},
    open, Bus, Unavailable,
};
use serde::de::IgnoredAny;
use std::{
    collections::VecDeque,
    ffi::OsStr,
    sync::{Mutex, MutexGuard, PoisonError},
};

const BODY: &str = "Ouvrez Ezerbot pour consulter cet événement.";
/// How many of our notifications stay on the desktop: past it the oldest is closed, so that
/// every notification still shown has a destination.
const REMEMBERED: usize = 64;

static LIVE: Mutex<Option<Notifier>> = Mutex::new(None);
/// Outlives the connection: a server that broadcasts clicks still reaches the next one.
static TARGETS: Mutex<VecDeque<(Shown, String)>> = Mutex::new(VecDeque::new());

fn live() -> MutexGuard<'static, Option<Notifier>> {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn targets() -> MutexGuard<'static, VecDeque<(Shown, String)>> {
    TARGETS.lock().unwrap_or_else(PoisonError::into_inner)
}

fn connect(bus: &Bus) -> Result<Notifier, Unavailable> {
    let notifier = Notifier::connect(bus)?;
    notifier.on_click(|clicked| {
        let conversation = targets()
            .iter()
            .find_map(|(shown, conversation)| (*shown == clicked).then(|| conversation.clone()));
        if let Some(conversation) = conversation {
            let link = format!(
                "ezerbot://app/v1/thread?id={}",
                crate::deep_links::encode(&conversation)
            );
            let _ = open::launch(OsStr::new(&link));
        }
    })?;
    Ok(notifier)
}

/// Runs `action` on the one process-wide notifier, connecting on first use. Servers send a
/// click to the connection that showed the notification, so a failure drops the connection
/// only when the bus is gone: the next delivery then reconnects.
fn with_live<T>(
    bus: &Bus,
    action: impl FnOnce(&Notifier) -> Result<T, Unavailable>,
) -> Result<T, Unavailable> {
    let mut slot = live();
    if slot.is_none() {
        *slot = Some(connect(bus)?);
    }
    let notifier = slot.as_ref().expect("connected above");
    let result = action(notifier);
    if result.is_err() && !notifier.is_connected() {
        *slot = None;
    }
    result
}

fn deliver(bus: &Bus, title: &str, conversation: &str) -> Result<(), Unavailable> {
    with_live(bus, |notifier| {
        let shown = notifier.send(&Alert { title, body: BODY })?;
        let evicted = {
            let mut targets = targets();
            targets.push_back((shown, conversation.to_owned()));
            (targets.len() > REMEMBERED)
                .then(|| targets.pop_front())
                .flatten()
        };
        if let Some((oldest, _)) = evicted {
            let _ = notifier.close(&oldest);
        }
        Ok(())
    })
}

#[cfg(not(debug_assertions))]
pub(super) fn background(title: &str, conversation: &str) -> Result<(), ApiError> {
    let bus = Bus::for_app()
        .ok_or_else(|| ApiError::new(500, "Service de notifications Ezerbot introuvable."))?;
    deliver(&bus, title, conversation).map_err(|_| {
        ApiError::new(
            500,
            "L’alerte n’a pas pu être envoyée. L’événement reste dans Notifications.",
        )
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    command: Command,
    event: Option<Event>,
    #[allow(dead_code)]
    conversation_id: Option<IgnoredAny>,
    #[allow(dead_code)]
    visible: Option<IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Command {
    Status,
    Authorize,
    Presence,
    Deliver,
}
/// The renderer sends the whole delivery object; only the id is trusted and looked up.
#[derive(Deserialize)]
struct Event {
    id: String,
}

/// `POST /api/notifications/native`, handled before the write transaction: a slow bus never
/// holds the database.
pub(super) fn route(conn: &Connection, method: &str, body: &[u8]) -> Result<ApiResponse, ApiError> {
    if method != "POST" {
        return Err(ApiError::new(
            404,
            "Cette route de notifications n’existe pas.",
        ));
    }
    let request: Request = crate::parse(body)?;
    let status = |status: &str| Ok(ApiResponse::Json(json!({ "status": status })));
    let Some(bus) = Bus::for_app() else {
        return status("disabled");
    };
    match request.command {
        Command::Presence => status("ok"),
        Command::Status | Command::Authorize => {
            let ready = with_live(&bus, |notifier| {
                notifier.is_ready().then_some(()).ok_or(Unavailable)
            });
            status(if ready.is_ok() { "granted" } else { "error" })
        }
        Command::Deliver => {
            let id = request
                .event
                .ok_or_else(|| ApiError::new(400, "Événement de notification manquant."))?
                .id;
            let (title, conversation): (String, String) = conn
                .query_row(
                    "SELECT title,conversation_id FROM wrapper_notifications WHERE account_id=?1 AND id=?2",
                    params![ACCOUNT, id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| ApiError::new(404, "Cette notification n’existe plus."))?;
            status(if deliver(&bus, &title, &conversation).is_ok() {
                "delivered"
            } else {
                "error"
            })
        }
    }
}
