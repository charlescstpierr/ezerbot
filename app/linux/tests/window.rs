#![cfg(all(target_os = "linux", debug_assertions))]

use grokbot_test_support::TestDir;
use gtk::{
    gio::{self, prelude::*},
    glib::{self, Variant},
};
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::{HashMap, HashSet},
    ffi::CString,
    io::{BufRead, BufReader},
    os::raw::{c_int, c_uint},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    ptr,
    sync::OnceLock,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use x11_dl::{
    xlib::{self, Xlib},
    xtest::Xf86vmode as Xtst,
};

const SHELL: &str = env!("CARGO_BIN_EXE_ezerbot-linux");
const MENU_HEIGHT: i32 = 40;
const WAIT: Duration = Duration::from_secs(120);
const RELAUNCH_LIMIT: Duration = Duration::from_secs(5);

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

fn brightness(pixel: u32) -> u32 {
    let [blue, green, red, _] = pixel.to_le_bytes();
    (u32::from(red) + u32::from(green) + u32::from(blue)) / 3
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
    manager: Option<Child>,
    name: String,
    xlib: Xlib,
    xtest: Xtst,
    display: *mut xlib::Display,
}

/// What a window manager lists for a window it manages.
struct Client {
    window: xlib::Window,
    name: String,
    above: bool,
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
            manager: None,
            name,
            xlib,
            xtest,
            display,
        }
    }

    fn start_with_openbox(home: &TestDir) -> Self {
        let mut screen = Self::start();
        let manager = Command::new("openbox")
            .env("DISPLAY", &screen.name)
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.join("config"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("openbox is required: sudo apt install openbox");
        screen.manager = Some(manager);
        // Its first seconds drop a window mapped meanwhile: map a probe until it is managed.
        let probe = screen.cover(Rectangle {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
        wait_for("openbox never managed a window", || {
            unsafe { (screen.xlib.XMapWindow)(screen.display, probe) };
            std::thread::sleep(Duration::from_millis(300));
            screen.stacking(probe).map(|_| ())
        });
        unsafe { (screen.xlib.XDestroyWindow)(screen.display, probe) };
        screen
    }

    fn atom(&self, name: &str) -> xlib::Atom {
        let name = CString::new(name).unwrap();
        unsafe { (self.xlib.XInternAtom)(self.display, name.as_ptr(), xlib::False) }
    }

    fn property(&self, window: xlib::Window, name: &str) -> (c_int, Vec<u8>) {
        unsafe {
            let (mut kind, mut format, mut items, mut after) = (0, 0, 0, 0);
            let mut data = ptr::null_mut();
            let status = (self.xlib.XGetWindowProperty)(
                self.display,
                window,
                self.atom(name),
                0,
                4096,
                xlib::False,
                xlib::AnyPropertyType as std::os::raw::c_ulong,
                &mut kind,
                &mut format,
                &mut items,
                &mut after,
                &mut data,
            );
            if status != 0 || data.is_null() {
                return (0, Vec::new());
            }
            let width = match format {
                8 => 1,
                16 => 2,
                _ => std::mem::size_of::<std::os::raw::c_long>(),
            };
            let bytes = std::slice::from_raw_parts(data, items as usize * width).to_vec();
            (self.xlib.XFree)(data.cast());
            (format, bytes)
        }
    }

    fn longs(&self, window: xlib::Window, name: &str) -> Vec<std::os::raw::c_ulong> {
        let (format, bytes) = self.property(window, name);
        if format != 32 {
            return Vec::new();
        }
        bytes
            .as_chunks::<{ std::mem::size_of::<std::os::raw::c_ulong>() }>()
            .0
            .iter()
            .map(|chunk| std::os::raw::c_ulong::from_ne_bytes(*chunk))
            .collect()
    }

    /// The managed windows, lowest first.
    fn clients(&self) -> Vec<Client> {
        let root = unsafe { (self.xlib.XDefaultRootWindow)(self.display) };
        let above = self.atom("_NET_WM_STATE_ABOVE");
        self.longs(root, "_NET_CLIENT_LIST_STACKING")
            .into_iter()
            .map(|window| Client {
                window,
                name: String::from_utf8_lossy(&self.property(window, "_NET_WM_NAME").1)
                    .into_owned(),
                above: self.longs(window, "_NET_WM_STATE").contains(&above),
            })
            .collect()
    }

    fn client(&self, name: &str) -> Option<Client> {
        self.clients()
            .into_iter()
            .find(|client| client.name == name)
    }

    fn stacking(&self, window: xlib::Window) -> Option<usize> {
        self.clients()
            .iter()
            .position(|client| client.window == window)
    }

    /// Takes a key for this client alone, as another application would.
    fn grab(&self, modifiers: c_uint, key: &str) {
        unsafe {
            (self.xlib.XGrabKey)(
                self.display,
                self.keycode(key) as c_int,
                modifiers,
                (self.xlib.XDefaultRootWindow)(self.display),
                xlib::False,
                xlib::GrabModeAsync,
                xlib::GrabModeAsync,
            );
            (self.xlib.XSync)(self.display, xlib::False);
        }
    }

    fn grabbed_presses(&self) -> usize {
        let mut presses = 0;
        unsafe {
            while (self.xlib.XPending)(self.display) > 0 {
                let mut event: xlib::XEvent = std::mem::zeroed();
                (self.xlib.XNextEvent)(self.display, &mut event);
                presses += usize::from(event.get_type() == xlib::KeyPress);
            }
        }
        presses
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

    /// Stands in for whatever the user opened over the app. Narrower than the
    /// app, so `app_window` still finds the app.
    fn cover(&self, area: Rectangle) -> xlib::Window {
        unsafe {
            let root = (self.xlib.XDefaultRootWindow)(self.display);
            let window = (self.xlib.XCreateSimpleWindow)(
                self.display,
                root,
                area.x,
                area.y,
                200,
                200,
                0,
                0,
                0,
            );
            (self.xlib.XMapRaised)(self.display, window);
            (self.xlib.XSync)(self.display, xlib::False);
            window
        }
    }

    fn top_window(&self) -> Option<xlib::Window> {
        self.windows().last().map(|(window, _)| *window)
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

    /// The sender's bubble sits right of this column: only a reply fills its
    /// author, text and action lines.
    fn text_lines(&self, area: Rectangle) -> usize {
        let pixels = self.pixels(area);
        let inked: Vec<bool> = pixels
            .chunks_exact(area.width as usize)
            .map(|row| row.iter().any(|pixel| brightness(*pixel) > 100))
            .collect();
        let starts = inked.windows(2).filter(|pair| !pair[0] && pair[1]).count();
        starts + usize::from(inked.first() == Some(&true))
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

    fn motion(&self, x: i32, y: i32) {
        unsafe {
            (self.xtest.XTestFakeMotionEvent)(self.display, -1, x, y, 0);
            (self.xlib.XSync)(self.display, xlib::False);
        }
    }

    fn button(&self, pressed: bool) {
        let state = if pressed { xlib::True } else { xlib::False };
        unsafe {
            (self.xtest.XTestFakeButtonEvent)(self.display, 1, state, 50);
            (self.xlib.XSync)(self.display, xlib::False);
        }
    }

    fn click(&self, x: i32, y: i32) {
        self.motion(x, y);
        self.button(true);
        self.button(false);
    }

    /// Holds the first button down at `from` and releases it at `to`, in steps
    /// the window manager follows.
    fn drag(&self, from: (i32, i32), to: (i32, i32)) {
        self.motion(from.0, from.1);
        self.button(true);
        // The page hands the drag to the window manager once it has the press.
        std::thread::sleep(Duration::from_millis(500));
        for step in 1..=10 {
            let at = |start: i32, end: i32| start + (end - start) * step / 10;
            self.motion(at(from.0, to.0), at(from.1, to.1));
            std::thread::sleep(Duration::from_millis(40));
        }
        self.button(false);
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
        if let Some(manager) = &mut self.manager {
            let _ = manager.kill();
            let _ = manager.wait();
        }
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
            .env("GDK_BACKEND", "x11")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("XDG_SESSION_TYPE")
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

    fn exited(&mut self) -> Option<ExitStatus> {
        self.0.as_mut().unwrap().try_wait().unwrap()
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
    let reply_column = Rectangle {
        x: area.x + 355,
        y: area.y + 60,
        width: 300,
        height: area.height - 230,
    };
    wait_for("the reply never reached the screen", || {
        let after = screen.pixels(area);
        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        (changed * 20 > before.len() && screen.text_lines(reply_column) >= 3).then_some(())
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

const SIGN_IN_BUTTON: (i32, i32) = (998, 424);

/// Fails once, as `xdg-open` does on a desktop without a default browser.
const BROWSER_FAILING_ONCE: &str = r#"#!/bin/sh
opened="$(dirname "$0")/../opened"
[ -e "$opened" ]
status=$?
printf '%s\n' "$1" >> "$opened"
exit "$status"
"#;

#[test]
fn the_real_sign_in_goes_through_xdg_open_and_recovers_from_a_failed_browser() {
    let home = TestDir::new("ezerbot-signin");
    std::fs::create_dir(home.join("bin")).unwrap();
    install_script(&home, "xdg-open", BROWSER_FAILING_ONCE);
    let screen = Screen::start();
    let path = format!(
        "{}:{}",
        home.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let app = App::start(
        shell_with_wrapper(),
        &screen,
        &home,
        &[("PATH", path.as_str())],
    );
    wait_for("the window stayed blank", || {
        (screen.colours().len() >= 16).then_some(())
    });
    let (_, window) = screen.app_window();
    let opened = || std::fs::read_to_string(home.join("opened")).unwrap_or_default();
    let attempt = |count: usize, failure: &str| {
        wait_for(failure, || {
            screen.click(window.x + SIGN_IN_BUTTON.0, window.y + SIGN_IN_BUTTON.1);
            std::thread::sleep(Duration::from_millis(400));
            (opened().lines().count() >= count).then_some(())
        })
    };
    attempt(1, "the sign-in page never reached xdg-open");
    attempt(2, "a failed browser left the sign-in pending");
    for link in opened().lines() {
        assert!(
            link.starts_with("https://auth.openai.com/oauth/authorize?"),
            "unexpected sign-in page"
        );
        assert!(link.contains("redirect_uri="), "unexpected sign-in page");
    }
    app.quit();
}

#[test]
fn a_second_launch_shows_the_open_window_and_a_later_launch_starts_again() {
    let home = TestDir::new("ezerbot-relaunch");
    let screen = Screen::start();
    let fake = [("EZERBOT_FAKE_SIGNIN", "1")];
    let app = App::start(shell_with_wrapper(), &screen, &home, &fake);
    wait_for("the window stayed blank", || {
        (screen.colours().len() >= 16).then_some(())
    });
    let (window, area) = screen.app_window();
    let cover = screen.cover(area);
    wait_for("the cover never went over the app", || {
        (screen.top_window() == Some(cover)).then_some(())
    });

    let started = Instant::now();
    let mut second = App::start(shell_with_wrapper(), &screen, &home, &fake);
    let status = loop {
        if let (Some(status), Some(top)) = (second.exited(), screen.top_window()) {
            if top == window {
                break status;
            }
        }
        assert!(
            started.elapsed() < RELAUNCH_LIMIT,
            "the relaunch did not show the open window in time"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success());
    assert_eq!(screen.windows().len(), 2, "the relaunch opened a window");
    app.quit();

    let third = App::start(shell_with_wrapper(), &screen, &home, &fake);
    wait_for("a launch after quitting opened nothing", || {
        (screen.windows().len() == 2).then_some(())
    });
    third.quit();
}

/// Serves a page per view and runs, in each, the bridge commands the test
/// appends to `commands-<view>`. Every outcome is posted back to `results`.
const SCRIPTED_WRAPPER: &str = r#"#!/usr/bin/python3
import http.server, json, os, sys, threading
sys.stdin.buffer.read()
root = os.path.dirname(os.path.abspath(__file__)) + "/.."
pages = {"/": "main.html", "/floating": "panel.html", "/frame": "frame.html"}
cursor = {"main": 0, "panel": 0}
lock = threading.Lock()
class Handler(http.server.BaseHTTPRequestHandler):
    def reply(self, status, body=b"", kind="text/plain"):
        self.send_response(status)
        self.send_header("Content-Type", kind + "; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        path, _, query = self.path.partition("?")
        if path in pages:
            self.reply(200, open(root + "/" + pages[path], "rb").read(), "text/html")
        elif path == "/next":
            view = query.split("=")[-1]
            try:
                lines = open(root + "/commands-" + view).read().splitlines()
            except FileNotFoundError:
                lines = []
            with lock:
                line = lines[cursor[view]] if cursor[view] < len(lines) else ""
                if line:
                    cursor[view] += 1
            self.reply(200, line.encode())
        else:
            self.reply(404)
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        with lock, open(root + "/results", "ab") as results:
            results.write(body + b"\n")
        self.reply(204)
    def log_message(self, *arguments):
        pass
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(json.dumps({"url": f"http://127.0.0.1:{server.server_port}"}), flush=True)
server.serve_forever()
"#;

fn scripted_page(view: &str, extra: &str) -> String {
    format!(
        r#"<!doctype html><body style="margin:0;background:#334"><h1 style="color:#fff">{view}</h1>{extra}<script>
const view = "{view}";
const record = (key, value) => fetch("/result", {{method: "POST", body: JSON.stringify([key, value])}});
window.EzerbotFloating = {{open() {{ record("reopened", true); }}}};
window.EzerbotFloatingMain = {{openConversation: async id => ({{opened: id === "present", reason: "Fil absent."}})}};
(async () => {{
  for (;;) {{
    const line = await (await fetch("/next?view=" + view)).text();
    if (!line) {{ await new Promise(resolve => setTimeout(resolve, 50)); continue; }}
    EzerbotShell.floating.postMessage(JSON.parse(line)).then(
      value => record(line, {{ok: value}}), error => record(line, {{error: String(error.message)}}));
  }}
}})();
</script>"#
    )
}

const FRAME_PAGE: &str = r#"<!doctype html><body><script>
const bridge = window.webkit.messageHandlers.ipc;
fetch("/result", {method: "POST", body: JSON.stringify(["frame", {
  shell: typeof window.EzerbotShell, ipc: typeof bridge.postMessage,
}])});
bridge.postMessage(JSON.stringify({launch: "", id: 1, body: {command: "configure", shortcut: "cmd+option+j"}}));
bridge.postMessage(JSON.stringify({launch: "x", id: 2, body: {command: "show"}}));
</script>"#;

/// A shell on a scripted page, under a private session bus when given one.
struct Scripted {
    // Dropped first: the shell leaves its screen and its files alone.
    app: App,
    screen: Screen,
    home: TestDir,
}

impl Scripted {
    fn start(main_extra: &str, environment: &[(&str, &str)]) -> Self {
        let home = TestDir::new("ezerbot-floating");
        std::fs::create_dir(home.join("bin")).unwrap();
        std::fs::write(home.join("main.html"), scripted_page("main", main_extra)).unwrap();
        std::fs::write(home.join("panel.html"), scripted_page("panel", "")).unwrap();
        std::fs::write(home.join("frame.html"), FRAME_PAGE).unwrap();
        install(&home, "ezerbot-linux", Path::new(SHELL));
        install_script(&home, "grokbot-wrapper", SCRIPTED_WRAPPER);
        let screen = Screen::start();
        let app = App::start(&home.join("bin/ezerbot-linux"), &screen, &home, environment);
        wait_for("the scripted page stayed blank", || {
            (screen.colours().len() >= 2).then_some(())
        });
        Self { home, screen, app }
    }

    fn command(&self, view: &str, body: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.home.join(format!("commands-{view}")))
            .unwrap();
        writeln!(file, "{body}").unwrap();
    }

    fn results(&self) -> Vec<(String, serde_json::Value)> {
        let text = std::fs::read_to_string(self.home.join("results")).unwrap_or_default();
        text.lines()
            .filter_map(|line| serde_json::from_str::<(String, serde_json::Value)>(line).ok())
            .collect()
    }

    /// Runs a command in a view and returns what its promise settled with.
    fn run(&self, view: &str, body: &str) -> serde_json::Value {
        self.command(view, body);
        wait_for("the command never settled", || {
            self.results()
                .into_iter()
                .find(|(key, _)| key == body)
                .map(|(_, value)| value)
        })
    }

    fn panel_open(&self) -> bool {
        self.screen
            .windows()
            .iter()
            .any(|(_, area)| area.width == 420 && area.height == 560)
    }

    fn wait_panel(&self, open: bool, what: &str) {
        wait_for(what, || (self.panel_open() == open).then_some(()));
    }

    fn assert_no_panel_for(&self, span: Duration, what: &str) {
        let until = Instant::now() + span;
        while Instant::now() < until {
            assert!(!self.panel_open(), "{what}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

struct Bus {
    daemon: Child,
    address: String,
}

impl Bus {
    /// No service directory: a call to an absent portal fails instead of
    /// starting the real one.
    fn start(home: &TestDir) -> Self {
        let config = home.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<busconfig><type>session</type><listen>unix:tmpdir={}</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>"#,
                home.path().display()
            ),
        )
        .unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .args(["--nofork", "--print-address=1"])
            .arg(format!("--config-file={}", config.display()))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon is required: sudo apt install dbus");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            daemon,
            address: address.trim().to_owned(),
        }
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SHORTCUTS_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

const PORTAL_XML: &str = r#"<node><interface name="org.freedesktop.portal.GlobalShortcuts">
<method name="CreateSession"><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
<method name="BindShortcuts"><arg type="o" direction="in"/><arg type="a(sa{sv})" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
</interface></node>"#;

const SESSION_XML: &str = r#"<node><interface name="org.freedesktop.portal.Session">
<method name="Close"/></interface></node>"#;

#[derive(Default)]
struct PortalState {
    calls: Mutex<Vec<String>>,
    taken: Mutex<Vec<String>>,
}

/// The GlobalShortcuts portal as the specification describes it, answering on
/// its own thread with its own main loop.
struct FakePortal {
    state: Arc<PortalState>,
    connection: gio::DBusConnection,
    main_loop: glib::MainLoop,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn request_sender(sender: &str) -> String {
    sender.trim_start_matches(':').replace('.', "_")
}

fn object_path(path: &str) -> glib::variant::ObjectPath {
    glib::variant::ObjectPath::try_from(path.to_owned()).unwrap()
}

fn token(options: &Variant, key: &str) -> String {
    glib::VariantDict::new(Some(options))
        .lookup::<String>(key)
        .unwrap()
        .unwrap()
}

impl FakePortal {
    fn start(address: &str) -> Self {
        let state = Arc::new(PortalState::default());
        let (ready, started) = mpsc::channel();
        let context = glib::MainContext::new();
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let (handler_state, address, quit) =
            (Arc::clone(&state), address.to_owned(), main_loop.clone());
        let thread = std::thread::spawn(move || {
            let _ = context.with_thread_default(|| {
                let connection = gio::DBusConnection::for_address_sync(
                    &address,
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                    None::<&gio::Cancellable>,
                )
                .unwrap();
                let node = gio::DBusNodeInfo::for_xml(PORTAL_XML).unwrap();
                let interface = node.lookup_interface(SHORTCUTS_INTERFACE).unwrap();
                connection
                    .register_object(
                        PORTAL_PATH,
                        &interface,
                        move |connection, sender, _, _, method, parameters, invocation| {
                            Self::answer(
                                &handler_state,
                                &connection,
                                sender,
                                method,
                                &parameters,
                                invocation,
                            );
                        },
                        |_, _, _, _, _| unreachable!(),
                        |_, _, _, _, _, _| false,
                    )
                    .unwrap();
                let acquired = ready.clone();
                gio::bus_own_name_on_connection(
                    &connection,
                    "org.freedesktop.portal.Desktop",
                    gio::BusNameOwnerFlags::NONE,
                    move |_, _| {
                        let _ = acquired.send(None);
                    },
                    |_, _| {},
                );
                ready.send(Some(connection)).unwrap();
                quit.run();
            });
        });
        let connection = started.recv().unwrap().expect("the connection comes first");
        started.recv().unwrap();
        Self {
            state,
            connection,
            main_loop,
            thread: Some(thread),
        }
    }

    fn answer(
        state: &Arc<PortalState>,
        connection: &gio::DBusConnection,
        sender: &str,
        method: &str,
        parameters: &Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        let who = request_sender(sender);
        let (options, mut results) = (
            parameters.child_value(parameters.n_children() - 1),
            HashMap::new(),
        );
        let request = format!(
            "{PORTAL_PATH}/request/{who}/{}",
            token(&options, "handle_token")
        );
        match method {
            "CreateSession" => {
                let session = format!(
                    "{PORTAL_PATH}/session/{who}/{}",
                    token(&options, "session_handle_token")
                );
                state
                    .calls
                    .lock()
                    .unwrap()
                    .push(format!("CreateSession {session}"));
                let node = gio::DBusNodeInfo::for_xml(SESSION_XML).unwrap();
                let interface = node
                    .lookup_interface("org.freedesktop.portal.Session")
                    .unwrap();
                let closing = Arc::clone(state);
                let path = session.clone();
                connection
                    .register_object(
                        &session,
                        &interface,
                        move |_, _, _, _, _, _, invocation| {
                            closing.calls.lock().unwrap().push(format!("Close {path}"));
                            invocation.return_value(None);
                        },
                        |_, _, _, _, _| unreachable!(),
                        |_, _, _, _, _, _| false,
                    )
                    .unwrap();
                results.insert("session_handle".to_owned(), session.to_variant());
            }
            _ => {
                let shortcuts: Vec<(String, HashMap<String, Variant>)> =
                    parameters.child_value(1).get().unwrap();
                let trigger: String = shortcuts[0].1["preferred_trigger"].get().unwrap();
                let session = parameters.child_value(0).str().unwrap().to_owned();
                state.calls.lock().unwrap().push(format!(
                    "BindShortcuts {} {trigger} {session}",
                    shortcuts[0].0
                ));
                let bound = if state.taken.lock().unwrap().contains(&trigger) {
                    Vec::new()
                } else {
                    shortcuts
                };
                results.insert("shortcuts".to_owned(), bound.to_variant());
            }
        }
        invocation.return_value(Some(&(object_path(&request),).to_variant()));
        connection
            .emit_signal(
                Some(sender),
                &request,
                "org.freedesktop.portal.Request",
                "Response",
                Some(&(0u32, results).to_variant()),
            )
            .unwrap();
    }

    fn calls(&self) -> Vec<String> {
        self.state.calls.lock().unwrap().clone()
    }

    /// The session the latest `CreateSession` returned.
    fn session(&self, nth: usize) -> String {
        self.calls()
            .iter()
            .filter_map(|call| call.strip_prefix("CreateSession "))
            .nth(nth)
            .unwrap()
            .to_owned()
    }

    fn activate(&self, session: &str, shortcut: &str) {
        let parameters = (
            object_path(session),
            shortcut.to_owned(),
            0u64,
            HashMap::<String, Variant>::new(),
        )
            .to_variant();
        self.connection
            .emit_signal(
                None,
                PORTAL_PATH,
                SHORTCUTS_INTERFACE,
                "Activated",
                Some(&parameters),
            )
            .unwrap();
    }
}

impl Drop for FakePortal {
    fn drop(&mut self) {
        self.main_loop.quit();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn status(outcome: &serde_json::Value) -> &str {
    outcome["ok"]["status"].as_str().unwrap_or("")
}

fn configure(shortcut: &str) -> String {
    format!(r#"{{"command":"configure","shortcut":"{shortcut}"}}"#)
}

#[test]
fn the_portal_opens_the_panel_and_a_replacement_never_loses_the_shortcut() {
    let home = TestDir::new("ezerbot-bus");
    let bus = Bus::start(&home);
    let portal = FakePortal::start(&bus.address);
    let shell = Scripted::start(
        "",
        &[
            ("DBUS_SESSION_BUS_ADDRESS", bus.address.as_str()),
            ("WAYLAND_DISPLAY", "ezerbot-test-0"),
        ],
    );

    assert_eq!(
        status(&shell.run("main", &configure("cmd+option+j"))),
        "registered"
    );
    let first = portal.session(0);
    assert!(portal.calls()[1].starts_with("BindShortcuts cmd+option+j LOGO+ALT+j "));
    portal.activate(&first, "cmd+option+j");
    shell.wait_panel(true, "the portal's Activated did not open the panel");
    shell.run("panel", r#"{"command":"close"}"#);
    shell.wait_panel(false, "the panel stayed open");

    assert_eq!(
        status(&shell.run("main", &configure("ctrl+option+space"))),
        "registered"
    );
    let second = portal.session(1);
    let calls = portal.calls();
    let position = |prefix: String| calls.iter().position(|call| call.starts_with(&prefix));
    assert!(
        position("BindShortcuts ctrl+option+space".into()) < position(format!("Close {first}")),
        "the old session closed before the new one was bound: {calls:?}"
    );
    portal.activate(&first, "cmd+option+j");
    shell.assert_no_panel_for(
        Duration::from_secs(1),
        "a closed session still opened the panel",
    );
    portal.activate(&second, "ctrl+option+space");
    shell.wait_panel(true, "the replacement did not open the panel");
    shell.run("panel", r#"{"command":"close"}"#);
    shell.wait_panel(false, "the panel stayed open");

    portal
        .state
        .taken
        .lock()
        .unwrap()
        .push("LOGO+SHIFT+space".into());
    assert_eq!(
        status(&shell.run("main", &configure("cmd+shift+space"))),
        "conflict"
    );
    let third = portal.session(2);
    wait_for("the refused session was never closed", || {
        portal
            .calls()
            .contains(&format!("Close {third}"))
            .then_some(())
    });
    assert!(
        !portal.calls().contains(&format!("Close {second}")),
        "the shortcut in force was closed"
    );
    portal.activate(&third, "cmd+shift+space");
    shell.assert_no_panel_for(
        Duration::from_secs(1),
        "the refused shortcut opened the panel",
    );
    portal.activate(&second, "ctrl+option+space");
    shell.wait_panel(
        true,
        "the shortcut in force stopped working after a conflict",
    );
    shell.app.quit();
}

#[test]
fn without_a_portal_the_shortcut_is_announced_unavailable() {
    let home = TestDir::new("ezerbot-bus");
    let bus = Bus::start(&home);
    let shell = Scripted::start(
        "",
        &[
            ("DBUS_SESSION_BUS_ADDRESS", bus.address.as_str()),
            ("WAYLAND_DISPLAY", "ezerbot-test-0"),
        ],
    );
    assert_eq!(
        status(&shell.run("main", &configure("cmd+option+j"))),
        "unavailable"
    );
    assert_eq!(
        status(&shell.run("main", &configure("disabled"))),
        "disabled"
    );
    shell.app.quit();
}

const SUPER_ALT_J: [&str; 3] = ["Super_L", "Alt_L", "j"];
const PANEL: &str = "Panneau · Ezerbot";

fn tool_present(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

#[test]
fn the_shortcut_opens_an_always_on_top_panel_that_takes_the_typing() {
    if std::env::var_os("GITHUB_ACTIONS").is_some() && !tool_present("openbox") {
        eprintln!("skipped: openbox is missing on this runner; add it to the apt-get install of linux-pr.yml and linux-shell.yml");
        return;
    }
    let home = TestDir::new("ezerbot-panel");
    let screen = Screen::start_with_openbox(&home);
    let mut app = App::start(
        shell_with_wrapper(),
        &screen,
        &home,
        &[("EZERBOT_FAKE_SIGNIN", "1")],
    );
    wait_for("the window stayed blank", || {
        (screen.colours().len() >= 16).then_some(())
    });
    let main = screen
        .client("Ezerbot")
        .expect("the main window is managed");
    // The shortcut is acquired once the main page has loaded: press until it opens.
    let open_panel = || {
        wait_for("the shortcut never opened the panel", || {
            if screen.client(PANEL).is_none() {
                screen.press(&SUPER_ALT_J);
                std::thread::sleep(Duration::from_millis(600));
            }
            screen.client(PANEL)
        })
    };
    let panel = open_panel();
    assert!(panel.above, "the panel is not above other windows");

    let text = "bonjour depuis le panneau";
    wait_for("the panel never drew its content", || {
        let area = screen
            .windows()
            .into_iter()
            .find(|(_, area)| area.width == 420 && area.height == 560)?
            .1;
        (screen
            .pixels(area)
            .into_iter()
            .collect::<HashSet<_>>()
            .len()
            >= 16)
            .then_some(())
    });
    // Keys sent before the page focused its field are lost: type again until one lands.
    let mut typed = Instant::now() - Duration::from_secs(60);
    wait_for("the exchange typed in the panel was not persisted", || {
        if typed.elapsed() > Duration::from_secs(8) {
            screen.type_text(text);
            screen.press(&["Return"]);
            typed = Instant::now();
        }
        exchange_persisted(&home.join("state/ezerbot.sqlite"), text).then_some(())
    });

    screen.press(&["Escape"]);
    wait_for("Escape did not close the panel", || {
        screen.client(PANEL).is_none().then_some(())
    });
    let cover = screen.cover(Rectangle {
        x: 700,
        y: 300,
        width: 200,
        height: 200,
    });
    wait_for("the first window never went over the app", || {
        screen.stacking(cover).map(|_| ())
    });
    let reopened = open_panel();
    assert!(
        reopened.above,
        "the reopened panel lost its place above other windows"
    );
    let second = screen.cover(Rectangle {
        x: 760,
        y: 340,
        width: 200,
        height: 200,
    });
    wait_for("the second window never went over the app", || {
        screen.stacking(second).map(|_| ())
    });
    let order = |window| screen.stacking(window).unwrap();
    assert!(
        order(reopened.window) > order(second),
        "a window opened later covers the panel"
    );
    assert!(
        order(reopened.window) > order(cover),
        "a window covers the panel"
    );
    assert!(
        order(main.window) < order(cover),
        "the shortcut brought the main window back"
    );

    let panel_area = || {
        screen
            .windows()
            .into_iter()
            .find(|(_, area)| area.width == 420 && area.height == 560)
            .map(|(_, area)| (area.x, area.y))
    };
    let (x, y) = panel_area().expect("the reopened panel has a geometry");
    screen.drag((x + 260, y + 40), (x + 560, y - 60));
    let moved = wait_for("dragging the header did not move the panel", || {
        panel_area().filter(|at| *at != (x, y))
    });
    std::thread::sleep(Duration::from_millis(500));
    let moved = panel_area().unwrap_or(moved);
    screen.press(&["Escape"]);
    wait_for("Escape did not close the moved panel", || {
        screen.client(PANEL).is_none().then_some(())
    });
    open_panel();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        panel_area(),
        Some(moved),
        "the panel did not reopen where it was left"
    );

    // The window manager's own close hides the panel, never the application.
    screen.press(&["Alt_L", "F4"]);
    wait_for("Alt+F4 did not close the panel", || {
        screen.client(PANEL).is_none().then_some(())
    });
    std::thread::sleep(Duration::from_millis(500));
    assert!(app.exited().is_none(), "closing the panel quit the app");
    open_panel();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        panel_area(),
        Some(moved),
        "the panel closed by the window manager did not reopen where it was"
    );
    app.quit();
}

#[test]
fn a_shortcut_another_client_holds_is_a_conflict_and_the_old_one_stays_in_force() {
    let shell = Scripted::start("", &[]);
    assert_eq!(
        status(&shell.run("main", &configure("cmd+option+j"))),
        "registered"
    );
    shell
        .screen
        .grab(xlib::ControlMask | xlib::Mod1Mask, "space");
    assert_eq!(
        status(&shell.run("main", &configure("ctrl+option+space"))),
        "conflict"
    );

    shell.screen.press(&["Control_L", "Alt_L", "space"]);
    wait_for("the other client lost its own key", || {
        (shell.screen.grabbed_presses() > 0).then_some(())
    });
    shell.assert_no_panel_for(
        Duration::from_secs(1),
        "the refused shortcut opened the panel",
    );
    shell.screen.press(&SUPER_ALT_J);
    shell.wait_panel(true, "the old shortcut stopped working after the conflict");
    shell.app.quit();
}

#[test]
fn the_bridge_refuses_a_frame_without_the_token_and_a_command_of_the_other_view() {
    let shell = Scripted::start(
        r#"<iframe src="/frame" style="width:100px;height:100px"></iframe>"#,
        &[],
    );
    let frame = wait_for("the frame never reported", || {
        shell
            .results()
            .into_iter()
            .find(|(key, _)| key == "frame")
            .map(|(_, value)| value)
    });
    // The frame reaches the handler the way the page does, but holds no token.
    assert_eq!(
        frame,
        serde_json::json!({"shell": "undefined", "ipc": "function"})
    );
    shell.screen.press(&SUPER_ALT_J);
    shell.assert_no_panel_for(
        Duration::from_secs(1),
        "a frame without the token got through",
    );

    let wrong = r#"{"command":"close"}"#;
    let refused = shell.run("main", wrong);
    assert!(
        refused["error"].as_str().unwrap().contains("refusée"),
        "{refused}"
    );
    assert_eq!(
        status(&shell.run("main", &configure("cmd+option+j"))),
        "registered"
    );
    shell.screen.press(&SUPER_ALT_J);
    shell.wait_panel(
        true,
        "the shortcut registered by the page did not open the panel",
    );
    let refused = shell.run("panel", &configure("disabled"));
    assert!(
        refused["error"].as_str().unwrap().contains("refusée"),
        "{refused}"
    );
    let refused = shell.run("panel", r#"{"command":"show"}"#);
    assert!(
        refused["error"].as_str().unwrap().contains("refusée"),
        "{refused}"
    );
    shell.app.quit();
}
