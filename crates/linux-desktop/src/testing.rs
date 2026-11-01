//! Fixtures shared by the tests of this crate, `grokbot-forms` and `grokbot-wrapper`. Every
//! fixture lives on a throwaway bus inside a throwaway home: nothing here may reach the
//! session bus or a real keyring.
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use grokbot_test_support::TestDir;
use zbus::{
    blocking::{connection::Builder, Connection},
    message::Header,
    zvariant::OwnedValue,
};

use crate::Bus;

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";

fn package(tool: &str) -> &'static str {
    match tool {
        "dbus-daemon" => "dbus-daemon",
        "gnome-keyring-daemon" => "gnome-keyring",
        _ => "xdg-utils",
    }
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(tool).is_file()))
}

/// Fails the test when a tool is missing, on CI too: a test that returns early proves nothing.
pub fn tools(names: &[&str]) {
    let test = std::thread::current().name().unwrap_or("test").to_owned();
    for name in names {
        assert!(
            on_path(name),
            "{test}: {name} is absent, install the package {}",
            package(name)
        );
    }
}

pub fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A HOME that the application accepts as a test home, with every XDG directory inside it so
/// that no desktop file, MIME default or keyring of the machine is read or written. Keep the
/// label short: the keyring control socket path is limited to about 100 bytes.
pub struct Home {
    dir: TestDir,
}

impl Home {
    pub fn new(label: &str) -> Self {
        let dir = TestDir::new(label);
        for sub in ["data", "config", "cache", "runtime", "share", "etc"] {
            std::fs::create_dir(dir.join(sub)).unwrap();
        }
        std::fs::set_permissions(dir.join("runtime"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        std::fs::write(dir.join(".ezerbot-test-home"), "test").unwrap();
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The isolated environment, without any desktop session that `xdg-open` could pick. Its
    /// generic path only looks for scheme handlers when a display is set, so one is named that
    /// no program could connect to. `BROWSER` is `false` so that a target without a handler fails
    /// instead of starting a real browser.
    pub fn env<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        for name in [
            "DBUS_SESSION_BUS_ADDRESS",
            "GROKBOT_TEST_SESSION_BUS",
            "XDG_CURRENT_DESKTOP",
            "DESKTOP_SESSION",
            "GNOME_DESKTOP_SESSION_ID",
            "KDE_FULL_SESSION",
            "WAYLAND_DISPLAY",
        ] {
            command.env_remove(name);
        }
        command
            .env("DISPLAY", ":ezerbot-none")
            .env("BROWSER", "false")
            .env("HOME", self.path())
            .env("XDG_DATA_HOME", self.dir.join("data"))
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("XDG_CACHE_HOME", self.dir.join("cache"))
            .env("XDG_RUNTIME_DIR", self.dir.join("runtime"))
            .env("XDG_DATA_DIRS", self.dir.join("share"))
            .env("XDG_CONFIG_DIRS", self.dir.join("etc"))
    }
}

/// A `dbus-daemon` that serves nothing by itself: no service directory, so no activation.
pub struct PrivateBus {
    address: String,
    _daemon: Process,
}

impl PrivateBus {
    pub fn new(home: &Home) -> Self {
        let address = format!("unix:path={}", home.path().join("bus").display());
        let daemon = Self::start(home, &address);
        Self {
            address,
            _daemon: daemon,
        }
    }

    /// Every connection dies with the daemon; the address stays the same.
    pub fn restart(&mut self, home: &Home) {
        let _ = self._daemon.0.kill();
        let _ = self._daemon.0.wait();
        self._daemon = Self::start(home, &self.address);
    }

    fn start(home: &Home, address: &str) -> Process {
        let socket = home.path().join("bus");
        let config = home.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>session</type><listen>{address}</listen>\
                 <policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>\
                 <allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>"
            ),
        )
        .unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .args(["--nofork", "--config-file"])
            .arg(&config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for("the private bus", || {
            assert!(daemon.try_wait().unwrap().is_none(), "dbus-daemon exited");
            std::os::unix::net::UnixStream::connect(&socket).is_ok()
        });
        Process(daemon)
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn bus(&self) -> Bus {
        Bus::private(&self.address)
    }

    pub fn connection(&self) -> Connection {
        self.bus().connect().unwrap()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<String>,
    pub desktop_entry: Option<String>,
    pub expire_timeout: i32,
}

#[derive(Default)]
struct Sent {
    calls: Mutex<Vec<Notification>>,
    clients: Mutex<Vec<String>>,
    closed: Mutex<Vec<u32>>,
    refuse_next: AtomicBool,
    slow_close_ms: std::sync::atomic::AtomicU64,
}

struct Server(Arc<Sent>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        #[zbus(header)] header: Header<'_>,
        app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        if self.0.refuse_next.swap(false, Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed("refused".into()));
        }
        let client = header.sender().expect("a bus names the sender").to_string();
        self.0.clients.lock().unwrap().push(client);
        let mut calls = self.0.calls.lock().unwrap();
        calls.push(Notification {
            app_name,
            summary,
            body,
            actions,
            desktop_entry: hints
                .get("desktop-entry")
                .and_then(|value| String::try_from(value.try_clone().ok()?).ok()),
            expire_timeout,
        });
        Ok(calls.len() as u32)
    }

    fn close_notification(&self, id: u32) {
        self.0.closed.lock().unwrap().push(id);
        std::thread::sleep(Duration::from_millis(
            self.0.slow_close_ms.load(Ordering::SeqCst),
        ));
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("fake".into(), "ezerbot".into(), "1".into(), "1.2".into())
    }
}
pub struct FakeNotifications {
    connection: Connection,
    sent: Arc<Sent>,
}

impl FakeNotifications {
    pub fn new(bus: &PrivateBus) -> Self {
        let sent = Arc::default();
        let connection = Builder::address(bus.address())
            .unwrap()
            .name(NOTIFICATIONS)
            .unwrap()
            .serve_at(NOTIFICATIONS_PATH, Server(Arc::clone(&sent)))
            .unwrap()
            .build()
            .unwrap();
        Self { connection, sent }
    }

    pub fn calls(&self) -> Vec<Notification> {
        self.sent.calls.lock().unwrap().clone()
    }

    /// The next `Notify` fails, as when the server is busy: it stays up and keeps its ids.
    pub fn refuse_next(&self) {
        self.sent.refuse_next.store(true, Ordering::SeqCst);
    }

    /// Each `CloseNotification` then takes this long to answer, as a busy server does.
    pub fn slow_close(&self, delay: Duration) {
        self.sent
            .slow_close_ms
            .store(delay.as_millis() as u64, Ordering::SeqCst);
    }

    pub fn closed(&self) -> Vec<u32> {
        self.sent.closed.lock().unwrap().clone()
    }

    /// Like GNOME Shell and Dunst: only the connection that sent the notification hears of it.
    pub fn click(&self, id: u32) {
        let client = self.sent.clients.lock().unwrap()[id as usize - 1].clone();
        self.emit(Some(&client), id, "default");
    }

    /// Like the servers that broadcast: every connection hears of it.
    pub fn invoke(&self, id: u32, action: &str) {
        self.emit(None, id, action);
    }

    fn emit(&self, destination: Option<&str>, id: u32, action: &str) {
        self.connection
            .emit_signal(
                destination,
                NOTIFICATIONS_PATH,
                NOTIFICATIONS,
                "ActionInvoked",
                &(id, action),
            )
            .unwrap();
    }
}

impl Drop for FakeNotifications {
    fn drop(&mut self) {
        let _ = self.connection.clone().close();
    }
}
pub fn recording_handler(directory: &Path, log: &Path) -> PathBuf {
    let script = directory.join("handler.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}
