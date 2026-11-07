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
    process::{Child, Command, ExitStatus, Stdio},
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
const RELAUNCH_LIMIT: Duration = Duration::from_secs(5);
const CLOSE_LIMIT: Duration = Duration::from_secs(10);

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

    /// The message a window manager sends when the user closes a window.
    fn request_close(&self, window: xlib::Window) {
        unsafe {
            let mut data = xlib::ClientMessageData::new();
            data.set_long(0, self.atom("WM_DELETE_WINDOW") as std::os::raw::c_long);
            data.set_long(1, xlib::CurrentTime as std::os::raw::c_long);
            let mut event = xlib::XEvent {
                client_message: xlib::XClientMessageEvent {
                    type_: xlib::ClientMessage,
                    serial: 0,
                    send_event: xlib::True,
                    display: self.display,
                    window,
                    message_type: self.atom("WM_PROTOCOLS"),
                    format: 32,
                    data,
                },
            };
            (self.xlib.XSendEvent)(
                self.display,
                window,
                xlib::False,
                xlib::NoEventMask,
                &mut event,
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
        Self::open(binary, screen, home, extra, &[])
    }

    fn open(
        binary: &Path,
        screen: &Screen,
        home: &TestDir,
        extra: &[(&str, &str)],
        links: &[&str],
    ) -> Self {
        // A test HOME: the shell's wrapper never reaches the session bus from here.
        std::fs::write(home.join(".ezerbot-test-home"), "test").unwrap();
        let shell = Command::new(binary)
            .args(links)
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

/// What a framed page would do with the bridge: none of it may ever run.
const FRAME_PAGE: &str = r#"<!doctype html><body><script>
fetch("/result", {method: "POST", body: JSON.stringify(["frame-ran", location.search])});
const kept = parent.EzerbotShell?.floating?.postMessage;
if (kept) {
  parent.setTimeout(kept, 500, {command: "show"});
  kept({command: "configure", shortcut: "cmd+option+j"}).then(
    () => "accepted", error => String(error.message)).then(outcome =>
    fetch("/result", {method: "POST", body: JSON.stringify(["frame-via-parent", outcome])}));
}
const ipc = window.webkit?.messageHandlers?.ipc;
if (ipc) ipc.postMessage(JSON.stringify({launch: "", id: 1, body: {command: "show"}}));
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

    fn panel_window(&self) -> Option<xlib::Window> {
        self.screen
            .windows()
            .into_iter()
            .find(|(_, area)| area.width == 420 && area.height == 560)
            .map(|(window, _)| window)
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

fn status(outcome: &serde_json::Value) -> &str {
    outcome["ok"]["status"].as_str().unwrap_or("")
}

fn configure(shortcut: &str) -> String {
    format!(r#"{{"command":"configure","shortcut":"{shortcut}"}}"#)
}

/// Under Wayland nothing is started: no portal, no D-Bus. The session bus here
/// does not exist, so any attempt would fail as `unavailable`. Issue #635.
#[test]
fn under_wayland_the_shortcut_is_announced_unavailable_without_any_portal() {
    let shell = Scripted::start(
        "",
        &[
            (
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/ezerbot-bus",
            ),
            ("WAYLAND_DISPLAY", "ezerbot-test-0"),
        ],
    );
    assert_eq!(
        status(&shell.run("main", &configure("cmd+option+j"))),
        "wayland"
    );
    assert_eq!(
        status(&shell.run("main", &configure("disabled"))),
        "disabled"
    );
    shell.app.quit();
}

const SUPER_ALT_J: [&str; 3] = ["Super_L", "Alt_L", "j"];
const PANEL: &str = "Panneau · Ezerbot";
/// The flag of `_MOTIF_WM_HINTS` that says its decorations field is meaningful.
const MOTIF_DECORATIONS: std::os::raw::c_ulong = 1 << 1;

fn tool_present(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

#[test]
fn the_shortcut_opens_an_always_on_top_panel_that_takes_the_typing() {
    // Never skipped: without openbox nothing proves the panel stays on top.
    assert!(
        tool_present("openbox"),
        "openbox is required: sudo apt-get install openbox"
    );
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

    // Moved again while visible, the shortcut keeps it where it now is.
    let (x, y) = moved;
    screen.drag((x + 260, y + 40), (x + 160, y + 140));
    let again = wait_for("dragging the reopened panel did not move it", || {
        panel_area().filter(|at| *at != moved)
    });
    std::thread::sleep(Duration::from_millis(500));
    let again = panel_area().unwrap_or(again);
    screen.press(&SUPER_ALT_J);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(
        panel_area(),
        Some(again),
        "the shortcut moved a visible panel back to an old position"
    );
    let moved = again;

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
    // Without a window manager nothing else could have set this after the mapping.
    // `_NET_WM_STATE_ABOVE` is not written without one: only the openbox test sees it.
    let panel = shell.panel_window().unwrap();
    let motif = shell.screen.longs(panel, "_MOTIF_WM_HINTS");
    assert!(
        motif.len() >= 3 && motif[0] & MOTIF_DECORATIONS != 0 && motif[2] == 0,
        "the panel asks for decorations: {motif:?}"
    );
    shell.app.quit();
}

#[test]
fn a_close_request_hides_the_panel_and_quits_from_the_main_window() {
    let mut shell = Scripted::start("", &[]);
    assert_eq!(
        status(&shell.run("main", &configure("cmd+option+j"))),
        "registered"
    );
    shell.screen.press(&SUPER_ALT_J);
    shell.wait_panel(true, "the shortcut did not open the panel");

    shell.screen.request_close(shell.panel_window().unwrap());
    shell.wait_panel(false, "the panel's close request did not hide it");
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        shell.app.exited().is_none(),
        "closing the panel quit the app"
    );

    shell.screen.press(&SUPER_ALT_J);
    shell.wait_panel(true, "the hidden panel did not reopen");
    shell.screen.request_close(shell.screen.app_window().0);
    let deadline = Instant::now() + CLOSE_LIMIT;
    let status = loop {
        if let Some(status) = shell.app.exited() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "closing the main window did not quit the app"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "{status}");
}

/// A frame in the page, a frame in a container, and the bypass the review
/// found: a container whose same-origin frame is removed with it in the same
/// turn. The shell refuses every framed document natively, so none ever runs.
const FRAMES: &str = r#"<iframe src="/frame?direct" style="width:100px;height:100px"></iframe>
<script>
const box = document.createElement("div");
box.innerHTML = '<iframe src="/frame?container"></iframe>';
document.body.append(box);
const quick = document.createElement("div");
quick.innerHTML = '<iframe src="/frame?removed"></iframe>';
document.body.append(quick);
quick.firstChild.remove();
quick.remove();
</script>"#;

#[test]
fn no_framed_document_ever_runs_so_no_frame_reaches_the_bridge() {
    let shell = Scripted::start(FRAMES, &[]);
    // The page's own loop is running once a command settles.
    assert_eq!(
        status(&shell.run("main", &configure("cmd+option+j"))),
        "registered"
    );
    shell.assert_no_panel_for(
        Duration::from_secs(3),
        "a framed page reached the bridge and opened the panel",
    );
    let ran: Vec<_> = shell
        .results()
        .into_iter()
        .filter(|(key, _)| key.starts_with("frame"))
        .collect();
    assert!(ran.is_empty(), "a framed document ran: {ran:?}");
    // Nothing to revoke: the page keeps its bridge.
    shell.screen.press(&SUPER_ALT_J);
    shell.wait_panel(true, "the page's own shortcut stopped working");
    shell.app.quit();
}

#[test]
fn the_bridge_refuses_a_command_of_the_other_view() {
    let shell = Scripted::start("", &[]);
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

/// The notification belongs to another thread than the one shown: only following the link
/// opens it. The visible thread marks its own notifications read by itself.
fn notification(database: &Path, id: &str) {
    let database = Connection::open(database).unwrap();
    let thread = format!("autre-{}", id.replace(':', "-"));
    database
        .execute_batch(&format!(
            "CREATE TEMP TABLE copy AS SELECT * FROM wrapper_conversations LIMIT 1;
             UPDATE copy SET id='{thread}';
             INSERT INTO wrapper_conversations SELECT * FROM copy;
             DROP TABLE copy;"
        ))
        .unwrap();
    database
        .execute(
            "INSERT INTO wrapper_notifications SELECT account_id,?1,'reply',id,'card','Réponse',1,1,1 FROM wrapper_conversations WHERE id=?2",
            [id, thread.as_str()],
        )
        .unwrap();
}

fn notification_opened(database: &Path, id: &str) -> Option<()> {
    let database = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let unread: i64 = database
        .query_row(
            "SELECT unread FROM wrapper_notifications WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .ok()?;
    (unread == 0).then_some(())
}

#[test]
fn a_link_reaches_the_open_window_and_the_window_it_starts() {
    let home = TestDir::new("ezerbot-links");
    let screen = Screen::start();
    let fake = [("EZERBOT_FAKE_SIGNIN", "1")];
    let app = App::start(shell_with_wrapper(), &screen, &home, &fake);
    wait_for("the window stayed blank", || {
        (screen.colours().len() >= 16).then_some(())
    });
    let area = screen.web_area();
    let text = "bonjour depuis un lien";
    screen.focus();
    screen.click(area.x + area.width as i32 / 2, area.y + 200);
    screen.press(&["Control_L", "n"]);
    std::thread::sleep(Duration::from_millis(500));
    screen.type_text(text);
    screen.press(&["Return"]);
    let database = home.join("state/ezerbot.sqlite");
    wait_for("the conversation was not persisted", || {
        exchange_persisted(&database, text).then_some(())
    });

    notification(&database, "reply:open");
    let link = "EZERBOT://app/v1/notification?id=reply%3Aopen";
    let mut second = App::open(shell_with_wrapper(), &screen, &home, &fake, &[link]);
    wait_for("the second launch did not leave", || second.exited());
    wait_for("the open window did not follow the link", || {
        notification_opened(&database, "reply:open")
    });
    app.quit();

    notification(&database, "reply:start");
    let link = "ezerbot://app/v1/notification?id=reply%3Astart";
    let third = App::open(shell_with_wrapper(), &screen, &home, &fake, &[link]);
    wait_for("the window it started did not follow the link", || {
        notification_opened(&database, "reply:start")
    });
    third.quit();
}

/// A state directory whose only secret lives in `keyring`: the plaintext file holds nothing.
fn migrated_state(keyring: &grokbot_linux_desktop::testing::Keyring, name: &str) -> TestDir {
    use grokbot_forms::vault::Vault;
    let home = TestDir::new(name);
    let state = home.join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = state.join("ezerbot.secrets.json");
    Vault::open(&file)
        .unwrap()
        .set(
            "PORCELAINE",
            "Piège synthétique",
            "porcelaine-ambree-synthetique-9931",
        )
        .unwrap();
    Vault::open_test_keychain(&file, keyring.address()).unwrap();
    home
}

/// The keyring banner sits right above the composer, which the layout pins to the bottom of the
/// window: text in that band is the banner, never the welcome cards, the toast or the thread.
fn banner_shown(screen: &Screen) -> bool {
    let area = screen.web_area();
    let band = Rectangle {
        x: area.x + 360,
        y: area.y + area.height as i32 - 188,
        width: 340,
        height: 30,
    };
    screen.text_lines(band) > 0
}

fn submissions(home: &TestDir) -> i64 {
    Connection::open_with_flags(
        home.join("state").join("ezerbot.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row("SELECT count(*) FROM submissions", [], |row| row.get(0))
    .unwrap()
}

fn live_shot(screen: &Screen, variable: &str) {
    if let Ok(shot) = std::env::var(variable) {
        let status = Command::new("import")
            .args(["-window", "root"])
            .arg(&shot)
            .env("DISPLAY", &screen.name)
            .status()
            .unwrap();
        assert!(status.success());
    }
}

/// Keyring locked after a restart: the window says so above the composer and sends nothing.
/// Enter neither stores a message nor starts a turn, so no draft can come back twice.
#[test]
fn a_locked_keyring_shows_the_banner_and_the_window_sends_nothing() {
    if !grokbot_linux_desktop::testing::keyring_runs_here() {
        return;
    }
    use grokbot_linux_desktop::testing::Keyring;
    let keyring = Keyring::start();
    let home = migrated_state(&keyring, "ezerbot-locked-window");
    keyring.lock();
    let screen = Screen::start();
    let keyring_home = keyring.home().path().to_str().unwrap().to_owned();
    let extra = [
        ("EZERBOT_FAKE_SIGNIN", "1"),
        ("HOME", keyring_home.as_str()),
        ("GROKBOT_TEST_SESSION_BUS", keyring.address()),
    ];
    let app = App::start(shell_with_wrapper(), &screen, &home, &extra);
    wait_for("the window stayed blank", || {
        (screen.colours().len() >= 16).then_some(())
    });
    std::thread::sleep(Duration::from_secs(2));
    let area = screen.web_area();
    screen.focus();
    screen.click(area.x + area.width as i32 / 2, area.y + 200);
    screen.press(&["Control_L", "n"]);
    std::thread::sleep(Duration::from_millis(500));
    screen.type_text("ce message attend le trousseau");
    screen.press(&["Return"]);
    screen.press(&["Return"]);
    std::thread::sleep(Duration::from_secs(3));
    assert!(banner_shown(&screen), "the locked banner is not on screen");
    assert_eq!(submissions(&home), 0, "a message was sent while locked");
    let runs: i64 = Connection::open_with_flags(
        home.join("state").join("ezerbot.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row("SELECT count(*) FROM wrapper_engine_runs", [], |row| {
        row.get(0)
    })
    .unwrap();
    assert_eq!(runs, 0, "a turn started while locked");
    live_shot(&screen, "LIVE_SHOT");
    app.quit();
}

/// The keyring locks while the window shows the composer, then Envoyer: nothing is kept and the
/// draft comes back. Unlocking outside the app wakes the idle window, and Enter alone sends that
/// draft once.
#[test]
fn the_window_follows_the_keyring_and_sends_the_draft_once() {
    if !grokbot_linux_desktop::testing::keyring_runs_here() {
        return;
    }
    use grokbot_linux_desktop::testing::Keyring;
    let mut keyring = Keyring::start();
    let home = migrated_state(&keyring, "ezerbot-relock-window");
    let screen = Screen::start();
    let keyring_home = keyring.home().path().to_str().unwrap().to_owned();
    let address = keyring.address().to_owned();
    let extra = [
        ("EZERBOT_FAKE_SIGNIN", "1"),
        ("HOME", keyring_home.as_str()),
        ("GROKBOT_TEST_SESSION_BUS", address.as_str()),
    ];
    let app = App::start(shell_with_wrapper(), &screen, &home, &extra);
    wait_for("the window stayed blank", || {
        (screen.colours().len() >= 16).then_some(())
    });
    std::thread::sleep(Duration::from_secs(2));
    let area = screen.web_area();
    screen.focus();
    screen.click(area.x + area.width as i32 / 2, area.y + 200);
    screen.press(&["Control_L", "n"]);
    std::thread::sleep(Duration::from_millis(500));
    screen.type_text("un seul envoi malgre le verrou");
    assert!(!banner_shown(&screen), "a banner showed before the lock");
    keyring.lock();
    screen.press(&["Return"]);
    std::thread::sleep(Duration::from_secs(3));
    assert!(banner_shown(&screen), "the banner did not follow the lock");
    assert_eq!(submissions(&home), 0, "a message was kept while locked");
    live_shot(&screen, "LIVE_SHOT_LOCKED");
    keyring.unlock();
    let started = Instant::now();
    wait_for("unlocking never re-enabled Envoyer", || {
        screen.press(&["Return"]);
        std::thread::sleep(Duration::from_millis(900));
        (submissions(&home) > 0).then_some(())
    });
    eprintln!("sent {:?} after the unlock", started.elapsed());
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(submissions(&home), 1, "the draft was sent twice");
    assert!(!banner_shown(&screen), "the banner stayed after the unlock");
    live_shot(&screen, "LIVE_SHOT_UNLOCKED");
    app.quit();
}
