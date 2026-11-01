//! Linux microphones, the counterpart of `app/native/microphones.h`.
//!
//! The audio server is driven through its command-line tools rather than linked
//! libraries: the app still starts on a machine without audio libraries, and the
//! hardware path needs no FFI. PipeWire is tried first, then PulseAudio (which
//! also answers through `pipewire-pulse`). Every tool call has a hard deadline
//! because `pw-dump` can hang and the API loop is single-threaded.
//!
//! Compiled in release builds only (and tests): a debug build keeps the fixture
//! microphones of `DebugVoice` and cannot reach a real audio device.
use crate::ApiError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::io::{self, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Per server, so a hung PipeWire still leaves PulseAudio its turn (about 2 s worst case).
const SERVER_DEADLINE: Duration = Duration::from_secs(1);
const MAX_OUTPUT: u64 = 8 << 20;
/// Same bound as the CoreAudio device UID in `microphones.h`.
const MAX_ID: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Microphone {
    pub id: String,
    pub name: String,
}

/// The microphone Ezerbot listens to. Serializes to `{id,name,fallback}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Selection {
    pub id: String,
    pub name: String,
    pub fallback: bool,
    /// No choice was honoured, so the recording follows the system default.
    #[serde(skip)]
    system: bool,
    #[serde(skip)]
    server: Server,
}

/// No audio server answered. Distinct from an empty list: a server that is only
/// briefly unreachable must not make the interface forget a chosen microphone.
#[derive(Debug, PartialEq, Eq)]
pub struct Unavailable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Server {
    PipeWire,
    PulseAudio,
}

/// The inputs of a server, the system default first.
#[derive(Debug, PartialEq, Eq)]
struct Sources {
    default: Option<Microphone>,
    inputs: Vec<Microphone>,
}

/// Raw PCM in the format of the macOS recorder: 16 kHz, mono, signed 16 bits.
pub const SAMPLE_RATE: u32 = 16_000;

impl Server {
    const ALL: [Server; 2] = [Server::PipeWire, Server::PulseAudio];

    fn list(self, session: &AudioSession) -> Option<Sources> {
        let end = Instant::now() + SERVER_DEADLINE;
        match self {
            Server::PipeWire => {
                // Without `--raw`, pw-record wraps the PCM in an AU header.
                let help = session.run("pw-record", &["--help"], end)?;
                help.contains("--raw").then_some(())?;
                parse_pipewire(&session.run("pw-dump", &[], end)?)
            }
            Server::PulseAudio => {
                let list = session.run("pactl", &["-f", "json", "list", "sources"], end)?;
                let default = session.run("pactl", &["get-default-source"], end);
                parse_pulse(&list, default.as_deref().map(str::trim))
            }
        }
    }

    fn record(self, target: Option<&str>) -> (&'static str, Vec<String>) {
        let rate = SAMPLE_RATE;
        match self {
            Server::PipeWire => {
                let mut args = vec![
                    "--raw".to_string(),
                    format!("--rate={rate}"),
                    "--channels=1".into(),
                    "--format=s16".into(),
                ];
                args.extend(target.map(|id| format!("--target={id}")));
                args.push("-".into());
                ("pw-record", args)
            }
            Server::PulseAudio => {
                let mut args = vec![
                    "--raw".to_string(),
                    format!("--rate={rate}"),
                    "--channels=1".into(),
                    "--format=s16le".into(),
                    "--latency-msec=100".into(),
                ];
                args.extend(target.map(|id| format!("--device={id}")));
                ("parec", args)
            }
        }
    }
}

/// A source id and the name shown for it, or nothing when the id is unusable.
fn microphone(id: &str, names: &[Option<&str>]) -> Option<Microphone> {
    if id.is_empty() || id.len() > MAX_ID {
        return None;
    }
    let name = names.iter().flatten().find(|name| !name.is_empty());
    Some(Microphone {
        id: id.into(),
        name: name.map_or(id, |name| *name).into(),
    })
}

impl Sources {
    /// `known` are sources that are not inputs to choose (monitors) but can still be the default.
    fn new(mut inputs: Vec<Microphone>, known: &[Microphone], default: Option<&str>) -> Self {
        let default = default.and_then(|id| inputs.iter().chain(known).find(|mic| mic.id == id));
        let default = default.cloned();
        if let Some(position) = default
            .as_ref()
            .and_then(|default| inputs.iter().position(|mic| mic.id == default.id))
        {
            let first = inputs.remove(position);
            inputs.insert(0, first);
        }
        Sources { default, inputs }
    }

    /// Mirrors `resolve:` of `microphones.h`: the chosen microphone if listed, else the
    /// system default as it is (listed or not), else the first one listed.
    fn resolve(&self, server: Server, requested: &str) -> Selection {
        let chosen = self
            .inputs
            .iter()
            .find(|mic| !requested.is_empty() && mic.id == requested);
        let effective = chosen.or(self.default.as_ref()).or(self.inputs.first());
        Selection {
            id: effective.map(|mic| mic.id.clone()).unwrap_or_default(),
            name: effective.map(|mic| mic.name.clone()).unwrap_or_default(),
            fallback: !requested.is_empty() && chosen.is_none(),
            system: chosen.is_none(),
            server,
        }
    }
}

fn parse_pipewire(dump: &str) -> Option<Sources> {
    let items: Vec<Value> = serde_json::from_str(dump).ok()?;
    let is = |item: &Value, kind: &str| item["type"].as_str().is_some_and(|t| t.ends_with(kind));
    fn prop<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
        item.pointer(&format!("/info/props/{key}"))?.as_str()
    }
    let inputs = items
        .iter()
        .filter(|item| is(item, "Interface:Node"))
        .filter(|item| {
            matches!(
                prop(item, "media.class"),
                Some("Audio/Source" | "Audio/Source/Virtual" | "Audio/Duplex")
            )
        })
        .filter_map(|item| {
            let names = [prop(item, "node.description"), prop(item, "node.nick")];
            microphone(prop(item, "node.name")?, &names)
        })
        .collect();
    let default = items
        .iter()
        .filter(|item| {
            is(item, "Interface:Metadata") && item["props"]["metadata.name"] == "default"
        })
        .filter_map(|item| item["metadata"].as_array())
        .flatten()
        .find(|entry| entry["key"] == "default.audio.source")
        .and_then(|entry| entry["value"]["name"].as_str());
    Some(Sources::new(inputs, &[], default))
}

fn parse_pulse(list: &str, default: Option<&str>) -> Option<Sources> {
    #[derive(Deserialize)]
    struct Source {
        name: String,
        #[serde(default)]
        description: String,
        /// Non-empty on the monitor of an output, empty on a real source.
        #[serde(default)]
        monitor_source: Option<String>,
    }
    let sources: Vec<Source> = serde_json::from_str(list).ok()?;
    let (mut inputs, mut monitors) = (Vec::new(), Vec::new());
    for source in &sources {
        let Some(mic) = microphone(&source.name, &[Some(&source.description)]) else {
            continue;
        };
        if source
            .monitor_source
            .as_deref()
            .is_some_and(|m| !m.is_empty())
        {
            monitors.push(mic);
        } else {
            inputs.push(mic);
        }
    }
    Some(Sources::new(inputs, &monitors, default))
}

/// The environment the audio tools run in.
pub struct AudioSession {
    clear: bool,
    env: Vec<(OsString, OsString)>,
}

impl AudioSession {
    /// The user's own audio session. `LC_ALL=C` because some pactl versions print
    /// JSON numbers with the locale's decimal separator.
    #[cfg(not(debug_assertions))]
    pub fn system() -> Self {
        AudioSession {
            clear: false,
            env: vec![("LC_ALL".into(), "C".into())],
        }
    }

    /// Only the given server: nothing of the user's session leaks into a test.
    #[cfg(test)]
    fn isolated(dir: &std::path::Path, extra: &[(&str, &str)]) -> Self {
        let mut env: Vec<(OsString, OsString)> = vec![
            ("LC_ALL".into(), "C".into()),
            ("HOME".into(), dir.into()),
            ("XDG_RUNTIME_DIR".into(), dir.into()),
            ("PATH".into(), std::env::var_os("PATH").unwrap_or_default()),
        ];
        env.extend(extra.iter().map(|(k, v)| ((*k).into(), (*v).into())));
        AudioSession { clear: true, env }
    }

    fn command(&self, program: &str, args: &[impl AsRef<std::ffi::OsStr>]) -> Command {
        let mut command = Command::new(program);
        if self.clear {
            command.env_clear();
        }
        command
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }

    /// Stdout of a tool that exits successfully before `end`; it is killed otherwise.
    fn run(&self, program: &str, args: &[&str], end: Instant) -> Option<String> {
        let mut child = self.command(program, args).spawn().ok()?;
        let mut stdout = child.stdout.take()?;
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let mut output = Vec::new();
            let _ = (&mut stdout).take(MAX_OUTPUT).read_to_end(&mut output);
            let _ = send.send(output);
        });
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < end => thread::sleep(Duration::from_millis(5)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let output = receive.recv_timeout(Duration::from_millis(200)).ok()?;
        status.filter(|status| status.success())?;
        String::from_utf8(output).ok()
    }

    fn sources(&self) -> Result<(Server, Sources), Unavailable> {
        // PipeWire can run for video only, next to a PulseAudio that holds the microphones.
        let mut silent = None;
        for server in Server::ALL {
            let Some(sources) = server.list(self) else {
                continue;
            };
            if !sources.inputs.is_empty() {
                return Ok((server, sources));
            }
            silent.get_or_insert((server, sources));
        }
        silent.ok_or(Unavailable)
    }

    /// The input devices, the system default first.
    pub fn available(&self) -> Result<Vec<Microphone>, Unavailable> {
        Ok(self.sources()?.1.inputs)
    }

    /// The microphone for the requested id (empty = system default).
    pub fn resolve(&self, requested: &str) -> Result<Selection, Unavailable> {
        let (server, sources) = self.sources()?;
        Ok(sources.resolve(server, requested))
    }

    /// Starts recording. The system selection names no target, so it follows the default.
    pub fn capture(&self, selection: &Selection) -> io::Result<Capture> {
        let target =
            (!selection.system && !selection.id.is_empty()).then_some(selection.id.as_str());
        let (program, args) = selection.server.record(target);
        let child = self.command(program, &args).spawn()?;
        Ok(Capture { child })
    }
}

/// A running recording that yields raw s16le PCM and stops with the value.
pub struct Capture {
    child: Child,
}

impl Read for Capture {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.child.stdout.as_mut() {
            Some(stdout) => stdout.read(buf),
            None => Ok(0),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Body of `GET /api/voice/microphones`.
pub fn listing(session: &AudioSession) -> Result<Value, ApiError> {
    session
        .available()
        .map(|microphones| json!({ "microphones": microphones }))
        .map_err(|Unavailable| {
            ApiError::new(
                503,
                "Aucun serveur audio ne répond. Vérifiez que PipeWire ou PulseAudio fonctionne, puis réessayez.",
            )
        })
}

#[cfg(test)]
#[path = "microphones_tests.rs"]
mod tests;
