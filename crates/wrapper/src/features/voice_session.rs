//! Voice preferences and the debug-only recording synthesizer/live transcription adapter.
//! Real audio remains in the Cocoa process, never in HTTP or model input.
use crate::{parse, ApiError, ACCOUNT};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

const CARRIED: &str = "voice-par-bot-v1";
/// The voice speed range of a bot. 2 is the fastest rate AVSpeechUtterance accepts.
pub const MIN_SPEED: f64 = 0.5;
pub const MAX_SPEED: f64 = 2.0;
const REFUSED_SPEED: &str =
    "Vitesse refusée : choisissez une vitesse de 0,5 à 2. La vitesse du bot reste inchangée.";

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    // `wrapper_voice_settings` is the former global setting: it only feeds the
    // bots that have no voice yet, once. `chosen` tells a saved choice of Charles
    // from an automatic attribution, which may change when a voice disappears.
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_voice_settings (id INTEGER PRIMARY KEY CHECK(id=1), voice TEXT NOT NULL, speed REAL NOT NULL);
        INSERT OR IGNORE INTO wrapper_voice_settings VALUES(1,'',1.0);
        CREATE TABLE IF NOT EXISTS wrapper_voice_bots (bot_id TEXT PRIMARY KEY, voice TEXT NOT NULL, speed REAL NOT NULL, language TEXT NOT NULL, chosen INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS wrapper_voice_catalogue (id INTEGER PRIMARY KEY CHECK(id=1), voices TEXT NOT NULL);
        INSERT OR IGNORE INTO wrapper_voice_catalogue VALUES(1,'[]');")?;
    carry_global_setting(conn)
}

/// The global voice and speed become, once, those of the bots that have none.
fn carry_global_setting(conn: &Connection) -> Result<(), ApiError> {
    let done: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_migrations WHERE account_id=?1 AND id=?2)",
        params![ACCOUNT, CARRIED],
        |r| r.get(0),
    )?;
    if done {
        return Ok(());
    }
    let (voice, speed): (String, f64) = conn.query_row(
        "SELECT voice,speed FROM wrapper_voice_settings WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let speed = if speed.is_finite() {
        speed.clamp(MIN_SPEED, MAX_SPEED)
    } else {
        1.0
    };
    if !voice.is_empty() || speed != 1.0 {
        conn.execute(
            "INSERT OR IGNORE INTO wrapper_voice_bots SELECT id,?1,?2,'',?3 FROM wrapper_bots WHERE account_id=?4 AND deleted=0",
            params![voice, speed, !voice.is_empty(), ACCOUNT],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_migrations (account_id,id) VALUES (?1,?2)",
        params![ACCOUNT, CARRIED],
    )?;
    Ok(())
}

fn valid_language(language: &str) -> bool {
    language.len() <= 32
        && language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn matches_language(voice_language: &str, wanted: &str) -> bool {
    wanted.is_empty()
        || voice_language
            .to_ascii_lowercase()
            .starts_with(&wanted.to_ascii_lowercase())
}

fn catalogue(conn: &Connection) -> Result<Vec<Value>, ApiError> {
    let raw: String = conn.query_row(
        "SELECT voices FROM wrapper_voice_catalogue WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

/// The voices the Mac says it can pronounce, as the interface read them. Bots
/// without a choice are given a voice from this list.
pub fn save_catalogue(conn: &Connection, body: &[u8]) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Voice {
        id: String,
        name: String,
        language: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Catalogue {
        voices: Vec<Voice>,
    }
    let input: Catalogue = parse(body)?;
    let invalid = || {
        ApiError::new(
            400,
            "Liste de voix invalide. Rouvrez l’application pour la relire.",
        )
    };
    let mut voices: Vec<Value> = Vec::new();
    for voice in input.voices {
        if voice.id.is_empty()
            || voice.id.len() > 256
            || voice.id.chars().any(char::is_control)
            || voice.name.len() > 256
            || voice.name.chars().any(char::is_control)
            || !valid_language(&voice.language)
        {
            return Err(invalid());
        }
        if !voices.iter().any(|known| known["id"] == voice.id) {
            voices.push(json!({"id":voice.id,"name":voice.name,"language":voice.language}));
        }
    }
    if voices.len() > 1000 {
        return Err(invalid());
    }
    let tx = super::write_transaction(conn)?;
    tx.execute(
        "UPDATE wrapper_voice_catalogue SET voices=?1 WHERE id=1",
        [serde_json::to_string(&voices).expect("voice JSON")],
    )?;
    assign_missing(&tx)?;
    tx.commit()?;
    Ok(json!({"voices":voices}))
}

struct Row {
    voice: String,
    language: String,
    chosen: bool,
}

/// Gives each live bot without a choice a voice. A bot takes the first voice of
/// its language that no other bot uses; when every voice is taken it shares the
/// least used one. Attributed voices are kept until they leave the list.
pub fn assign_missing(conn: &Connection) -> Result<(), ApiError> {
    let voices = catalogue(conn)?;
    if voices.is_empty() {
        return Ok(());
    }
    let mut bots = conn.prepare(
        "SELECT b.id,COALESCE(v.voice,''),COALESCE(v.language,''),COALESCE(v.chosen,0) FROM wrapper_bots b LEFT JOIN wrapper_voice_bots v ON v.bot_id=b.id WHERE b.account_id=?1 AND b.deleted=0 ORDER BY b.created_at,b.id",
    )?;
    let mut rows: Vec<(String, Row)> = bots
        .query_map([ACCOUNT], |r| {
            Ok((
                r.get(0)?,
                Row {
                    voice: r.get(1)?,
                    language: r.get(2)?,
                    chosen: r.get(3)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let listed = |voice: &str| voices.iter().any(|known| known["id"] == voice);
    for index in 0..rows.len() {
        let current = &rows[index].1;
        if current.chosen || (!current.voice.is_empty() && listed(&current.voice)) {
            continue;
        }
        let uses = |voice: &str| {
            rows.iter()
                .enumerate()
                .filter(|(other, (_, row))| *other != index && row.voice == voice)
                .count()
        };
        let pick = voices
            .iter()
            .filter(|voice| {
                matches_language(
                    voice["language"].as_str().unwrap_or_default(),
                    &current.language,
                )
            })
            .min_by_key(|voice| uses(voice["id"].as_str().unwrap_or_default()))
            .map(|voice| voice["id"].as_str().unwrap_or_default().to_owned());
        let Some(pick) = pick else { continue };
        conn.execute(
            "INSERT INTO wrapper_voice_bots VALUES(?1,?2,1.0,'',0) ON CONFLICT(bot_id) DO UPDATE SET voice=?2",
            params![rows[index].0, pick],
        )?;
        rows[index].1.voice = pick;
    }
    Ok(())
}

/// Voice, speed and language of every live bot, plus the list they come from.
pub fn state(conn: &Connection) -> Result<Value, ApiError> {
    let voices = catalogue(conn)?;
    let mut bots = serde_json::Map::new();
    let mut query = conn.prepare(
        "SELECT b.id,COALESCE(v.voice,''),COALESCE(v.speed,1.0),COALESCE(v.language,''),COALESCE(v.chosen,0) FROM wrapper_bots b LEFT JOIN wrapper_voice_bots v ON v.bot_id=b.id WHERE b.account_id=?1 AND b.deleted=0",
    )?;
    let rows = query.query_map([ACCOUNT], |r| {
        Ok((r.get::<_, String>(0)?, json!({"voice":r.get::<_,String>(1)?,"speed":r.get::<_,f64>(2)?,"language":r.get::<_,String>(3)?,"chosen":r.get::<_,bool>(4)?})))
    })?;
    for row in rows {
        let (id, value) = row?;
        bots.insert(id, value);
    }
    Ok(json!({"voices":voices,"single":voices.len() == 1,"bots":bots}))
}

/// A choice of Charles for one bot. A refused value changes nothing.
pub fn save_bot(conn: &Connection, body: &[u8]) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Choice {
        bot_id: String,
        voice: String,
        speed: f64,
        #[serde(default)]
        language: String,
    }
    let input: Choice = parse(body)?;
    if !input.speed.is_finite() || !(MIN_SPEED..=MAX_SPEED).contains(&input.speed) {
        return Err(ApiError::new(400, REFUSED_SPEED));
    }
    if input.voice.len() > 256
        || input.voice.chars().any(char::is_control)
        || !valid_language(&input.language)
    {
        return Err(ApiError::new(
            400,
            "Réglages vocaux invalides. Choisissez une voix de la liste et une langue ou Auto.",
        ));
    }
    let tx = super::write_transaction(conn)?;
    let live: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_bots WHERE account_id=?1 AND id=?2 AND deleted=0)",
        params![ACCOUNT, input.bot_id],
        |r| r.get(0),
    )?;
    if !live {
        return Err(ApiError::new(404, "Bot introuvable."));
    }
    let voices = catalogue(&tx)?;
    if !input.voice.is_empty()
        && !voices.is_empty()
        && !voices.iter().any(|v| v["id"] == input.voice)
    {
        return Err(ApiError::new(
            400,
            "Cette voix n’est pas installée sur ce Mac. Choisissez une voix de la liste.",
        ));
    }
    tx.execute(
        "INSERT INTO wrapper_voice_bots VALUES(?1,?2,?3,?4,?5) ON CONFLICT(bot_id) DO UPDATE SET voice=?2,speed=?3,language=?4,chosen=?5",
        params![input.bot_id, input.voice, input.speed, input.language, true],
    )?;
    tx.commit()?;
    Ok(state(conn)?["bots"][&input.bot_id].clone())
}

/// What the debug build says it can pronounce. The release list comes from the Mac.
#[cfg(debug_assertions)]
const FAKE_VOICES: [(&str, &str, &str); 2] = [
    ("fake-fr-CA", "Voix française de test (Canada)", "fr-CA"),
    ("fake-fr-FR", "Voix française de test (France)", "fr-FR"),
];

#[cfg(debug_assertions)]
#[derive(Default)]
pub(crate) struct DebugSession {
    source: super::voice::DebugVoice,
    token: String,
    active: bool,
    transcript: String,
    utterance: u64,
    final_text: String,
    barged: u64,
    paused: bool,
    fault: Option<String>,
    microphone: Value,
    queue: Vec<String>,
    spoken: Vec<String>,
    voices: Vec<Value>,
    until: u64,
}
#[cfg(debug_assertions)]
impl DebugSession {
    pub fn command(&mut self, body: &[u8]) -> Result<Value, ApiError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Command {
            command: String,
            #[serde(default)]
            token: String,
            #[serde(default)]
            text: String,
            #[serde(default)]
            final_result: bool,
            #[serde(default)]
            scenario: String,
            #[serde(default, rename = "reason")]
            _reason: String,
            #[serde(default)]
            voice: String,
            #[serde(default)]
            barge: Option<u64>,
            #[serde(default)]
            microphone: String,
            #[serde(default = "default_speed")]
            speed: f64,
            #[serde(default)]
            language: String,
        }
        fn default_speed() -> f64 {
            1.0
        }
        let c: Command = parse(body)?;
        if c.command == "options" {
            return Ok(
                json!({"voices":FAKE_VOICES.map(|(id,name,language)| json!({"id":id,"name":name,"language":language})),"fake":true}),
            );
        }
        if c.command == "start" {
            let source = self.source.command(
                &serde_json::to_vec(
                    &json!({"command":"start","token":c.token,"scenario":c.scenario,"microphone":c.microphone}),
                )
                .unwrap(),
            )?;
            self.token = c.token;
            self.microphone = json!({"microphone_id":source["microphone_id"],"microphone_name":source["microphone_name"],"microphone_fallback":source["microphone_fallback"]});
            self.active = source["status"] == "recording";
            self.transcript.clear();
            self.final_text.clear();
            self.utterance = 0;
            self.barged = 0;
            self.queue.clear();
            self.spoken.clear();
            self.voices.clear();
            self.paused = false;
            self.fault = None;
            if !self.active {
                return Ok(json!({"status":source["status"],"token":self.token,"fake":true}));
            }
        } else if c.token != self.token
            || (!self.active && c.command != "settings" && c.command != "cancel")
        {
            return Err(ApiError::new(409, "Ce mode voix n’est plus actif."));
        } else {
            match c.command.as_str() {
                "feed" => {
                    self.transcript = c.text;
                    if !self.transcript.is_empty() {
                        self.queue.clear();
                        self.barged += 1;
                    }
                    if c.final_result {
                        self.utterance += 1;
                        self.final_text = self.transcript.clone();
                    }
                }
                "speak" => {
                    if c.barge.is_some_and(|barge| barge != self.barged) {
                        return Ok(
                            json!({"status":"listening","token":self.token,"barged":self.barged}),
                        );
                    }
                    if c.text.trim().is_empty()
                        || c.text.encode_utf16().count() > 1500
                        || self.queue.len() >= 128
                        || !c.speed.is_finite()
                        || !(MIN_SPEED..=MAX_SPEED).contains(&c.speed)
                        || (!c.voice.is_empty()
                            && !FAKE_VOICES.iter().any(|(id, ..)| *id == c.voice))
                    {
                        return Err(ApiError::new(400,"Synthèse vocale indisponible. Vérifiez la voix française dans Paramètres."));
                    }
                    self.voices
                        .push(json!({"voice":c.voice,"speed":c.speed,"language":c.language}));
                    self.spoken.push(c.text.clone());
                    self.queue.push(c.text);
                    if self.queue.len() == 1 {
                        self.until = crate::now() + 2000;
                    }
                }
                "interrupt" => {
                    self.queue.clear();
                    self.transcript.clear();
                }
                "pause" => self.paused = true,
                "resume" => {
                    self.paused = false;
                    self.until = crate::now() + 2000;
                }
                "cancel" => {
                    self.active = false;
                    self.queue.clear();
                    self.transcript.clear();
                    self.final_text.clear();
                    self.source.command(
                        &serde_json::to_vec(&json!({"command":"cancel","token":self.token}))
                            .unwrap(),
                    )?;
                }
                "finish_speech" => self.queue.clear(),
                "status" | "settings" => {}
                "fail" => {
                    self.queue.clear();
                    self.fault = Some("failed".into());
                }
                _ => return Err(ApiError::new(400, "Commande du mode voix inconnue.")),
            }
        }
        if !self.paused && !self.queue.is_empty() && crate::now() >= self.until {
            self.queue.remove(0);
            self.until = crate::now() + 2000;
        }
        Ok(
            json!({"status":if self.active {self.fault.as_deref().unwrap_or("listening")} else {"idle"},"token":self.token,"transcript":self.transcript,
            "utterance_id":self.utterance,"final_text":self.final_text,"barged":self.barged,"speaking":!self.queue.is_empty() && !self.paused,
            "paused":self.paused,"recorded_speech":self.spoken,"recorded_voices":self.voices,"microphone_id":self.microphone["microphone_id"],"microphone_name":self.microphone["microphone_name"],"microphone_fallback":self.microphone["microphone_fallback"],"fake":true}),
        )
    }
}
