//! Fixtures shared by the tests of this crate, `grokbot-forms` and `grokbot-wrapper`. Every
//! fixture lives on a throwaway bus inside a throwaway home: nothing here may reach the
//! session bus or a real keyring.
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use grokbot_test_support::TestDir;
use zbus::blocking::Connection;

use crate::Bus;

fn package(tool: &str) -> &'static str {
    match tool {
        "dbus-daemon" => "dbus-daemon",
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

/// An executable script that appends its arguments to `log`, to stand in for a handler.
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
