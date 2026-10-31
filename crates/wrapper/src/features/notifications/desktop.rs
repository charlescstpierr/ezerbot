//! Linux delivery over `org.freedesktop.Notifications`, the counterpart of the macOS
//! `EzerbotNotifier`. Same rule as there: the title comes from the stored row and the body is
//! fixed, so no model text, tool argument or secret name reaches the desktop.
use super::*;
use grokbot_linux_desktop::{
    notifications::{Alert, Notifier},
    open, Bus, Unavailable,
};
use serde::de::IgnoredAny;
use std::{
    collections::VecDeque,
    ffi::OsStr,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

const BODY: &str = "Ouvrez Ezerbot pour consulter cet événement.";
const REMEMBERED: usize = 64;

type Targets = Arc<Mutex<VecDeque<(u32, String)>>>;
struct Live {
    notifier: Notifier,
    targets: Targets,
}
static LIVE: Mutex<Option<Live>> = Mutex::new(None);

fn live() -> MutexGuard<'static, Option<Live>> {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Live {
    fn connect(bus: &Bus) -> Result<Self, Unavailable> {
        let notifier = Notifier::connect(bus)?;
        let targets = Targets::default();
        let clicked = Arc::clone(&targets);
        notifier.on_click(move |id| {
            let conversation = clicked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                // A restarted server counts from 1 again: the newest use of an id is the live one.
                .rev()
                .find_map(|(known, conversation)| (*known == id).then(|| conversation.clone()));
            if let Some(conversation) = conversation {
                let link = format!(
                    "ezerbot://app/v1/thread?id={}",
                    crate::deep_links::encode(&conversation)
                );
                let _ = open::launch(OsStr::new(&link));
            }
        })?;
        Ok(Self { notifier, targets })
    }
}

/// Runs `action` on the one process-wide notifier, connecting on first use. A failure drops
/// the connection so that the next delivery reconnects.
fn with_live<T>(
    bus: &Bus,
    action: impl FnOnce(&Live) -> Result<T, Unavailable>,
) -> Result<T, Unavailable> {
    let mut slot = live();
    if slot.is_none() {
        *slot = Some(Live::connect(bus)?);
    }
    let result = action(slot.as_ref().expect("connected above"));
    if result.is_err() {
        *slot = None;
    }
    result
}

fn deliver(bus: &Bus, title: &str, conversation: &str) -> Result<(), Unavailable> {
    with_live(bus, |live| {
        let id = live.notifier.send(&Alert { title, body: BODY })?;
        let mut targets = live.targets.lock().unwrap_or_else(PoisonError::into_inner);
        targets.push_back((id, conversation.to_owned()));
        if targets.len() > REMEMBERED {
            targets.pop_front();
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
            let ready = with_live(&bus, |live| {
                live.notifier.is_ready().then_some(()).ok_or(Unavailable)
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
