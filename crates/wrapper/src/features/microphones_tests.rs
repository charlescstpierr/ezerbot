use super::*;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

fn mic(id: &str, name: &str) -> Microphone {
    Microphone {
        id: id.into(),
        name: name.into(),
    }
}

const PIPEWIRE_DUMP: &str = r#"[
 {"id":30,"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Source","node.name":"alsa_input.usb","node.description":"Micro USB é","node.nick":"USB"}}},
 {"id":31,"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Sink","node.name":"alsa_output.pci","node.description":"Haut-parleurs"}}},
 {"id":32,"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Source/Virtual","node.name":"boucle","node.nick":"Boucle"}}},
 {"id":33,"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Stream/Input/Audio","node.name":"flux"}}},
 {"id":34,"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Duplex","node.name":"duplex","node.description":"Interface audio"}}},
 {"id":35,"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Source","node.name":"TROP_LONG"}}},
 {"id":36,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"settings"},"metadata":[{"subject":0,"key":"default.audio.source","value":{"name":"alsa_input.usb"}}]},
 {"id":37,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"default"},"metadata":[{"subject":0,"key":"default.audio.sink","value":{"name":"alsa_output.pci"}},{"subject":0,"key":"default.audio.source","type":"Spa:String:JSON","value":{"name":"boucle"}}]}
]"#;

const PULSE_LIST: &str = r#"[
 {"index":1,"name":"alsa_output.pci.monitor","description":"Moniteur de Haut-parleurs","monitor_source":"alsa_output.pci","volume":{"mono":{"value":65536,"value_percent":"100%"}}},
 {"index":2,"name":"alsa_input.usb","description":"Micro USB é","monitor_source":"","volume":{}},
 {"index":3,"name":"alsa_input.pci","description":"Micro intégré","monitor_source":""},
 {"index":4,"name":"sans-description","monitor_source":""},
 {"index":5,"name":"","description":"Sans identifiant","monitor_source":""}
]"#;

#[test]
fn pipewire_lists_sources_with_the_metadata_default_first() {
    let dump = PIPEWIRE_DUMP.replace("TROP_LONG", &"x".repeat(MAX_ID + 1));
    let sources = parse_pipewire(&dump).unwrap();
    assert_eq!(
        sources.inputs,
        [
            mic("boucle", "Boucle"),
            mic("alsa_input.usb", "Micro USB é"),
            mic("duplex", "Interface audio"),
        ]
    );
    assert_eq!(sources.default, Some(mic("boucle", "Boucle")));
}

#[test]
fn pipewire_without_default_metadata_keeps_the_dump_order() {
    let dump = PIPEWIRE_DUMP
        .replace("\"default.audio.source\"", "\"default.audio.autre\"")
        .replace("TROP_LONG", "court");
    let sources = parse_pipewire(&dump).unwrap();
    assert_eq!(sources.default, None);
    assert_eq!(sources.inputs[0].id, "alsa_input.usb");
}

#[test]
fn pulse_excludes_monitors_and_unusable_ids() {
    let sources = parse_pulse(PULSE_LIST, Some("alsa_input.pci")).unwrap();
    assert_eq!(
        sources.inputs,
        [
            mic("alsa_input.pci", "Micro intégré"),
            mic("alsa_input.usb", "Micro USB é"),
            mic("sans-description", "sans-description"),
        ]
    );
    assert_eq!(
        sources.default,
        Some(mic("alsa_input.pci", "Micro intégré"))
    );
}

#[test]
fn pulse_default_that_is_a_monitor_stays_the_default_without_being_listed() {
    let sources = parse_pulse(PULSE_LIST, Some("alsa_output.pci.monitor")).unwrap();
    assert_eq!(sources.inputs[0].id, "alsa_input.usb");
    assert_eq!(
        sources.default,
        Some(mic("alsa_output.pci.monitor", "Moniteur de Haut-parleurs"))
    );
    let selection = sources.resolve(Server::PulseAudio, "");
    assert_eq!(selection.id, "alsa_output.pci.monitor");
    assert!(!selection.fallback);
}

#[test]
fn unparsable_output_is_not_an_empty_list() {
    assert_eq!(parse_pulse("", None), None);
    assert_eq!(parse_pulse("{\"a\":1}", None), None);
    assert_eq!(parse_pipewire("Connection refused"), None);
}

#[test]
fn resolve_mirrors_microphones_h() {
    let sources = parse_pulse(PULSE_LIST, Some("alsa_input.pci")).unwrap();
    let json = |requested: &str| {
        serde_json::to_value(sources.resolve(Server::PulseAudio, requested)).unwrap()
    };
    assert_eq!(
        json(""),
        serde_json::json!({"id":"alsa_input.pci","name":"Micro intégré","fallback":false})
    );
    assert_eq!(
        json("alsa_input.usb"),
        serde_json::json!({"id":"alsa_input.usb","name":"Micro USB é","fallback":false})
    );
    assert_eq!(
        json("disparu"),
        serde_json::json!({"id":"alsa_input.pci","name":"Micro intégré","fallback":true})
    );
    let chosen = sources.resolve(Server::PulseAudio, "alsa_input.usb");
    assert!(!chosen.system);
    assert!(sources.resolve(Server::PulseAudio, "disparu").system);
    let nothing = Sources::new(vec![], &[], None).resolve(Server::PulseAudio, "disparu");
    assert_eq!((nothing.id.as_str(), nothing.fallback), ("", true));
    let first =
        Sources::new(vec![mic("a", "A")], &[], Some("absent")).resolve(Server::PulseAudio, "");
    assert_eq!(first.id, "a");
}

#[test]
fn recording_targets_the_server_flags() {
    let (program, args) = Server::PipeWire.record(Some("fixture"));
    assert_eq!(program, "pw-record");
    assert_eq!(args.first().map(String::as_str), Some("--raw"));
    assert!(args.contains(&"--target=fixture".to_string()));
    assert_eq!(args.last().map(String::as_str), Some("-"));
    let (program, args) = Server::PulseAudio.record(None);
    assert_eq!(program, "parec");
    assert!(!args.iter().any(|arg| arg.starts_with("--device")));
    assert!(args.contains(&"--format=s16le".to_string()));
}

#[test]
fn no_server_is_unavailable_not_an_empty_list() {
    let dir = TempDir::new();
    let session = AudioSession::isolated(dir.path(), &[("PULSE_SERVER", "unix:/ezerbot-absent")]);
    assert_eq!(session.available(), Err(Unavailable));
    assert!(session.resolve("fixture").is_err());
    let error = listing(&session).unwrap_err();
    assert_eq!(error.status, 503);
    assert!(error.message.starts_with("Aucun serveur audio"));
}

/// A PATH where `pw-dump` answers `[]` and `pactl` lists `pulse_sources`.
fn stub_tools(dir: &Path, pulse_sources: &str) -> PathBuf {
    let bin = dir.join("bin");
    fs::create_dir(&bin).unwrap();
    for (name, body) in [
        ("pw-record", "echo --raw".to_string()),
        ("pw-dump", "echo '[]'".to_string()),
        (
            "pactl",
            format!("[ \"$1\" = get-default-source ] && echo usb || echo '{pulse_sources}'"),
        ),
    ] {
        let mut tool = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o755)
            .open(bin.join(name))
            .unwrap();
        writeln!(tool, "#!/bin/sh\n{body}").unwrap();
    }
    bin
}

#[test]
fn a_pipewire_without_audio_inputs_leaves_the_turn_to_pulseaudio() {
    let dir = TempDir::new();
    let bin = stub_tools(
        dir.path(),
        r#"[{"name":"usb","description":"Micro USB","monitor_source":""}]"#,
    );
    let session = AudioSession::isolated(dir.path(), &[("PATH", bin.to_str().unwrap())]);
    assert_eq!(session.available().unwrap(), vec![mic("usb", "Micro USB")]);
    let selection = session.resolve("usb").unwrap();
    assert!(!selection.fallback);
    assert_eq!(selection.server, Server::PulseAudio);
}

#[test]
fn servers_that_answer_without_inputs_are_an_empty_list() {
    let dir = TempDir::new();
    let bin = stub_tools(dir.path(), "[]");
    let session = AudioSession::isolated(dir.path(), &[("PATH", bin.to_str().unwrap())]);
    assert_eq!(session.available(), Ok(Vec::new()));
}

#[test]
fn an_empty_pipewire_next_to_a_failing_pulseaudio_is_unavailable_not_an_empty_list() {
    let dir = TempDir::new();
    let bin = stub_tools(dir.path(), "[]");
    fs::write(bin.join("pactl"), "#!/bin/sh\n[ \"$1\" = --version ]\n").unwrap();
    let session = AudioSession::isolated(dir.path(), &[("PATH", bin.to_str().unwrap())]);
    assert_eq!(session.available(), Err(Unavailable));
}

#[test]
fn an_empty_pipewire_without_pulseaudio_tools_is_an_empty_list() {
    let dir = TempDir::new();
    let bin = stub_tools(dir.path(), "[]");
    fs::remove_file(bin.join("pactl")).unwrap();
    let session = AudioSession::isolated(dir.path(), &[("PATH", bin.to_str().unwrap())]);
    assert_eq!(session.available(), Ok(Vec::new()));
}

#[test]
fn a_hung_tool_is_killed_at_its_deadline() {
    let dir = TempDir::new();
    let session = AudioSession::isolated(dir.path(), &[]);
    let started = Instant::now();
    let end = started + Duration::from_millis(300);
    assert_eq!(session.run("sleep", &["30"], end), None);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        session
            .run("echo", &["é"], Instant::now() + Duration::from_secs(5))
            .as_deref(),
        Some("é\n")
    );
}

// Headless servers. Each scenario starts a private daemon in a short /tmp directory,
// feeds the fixture through a FIFO source and records it with the production code.

const REQUIRE_AUDIO: &str = "EZERBOT_REQUIRE_AUDIO";
const STARTUP: Duration = Duration::from_secs(25);
/// PulseAudio 17 rejects non-ASCII module arguments ("Invalid non-ASCII character"), so only
/// the PipeWire fixture is accented; the Pulse parser is covered on an accented sample.
const NAMES: [(&str, &str); 2] = [
    ("Micro systeme", "Micro fixture"),
    ("Micro système", "Micro fixture é"),
];
/// Largest sample difference accepted between the fixture and the capture. Both servers are
/// bit-exact: the FIFO is fed in real time, and the PipeWire tunnel drives its own clock so nothing
/// is resampled (as a follower it was off by up to a sample and glitched in one run in twenty).
const TOLERANCE: i32 = 0;
/// The tunnel buffers little: stay slightly ahead of the clock, never far ahead. Small chunks keep
/// its rate matching steady (20 ms chunks glitched the capture in about one run in fourteen).
const LEAD: Duration = Duration::from_millis(100);
const CHUNK: usize = 160;
/// Silence between two copies of the fixture, in bytes.
const GAP: usize = BYTES_PER_SECOND / 10;
const BYTES_PER_SECOND: usize = SAMPLE_RATE as usize * 2;

fn fixture_pcm() -> &'static [u8] {
    &include_bytes!("../../tests/fixtures/voice.wav")[44..]
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        // Not $TMPDIR: a long path overflows sun_path.
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let path = PathBuf::from(format!(
            "/tmp/ezmic-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        TempDir(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Process(Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Headless {
    server: Server,
    /// Descriptions of the default source and of the fixture source.
    names: (&'static str, &'static str),
    session: AudioSession,
    fifo: PathBuf,
    /// Dropped before `dir`, which they run in.
    _processes: Vec<Process>,
    dir: TempDir,
}

fn present(binaries: &[&str]) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for binary in binaries {
        if !std::env::split_paths(&path).any(|dir| dir.join(binary).is_file()) {
            assert!(
                std::env::var(REQUIRE_AUDIO).as_deref() != Ok("1"),
                "audio-headless: {binary} absent et {REQUIRE_AUDIO}=1"
            );
            println!("audio-headless: {binary} absent, test non exécuté");
            return false;
        }
    }
    true
}

fn wait_until(limit: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    while !ready() {
        if Instant::now() >= end {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
    true
}

fn daemon(program: &str, args: &[&str], dir: &Path, env: &[(&str, &str)]) -> Process {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir)
        .env("XDG_RUNTIME_DIR", dir)
        .envs(env.iter().copied())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Process(command.spawn().unwrap())
}

impl Headless {
    fn pulse() -> Option<Headless> {
        if !present(&["pulseaudio", "pactl", "parec"]) {
            return None;
        }
        let (system_name, fixture_name) = NAMES[0];
        let dir = TempDir::new();
        let path = dir.path().display().to_string();
        let socket = format!("unix:{path}/native");
        let processes = vec![daemon(
            "pulseaudio",
            &[
                "-n",
                "--daemonize=no",
                "--exit-idle-time=-1",
                "--disable-shm=yes",
                "-L",
                &format!("module-native-protocol-unix socket={path}/native auth-anonymous=1"),
                "-L",
                &format!("module-null-source source_name=systeme description=\"{system_name}\""),
                "-L",
                "module-null-sink sink_name=sortie",
            ],
            dir.path(),
            &[],
        )];
        let session = AudioSession::isolated(dir.path(), &[("PULSE_SERVER", &socket)]);
        let up = || {
            session
                .run("pactl", &["info"], Instant::now() + Duration::from_secs(1))
                .is_some()
        };
        assert!(wait_until(STARTUP, up), "pulseaudio did not start");
        let fifo = dir.path().join("mic.fifo");
        let end = Instant::now() + Duration::from_secs(5);
        let module = session
            .run(
                "pactl",
                &[
                    "load-module",
                    "module-pipe-source",
                    "source_name=fixture",
                    &format!("file={}", fifo.display()),
                    "format=s16le",
                    "rate=16000",
                    "channels=1",
                    &format!("source_properties='device.description=\"{fixture_name}\"'"),
                ],
                end,
            )
            .expect("module-pipe-source");
        fs::write(dir.path().join("module"), module.trim()).unwrap();
        session
            .run("pactl", &["set-default-source", "systeme"], end)
            .unwrap();
        Some(Headless {
            server: Server::PulseAudio,
            names: NAMES[0],
            session,
            fifo,
            _processes: processes,
            dir,
        })
    }

    fn pipewire() -> Option<Headless> {
        if !present(&["pipewire", "wireplumber", "pw-dump", "pw-record", "pw-cli"]) {
            return None;
        }
        let (system_name, fixture_name) = NAMES[1];
        let dir = TempDir::new();
        let path = dir.path().display().to_string();
        let config = dir.path().join("cfg/pipewire/pipewire.conf.d");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("fixture.conf"),
            format!(
                r#"context.properties = {{ default.clock.rate = 16000 default.clock.allowed-rates = [ 16000 ] }}
context.objects = [ {{ factory = adapter args = {{ factory.name = support.null-audio-sink node.name = systeme node.description = "{system_name}" media.class = Audio/Source/Virtual audio.position = [ MONO ] priority.session = 2000 }} }} ]
context.modules = [ {{ name = libpipewire-module-pipe-tunnel args = {{ tunnel.mode = source pipe.filename = "{path}/mic.fifo" audio.format = S16LE audio.rate = 16000 audio.channels = 1 audio.position = [ MONO ] stream.props = {{ node.driver = true node.name = fixture node.description = "{fixture_name}" media.class = Audio/Source }} }} }} ]
"#
            ),
        )
        .unwrap();
        let session_config = dir.path().join("cfg/wireplumber/wireplumber.conf.d");
        fs::create_dir_all(&session_config).unwrap();
        // The private directories do not hide /dev/snd: without this a real microphone joins the list.
        fs::write(
            session_config.join("fixture.conf"),
            "wireplumber.profiles = { main = { hardware.audio = disabled hardware.bluetooth = disabled hardware.video-capture = disabled } }\n",
        )
        .unwrap();
        let env = [
            ("XDG_CONFIG_HOME", format!("{path}/cfg")),
            ("XDG_STATE_HOME", format!("{path}/state")),
        ];
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        // A custom minimal config hung pw-dump: keep the stock one and add a drop-in.
        let processes = vec![
            daemon("pipewire", &[], dir.path(), &env),
            daemon("wireplumber", &[], dir.path(), &env),
        ];
        let session = AudioSession::isolated(dir.path(), &[]);
        let fifo = dir.path().join("mic.fifo");
        let ready = || {
            session.available().is_ok_and(|inputs| {
                inputs.first().is_some_and(|first| first.id == "systeme") && inputs.len() == 2
            })
        };
        assert!(
            wait_until(STARTUP, ready),
            "pipewire did not publish its default source"
        );
        Some(Headless {
            server: Server::PipeWire,
            names: NAMES[1],
            session,
            fifo,
            _processes: processes,
            dir,
        })
    }

    fn remove_fixture(&self) {
        let end = || Instant::now() + Duration::from_secs(5);
        match self.server {
            Server::PulseAudio => {
                let module = fs::read_to_string(self.dir.path().join("module")).unwrap();
                self.session
                    .run("pactl", &["unload-module", &module], end())
                    .unwrap();
            }
            Server::PipeWire => {
                let dump = self.session.run("pw-dump", &[], end()).unwrap();
                let items: Vec<Value> = serde_json::from_str(&dump).unwrap();
                let node = items
                    .iter()
                    .find(|item| {
                        item.pointer("/info/props/node.name")
                            .is_some_and(|name| name == "fixture")
                    })
                    .expect("fixture node");
                let id = node["id"].as_u64().unwrap().to_string();
                self.session
                    .run("pw-cli", &["destroy", &id], end())
                    .unwrap();
            }
        }
        let gone = || {
            self.session
                .available()
                .is_ok_and(|inputs| inputs.iter().all(|mic| mic.id != "fixture"))
        };
        assert!(
            wait_until(Duration::from_secs(10), gone),
            "fixture source still listed"
        );
    }

    /// Writes silence in real time until `go`, then the fixture over and over, a gap apart.
    fn feed(&self, go: Arc<AtomicBool>, quit: Arc<AtomicBool>) -> JoinHandle<()> {
        let fifo = self.fifo.clone();
        thread::spawn(move || {
            let mut file = None;
            wait_until(STARTUP, || {
                // Without a reader yet the non-blocking open fails instead of waiting forever.
                file = OpenOptions::new()
                    .write(true)
                    .custom_flags(0o4000)
                    .open(&fifo)
                    .ok();
                file.is_some()
            });
            let Some(mut file) = file else { return };
            let (start, silence) = (Instant::now(), [0u8; CHUNK]);
            let (mut sent, mut played) = (0usize, None::<usize>);
            while !quit.load(Ordering::Relaxed) {
                let due = start + Duration::from_secs_f64(sent as f64 / BYTES_PER_SECOND as f64);
                thread::sleep(due.saturating_duration_since(Instant::now() + LEAD));
                if go.load(Ordering::Relaxed) && played.is_none() {
                    played = Some(0);
                }
                let mut chunk = &silence[..];
                if let Some(position) = played.as_mut() {
                    let pcm = fixture_pcm();
                    if *position < pcm.len() {
                        chunk = &pcm[*position..*position + CHUNK];
                    }
                    *position = (*position + CHUNK) % (pcm.len() + GAP);
                }
                let mut written = 0;
                while written < chunk.len() {
                    match file.write(&chunk[written..]) {
                        Ok(count) => written += count,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            // A full FIFO means nobody reads it any more: stop with the test.
                            if quit.load(Ordering::Relaxed) {
                                return;
                            }
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => return,
                    }
                }
                sent += chunk.len();
            }
        })
    }
}

/// Reads a capture until `step` is satisfied or `limit` passes; the recorder is then killed by pid.
fn pump(capture: Capture, limit: Duration, mut step: impl FnMut(&[u8]) -> bool) -> Vec<u8> {
    let pid = Pid::from_raw(capture.child.id() as i32);
    let (send, receive) = mpsc::channel();
    let reader = thread::spawn(move || {
        let (mut capture, mut buffer) = (capture, [0u8; 4096]);
        while let Ok(count @ 1..) = capture.read(&mut buffer) {
            if send.send(buffer[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    let end = Instant::now() + limit;
    let mut all = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let Ok(chunk) = receive.recv_timeout(left) else {
            break;
        };
        all.extend(chunk);
        if step(&all) {
            break;
        }
    }
    let _ = kill(pid, Signal::SIGKILL);
    drop(receive);
    reader.join().unwrap();
    all
}

fn samples(pcm: &[u8]) -> Vec<i16> {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair))
        .collect()
}

/// Where a copy of the fixture starts in the capture, every sample within `TOLERANCE` of it.
fn find_fixture(heard: &[i16]) -> Option<usize> {
    let fixture = samples(fixture_pcm());
    let close = |a: i16, b: i16| (i32::from(a) - i32::from(b)).abs() <= TOLERANCE;
    heard
        .windows(fixture.len())
        .position(|window| window.iter().zip(&fixture).all(|(a, b)| close(*a, *b)))
}

fn scenario(headless: Option<Headless>, label: &str) {
    let Some(headless) = headless else { return };
    let session = &headless.session;
    let (system_name, fixture_name) = headless.names;
    // The sink's monitor is not a microphone, and the system default comes first.
    let expected = [mic("systeme", system_name), mic("fixture", fixture_name)];
    assert_eq!(session.available().unwrap(), expected);
    assert_eq!(
        listing(session).unwrap(),
        serde_json::json!({"microphones": expected})
    );

    let system = session.resolve("").unwrap();
    assert_eq!(
        serde_json::to_value(&system).unwrap(),
        serde_json::json!({"id":"systeme","name":system_name,"fallback":false})
    );
    let chosen = session.resolve("fixture").unwrap();
    assert_eq!(
        serde_json::to_value(&chosen).unwrap(),
        serde_json::json!({"id":"fixture","name":fixture_name,"fallback":false})
    );

    let (go, quit) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let feeder = headless.feed(go.clone(), quit.clone());
    let capture = session.capture(&chosen).unwrap();
    // The tunnel can drop what is fed while the stream is still settling: the fixture is fed
    // repeatedly and the capture ends at the first copy that arrives whole.
    let captured = pump(capture, Duration::from_secs(15), |all| {
        go.store(true, Ordering::Relaxed);
        find_fixture(&samples(all)).is_some()
    });
    quit.store(true, Ordering::Relaxed);
    feeder.join().unwrap();
    let start = find_fixture(&samples(&captured)).expect("the fixture never arrived whole");
    println!(
        "audio-headless: {label}: fixture found at sample {start} within {} bytes captured, largest difference {TOLERANCE} allowed",
        captured.len()
    );

    headless.remove_fixture();
    let fallback = session.resolve("fixture").unwrap();
    assert_eq!(
        serde_json::to_value(&fallback).unwrap(),
        serde_json::json!({"id":"systeme","name":system_name,"fallback":true})
    );
    let wanted = BYTES_PER_SECOND / 5;
    let silence = pump(
        session.capture(&fallback).unwrap(),
        Duration::from_secs(10),
        |all| all.len() >= wanted,
    );
    println!(
        "audio-headless: {label}: {} bytes recorded from the system default after the fallback",
        silence.len()
    );
    assert!(silence.len() >= wanted);
    assert!(
        samples(&silence).iter().all(|sample| *sample == 0),
        "the silent system default was not the source recorded"
    );
}

#[test]
fn pulseaudio_headless_lists_selects_captures_and_falls_back() {
    scenario(Headless::pulse(), "pulseaudio");
}

#[test]
fn pipewire_headless_lists_selects_captures_and_falls_back() {
    scenario(Headless::pipewire(), "pipewire");
}
