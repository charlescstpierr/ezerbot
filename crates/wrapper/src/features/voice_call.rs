//! L’appel vocal : une conversation en direct avec un fournisseur de voix, distincte du mode voix.
//! L’audio part vers le fournisseur le temps de l’appel ; le Mac ne garde que le transcript texte.
//! Le secret éphémère qui ouvre la session passe une seule fois de cette route au fournisseur :
//! il n’est ni stocké, ni écrit dans le fil, ni journalisé.
//! Les lignes sont écrites au fil de l’appel, pour que fermer l’app ne perde que la parole non finie.
use super::{write_transaction, Context};
use crate::{parse, validate_id, ApiError, ApiResponse, App, Message, ACCOUNT};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

empty_hooks!(
    tick,
    tools,
    call_tool,
    context,
    on_run_event,
    link_attachments
);

const LABEL: &str = "Appel vocal";
const MAX_LINES: usize = 2000;
const MAX_LINE_BYTES: usize = 4000;
const UNAVAILABLE: &str =
    "L’appel vocal n’est pas disponible : aucun fournisseur de voix n’est configuré sur ce Mac.";

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_voice_calls (
            id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, bot_id TEXT NOT NULL,
            status TEXT NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER);
        CREATE TABLE IF NOT EXISTS wrapper_voice_call_lines (
            call_id TEXT NOT NULL, seq INTEGER NOT NULL, role TEXT NOT NULL, text TEXT NOT NULL,
            PRIMARY KEY (call_id, seq));",
    )?;
    end_open_calls(conn)
}

/// Fermer l’app ou éteindre le Mac termine l’appel, et il ne redémarre pas seul. Le transcript déjà écrit
/// reste dans le fil ; un appel sans une seule ligne n’y laisse rien.
fn end_open_calls(conn: &Connection) -> Result<(), ApiError> {
    let open: Vec<(String, String)> = conn
        .prepare("SELECT id,conversation_id FROM wrapper_voice_calls WHERE status='active'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, conversation) in open {
        let lines: i64 = conn.query_row(
            "SELECT count(*) FROM wrapper_voice_call_lines WHERE call_id=?1",
            [&id],
            |r| r.get(0),
        )?;
        if lines == 0 {
            conn.execute("DELETE FROM wrapper_voice_calls WHERE id=?1", [&id])?;
            continue;
        }
        let now = crate::now();
        conn.execute(
            "UPDATE wrapper_voice_calls SET status='interrupted',ended_at=?2 WHERE id=?1",
            params![id, now],
        )?;
        super::conversations::append_entry(conn, &conversation, "voice_call", &id, now)?;
    }
    Ok(())
}

/// Release builds have no voice provider yet: the call stays unavailable rather than pretending.
fn available() -> bool {
    cfg!(debug_assertions)
}

fn lines(conn: &Connection, id: &str) -> Result<Vec<Value>, ApiError> {
    let mut q = conn
        .prepare("SELECT role,text FROM wrapper_voice_call_lines WHERE call_id=?1 ORDER BY seq")?;
    let rows = q
        .query_map([id], |r| {
            Ok(json!({"role":r.get::<_,String>(0)?,"text":r.get::<_,String>(1)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn dto(conn: &Connection, id: &str) -> Result<Value, ApiError> {
    validate_id(id)?;
    let row = conn
        .query_row(
            "SELECT id,conversation_id,bot_id,status,started_at,ended_at FROM wrapper_voice_calls WHERE id=?1",
            [id],
            |r| {
                Ok(json!({"id":r.get::<_,String>(0)?,"kind":"voice_call","label":LABEL,
                "conversation_id":r.get::<_,String>(1)?,"bot_id":r.get::<_,String>(2)?,
                "status":r.get::<_,String>(3)?,"started_at":r.get::<_,u64>(4)?,
                "ended_at":r.get::<_,Option<u64>>(5)?}))
            },
        )
        .optional()?
        .ok_or_else(|| ApiError::new(404, "Appel vocal introuvable."))?;
    let mut call = row;
    call["lines"] = json!(lines(conn, id)?);
    Ok(call)
}

pub fn state(ctx: &Context<'_>) -> Value {
    let active = ctx
        .connection
        .query_row(
            "SELECT id,conversation_id FROM wrapper_voice_calls WHERE status='active'",
            [],
            |r| Ok(json!({"id":r.get::<_,String>(0)?,"conversation_id":r.get::<_,String>(1)?})),
        )
        .optional()
        .ok()
        .flatten();
    json!({"available":available(),"active":active,"debug_fake":cfg!(debug_assertions)})
}

pub fn entry(ctx: &Context<'_>, kind: &str, id: &str) -> Option<Message> {
    if kind != "voice_call" {
        return None;
    }
    let call = dto(ctx.connection, id).ok()?;
    Some(Message {
        id: id.into(),
        role: "assistant",
        body: String::new(),
        state: "echoed",
        created_at: call["ended_at"]
            .as_u64()
            .or_else(|| call["started_at"].as_u64())?,
        error: None,
        error_kind: None,
        notice: None,
        kind: "voice_call".into(),
        payload: call,
        attachments: vec![],
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    id: String,
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    #[cfg(debug_assertions)]
    if method == "POST" && path == "/api/voice/call/debug" {
        let mode_active = app.debug_voice_session.active();
        return Some(
            app.debug_voice_call
                .command(body, mode_active)
                .map(ApiResponse::Json),
        );
    }
    match (method, path) {
        ("POST", "/api/voice/call/start") => Some(start(app, body).map(ApiResponse::Json)),
        ("POST", "/api/voice/call/line") => Some(line(app, body).map(ApiResponse::Json)),
        ("POST", "/api/voice/call/end") => Some((|| {
            let input: Identity = parse(body)?;
            let tx = write_transaction(&app.connection)?;
            let call = dto(&tx, &input.id)?;
            if call["status"] == "active" {
                let now = crate::now();
                tx.execute(
                    "UPDATE wrapper_voice_calls SET status='ended',ended_at=?2 WHERE id=?1",
                    params![input.id, now],
                )?;
                // A call nobody spoke in leaves no empty card behind.
                if call["lines"].as_array().is_some_and(|l| l.is_empty()) {
                    tx.execute("DELETE FROM wrapper_voice_calls WHERE id=?1", [&input.id])?;
                    tx.commit()?;
                    return Ok(ApiResponse::Json(
                        json!({"id":input.id,"status":"ended","lines":[]}),
                    ));
                }
                super::conversations::append_entry(
                    &tx,
                    call["conversation_id"].as_str().unwrap_or_default(),
                    "voice_call",
                    &input.id,
                    now,
                )?;
            }
            tx.commit()?;
            Ok(ApiResponse::Json(dto(&app.connection, &input.id)?))
        })()),
        _ => None,
    }
}

fn start(app: &mut App, body: &[u8]) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Start {
        conversation_id: String,
    }
    let input: Start = parse(body)?;
    if !available() {
        return Err(ApiError::new(409, UNAVAILABLE));
    }
    app.require_conversation(&input.conversation_id)?;
    let tx = write_transaction(&app.connection)?;
    let open: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_voice_calls WHERE status='active')",
        [],
        |r| r.get(0),
    )?;
    if open {
        return Err(ApiError::new(409, "Un appel vocal est déjà en cours."));
    }
    let bot: String = tx.query_row(
        "SELECT bot_id FROM wrapper_conversations WHERE account_id=?1 AND id=?2",
        params![ACCOUNT, input.conversation_id],
        |r| r.get(0),
    )?;
    let id: String = tx.query_row("SELECT 'call_'||lower(hex(randomblob(16)))", [], |r| {
        r.get(0)
    })?;
    // The provider's ephemeral secret: handed to the caller once, never kept.
    let secret: String = tx.query_row("SELECT 'ezc_'||lower(hex(randomblob(24)))", [], |r| {
        r.get(0)
    })?;
    tx.execute(
        "INSERT INTO wrapper_voice_calls (id,conversation_id,bot_id,status,started_at) VALUES (?1,?2,?3,'active',?4)",
        params![id, input.conversation_id, bot, crate::now()],
    )?;
    tx.commit()?;
    Ok(json!({"call":dto(&app.connection, &id)?,"secret":secret}))
}

fn line(app: &mut App, body: &[u8]) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Line {
        id: String,
        seq: u32,
        role: String,
        text: String,
    }
    let input: Line = parse(body)?;
    let text = input.text.trim();
    if !["user", "assistant"].contains(&input.role.as_str())
        || text.is_empty()
        || text.len() > MAX_LINE_BYTES
        || input.seq as usize >= MAX_LINES
    {
        return Err(ApiError::new(400, "Ligne d’appel invalide."));
    }
    let tx = write_transaction(&app.connection)?;
    let call = dto(&tx, &input.id)?;
    if call["status"] != "active" {
        return Err(ApiError::new(409, "Cet appel est terminé."));
    }
    // The sequence number makes a replayed line a no-op.
    tx.execute(
        "INSERT INTO wrapper_voice_call_lines (call_id,seq,role,text) VALUES (?1,?2,?3,?4) ON CONFLICT(call_id,seq) DO NOTHING",
        params![input.id, input.seq, input.role, text],
    )?;
    tx.commit()?;
    Ok(json!({"recorded":true}))
}

#[cfg(debug_assertions)]
use super::voice_session::microphone_of;

/// The loopback voice provider of debug builds. Release builds link none.
#[cfg(debug_assertions)]
#[derive(Default)]
pub(crate) struct DebugCall {
    source: super::voice::DebugVoice,
    token: String,
    active: bool,
    paused: bool,
    muted: bool,
    transcript: String,
    lines: Vec<Value>,
    barged: u64,
    held: u64,
    speaking_until: u64,
    requested_microphone: String,
    microphone: Value,
    voice: Value,
}
#[cfg(debug_assertions)]
impl DebugCall {
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn command(&mut self, body: &[u8], mode_active: bool) -> Result<Value, ApiError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Command {
            command: String,
            #[serde(default)]
            token: String,
            #[serde(default)]
            secret: String,
            #[serde(default)]
            text: String,
            #[serde(default)]
            final_result: bool,
            #[serde(default)]
            microphone: String,
            #[serde(default)]
            voice: String,
            #[serde(default = "default_speed")]
            speed: f64,
            #[serde(default)]
            language: String,
        }
        fn default_speed() -> f64 {
            1.0
        }
        let c: Command = parse(body)?;
        if c.command == "start" {
            // The provider only opens a session for the secret it minted.
            if c.secret.len() < 24 {
                return Ok(json!({"status":"secret_refused","token":c.token,"fake":true}));
            }
            if mode_active || self.active {
                return Ok(json!({"status":"microphone_busy","token":c.token,"fake":true}));
            }
            let source = self.source.command(
                &serde_json::to_vec(
                    &json!({"command":"start","token":c.token,"microphone":c.microphone}),
                )
                .unwrap(),
            )?;
            if source["status"] != "recording" {
                return Ok(json!({"status":source["status"],"token":c.token,"fake":true}));
            }
            self.token = c.token;
            self.active = true;
            self.paused = false;
            self.muted = false;
            self.transcript.clear();
            self.lines.clear();
            self.barged = 0;
            self.held = 0;
            self.speaking_until = 0;
            self.requested_microphone = c.microphone;
            self.microphone = microphone_of(&source);
            self.voice = json!({"voice":c.voice,"speed":c.speed,"language":c.language});
        } else if c.token != self.token || !self.active {
            return Err(ApiError::new(409, "Cet appel vocal n’est plus actif."));
        } else {
            match c.command.as_str() {
                // Muted, nothing is heard: not even a held sentence is counted.
                "feed" if self.muted => {}
                "feed" => {
                    if self.paused {
                        // A card waits for a click: what is said is not sent, and never answers the card.
                        if c.final_result {
                            self.held += 1;
                        }
                    } else {
                        if self.speaking() {
                            self.speaking_until = 0;
                            self.barged += 1;
                        }
                        self.transcript = c.text.clone();
                        if c.final_result && !c.text.trim().is_empty() {
                            self.lines.push(json!({"role":"user","text":c.text.trim()}));
                            self.lines.push(json!({"role":"assistant","text":format!("Je vous ai entendu : {}", c.text.trim())}));
                            self.speaking_until = crate::now() + 4000;
                            self.transcript.clear();
                        }
                    }
                }
                "interrupt" => self.speaking_until = 0,
                "pause" => self.paused = true,
                "resume" => {
                    self.paused = false;
                    self.held = 0;
                }
                "mute" | "unmute" if (c.command == "mute") != self.muted => {
                    self.muted = !self.muted;
                    let request = if self.muted {
                        self.transcript.clear();
                        json!({"command":"cancel","token":self.token})
                    } else {
                        json!({"command":"start","token":self.token,"microphone":self.requested_microphone})
                    };
                    let source = self
                        .source
                        .command(&serde_json::to_vec(&request).unwrap())?;
                    if !self.muted {
                        self.microphone = microphone_of(&source);
                    }
                }
                "mute" | "unmute" => {}
                "cancel" => {
                    self.active = false;
                    self.muted = false;
                    self.paused = false;
                    self.speaking_until = 0;
                    self.source.command(
                        &serde_json::to_vec(&json!({"command":"cancel","token":self.token}))
                            .unwrap(),
                    )?;
                }
                "status" => {}
                _ => return Err(ApiError::new(400, "Commande d’appel vocal inconnue.")),
            }
        }
        Ok(
            json!({"status":if self.active {"listening"} else {"idle"},"token":self.token,
            "transcript":self.transcript,"lines":self.lines,"barged":self.barged,"held":self.held,
            "speaking":self.speaking(),"paused":self.paused,"muted":self.muted,
            "microphone_id":self.microphone["microphone_id"],"microphone_name":self.microphone["microphone_name"],
            "microphone_fallback":self.microphone["microphone_fallback"],"recorded_voice":self.voice,"fake":true}),
        )
    }
    fn speaking(&self) -> bool {
        self.active && !self.paused && crate::now() < self.speaking_until
    }
}
