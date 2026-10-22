//! The agenda of the connected Google account, read with the Gmail consent. Two reads:
//! the events of a day for the model, and the normalized events a routine consumes
//! (created, modified, cancelled, begun). Every request is a GET on the primary calendar.
use super::*;
use jiff::{tz::TimeZone, Span, Timestamp};

pub(super) const TOOL: &str = "calendar_list_events";
pub(super) const MODEL: &str = "Google Agenda · lire les rendez-vous d’un jour de l’agenda principal (lecture seule, compte Gmail connecté dans Ezerbot ; demain par défaut, ou aujourd’hui). Reprenez le champ `texte` du résultat tel quel dans votre réponse : il nomme le compte, le fuseau et l’heure de chaque rendez-vous.";
pub(super) const SUMMARY: &str = "Lire les rendez-vous d’un jour (demain par défaut) de l’agenda principal du compte Google connecté, en lecture seule.";
const CALENDAR: &str = "calendar/v3/calendars/primary";
/// Events asked for in one page, for a day and for the watch.
const LISTED: u32 = 25;
const WATCHED: u32 = 100;
const DAY: &str = "Jour invalide : choisissez « demain » (par défaut) ou « aujourd_hui ».";

/// Serializes the watch: two callers must not both see an event as new.
static WATCH: Mutex<()> = Mutex::new(());

/// Why an agenda event reaches a routine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarCase {
    Cree,
    Modifie,
    Annule,
    Commence,
}
impl CalendarCase {
    /// The word a history shows.
    pub fn libelle(self) -> &'static str {
        match self {
            Self::Cree => "créé",
            Self::Modifie => "modifié",
            Self::Annule => "annulé",
            Self::Commence => "commence",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Cree => "created",
            Self::Modifie => "modified",
            Self::Annule => "cancelled",
            Self::Commence => "begins",
        }
    }
}
/// An agenda event, as the routines read it. It carries no token and no provider detail.
/// `debut` is the start instant told in `fuseau`; both are empty or zero when Google
/// gave no start for a cancelled event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: String,
    pub cas: CalendarCase,
    pub debut: String,
    pub debut_ms: i64,
    pub fuseau: String,
    pub titre: String,
}

pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_calendar_watch (account_id TEXT PRIMARY KEY,updated_ms INTEGER NOT NULL,started_ms INTEGER NOT NULL);CREATE TABLE IF NOT EXISTS wrapper_calendar_delivered (account_id TEXT NOT NULL,event_id TEXT NOT NULL,cas TEXT NOT NULL,stamp INTEGER NOT NULL,PRIMARY KEY(account_id,event_id,cas,stamp));")?;
    Ok(())
}
pub(super) fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "jour":{"type":"string","enum":["aujourd_hui","demain"],"description":"Le jour à lire, dans le fuseau de l’agenda ; demain si omis."}
    }})
}

fn query(pairs: &[(&str, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())))
        .finish()
}
fn unreadable() -> ApiError {
    invalid("La réponse de l’agenda Google est illisible.")
}
/// One read of the primary calendar, as a JSON answer or the sentence that says why not.
fn fetch(ctx: &Context<'_>, account: &Account, route: &str) -> Result<Value, ApiError> {
    let reply = read(ctx, account, "calendar", route, Want::Json)?;
    if let Some(error) = refused_for(&reply, &account.email, "l’agenda") {
        return Err(error);
    }
    if reply.status != 200 {
        return Err(unreadable());
    }
    Ok(reply.body)
}
/// The IANA zone of the primary calendar, or the Mac's when Google names none that is known.
fn calendar_zone(ctx: &Context<'_>, account: &Account) -> Result<String, ApiError> {
    let body = fetch(ctx, account, CALENDAR)?;
    Ok(body["timeZone"]
        .as_str()
        .filter(|zone| TimeZone::get(zone).is_ok())
        .map(str::to_owned)
        .or_else(|| TimeZone::system().iana_name().map(str::to_owned))
        .unwrap_or_else(|| "UTC".into()))
}
fn zone_named(name: &str) -> Result<TimeZone, ApiError> {
    TimeZone::get(name).map_err(|_| unreadable())
}
fn instant(text: &str) -> Option<i64> {
    text.parse::<Timestamp>().ok().map(|at| at.as_millisecond())
}
fn utc(ms: i64) -> Result<String, ApiError> {
    Timestamp::from_millisecond(ms)
        .map(|at| at.to_string())
        .map_err(|_| unreadable())
}
/// An instant told in a zone: `2026-10-23T09:00:00-04:00`.
fn told(ms: i64, zone: &str) -> String {
    Timestamp::from_millisecond(ms)
        .ok()
        .and_then(|at| at.in_tz(zone).ok())
        .map(|at| at.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
        .unwrap_or_default()
}
/// The start or the end of an event: its instant, its zone, and whether it lasts all day.
fn moment(part: &Value, calendar_zone: &str) -> Option<(i64, String, bool)> {
    let zone = part["timeZone"]
        .as_str()
        .filter(|zone| TimeZone::get(zone).is_ok())
        .unwrap_or(calendar_zone);
    if let Some(text) = part["dateTime"].as_str() {
        return Some((instant(text)?, zone.to_owned(), false));
    }
    let day: jiff::civil::Date = part["date"].as_str()?.parse().ok()?;
    let midnight = day.to_zoned(TimeZone::get(zone).ok()?).ok()?;
    Some((midnight.timestamp().as_millisecond(), zone.to_owned(), true))
}
fn title(event: &Value) -> String {
    let title = clean(event["summary"].as_str().unwrap_or(""), 200);
    let title = title.trim();
    if title.is_empty() {
        "(sans titre)".into()
    } else {
        title.to_owned()
    }
}
fn event_id(event: &Value) -> Option<&str> {
    event["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
}

/// The events of one day, for the model.
pub(super) fn list(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        jour: Option<String>,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| invalid(DAY))?;
    let (days, label) = match args.jour.as_deref() {
        None | Some("demain") => (1, "demain"),
        Some("aujourd_hui") => (0, "aujourd’hui"),
        Some(_) => return Err(invalid(DAY)),
    };
    let account = usable(ctx)?;
    let zone_name = calendar_zone(ctx, &account)?;
    let zone = zone_named(&zone_name)?;
    let today = Timestamp::from_millisecond(ctx.now_ms as i64)
        .map_err(|_| unreadable())?
        .to_zoned(zone.clone())
        .date();
    let day = today
        .checked_add(Span::new().days(days))
        .map_err(|_| unreadable())?;
    let bound = |date: jiff::civil::Date| -> Result<String, ApiError> {
        let start = date.to_zoned(zone.clone()).map_err(|_| unreadable())?;
        utc(start.timestamp().as_millisecond())
    };
    let next = day
        .checked_add(Span::new().days(1))
        .map_err(|_| unreadable())?;
    let body = fetch(
        ctx,
        &account,
        &format!(
            "{CALENDAR}/events?{}",
            query(&[
                ("timeMin", bound(day)?),
                ("timeMax", bound(next)?),
                ("singleEvents", "true".into()),
                ("orderBy", "startTime".into()),
                ("maxResults", LISTED.to_string()),
            ])
        ),
    )?;
    let email = account.email.as_str();
    let mut events = Vec::new();
    let mut lines = Vec::new();
    for item in body["items"].as_array().into_iter().flatten() {
        if item["status"] == "cancelled" {
            continue;
        }
        let (Some(id), Some((start, start_zone, all_day))) =
            (event_id(item), moment(&item["start"], &zone_name))
        else {
            continue;
        };
        let end = moment(&item["end"], &zone_name).map_or(start, |(end, _, _)| end);
        let title = title(item);
        let hours = if all_day {
            "toute la journée".to_owned()
        } else {
            let hour = |ms: i64| told(ms, &start_zone)[11..16].to_owned();
            format!("{}–{}", hour(start), hour(end))
        };
        lines.push(format!("{}. {hours} · {title}", lines.len() + 1));
        events.push(json!({"id":id,"titre":title,"debut":told(start,&start_zone),"fin":told(end,&start_zone),"fuseau":start_zone,"journee":all_day}));
    }
    let heading = format!(
        "{} · compte Gmail : {email} · fuseau de l’agenda : {zone_name}",
        if events.is_empty() {
            format!("Aucun rendez-vous {label} ({day})")
        } else {
            format!("Rendez-vous de {label} ({day})")
        }
    );
    let text = std::iter::once(heading)
        .chain(lines)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(
        json!({"compte":email,"fuseau":zone_name,"jour":day.to_string(),"evenements":events,"texte":text}),
    )
}

/// What the agenda told about one event, with the version that makes a delivery new.
struct Seen {
    event: CalendarEvent,
    stamp: i64,
}
fn calendar_event(item: &Value, cas: CalendarCase, zone: &str, stamp: i64) -> Option<Seen> {
    let id = event_id(item)?.to_owned();
    let (debut_ms, fuseau) = match moment(&item["start"], zone) {
        Some((ms, fuseau, _)) => (ms, fuseau),
        None => (0, zone.to_owned()),
    };
    Some(Seen {
        event: CalendarEvent {
            id,
            cas,
            debut: if debut_ms == 0 {
                String::new()
            } else {
                told(debut_ms, &fuseau)
            },
            debut_ms,
            fuseau,
            titre: title(item),
        },
        stamp,
    })
}

/// The events that changed or began since the last call, the changes first in the order
/// Google made them, then the starts in the order of their time. The first call only
/// learns the agenda. A version is delivered once; what could not be read is tried again.
pub(super) fn new_events(ctx: &Context<'_>) -> Result<Vec<CalendarEvent>, ApiError> {
    let _watch = WATCH.lock().unwrap_or_else(|e| e.into_inner());
    let account = account(ctx)?;
    let owner = account.email.to_lowercase();
    let now = ctx.now_ms as i64;
    let floors: Option<(i64, i64)> = ctx
        .connection
        .query_row(
            "SELECT updated_ms,started_ms FROM wrapper_calendar_watch WHERE account_id=?1",
            [&owner],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((updated_floor, started_floor)) = floors else {
        ctx.connection.execute(
            "INSERT OR REPLACE INTO wrapper_calendar_watch(account_id,updated_ms,started_ms) VALUES (?1,?2,?2)",
            params![owner, now],
        )?;
        return Ok(Vec::new());
    };
    let zone = calendar_zone(ctx, &account)?;
    let changed = fetch(
        ctx,
        &account,
        &format!(
            "{CALENDAR}/events?{}",
            query(&[
                ("updatedMin", utc(updated_floor)?),
                ("showDeleted", "true".into()),
                ("singleEvents", "false".into()),
                ("orderBy", "updated".into()),
                ("maxResults", WATCHED.to_string()),
            ])
        ),
    )?;
    let begun = if now > started_floor {
        Some(fetch(
            ctx,
            &account,
            &format!(
                "{CALENDAR}/events?{}",
                query(&[
                    ("timeMin", utc(started_floor)?),
                    ("timeMax", utc(now)?),
                    ("singleEvents", "true".into()),
                    ("orderBy", "startTime".into()),
                    ("maxResults", WATCHED.to_string()),
                ])
            ),
        )?)
    } else {
        None
    };
    let mut stmt = ctx.connection.prepare(
        "SELECT EXISTS(SELECT 1 FROM wrapper_calendar_delivered WHERE account_id=?1 AND event_id=?2 AND cas=?3 AND stamp=?4)",
    )?;
    let mut delivered = |seen: &Seen| -> Result<bool, ApiError> {
        Ok(stmt.query_row(
            params![owner, seen.event.id, seen.event.cas.key(), seen.stamp],
            |r| r.get::<_, bool>(0),
        )?)
    };
    let mut fresh: Vec<Seen> = Vec::new();
    let mut newest = updated_floor;
    for item in changed["items"].as_array().into_iter().flatten() {
        let Some(updated) = item["updated"].as_str().and_then(instant) else {
            continue;
        };
        newest = newest.max(updated);
        let cas = if item["status"] == "cancelled" {
            CalendarCase::Annule
        } else if item["created"].as_str().and_then(instant) > Some(updated_floor) {
            CalendarCase::Cree
        } else {
            CalendarCase::Modifie
        };
        if let Some(seen) = calendar_event(item, cas, &zone, updated) {
            if !delivered(&seen)? {
                fresh.push(seen);
            }
        }
    }
    // A full page leaves more to read: the floor stops at what was read, not at now.
    let mut started = now;
    if let Some(begun) = &begun {
        let full = begun["items"].as_array().map_or(0, Vec::len) as u32 >= WATCHED;
        let mut last = started_floor;
        for item in begun["items"].as_array().into_iter().flatten() {
            let Some((start, _, _)) = moment(&item["start"], &zone) else {
                continue;
            };
            last = last.max(start);
            if item["status"] == "cancelled" || start <= started_floor || start > now {
                continue;
            }
            if let Some(seen) = calendar_event(item, CalendarCase::Commence, &zone, start) {
                if !delivered(&seen)? {
                    fresh.push(seen);
                }
            }
        }
        if full {
            started = last;
        }
    } else {
        started = started_floor;
    }
    drop(stmt);
    let tx = write_transaction(ctx.connection)?;
    tx.execute(
        "UPDATE wrapper_calendar_watch SET updated_ms=?2,started_ms=?3 WHERE account_id=?1",
        params![owner, newest, started],
    )?;
    for seen in &fresh {
        tx.execute(
            "INSERT OR IGNORE INTO wrapper_calendar_delivered(account_id,event_id,cas,stamp) VALUES (?1,?2,?3,?4)",
            params![owner, seen.event.id, seen.event.cas.key(), seen.stamp],
        )?;
    }
    // Only the most recent deliveries matter: older versions are no longer listed.
    tx.execute(
        "DELETE FROM wrapper_calendar_delivered WHERE account_id=?1 AND rowid NOT IN (SELECT rowid FROM wrapper_calendar_delivered WHERE account_id=?1 ORDER BY rowid DESC LIMIT 1000)",
        [&owner],
    )?;
    tx.commit()?;
    Ok(fresh.into_iter().map(|seen| seen.event).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_told_in_its_own_zone_keeps_it() {
        let item = json!({"id":"a","summary":"Dîner","start":{"dateTime":"2026-10-23T19:30:00+02:00","timeZone":"Europe/Paris"}});
        let seen = calendar_event(&item, CalendarCase::Commence, "America/Toronto", 7).unwrap();
        assert_eq!(seen.event.fuseau, "Europe/Paris");
        assert_eq!(seen.event.debut, "2026-10-23T19:30:00+02:00");
        assert_eq!(seen.event.debut_ms, 1_792_776_600_000);
    }
    #[test]
    fn an_event_without_a_zone_takes_the_calendar_zone() {
        let item = json!({"id":"a","summary":"Appel","start":{"dateTime":"2026-10-23T13:00:00Z"}});
        let seen = calendar_event(&item, CalendarCase::Cree, "America/Toronto", 7).unwrap();
        assert_eq!(seen.event.fuseau, "America/Toronto");
        assert_eq!(seen.event.debut, "2026-10-23T09:00:00-04:00");
    }
    #[test]
    fn an_all_day_event_begins_at_midnight_of_its_zone() {
        let item = json!({"id":"a","summary":"Congé","start":{"date":"2026-10-23"},"end":{"date":"2026-10-24"}});
        let seen = calendar_event(&item, CalendarCase::Commence, "America/Toronto", 7).unwrap();
        assert_eq!(seen.event.debut, "2026-10-23T00:00:00-04:00");
        assert_eq!(seen.event.fuseau, "America/Toronto");
    }
    #[test]
    fn a_cancelled_event_without_a_start_is_still_an_event() {
        let item = json!({"id":"a","status":"cancelled","updated":"2026-10-22T12:00:00Z"});
        let seen = calendar_event(&item, CalendarCase::Annule, "America/Toronto", 7).unwrap();
        assert_eq!(seen.event.id, "a");
        assert_eq!((seen.event.debut.as_str(), seen.event.debut_ms), ("", 0));
        assert_eq!(seen.event.titre, "(sans titre)");
    }
    #[test]
    fn an_identifier_with_a_control_character_is_refused() {
        assert!(calendar_event(&json!({"id":"a\nb"}), CalendarCase::Cree, "UTC", 1).is_none());
        assert!(calendar_event(&json!({"id":""}), CalendarCase::Cree, "UTC", 1).is_none());
    }
}
