#![cfg(all(target_os = "linux", debug_assertions))]

use grokbot_test_support::TestDir;
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::HashSet,
    ffi::CString,
    io::{BufRead, BufReader},
    os::raw::{c_int, c_uint},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr,
    sync::OnceLock,
    time::{Duration, Instant},
};
use x11_dl::{
    xlib::{self, Xlib},
    xtest::Xf86vmode as Xtst,
};

const SHELL: &str = env!("CARGO_BIN_EXE_ezerbot-linux");
const MENU_HEIGHT: i32 = 40;
const WAIT: Duration = Duration::from_secs(120);

fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(value) = ready() {
            return value;
        }
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

unsafe extern "C" fn ignore_x_error(_: *mut xlib::Display, _: *mut xlib::XErrorEvent) -> c_int {
    0
}

#[derive(Clone, Copy)]
struct Rectangle {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

struct Screen {
    server: Child,
    name: String,
    xlib: Xlib,
    xtest: Xtst,
    display: *mut xlib::Display,
}

impl Screen {
    fn start() -> Self {
        let mut server = Command::new("Xvfb")
            .args(["-displayfd", "1", "-screen", "0", "1280x900x24"])
            .args(["-nolisten", "tcp"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("Xvfb is required: sudo apt install xvfb");
        let mut number = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut number)
            .unwrap();
        let name = format!(":{}", number.trim());
        let address = CString::new(name.as_str()).unwrap();
        let xlib = Xlib::open().unwrap();
        let xtest = Xtst::open().unwrap();
        let display = unsafe {
            (xlib.XSetErrorHandler)(Some(ignore_x_error));
            (xlib.XOpenDisplay)(address.as_ptr())
        };
        assert!(!display.is_null(), "Xvfb refused the connection");
        Self {
            server,
            name,
            xlib,
            xtest,
            display,
        }
    }

    fn windows(&self) -> Vec<(xlib::Window, Rectangle)> {
        let mut found = Vec::new();
        unsafe {
            let root = (self.xlib.XDefaultRootWindow)(self.display);
            let (mut parent, mut same_root, mut children, mut count) =
                (0, 0, ptr::null_mut(), 0 as c_uint);
            let listed = (self.xlib.XQueryTree)(
                self.display,
                root,
                &mut same_root,
                &mut parent,
                &mut children,
                &mut count,
            );
            if listed == 0 {
                return found;
            }
            for index in 0..count as usize {
                let window = *children.add(index);
                let mut state: xlib::XWindowAttributes = std::mem::zeroed();
                let known = (self.xlib.XGetWindowAttributes)(self.display, window, &mut state) != 0;
                if known && state.map_state == xlib::IsViewable && state.x >= 0 && state.width > 100
                {
                    let area = Rectangle {
                        x: state.x,
                        y: state.y,
                        width: state.width as u32,
                        height: state.height as u32,
                    };
                    found.push((window, area));
                }
            }
            if !children.is_null() {
                (self.xlib.XFree)(children.cast());
            }
        }
        found
    }

    fn app_window(&self) -> (xlib::Window, Rectangle) {
        wait_for("the native window never appeared", || {
            self.windows()
                .into_iter()
                .max_by_key(|(_, area)| area.width)
        })
    }

    fn pixels(&self, area: Rectangle) -> Vec<u32> {
        unsafe {
            let image = (self.xlib.XGetImage)(
                self.display,
                (self.xlib.XDefaultRootWindow)(self.display),
                area.x,
                area.y,
                area.width,
                area.height,
                !0,
                xlib::ZPixmap,
            );
            assert!(!image.is_null());
            let (stride, rows) = ((*image).bytes_per_line as usize, area.height as usize);
            let bytes = std::slice::from_raw_parts((*image).data as *const u8, stride * rows);
            let pixels = bytes
                .chunks_exact(stride)
                .flat_map(|row| row[..area.width as usize * 4].as_chunks::<4>().0)
                .map(|pixel| u32::from_le_bytes(*pixel) & 0xff_ff_ff)
                .collect();
            ((*image).funcs.destroy_image.unwrap())(image);
            pixels
        }
    }

    fn web_area(&self) -> Rectangle {
        let (_, window) = self.app_window();
        Rectangle {
            y: window.y + MENU_HEIGHT,
            height: window.height - MENU_HEIGHT as u32,
            ..window
        }
    }

    fn colours(&self) -> HashSet<u32> {
        self.pixels(self.web_area()).into_iter().collect()
    }

    fn focus(&self) {
        let (window, _) = self.app_window();
        unsafe {
            (self.xlib.XSetInputFocus)(
                self.display,
                window,
                xlib::RevertToParent,
                xlib::CurrentTime,
            );
            (self.xlib.XSync)(self.display, xlib::False);
        }
    }

    fn click(&self, x: i32, y: i32) {
        unsafe {
            (self.xtest.XTestFakeMotionEvent)(self.display, -1, x, y, 0);
            (self.xtest.XTestFakeButtonEvent)(self.display, 1, xlib::True, 50);
            (self.xtest.XTestFakeButtonEvent)(self.display, 1, xlib::False, 50);
            (self.xlib.XSync)(self.display, xlib::False);
        }
    }

    fn keycode(&self, name: &str) -> c_uint {
        let name = CString::new(name).unwrap();
        unsafe {
            let symbol = (self.xlib.XStringToKeysym)(name.as_ptr());
            c_uint::from((self.xlib.XKeysymToKeycode)(self.display, symbol))
        }
    }

    fn press(&self, keys: &[&str]) {
        let codes: Vec<_> = keys.iter().map(|key| self.keycode(key)).collect();
        unsafe {
            for code in &codes {
                (self.xtest.XTestFakeKeyEvent)(self.display, *code, xlib::True, 15);
            }
            for code in codes.iter().rev() {
                (self.xtest.XTestFakeKeyEvent)(self.display, *code, xlib::False, 15);
            }
            (self.xlib.XSync)(self.display, xlib::False);
        }
    }

    fn type_text(&self, text: &str) {
        for letter in text.chars() {
            match letter {
                ' ' => self.press(&["space"]),
                letter => self.press(&[letter.to_string().as_str()]),
            }
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        unsafe { (self.xlib.XCloseDisplay)(self.display) };
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

struct App(Option<Child>);

impl App {
    fn start(binary: &Path, screen: &Screen, home: &TestDir, extra: &[(&str, &str)]) -> Self {
        let shell = Command::new(binary)
            .arg("--data-dir")
            .arg(home.join("state"))
            .env("DISPLAY", &screen.name)
            .env("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_DATA_HOME", home.join("data"))
            .envs(extra.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self(Some(shell))
    }

    fn quit(mut self) {
        let mut shell = self.0.take().unwrap();
        kill(Pid::from_raw(shell.id() as i32), Signal::SIGTERM).unwrap();
        assert!(shell.wait().unwrap().success());
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(mut shell) = self.0.take() {
            let _ = kill(Pid::from_raw(shell.id() as i32), Signal::SIGTERM);
            let _ = shell.wait();
        }
    }
}

/// The shell finds the wrapper beside its own executable.
fn shell_with_wrapper() -> &'static Path {
    static SHELL_PATH: OnceLock<PathBuf> = OnceLock::new();
    SHELL_PATH.get_or_init(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "--locked", "-p", "grokbot-wrapper"])
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .status()
            .unwrap();
        assert!(status.success());
        let shell = PathBuf::from(SHELL);
        assert!(shell.with_file_name("grokbot-wrapper").exists());
        shell
    })
}

fn exchange_persisted(database: &Path, text: &str) -> bool {
    let read = || -> rusqlite::Result<bool> {
        let database = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let sent: i64 = database.query_row(
            "SELECT count(*) FROM submissions WHERE body LIKE ?1",
            [format!("%{text}%")],
            |row| row.get(0),
        )?;
        let answered: i64 = database.query_row(
            "SELECT count(*) FROM engine_run_events WHERE payload LIKE '%OK%'",
            [],
            |row| row.get(0),
        )?;
        Ok(sent > 0 && answered > 0)
    };
    read().unwrap_or(false)
}

#[test]
fn the_real_window_shows_the_app_and_completes_an_exchange() {
    let home = TestDir::new("ezerbot-window");
    let screen = Screen::start();
    let app = App::start(
        shell_with_wrapper(),
        &screen,
        &home,
        &[("EZERBOT_FAKE_SIGNIN", "1")],
    );
    wait_for("the window stayed blank", || {
        (screen.colours().len() >= 16).then_some(())
    });
    let area = screen.web_area();
    let before = screen.pixels(area);

    let text = "bonjour depuis xvfb";
    screen.focus();
    screen.click(area.x + area.width as i32 / 2, area.y + 200);
    screen.press(&["Control_L", "n"]);
    std::thread::sleep(Duration::from_millis(500));
    screen.type_text(text);
    screen.press(&["Return"]);
    wait_for("the message and the OK reply were not persisted", || {
        exchange_persisted(&home.join("state/ezerbot.sqlite"), text).then_some(())
    });
    wait_for("the exchange never reached the screen", || {
        (screen.pixels(area) != before).then_some(())
    });
    app.quit();
}

const LINKS_PAGE: &str = r##"<!doctype html><body style="margin:0">
<script>window.open("https://example.test/sans-geste")</script>
<a href="file:///etc/passwd" target="_blank" style="position:fixed;left:0;width:25%;height:100%;background:#f00"></a>
<a href="/meme-origine" target="_blank" style="position:fixed;left:25%;width:25%;height:100%;background:#0f0"></a>
<a href="https://example.test/autoriser?etat=1" target="_blank" rel="noopener noreferrer" style="position:fixed;left:50%;width:25%;height:100%;background:#00f"></a>
<a href="https://example.test/echec" target="_blank" style="position:fixed;left:75%;width:25%;height:100%;background:#ff0"></a>"##;

const PAGE_WRAPPER: &str = r#"#!/usr/bin/python3
import http.server, json, os, sys
sys.stdin.buffer.read()
page = open(os.path.dirname(__file__) + "/../page.html", "rb").read()
class Page(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(page)))
        self.end_headers()
        self.wfile.write(page)
    def log_message(self, *arguments):
        pass
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page)
print(json.dumps({"url": f"http://127.0.0.1:{server.server_port}"}), flush=True)
server.serve_forever()
"#;

const FAKE_BROWSER: &str = r#"#!/bin/sh
printf '%s\n' "$1" >> "$(dirname "$0")/../opened"
case "$1" in *echec) exit 3 ;; esac
"#;

/// `cp` keeps the write descriptor out of this process: a concurrent test's
/// fork would inherit it, and exec would then fail with ETXTBSY.
fn install(home: &TestDir, name: &str, source: &Path) {
    let status = Command::new("cp")
        .arg(source)
        .arg(home.join("bin").join(name))
        .status()
        .unwrap();
    assert!(status.success());
}

fn install_script(home: &TestDir, name: &str, body: &str) {
    let source = home.join(format!("{name}.source"));
    std::fs::write(&source, body).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o700)).unwrap();
    install(home, name, &source);
}

#[test]
fn new_window_requests_open_only_web_pages_in_the_default_browser() {
    let home = TestDir::new("ezerbot-links");
    std::fs::create_dir(home.join("bin")).unwrap();
    std::fs::write(home.join("page.html"), LINKS_PAGE).unwrap();
    install(&home, "ezerbot-linux", Path::new(SHELL));
    install_script(&home, "grokbot-wrapper", PAGE_WRAPPER);
    install_script(&home, "xdg-open", FAKE_BROWSER);
    let screen = Screen::start();
    let path = format!(
        "{}:{}",
        home.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let app = App::start(
        &home.join("bin/ezerbot-linux"),
        &screen,
        &home,
        &[("PATH", path.as_str())],
    );
    wait_for("the links page never appeared", || {
        let colours = screen.colours();
        [0xff_00_00, 0x00_ff_00, 0x00_00_ff, 0xff_ff_00]
            .iter()
            .all(|colour| colours.contains(colour))
            .then_some(())
    });
    assert!(
        screen.colours().len() < 16,
        "the blank check must not accept a page without text"
    );

    let opened = || std::fs::read_to_string(home.join("opened")).unwrap_or_default();
    let area = screen.web_area();
    let band = |index: i32| area.x + (2 * index + 1) * area.width as i32 / 8;
    let middle = area.y + area.height as i32 / 2;
    screen.focus();
    for index in [0, 1, 2] {
        screen.click(band(index), middle);
    }
    wait_for("the OAuth link never reached the browser", || {
        (!opened().is_empty()).then_some(())
    });
    assert_eq!(opened(), "https://example.test/autoriser?etat=1\n");

    assert_eq!(screen.windows().len(), 1);
    screen.click(band(3), middle);
    wait_for("a failed browser launch stayed silent", || {
        (screen.windows().len() == 2).then_some(())
    });
    app.quit();
}
