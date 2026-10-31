//! Fixtures shared by the tests of this crate, `grokbot-forms` and `grokbot-wrapper`. Every
//! fixture lives on a throwaway bus inside a throwaway home: nothing here may reach the
//! session bus or a real keyring.
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use grokbot_test_support::TestDir;
use zbus::{
    blocking::{connection::Builder, Connection, Proxy},
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue},
};

use crate::Bus;

const KEYRING_PASSWORD: &str = "ezerbot-throwaway-test-991";
const SECRETS: &str = "org.freedesktop.secrets";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";

fn package(tool: &str) -> &'static str {
    match tool {
        "dbus-daemon" => "dbus-daemon",
        "gnome-keyring-daemon" => "gnome-keyring",
        "update-desktop-database" => "desktop-file-utils",
        _ => "xdg-utils",
    }
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(tool).is_file()))
}

/// Whether every tool is installed. A missing one fails the test, except on CI where the
/// packages are not installed yet: there the test says so and skips.
pub fn tools(names: &[&str]) -> bool {
    let Some(missing) = names.iter().find(|name| !on_path(name)) else {
        return true;
    };
    let test = std::thread::current().name().unwrap_or("test").to_owned();
    assert!(
        std::env::var_os("CI").is_some(),
        "{test}: {missing} is absent, install the package {}",
        package(missing)
    );
    eprintln!("SKIP {test}: {missing} absent");
    false
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
        let socket = home.path().join("bus");
        let address = format!("unix:path={}", socket.display());
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
        Self {
            address,
            _daemon: Process(daemon),
        }
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

/// A real `gnome-keyring-daemon` with an unlocked `login` collection, on its own bus.
pub struct Keyring {
    bus: PrivateBus,
    home: Home,
    daemon: Option<Process>,
}

impl Keyring {
    pub fn new() -> Option<Self> {
        if !tools(&["dbus-daemon", "gnome-keyring-daemon"]) {
            return None;
        }
        let home = Home::new("ezk");
        let bus = PrivateBus::new(&home);
        let mut keyring = Self {
            bus,
            home,
            daemon: None,
        };
        keyring.unlock();
        Some(keyring)
    }

    pub fn bus(&self) -> Bus {
        self.bus.bus()
    }

    pub fn address(&self) -> &str {
        self.bus.address()
    }

    pub fn home(&self) -> &Home {
        &self.home
    }

    fn service(&self) -> Proxy<'static> {
        Proxy::new(
            &self.bus.connection(),
            SECRETS,
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
        )
        .unwrap()
    }

    fn collection(&self) -> Option<OwnedObjectPath> {
        let path: OwnedObjectPath = self.service().call("ReadAlias", &"default").ok()?;
        (path.as_str() != "/").then_some(path)
    }

    fn locked(&self) -> Option<bool> {
        let path = self.collection()?;
        Proxy::new(
            &self.bus.connection(),
            SECRETS,
            path,
            "org.freedesktop.Secret.Collection",
        )
        .ok()?
        .get_property("Locked")
        .ok()
    }
    pub fn lock(&self) {
        let path = self.collection().unwrap();
        let _: (Vec<OwnedObjectPath>, OwnedObjectPath) = self
            .service()
            .call("Lock", &(vec![ObjectPath::from(&path)],))
            .unwrap();
        assert_eq!(self.locked(), Some(true));
    }

    /// Restarts the daemon with the password: the `login` keyring file persists.
    pub fn unlock(&mut self) {
        self.stop();
        let mut daemon = Command::new("gnome-keyring-daemon");
        self.home
            .env(&mut daemon)
            .env("DBUS_SESSION_BUS_ADDRESS", self.bus.address())
            .args(["--foreground", "--unlock", "--components=secrets"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut daemon = daemon.spawn().unwrap();
        {
            use std::io::Write;
            let mut input = daemon.stdin.take().unwrap();
            input.write_all(KEYRING_PASSWORD.as_bytes()).unwrap();
        }
        self.daemon = Some(Process(daemon));
        wait_for("the unlocked login collection", || {
            self.locked() == Some(false)
        });
    }
    pub fn stop(&mut self) {
        self.daemon = None;
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

struct Server(Arc<Mutex<Vec<Notification>>>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        _replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let mut calls = self.0.lock().unwrap();
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
        calls.len() as u32
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
    calls: Arc<Mutex<Vec<Notification>>>,
}

impl FakeNotifications {
    pub fn new(bus: &PrivateBus) -> Self {
        let calls = Arc::default();
        let connection = Builder::address(bus.address())
            .unwrap()
            .name(NOTIFICATIONS)
            .unwrap()
            .serve_at(NOTIFICATIONS_PATH, Server(Arc::clone(&calls)))
            .unwrap()
            .build()
            .unwrap();
        Self { connection, calls }
    }

    pub fn calls(&self) -> Vec<Notification> {
        self.calls.lock().unwrap().clone()
    }
    pub fn click(&self, id: u32) {
        self.invoke(id, "default");
    }

    pub fn invoke(&self, id: u32, action: &str) {
        self.connection
            .emit_signal(
                None::<&str>,
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
