//! Voice preferences and the debug-only recording synthesizer/live transcription adapter.
//! Real audio remains in the Cocoa process, never in HTTP or model input.
use crate::{parse, ApiError};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_voice_settings (id INTEGER PRIMARY KEY CHECK(id=1), voice TEXT NOT NULL, speed REAL NOT NULL);
        INSERT OR IGNORE INTO wrapper_voice_settings VALUES(1,'',1.0);")?;
    Ok(())
}
pub fn settings(conn: &Connection) -> Result<Value, ApiError> {
    Ok(conn.query_row(
        "SELECT voice,speed FROM wrapper_voice_settings WHERE id=1",
        [],
        |r| Ok(json!({"voice":r.get::<_,String>(0)?,"speed":r.get::<_,f64>(1)?})),
    )?)
}
pub fn save(conn: &Connection, body: &[u8]) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Settings {
        voice: String,
        speed: f64,
    }
    let input: Settings = parse(body)?;
    if input.voice.len() > 256
        || input.voice.chars().any(char::is_control)
        || !input.speed.is_finite()
        || !(0.5..=1.5).contains(&input.speed)
    {
        return Err(ApiError::new(400,"Réglages vocaux invalides. Choisissez une voix française et une vitesse entre 0,5 et 1,5."));
    }
    conn.execute(
        "UPDATE wrapper_voice_settings SET voice=?1,speed=?2 WHERE id=1",
        params![input.voice, input.speed],
    )?;
    settings(conn)
}

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
    queue: Vec<String>,
    spoken: Vec<String>,
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
            #[serde(default = "default_speed")]
            speed: f64,
        }
        fn default_speed() -> f64 {
            1.0
        }
        let c: Command = parse(body)?;
        if c.command == "options" {
            return Ok(
                json!({"voices":[{"id":"fake-fr-CA","name":"Voix française de test","language":"fr-CA"}],"fake":true}),
            );
        }
        if c.command == "start" {
            let source = self.source.command(
                &serde_json::to_vec(
                    &json!({"command":"start","token":c.token,"scenario":c.scenario}),
                )
                .unwrap(),
            )?;
            self.token = c.token;
            self.active = source["status"] == "recording";
            self.transcript.clear();
            self.final_text.clear();
            self.utterance = 0;
            self.barged = 0;
            self.queue.clear();
            self.spoken.clear();
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
                        || !(0.5..=1.5).contains(&c.speed)
                        || (!c.voice.is_empty() && c.voice != "fake-fr-CA")
                    {
                        return Err(ApiError::new(400,"Synthèse vocale indisponible. Vérifiez la voix française dans Paramètres."));
                    }
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
            "paused":self.paused,"recorded_speech":self.spoken,"fake":true}),
        )
    }
}
