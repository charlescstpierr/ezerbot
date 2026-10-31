//! Navigation only. REA desktop.deep_links: app authority, v1 paths, 2048 bytes,
//! strict query grammar. Local extensions: routine/skill IDs and composer drafts.
//! Neither parsing nor resolution grants permission or invokes a capability.
use std::collections::BTreeMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    decode_query, parse, validate_conversation_id, validate_id, ApiError, ApiResponse, App, ACCOUNT,
};

pub const SETTINGS: &[&str] = &[
    "account",
    "appearance",
    "bots",
    "forms",
    "routines",
    "storage",
    "notifications",
    "mcp",
    "local",
    "shortcuts",
    "usage",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Destination {
    Bot {
        id: String,
    },
    Conversation {
        id: String,
        agent: Option<String>,
    },
    Routine {
        id: String,
        agent: String,
    },
    Skill {
        id: String,
        agent: String,
    },
    Settings {
        id: String,
    },
    Compose {
        text: String,
        agent: Option<String>,
    },
    BotTemplate {
        id: Option<String>,
        data: Option<String>,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Link {
    pub canonical: String,
    pub destination: Destination,
}

fn invalid() -> ApiError {
    ApiError::new(
        400,
        "Ce lien Ezerbot est invalide ou n’est pas pris en charge.",
    )
}

fn atom(value: &str) -> Result<(), ApiError> {
    if matches!(value, "." | "..") {
        return Err(invalid());
    }
    validate_id(value).map_err(|_| invalid())
}

pub(crate) fn encode(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            result.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(result, "%{byte:02X}").expect("string write");
        }
    }
    result
}

pub fn parse_link(raw: &str) -> Result<Link, ApiError> {
    if raw.is_empty()
        || raw.len() > 2048
        || raw.contains(['#', '\\'])
        || raw.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(invalid());
    }
    let (scheme, rest) = raw.split_once("://").ok_or_else(invalid)?;
    if !scheme.eq_ignore_ascii_case("ezerbot") {
        return Err(invalid());
    }
    let (authority, rest) = rest.split_once('/').ok_or_else(invalid)?;
    if !authority.eq_ignore_ascii_case("app") {
        return Err(invalid());
    }
    let (path, query) = rest.split_once('?').ok_or_else(invalid)?;
    let mut fields = BTreeMap::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or_else(invalid)?;
        let value = decode_query(value).map_err(|_| invalid())?;
        if value.is_empty() || fields.insert(key, value).is_some() {
            return Err(invalid());
        }
    }
    let allowed: &[&str] = match path {
        "v1/agent" | "v1/settings" => &["id"],
        "v1/thread" | "v1/routine" | "v1/skill" => &["id", "agent"],
        "v1/open" => &["text", "agent"],
        "v1/bot-template" => &["id", "data"],
        _ => return Err(invalid()),
    };
    if fields.keys().any(|key| !allowed.contains(key)) {
        return Err(invalid());
    }
    let get = |key| fields.get(key).cloned().ok_or_else(invalid);
    let agent = fields.get("agent").cloned();
    if let Some(agent) = &agent {
        atom(agent)?;
    }
    let destination = match path {
        "v1/agent" => {
            let id = get("id")?;
            atom(&id)?;
            Destination::Bot { id }
        }
        "v1/thread" => {
            let id = get("id")?;
            validate_conversation_id(&id).map_err(|_| invalid())?;
            if id.split(':').any(|part| matches!(part, "." | "..")) {
                return Err(invalid());
            }
            Destination::Conversation { id, agent }
        }
        "v1/routine" | "v1/skill" => {
            let id = get("id")?;
            atom(&id)?;
            let agent = agent.ok_or_else(invalid)?;
            if path == "v1/routine" {
                Destination::Routine { id, agent }
            } else {
                Destination::Skill { id, agent }
            }
        }
        "v1/settings" => {
            let id = get("id")?;
            if !SETTINGS.contains(&id.as_str()) {
                return Err(invalid());
            }
            Destination::Settings { id }
        }
        "v1/open" => {
            let text = get("text")?;
            if text.trim().is_empty()
                || text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err(invalid());
            }
            Destination::Compose { text, agent }
        }
        // A catalogue template by ID, or a shared bot whose code fits in 2048 bytes.
        "v1/bot-template" => {
            let id = fields.get("id").cloned();
            let data = fields.get("data").cloned();
            match (&id, &data) {
                (Some(id), None) => atom(id)?,
                (None, Some(data))
                    if data
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')) => {}
                _ => return Err(invalid()),
            }
            Destination::BotTemplate { id, data }
        }
        _ => unreachable!(),
    };
    let query = fields
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    Ok(Link {
        canonical: format!("ezerbot://app/{path}?{query}"),
        destination,
    })
}

impl App {
    /// Resolve against the current local workspace, returning IDs only. No tool,
    /// approval, routine run, credential callback or external URL is accepted.
    pub fn resolve_link(&self, raw: &str) -> Result<Link, ApiError> {
        let link = parse_link(raw)?;
        let inaccessible = || {
            ApiError::new(
                404,
                "La cible de ce lien est introuvable ou inaccessible sur ce Mac.",
            )
        };
        let bot = |id: &str| self.require_bot(id).map_err(|_| inaccessible());
        match &link.destination {
            Destination::Bot { id } => bot(id)?,
            Destination::Compose {
                agent: Some(id), ..
            } => bot(id)?,
            Destination::Conversation { id, agent } => {
                self.require_conversation(id).map_err(|_| inaccessible())?;
                if let Some(agent) = agent {
                    let matches: bool = self.connection.query_row(
                        "SELECT bot_id = ?3 FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2",
                        params![ACCOUNT, id, agent], |row| row.get(0))?;
                    if !matches {
                        return Err(inaccessible());
                    }
                }
            }
            Destination::Routine { id, agent } | Destination::Skill { id, agent } => {
                bot(agent)?;
                let sql = if matches!(&link.destination, Destination::Routine { .. }) {
                    "SELECT EXISTS(SELECT 1 FROM scheduler_routines WHERE routine_id = ?1 AND bot_id = ?2 AND deleted = 0)"
                } else {
                    "SELECT EXISTS(SELECT 1 FROM skills_library WHERE skill_id = ?1 AND bot_id = ?2 AND deleted = 0)"
                };
                let exists: bool = self
                    .connection
                    .query_row(sql, params![id, agent], |row| row.get(0))?;
                if !exists {
                    return Err(inaccessible());
                }
            }
            Destination::BotTemplate { id: Some(id), .. } => {
                if !crate::features::templates::exists(id) {
                    return Err(inaccessible());
                }
            }
            Destination::Compose { agent: None, .. }
            | Destination::Settings { .. }
            | Destination::BotTemplate { id: None, .. } => {}
        }
        Ok(link)
    }

    /// Host-owned 2s deduplication also covers renderer reloads. No durable
    /// navigation journal: restarting the app must not replay an old draft.
    #[doc(hidden)]
    pub fn admit_link(&mut self, raw: &str, now_ms: u64) -> Result<(Link, bool), ApiError> {
        let link = parse_link(raw)?;
        self.deep_link_seen
            .retain(|_, at| now_ms.saturating_sub(*at) < 2000);
        let duplicate = self.deep_link_seen.contains_key(&link.canonical);
        if !duplicate {
            if self.deep_link_seen.len() >= 16 {
                return Err(ApiError::new(
                    429,
                    "Trop de liens reçus. Réessayez dans quelques secondes.",
                ));
            }
            self.deep_link_seen.insert(link.canonical.clone(), now_ms);
            // Admission precedes resolution, like the observed host: an
            // inaccessible target also produces just one error per window.
            self.resolve_link(raw)?;
        }
        Ok((link, duplicate))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    url: String,
}

pub(crate) fn route(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if method != "POST" || path != "/api/deep-links/resolve" {
        return None;
    }
    Some((|| {
        let input: Input = parse(body)?;
        let (link, duplicate) = app.admit_link(&input.url, crate::now())?;
        Ok(ApiResponse::Json(
            json!({"canonical":link.canonical, "destination":link.destination, "duplicate":duplicate}),
        ))
    })())
}
