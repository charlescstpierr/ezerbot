//! Five-field cron and intervals, following `reports/cron-detail.md` CR08–CR13.
//! DOM/DOW use OR when both are restricted; intervals with a phase align to UTC
//! epoch, while intervals without one are relative to the supplied reference.
//!
//! Unlike the client's *candidate* preview (CR11), this scheduler implements the
//! explicit policy in `reports/architecture-rust.md`: gaps move to the first
//! valid instant, folds fire only at their first instant. Calendar search covers
//! a Gregorian cycle (400 years), so leap-day schedules are not lost after 366
//! days. Invalid zones fail closed instead of silently using the host's zone.
use grokbot_engine::contracts::{Error, Result};
use jiff::{civil::Date, tz::AmbiguousOffset, tz::TimeZone, Timestamp};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    Cron {
        expression: String,
        time_zone: Option<String>,
    },
    Every {
        interval_ms: u64,
        phase_ms: Option<u64>,
    },
    Once {
        at: i64,
    },
    /// Never due by itself: an authenticated webhook call starts the routine.
    Webhook,
    /// Never due by itself: an event of one repository, delivered by the GitHub
    /// connector, starts the routine. Only the five events of [`GithubEvent`] do.
    Github {
        repo: String,
        events: Vec<GithubEvent>,
    },
    /// Starts as soon as one of its simple triggers starts. The members are a
    /// flat list: a group inside a group is refused when the routine is saved,
    /// but the type stays open to the simple kinds added later.
    Group {
        members: Vec<Member>,
    },
}

/// What a GitHub trigger can listen to. Any other event of the connector is
/// ignored: it cannot be chosen here and never starts a routine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubEvent {
    PullRequestOpened,
    PullRequestPushed,
    PullRequestMerged,
    /// A comment on a pull request, on an issue, or an inline review comment.
    Comment,
    IssueOpened,
}

impl GithubEvent {
    pub const ALL: [GithubEvent; 5] = [
        GithubEvent::PullRequestOpened,
        GithubEvent::PullRequestPushed,
        GithubEvent::PullRequestMerged,
        GithubEvent::Comment,
        GithubEvent::IssueOpened,
    ];
    /// The name the connector gives the event.
    pub fn wire(self) -> &'static str {
        match self {
            GithubEvent::PullRequestOpened => "pull_request_opened",
            GithubEvent::PullRequestPushed => "pull_request_pushed",
            GithubEvent::PullRequestMerged => "pull_request_merged",
            GithubEvent::Comment => "comment",
            GithubEvent::IssueOpened => "issue_opened",
        }
    }
    pub fn from_wire(name: &str) -> Option<GithubEvent> {
        Self::ALL.into_iter().find(|event| event.wire() == name)
    }
    pub fn label_fr(self) -> &'static str {
        match self {
            GithubEvent::PullRequestOpened => "pull request ouverte",
            GithubEvent::PullRequestPushed => "pull request poussée",
            GithubEvent::PullRequestMerged => "pull request mergée",
            GithubEvent::Comment => "commentaire",
            GithubEvent::IssueOpened => "issue ouverte",
        }
    }
}

/// `owner/name` as GitHub spells it: the characters of a login and of a repository.
pub fn valid_github_repo(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    let part = |text: &str, limit: usize| {
        !text.is_empty()
            && text.len() <= limit
            && !matches!(text, "." | "..")
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    part(owner, 39) && part(name, 100)
}

/// A simple trigger of a group. The id is stable across edits, so that each
/// member keeps its own schedule version and its own webhook token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    #[serde(default)]
    pub id: String,
    pub trigger: Schedule,
}

impl Schedule {
    /// A webhook call can start the routine without any clock slot: either it is
    /// a webhook, or a group holding one.
    pub fn has_webhook(&self) -> bool {
        match self {
            Schedule::Webhook => true,
            Schedule::Group { members } => members.iter().any(|m| m.trigger == Schedule::Webhook),
            _ => false,
        }
    }

    /// Something other than the clock starts the routine: a webhook call or a
    /// GitHub event, alone or in a group. Such a routine has no next slot to wait for.
    pub fn starts_on_event(&self) -> bool {
        match self {
            Schedule::Webhook | Schedule::Github { .. } => true,
            Schedule::Group { members } => members.iter().any(|m| m.trigger.starts_on_event()),
            _ => false,
        }
    }

    /// The GitHub trigger is a simple trigger: alone or as a member of a group.
    pub fn has_github(&self) -> bool {
        match self {
            Schedule::Github { .. } => true,
            Schedule::Group { members } => members
                .iter()
                .any(|m| matches!(m.trigger, Schedule::Github { .. })),
            _ => false,
        }
    }
}

fn invalid(message: impl std::fmt::Display) -> Error {
    Error::InvalidInput(format!("invalid schedule: {message}"))
}

/// Parse client cron syntax. One-shots are constructed as `Schedule::Once`, not
/// inferred from an annual cron (CR15). Durations must fit whole milliseconds,
/// the precision of the shared schedule contract.
pub fn parse(expression: &str) -> Result<Schedule> {
    let mut tokens: Vec<_> = expression.split_whitespace().collect();
    let time_zone = tokens
        .first()
        .and_then(|token| {
            token
                .strip_prefix("CRON_TZ=")
                .or_else(|| token.strip_prefix("TZ="))
        })
        .map(str::to_owned);
    if let Some(zone) = &time_zone {
        TimeZone::get(zone).map_err(invalid)?;
        tokens.remove(0);
    }
    if tokens
        .first()
        .is_some_and(|token| token.eq_ignore_ascii_case("@every"))
    {
        if time_zone.is_some() || tokens.len() != 2 {
            return Err(invalid("@every needs one duration and no timezone prefix"));
        }
        let mut parts = tokens[1].split('/');
        let interval_ms = duration_ms(parts.next().unwrap())?;
        let phase_ms = parts.next().map(duration_ms).transpose()?;
        if parts.next().is_some() {
            return Err(invalid("too many interval phases"));
        }
        validate_every(interval_ms, phase_ms)?;
        return Ok(Schedule::Every {
            interval_ms,
            phase_ms,
        });
    }
    let expression = if tokens.len() == 1 {
        match tokens[0].to_ascii_lowercase().as_str() {
            "@hourly" => "0 * * * *".to_owned(),
            "@daily" | "@midnight" => "0 0 * * *".to_owned(),
            "@weekly" => "0 0 * * 0".to_owned(),
            "@monthly" => "0 0 1 * *".to_owned(),
            "@yearly" | "@annually" => "0 0 1 1 *".to_owned(),
            _ => tokens.join(" "),
        }
    } else {
        tokens.join(" ")
    };
    Cron::parse(&expression)?;
    Ok(Schedule::Cron {
        expression,
        time_zone,
    })
}

/// Human-readable French presentation, using the same parser and DOM/DOW OR
/// semantics as execution. Calendar schedules name the effective timezone;
/// elapsed intervals do not use one.
/// This describes a schedule, never predicts its next occurrence.
pub fn describe_fr(schedule: &Schedule, time_zone: &str) -> Result<String> {
    let (text, zone) = match schedule {
        Schedule::Once { at } => {
            let zone = TimeZone::get(time_zone).map_err(invalid)?;
            let date = Timestamp::from_millisecond(*at)
                .map_err(invalid)?
                .to_zoned(zone);
            (
                format!("Une fois le {}", date.strftime("%d/%m/%Y à %H:%M:%S")),
                time_zone,
            )
        }
        Schedule::Webhook => {
            return Ok("Webhook : démarre à chaque appel authentifié".into());
        }
        Schedule::Github { repo, events } => {
            let events: Vec<&str> = events.iter().map(|event| event.label_fr()).collect();
            return Ok(format!("GitHub {repo} : {}", events.join(", ")));
        }
        Schedule::Group { members } => {
            let parts = members
                .iter()
                .map(|member| describe_fr(&member.trigger, time_zone))
                .collect::<Result<Vec<_>>>()?;
            return Ok(format!("Groupe, dès que l’un part : {}", parts.join(" · ")));
        }
        Schedule::Every {
            interval_ms,
            phase_ms,
        } => {
            validate_every(*interval_ms, *phase_ms)?;
            let duration = readable_duration(*interval_ms);
            let cadence = if let Some(unit) = duration.strip_prefix("1 ") {
                format!("Chaque {unit}")
            } else {
                format!(
                    "{} {duration}",
                    if interval_ms.is_multiple_of(86_400_000) {
                        "Tous les"
                    } else {
                        "Toutes les"
                    }
                )
            };
            let text = match phase_ms {
                Some(phase) => format!(
                    "{cadence}, décalé de {} sur l’horloge UTC",
                    readable_duration(*phase)
                ),
                None => format!("{cadence}, à partir de l’activation"),
            };
            return Ok(text);
        }
        Schedule::Cron {
            expression,
            time_zone: override_zone,
        } => {
            let Schedule::Cron {
                expression,
                time_zone: embedded_zone,
            } = parse(expression)?
            else {
                return Err(invalid("a Cron schedule must contain cron syntax"));
            };
            let zone = embedded_zone
                .as_deref()
                .or(override_zone.as_deref())
                .unwrap_or(time_zone);
            TimeZone::get(zone).map_err(invalid)?;
            let cron = Cron::parse(&expression)?;
            let [minute, hour, dom, month, dow] = cron.fields;
            let values = |field: Field, min, max| {
                (min..=max)
                    .filter(|v| field.contains(*v))
                    .collect::<Vec<_>>()
            };
            let list = |values: Vec<u32>| {
                values
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let minutes = values(minute, 0, 59);
            let hours = values(hour, 0, 23);
            let time = if minutes.len() == 1 && hours.len() == 1 {
                format!("à {:02}:{:02}", hours[0], minutes[0])
            } else if minutes.len() == 60 && hours.len() == 24 {
                "à chaque minute".into()
            } else if hours.len() == 24 {
                format!("à chaque heure, aux minutes {}", list(minutes))
            } else {
                format!(
                    "aux heures {} et aux minutes {}",
                    list(hours),
                    list(minutes)
                )
            };
            let weekdays = [
                "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
            ];
            let week = if dow.bits == 0b11_1110 {
                "du lundi au vendredi".into()
            } else {
                join_fr(
                    &values(dow, 0, 6)
                        .iter()
                        .map(|v| format!("le {}", weekdays[*v as usize]))
                        .collect::<Vec<_>>(),
                )
            };
            let month_days = format!("{} du mois", calendar_days_fr(&values(dom, 1, 31)));
            let days = match (dom.wildcard, dow.wildcard) {
                (true, true) => "tous les jours".into(),
                (true, false) => week,
                (false, true) => month_days,
                (false, false) => format!("{month_days} OU {week}"),
            };
            let months = if month.wildcard {
                String::new()
            } else {
                let names = [
                    "",
                    "janvier",
                    "février",
                    "mars",
                    "avril",
                    "mai",
                    "juin",
                    "juillet",
                    "août",
                    "septembre",
                    "octobre",
                    "novembre",
                    "décembre",
                ];
                format!(
                    ", en {}",
                    join_fr(
                        &values(month, 1, 12)
                            .iter()
                            .map(|v| names[*v as usize].to_owned())
                            .collect::<Vec<_>>()
                    )
                )
            };
            let text = format!("{days} {time}{months}");
            return Ok(format!(
                "{}{} ({zone})",
                text[..1].to_uppercase(),
                &text[1..]
            ));
        }
    };
    Ok(format!("{text} ({zone})"))
}

fn calendar_days_fr(days: &[u32]) -> String {
    let ordinal = |day| {
        if day == 1 {
            "1er".into()
        } else {
            format!("{day}")
        }
    };
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < days.len() {
        let mut end = i;
        while end + 1 < days.len() && days[end + 1] == days[end] + 1 {
            end += 1;
        }
        if end - i >= 2 {
            ranges.push(format!("du {} au {}", ordinal(days[i]), ordinal(days[end])));
        } else {
            ranges.extend(
                days[i..=end]
                    .iter()
                    .map(|day| format!("le {}", ordinal(*day))),
            );
        }
        i = end + 1;
    }
    join_fr(&ranges)
}

fn join_fr(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} et {last}", rest.join(", ")),
    }
}

fn readable_duration(ms: u64) -> String {
    for (unit, divisor) in [
        ("jour", 86_400_000),
        ("heure", 3_600_000),
        ("minute", 60_000),
        ("seconde", 1_000),
    ] {
        if ms > 0 && ms.is_multiple_of(divisor) {
            let n = ms / divisor;
            return format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
        }
    }
    format!("{ms} millisecondes")
}

/// The first occurrence strictly after `reference_ms`, or `None` if no future
/// slot is representable. An expression's zone takes precedence over the
/// routine's zone (CR13). Absolute one-shots and intervals do not use a zone.
/// Revalidate schedules here as serde and callers can bypass `parse`.
pub fn next_after(schedule: &Schedule, time_zone: &str, reference_ms: i64) -> Result<Option<i64>> {
    match schedule {
        Schedule::Once { at } => Ok((*at > reference_ms).then_some(*at)),
        Schedule::Webhook | Schedule::Github { .. } => Ok(None),
        Schedule::Group { members } => {
            let mut first: Option<i64> = None;
            for member in members {
                if let Some(next) = next_after(&member.trigger, time_zone, reference_ms)? {
                    first = Some(first.map_or(next, |current| current.min(next)));
                }
            }
            Ok(first)
        }
        Schedule::Every {
            interval_ms,
            phase_ms,
        } => {
            validate_every(*interval_ms, *phase_ms)?;
            let reference = i128::from(reference_ms);
            let interval = i128::from(*interval_ms);
            let next = match phase_ms {
                None => reference + interval,
                Some(phase) => {
                    let phase = i128::from(*phase);
                    ((reference - phase).div_euclid(interval) + 1) * interval + phase
                }
            };
            Ok(i64::try_from(next).ok())
        }
        Schedule::Cron {
            expression,
            time_zone: override_zone,
        } => {
            // Also accept aliases/prefixes in a manually constructed Cron DTO.
            let Schedule::Cron {
                expression,
                time_zone: embedded_zone,
            } = parse(expression)?
            else {
                return Err(invalid("a Cron schedule must contain cron syntax"));
            };
            let zone = TimeZone::get(
                embedded_zone
                    .as_deref()
                    .or(override_zone.as_deref())
                    .unwrap_or(time_zone),
            )
            .map_err(invalid)?;
            let cron = Cron::parse(&expression)?;
            let reference = Timestamp::from_millisecond(reference_ms).map_err(invalid)?;
            let mut date = zone.to_datetime(reference).date();
            // All possible month/DOM/DOW combinations repeat every 146097 days.
            for _ in 0..=146_097 {
                if cron.matches_date(date) {
                    for hour in 0..24 {
                        if !cron.fields[1].contains(hour) {
                            continue;
                        }
                        for minute in 0..60 {
                            if !cron.fields[0].contains(minute) {
                                continue;
                            }
                            let local = date.at(hour as i8, minute as i8, 0, 0);
                            let ambiguous = zone.to_ambiguous_timestamp(local);
                            let candidate = match ambiguous.offset() {
                                AmbiguousOffset::Gap { .. } => {
                                    // `later()` would preserve the minute into the
                                    // gap (02:30 -> 03:30), not its first valid
                                    // instant (03:00). Take the transition itself.
                                    let before = ambiguous.earlier().map_err(invalid)?;
                                    zone.following(before)
                                        .next()
                                        .ok_or_else(|| invalid("gap has no ending transition"))?
                                        .timestamp()
                                }
                                _ => ambiguous.earlier().map_err(invalid)?,
                            };
                            if candidate > reference {
                                return Ok(Some(candidate.as_millisecond()));
                            }
                        }
                    }
                }
                date = match date.tomorrow() {
                    Ok(date) => date,
                    Err(_) => return Ok(None),
                };
            }
            Ok(None)
        }
    }
}

#[derive(Clone, Copy)]
struct Field {
    bits: u64,
    wildcard: bool,
}

impl Field {
    fn contains(self, value: u32) -> bool {
        self.bits & (1 << value) != 0
    }

    fn parse(text: &str, min: u32, max: u32, sunday: bool) -> Result<Self> {
        let number = |text: &str| -> Result<u32> {
            if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid(format!("expected a number in {text:?}")));
            }
            text.parse().map_err(invalid)
        };
        let mut bits = 0;
        for item in text.split(',') {
            let (range, step) = match item.split_once('/') {
                None => (item, 1),
                Some((range, step)) => (range, number(step)?),
            };
            if step == 0 {
                return Err(invalid("a field step must be positive"));
            }
            let (start, end) = if range == "*" {
                (min, max)
            } else if let Some((start, end)) = range.split_once('-') {
                (number(start)?, number(end)?)
            } else {
                let start = number(range)?;
                (start, if item.contains('/') { max } else { start })
            };
            if start < min || end > max || start > end {
                return Err(invalid(format!("field {text:?} is outside {min}..{max}")));
            }
            for value in (start..=end).step_by(step as usize) {
                bits |= 1 << if sunday && value == 7 { 0 } else { value };
            }
        }
        Ok(Self {
            bits,
            wildcard: text == "*",
        })
    }
}

struct Cron {
    fields: [Field; 5],
}

impl Cron {
    fn parse(expression: &str) -> Result<Self> {
        let fields: Vec<_> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(invalid("cron requires exactly five fields (no seconds)"));
        }
        Ok(Self {
            fields: [
                Field::parse(fields[0], 0, 59, false)?,
                Field::parse(fields[1], 0, 23, false)?,
                Field::parse(fields[2], 1, 31, false)?,
                Field::parse(fields[3], 1, 12, false)?,
                Field::parse(fields[4], 0, 7, true)?,
            ],
        })
    }

    fn matches_date(&self, date: Date) -> bool {
        let [_, _, dom, month, dow] = self.fields;
        let day_matches = dom.contains(date.day() as u32);
        let weekday_matches = dow.contains(date.weekday().to_sunday_zero_offset() as u32);
        month.contains(date.month() as u32)
            && match (dom.wildcard, dow.wildcard) {
                (false, false) => day_matches || weekday_matches,
                _ => day_matches && weekday_matches,
            }
    }
}

fn validate_every(interval: u64, phase: Option<u64>) -> Result<()> {
    if interval == 0 || phase.is_some_and(|phase| phase >= interval) {
        return Err(invalid(
            "interval must be positive and phase less than interval",
        ));
    }
    Ok(())
}

// Accumulate decimal components as exact fractions, not floats: 1.001s is
// exactly 1001ms. A fractional-millisecond total cannot fit the public DTO.
fn duration_ms(mut text: &str) -> Result<u64> {
    let mut total = 0_u128;
    let mut scale = 1_u128;
    if text.is_empty() {
        return Err(invalid("empty duration"));
    }
    while !text.is_empty() {
        let end = text
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .ok_or_else(|| invalid("duration needs ms, s, m, h or d units"))?;
        let (number, rest) = text.split_at(end);
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        if whole.is_empty() && fraction.is_empty()
            || number.ends_with('.')
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return Err(invalid("invalid decimal duration"));
        }
        let fraction = fraction.trim_end_matches('0');
        let component_scale = 10_u128
            .checked_pow(u32::try_from(fraction.len()).map_err(invalid)?)
            .ok_or_else(|| invalid("duration precision overflow"))?;
        let coefficient = format!("0{whole}{fraction}")
            .parse::<u128>()
            .map_err(invalid)?;
        let (unit, multiplier) = [
            ("ms", 1),
            ("s", 1_000),
            ("m", 60_000),
            ("h", 3_600_000),
            ("d", 86_400_000),
        ]
        .into_iter()
        .find(|(unit, _)| rest.starts_with(unit))
        .ok_or_else(|| invalid("unsupported duration unit"))?;
        let new_scale = scale.max(component_scale);
        total = total
            .checked_mul(new_scale / scale)
            .and_then(|total| {
                coefficient
                    .checked_mul(multiplier)
                    .and_then(|part| part.checked_mul(new_scale / component_scale))
                    .and_then(|part| total.checked_add(part))
            })
            .ok_or_else(|| invalid("duration overflow"))?;
        scale = new_scale;
        text = &rest[unit.len()..];
    }
    if !total.is_multiple_of(scale) {
        return Err(invalid(
            "duration must be representable in whole milliseconds",
        ));
    }
    u64::try_from(total / scale).map_err(invalid)
}

#[cfg(test)]
mod cron_tests {
    use super::*;

    fn ms(instant: &str) -> i64 {
        instant.parse::<Timestamp>().unwrap().as_millisecond()
    }

    fn next(expression: &str, reference: &str) -> Option<i64> {
        next_after(&parse(expression).unwrap(), "UTC", ms(reference)).unwrap()
    }

    #[test]
    fn aliases_and_tz_prefix_parse() {
        for (alias, expression) in [
            ("@HOURLY", "0 * * * *"),
            ("@daily", "0 0 * * *"),
            ("@midnight", "0 0 * * *"),
            ("@weekly", "0 0 * * 0"),
            ("@monthly", "0 0 1 * *"),
            ("@yearly", "0 0 1 1 *"),
            ("@annually", "0 0 1 1 *"),
        ] {
            assert_eq!(parse(alias).unwrap(), parse(expression).unwrap());
        }
        for prefix in ["TZ", "CRON_TZ"] {
            let schedule = parse(&format!("  {prefix}=America/New_York  @daily  ")).unwrap();
            assert_eq!(
                next_after(&schedule, "not-a-zone", ms("2025-01-01T00:00:00Z")).unwrap(),
                Some(ms("2025-01-01T05:00:00Z"))
            );
        }
        assert!(parse("TZ=Not/A_Zone @daily").is_err());
        assert!(parse("TZ=UTC @every 1h").is_err());
        assert!(next_after(&parse("@daily").unwrap(), "invalid", 0).is_err());
    }

    #[test]
    fn readable_schedules_keep_calendar_semantics_and_effective_zone() {
        for (expression, expected) in [
            (
                "0 8 * * 1-5",
                "Du lundi au vendredi à 08:00 (America/Toronto)",
            ),
            ("@daily", "Tous les jours à 00:00 (America/Toronto)"),
            ("0 9 * * 7", "Le dimanche à 09:00 (America/Toronto)"),
            (
                "0 9 1-7 2 1",
                "Du 1er au 7 du mois OU le lundi à 09:00, en février (America/Toronto)",
            ),
            (
                "*/15 * * * *",
                "Tous les jours à chaque heure, aux minutes 0, 15, 30, 45 (America/Toronto)",
            ),
            (
                "* * * * *",
                "Tous les jours à chaque minute (America/Toronto)",
            ),
            ("TZ=UTC @monthly", "Le 1er du mois à 00:00 (UTC)"),
            (
                "30 7 1,15 * *",
                "Le 1er et le 15 du mois à 07:30 (America/Toronto)",
            ),
            (
                "0 9 * 1,2 1,3",
                "Le lundi et le mercredi à 09:00, en janvier et février (America/Toronto)",
            ),
        ] {
            assert_eq!(
                describe_fr(&parse(expression).unwrap(), "America/Toronto").unwrap(),
                expected
            );
        }
        let prefixed = Schedule::Cron {
            expression: "TZ=UTC @daily".into(),
            time_zone: Some("Europe/Paris".into()),
        };
        assert_eq!(
            describe_fr(&prefixed, "America/Toronto").unwrap(),
            "Tous les jours à 00:00 (UTC)"
        );
        assert_eq!(
            describe_fr(&parse("@every 1h/0ms").unwrap(), "UTC").unwrap(),
            "Chaque heure, décalé de 0 millisecondes sur l’horloge UTC"
        );
        assert_eq!(
            describe_fr(
                &Schedule::Once {
                    at: ms("2025-01-01T13:00:00Z")
                },
                "America/Toronto"
            )
            .unwrap(),
            "Une fois le 01/01/2025 à 08:00:00 (America/Toronto)"
        );
        assert!(describe_fr(&parse("@daily").unwrap(), "not-a-zone").is_err());
    }

    #[test]
    fn dom_dow_both_restricted_use_or() {
        // Jan 2 is not Monday; Jan 13 is outside 1–7. Both must match CR09.
        assert_eq!(
            next("0 9 1-7 * 1", "2025-01-01T10:00:00Z"),
            Some(ms("2025-01-02T09:00:00Z"))
        );
        assert_eq!(
            next("0 9 1-7 * 1", "2025-01-07T10:00:00Z"),
            Some(ms("2025-01-13T09:00:00Z"))
        );
        assert_eq!(
            next("0 9 * * 1", "2025-01-07T10:00:00Z"),
            Some(ms("2025-01-13T09:00:00Z"))
        );
        assert_eq!(
            next("0 9 1 * *", "2025-01-01T10:00:00Z"),
            Some(ms("2025-02-01T09:00:00Z"))
        );
        assert_eq!(
            next("0 9 1-7 2 1", "2025-01-01T00:00:00Z"),
            Some(ms("2025-02-01T09:00:00Z"))
        );
    }

    #[test]
    fn every_with_phase_aligns_to_epoch() {
        let schedule = parse("@every 1h30m/15m").unwrap();
        assert_eq!(
            schedule,
            Schedule::Every {
                interval_ms: 5_400_000,
                phase_ms: Some(900_000)
            }
        );
        for (reference, expected) in [
            (-1, 900_000),
            (-5_400_001, -4_500_000),
            (900_000, 6_300_000),
            (1_000_000, 6_300_000),
        ] {
            assert_eq!(
                next_after(&schedule, "ignored", reference).unwrap(),
                Some(expected)
            );
        }
        assert_eq!(
            next_after(&parse("@every 0.5s").unwrap(), "ignored", 123).unwrap(),
            Some(623)
        );
        assert_eq!(
            next_after(&parse("@every 1s/0ms").unwrap(), "ignored", -1).unwrap(),
            Some(0)
        );
        assert_eq!(
            parse("@every 1.001s0.1ms0.9ms").unwrap(),
            Schedule::Every {
                interval_ms: 1002,
                phase_ms: None
            }
        );
        assert_eq!(
            parse("@every 1d2h3m4s5ms/.0s").unwrap(),
            Schedule::Every {
                interval_ms: 93_784_005,
                phase_ms: Some(0)
            }
        );
        assert_eq!(
            parse("@every 18446744073709551615ms").unwrap(),
            Schedule::Every {
                interval_ms: u64::MAX,
                phase_ms: None
            }
        );
        for expression in [
            "@every 18446744073709551616ms",
            "@every 0s",
            "@every 1h/1h",
            "@every 1h/-1m",
            "@every 1h/",
            "@every 1h/0/0",
            "@every 1h 30m",
            "@every 1w",
            "@every .1ms",
            "@every 1e3s",
            "@every 1..2s",
            "@every 1.s",
        ] {
            assert!(parse(expression).is_err(), "{expression}");
        }
    }

    #[test]
    fn spring_forward_gap_moves_to_first_valid_instant() {
        assert_eq!(
            next("TZ=America/New_York 30 2 * * *", "2025-03-09T05:00:00Z"),
            Some(ms("2025-03-09T07:00:00Z"))
        );
        // All nonexistent minutes collapse to one slot, not one run per minute.
        assert_eq!(
            next("TZ=America/New_York * 2 * * *", "2025-03-09T07:00:00Z"),
            Some(ms("2025-03-10T06:00:00Z"))
        );
        // Non-hour gap: 02:15 moves to 02:30, not 02:45.
        assert_eq!(
            next("TZ=Australia/Lord_Howe 15 2 * * *", "2025-10-04T13:00:00Z"),
            Some(ms("2025-10-04T15:30:00Z"))
        );
        // A whole skipped date also resolves to the transition instant.
        assert_eq!(
            next("TZ=Pacific/Apia 0 12 30 12 *", "2011-12-29T22:00:00Z"),
            Some(ms("2011-12-30T10:00:00Z"))
        );
    }

    #[test]
    fn fall_back_fold_fires_once() {
        let expression = "TZ=America/New_York 30 1 * * *";
        let first = ms("2025-11-02T05:30:00Z");
        assert_eq!(next(expression, "2025-11-02T04:00:00Z"), Some(first));
        let schedule = parse(expression).unwrap();
        for reference in [
            first,
            ms("2025-11-02T06:00:00Z"),
            ms("2025-11-02T06:30:00Z"),
        ] {
            assert_eq!(
                next_after(&schedule, "UTC", reference).unwrap(),
                Some(ms("2025-11-03T06:30:00Z"))
            );
        }
    }

    #[test]
    #[allow(non_snake_case)] // Keep the test name from the slice acceptance spec.
    fn unsupported_seconds_names_and_L_W_hash_are_rejected() {
        for expression in [
            "",
            "* * * * * *",
            "0 9 * JAN *",
            "0 9 * * MON",
            "0 9 L * *",
            "0 9 1W * *",
            "0 9 * * 1#2",
            "@reboot",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-2 * * * *",
            "1,,2 * * * *",
            "1/2/3 * * * *",
            "-1 * * * *",
            "? * * * *",
        ] {
            assert!(parse(expression).is_err(), "{expression}");
        }
    }

    #[test]
    fn lists_ranges_steps_sunday_and_strict_minute_boundary() {
        assert_eq!(
            next("5,20-40/10 9-17/2 * * 7", "2025-01-05T09:05:00Z"),
            Some(ms("2025-01-05T09:20:00Z"))
        );
        assert_eq!(
            next("0 0 * * 7", "2025-01-05T00:00:00Z"),
            next("0 0 * * 0", "2025-01-05T00:00:00Z")
        );
        assert_eq!(next("* * * * *", "1969-12-31T23:59:59.999Z"), Some(0));
        assert_eq!(
            next("* * * * *", "2025-01-01T00:00:00.001Z"),
            Some(ms("2025-01-01T00:01:00Z"))
        );
        assert_eq!(
            next("0 0 29 2 *", "2025-01-01T00:00:00Z"),
            Some(ms("2028-02-29T00:00:00Z"))
        );
        assert_eq!(next("0 0 31 2 *", "2025-01-01T00:00:00Z"), None);
    }

    #[test]
    fn webhook_is_never_due_by_the_clock_and_keeps_its_wire_shape() {
        let webhook = Schedule::Webhook;
        assert_eq!(
            serde_json::to_string(&webhook).unwrap(),
            r#"{"kind":"webhook"}"#
        );
        assert_eq!(
            serde_json::from_str::<Schedule>(r#"{"kind":"webhook"}"#).unwrap(),
            webhook
        );
        assert_eq!(next_after(&webhook, "UTC", i64::MIN).unwrap(), None);
        assert_eq!(
            describe_fr(&webhook, "UTC").unwrap(),
            "Webhook : démarre à chaque appel authentifié"
        );
    }

    #[test]
    fn github_is_never_due_by_the_clock_and_keeps_its_wire_shape() {
        let github = Schedule::Github {
            repo: "charles/essai".into(),
            events: vec![GithubEvent::PullRequestOpened, GithubEvent::Comment],
        };
        assert_eq!(
            serde_json::to_string(&github).unwrap(),
            r#"{"kind":"github","repo":"charles/essai","events":["pull_request_opened","comment"]}"#
        );
        assert_eq!(
            serde_json::from_str::<Schedule>(&serde_json::to_string(&github).unwrap()).unwrap(),
            github
        );
        // An event outside the five cannot be chosen.
        assert!(serde_json::from_str::<Schedule>(
            r#"{"kind":"github","repo":"charles/essai","events":["pull_request_closed"]}"#
        )
        .is_err());
        assert_eq!(next_after(&github, "UTC", i64::MIN).unwrap(), None);
        assert_eq!(
            describe_fr(&github, "UTC").unwrap(),
            "GitHub charles/essai : pull request ouverte, commentaire"
        );
        assert!(github.starts_on_event() && github.has_github() && !github.has_webhook());
        let schedule = Schedule::Every {
            interval_ms: 60_000,
            phase_ms: None,
        };
        let group = Schedule::Group {
            members: vec![
                Member {
                    id: "a".into(),
                    trigger: schedule.clone(),
                },
                Member {
                    id: "b".into(),
                    trigger: github,
                },
            ],
        };
        assert!(group.starts_on_event() && group.has_github());
        assert!(!schedule.starts_on_event() && !schedule.has_github());
        for event in GithubEvent::ALL {
            assert_eq!(GithubEvent::from_wire(event.wire()), Some(event));
        }
        assert_eq!(GithubEvent::from_wire("check_run"), None);
    }

    #[test]
    fn a_github_repository_is_owner_slash_name() {
        for good in ["charles/essai", "a/b", "Org-1/repo_name.v2"] {
            assert!(valid_github_repo(good), "{good}");
        }
        for bad in [
            "", "charles", "charles/", "/essai", "a/b/c", "../etc", "a/..", "a b/c", "é/c",
        ] {
            assert!(!valid_github_repo(bad), "{bad}");
        }
    }

    #[test]
    fn once_and_untrusted_dto_boundaries() {
        let once = Schedule::Once { at: 100 };
        assert_eq!(next_after(&once, "ignored", 99).unwrap(), Some(100));
        assert_eq!(next_after(&once, "ignored", 100).unwrap(), None);
        assert_eq!(next_after(&once, "ignored", 101).unwrap(), None);
        let interval = parse("@every 1ms").unwrap();
        assert_eq!(next_after(&interval, "ignored", i64::MAX).unwrap(), None);
        assert_eq!(
            next_after(
                &Schedule::Every {
                    interval_ms: u64::MAX,
                    phase_ms: Some(0)
                },
                "ignored",
                i64::MIN
            )
            .unwrap(),
            Some(0)
        );
        for schedule in [
            Schedule::Every {
                interval_ms: 0,
                phase_ms: None,
            },
            Schedule::Every {
                interval_ms: 1,
                phase_ms: Some(1),
            },
            Schedule::Cron {
                expression: "@every 1h".into(),
                time_zone: None,
            },
            Schedule::Cron {
                expression: "* * * * * *".into(),
                time_zone: None,
            },
        ] {
            assert!(next_after(&schedule, "UTC", 0).is_err());
        }
        let serialized = serde_json::to_string(&parse("TZ=UTC @daily").unwrap()).unwrap();
        let round_trip: Schedule = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            next_after(&round_trip, "ignored", 0).unwrap(),
            Some(86_400_000)
        );
    }
}
