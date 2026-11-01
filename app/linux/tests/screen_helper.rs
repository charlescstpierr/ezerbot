//! The real debug binary under a private Xvfb: its stdout and the X pixels are
//! what the wrapper and the person see. It only ever holds the drawn fixture.
#![cfg(all(target_os = "linux", debug_assertions))]

#[path = "support/xvfb.rs"]
mod xvfb;

use serde_json::{json, Value};
use std::{
    ffi::CString,
    io::{BufRead, BufReader, Lines, Write},
    os::raw::c_uint,
    process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
    ptr,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use x11_dl::{
    xlib::{self, Xlib},
    xtest::Xf86vmode as Xtst,
};
use xvfb::Xvfb;

const HELPER: &str = env!("CARGO_BIN_EXE_ezerbot-linux");
const WAIT: Duration = Duration::from_secs(20);
const OVERLAY_WIDTH: u32 = 560;

fn command(display: &str, arguments: &[&str]) -> Command {
    let mut command = Command::new(HELPER);
    command
        .args(arguments)
        .env_clear()
        .env("DISPLAY", display)
        .env("GDK_BACKEND", "x11")
        .env("NO_AT_BRIDGE", "1")
        .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
        .stderr(Stdio::null());
    command
}

struct Helper {
    child: Child,
    input: Option<ChildStdin>,
    lines: Receiver<Value>,
}

impl Helper {
    fn start(display: &str, milliseconds: u64, delay: Option<u64>) -> Self {
        let mut command = command(display, &["--screen-helper", &milliseconds.to_string()]);
        if let Some(delay) = delay {
            command.env("EZERBOT_SCREEN_FIXTURE_DELAY_MS", delay.to_string());
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let reader: Lines<BufReader<ChildStdout>> =
            BufReader::new(child.stdout.take().unwrap()).lines();
        let (line, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for text in reader.map_while(Result::ok) {
                if line.send(serde_json::from_str(&text).unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut helper = Self {
            child,
            input,
            lines,
        };
        assert_eq!(helper.line(), json!({"ready":true}));
        helper
    }

    /// Started where the banner cannot be proven visible: it announces a pause.
    fn start_paused(display: &str) -> Self {
        let mut helper = Self::start(display, 60_000, None);
        assert_eq!(helper.line(), json!({"event":"paused"}));
        helper
    }

    fn line(&mut self) -> Value {
        self.lines
            .recv_timeout(WAIT)
            .expect("a line from the helper")
    }

    fn send(&mut self, command: Value) {
        writeln!(self.input.as_mut().unwrap(), "{command}").unwrap();
    }

    fn act(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        self.send(json!({"command":"action","id":id,"name":name,"arguments":arguments}));
        self.line()
    }

    fn screenshot(&mut self, id: u64) -> Value {
        self.act(id, "computer_screenshot", json!({}))
    }

    fn finished(&mut self) -> ExitStatus {
        assert!(
            self.lines.recv_timeout(WAIT).is_err(),
            "output after the last line"
        );
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(start.elapsed() < WAIT, "the helper outlived its lease");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Viewer {
    xlib: Xlib,
    xtest: Xtst,
    display: *mut xlib::Display,
}

impl Viewer {
    fn on(name: &str) -> Self {
        let xlib = Xlib::open().unwrap();
        let xtest = Xtst::open().unwrap();
        let address = CString::new(name).unwrap();
        let display = unsafe { (xlib.XOpenDisplay)(address.as_ptr()) };
        assert!(!display.is_null());
        Self {
            xlib,
            xtest,
            display,
        }
    }

    fn overlay(&self) -> Option<(i32, i32, u32, u32)> {
        self.overlay_window().map(|(_, area)| area)
    }

    /// The overlay is the only top-level window of this private server.
    fn overlay_window(&self) -> Option<(xlib::Window, (i32, i32, u32, u32))> {
        let mut found = None;
        unsafe {
            let root = (self.xlib.XDefaultRootWindow)(self.display);
            let (mut same_root, mut parent, mut children, mut count) =
                (0, 0, ptr::null_mut(), 0 as c_uint);
            (self.xlib.XQueryTree)(
                self.display,
                root,
                &mut same_root,
                &mut parent,
                &mut children,
                &mut count,
            );
            for index in 0..count as usize {
                let mut state: xlib::XWindowAttributes = std::mem::zeroed();
                let known = (self.xlib.XGetWindowAttributes)(
                    self.display,
                    *children.add(index),
                    &mut state,
                ) != 0;
                if known
                    && state.map_state == xlib::IsViewable
                    && state.width as u32 == OVERLAY_WIDTH
                {
                    let area = (state.x, state.y, state.width as u32, state.height as u32);
                    found = Some((*children.add(index), area));
                }
            }
            if !children.is_null() {
                (self.xlib.XFree)(children.cast());
            }
        }
        found
    }

    fn pixel(&self, x: i32, y: i32) -> [u8; 3] {
        unsafe {
            let root = (self.xlib.XDefaultRootWindow)(self.display);
            let image = (self.xlib.XGetImage)(self.display, root, x, y, 1, 1, !0, xlib::ZPixmap);
            assert!(!image.is_null());
            let [blue, green, red, _] = ((self.xlib.XGetPixel)(image, 0, 0) as u32).to_le_bytes();
            (self.xlib.XDestroyImage)(image);
            [red, green, blue]
        }
    }

    fn click(&self, x: i32, y: i32) {
        unsafe {
            (self.xtest.XTestFakeMotionEvent)(self.display, 0, x, y, 0);
            (self.xlib.XSync)(self.display, 0);
        }
        std::thread::sleep(Duration::from_millis(100));
        unsafe {
            (self.xtest.XTestFakeButtonEvent)(self.display, 1, 1, 0);
            (self.xtest.XTestFakeButtonEvent)(self.display, 1, 0, 0);
            (self.xlib.XSync)(self.display, 0);
        }
    }

    /// With no window manager here, X reports this as an iconified window to GDK.
    fn minimize(&self) {
        let (window, _) = self.overlay_window().unwrap();
        unsafe {
            (self.xlib.XUnmapWindow)(self.display, window);
            (self.xlib.XSync)(self.display, 0);
        }
    }

    /// Another application's window, placed over `area` and raised above everything.
    fn cover(&self, (x, y, width, height): (i32, i32, u32, u32)) -> xlib::Window {
        unsafe {
            let root = (self.xlib.XDefaultRootWindow)(self.display);
            let mut attributes: xlib::XSetWindowAttributes = std::mem::zeroed();
            attributes.override_redirect = 1;
            attributes.background_pixel = 0x00ff_ffff;
            let window = (self.xlib.XCreateWindow)(
                self.display,
                root,
                x - 10,
                y - 5,
                width + 20,
                height + 10,
                0,
                0,
                xlib::InputOutput as u32,
                ptr::null_mut(),
                xlib::CWOverrideRedirect | xlib::CWBackPixel,
                &mut attributes,
            );
            (self.xlib.XMapRaised)(self.display, window);
            (self.xlib.XSync)(self.display, 0);
            window
        }
    }

    fn uncover(&self, window: xlib::Window) {
        unsafe {
            (self.xlib.XDestroyWindow)(self.display, window);
            (self.xlib.XSync)(self.display, 0);
        }
    }

    /// Claims `_NET_WM_CM_S0`, as a compositing manager does.
    fn compositor(&self) -> xlib::Window {
        unsafe {
            let root = (self.xlib.XDefaultRootWindow)(self.display);
            let owner = (self.xlib.XCreateSimpleWindow)(self.display, root, 0, 0, 1, 1, 0, 0, 0);
            let name = CString::new("_NET_WM_CM_S0").unwrap();
            let atom = (self.xlib.XInternAtom)(self.display, name.as_ptr(), 0);
            (self.xlib.XSetSelectionOwner)(self.display, atom, owner, xlib::CurrentTime);
            (self.xlib.XSync)(self.display, 0);
            owner
        }
    }

    /// What a window manager sends when the person closes a window.
    fn ask_to_close(&self) {
        let (window, _) = self.overlay_window().unwrap();
        unsafe {
            let atom = |name: &str| {
                let name = CString::new(name).unwrap();
                (self.xlib.XInternAtom)(self.display, name.as_ptr(), 0)
            };
            let mut event: xlib::XEvent = std::mem::zeroed();
            event.client_message.type_ = xlib::ClientMessage;
            event.client_message.window = window;
            event.client_message.message_type = atom("WM_PROTOCOLS");
            event.client_message.format = 32;
            event
                .client_message
                .data
                .set_long(0, atom("WM_DELETE_WINDOW") as _);
            (self.xlib.XSendEvent)(self.display, window, 0, 0, &mut event);
            (self.xlib.XSync)(self.display, 0);
        }
    }

    fn visible_overlay(&self) -> (i32, i32, u32, u32) {
        let start = Instant::now();
        loop {
            if let Some(area @ (x, y, ..)) = self.overlay() {
                if self.pixel(x + 3, y + 3) == [0x1f, 0x21, 0x29] {
                    return area;
                }
            }
            assert!(start.elapsed() < WAIT, "the overlay never appeared");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        unsafe { (self.xlib.XCloseDisplay)(self.display) };
    }
}

#[test]
fn a_lease_serves_the_fixture_then_stops_on_revoke() {
    let server = Xvfb::start("1280x900x24");
    let mut helper = Helper::start(&server.name, 60_000, None);
    let reply = helper.screenshot(1);
    assert_eq!(reply["id"], 1);
    assert_eq!(
        (&reply["width"], &reply["height"]),
        (&json!(1280), &json!(800))
    );
    assert!(reply["image"]
        .as_str()
        .unwrap()
        .starts_with("data:image/png;base64,iVBOR"));
    helper.send(json!({"command":"revoke"}));
    assert_eq!(helper.line(), json!({"event":"revoked"}));
    assert!(helper.finished().success());
}

#[test]
fn a_duration_outside_the_limits_exits_64_before_any_display() {
    for arguments in [["--screen-helper", "99"], ["--screen-helper", "900001"]] {
        let status = command(":59999", &arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(64));
    }
}

#[test]
fn no_display_is_an_error_line_in_place_of_ready() {
    let output = command(":59999", &["--screen-helper", "1000"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(reply["error"].is_string());
}

#[test]
fn the_debug_permission_check_answers_without_a_display() {
    let start = Instant::now();
    let status = command(":59999", &["--screen-permissions"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_revocation_during_a_capture_is_the_last_line_and_no_image_follows() {
    let server = Xvfb::start("1280x900x24");
    let mut helper = Helper::start(&server.name, 60_000, Some(1500));
    helper.send(json!({"command":"action","id":1,"name":"computer_screenshot","arguments":{}}));
    std::thread::sleep(Duration::from_millis(200));
    helper.send(json!({"command":"revoke"}));
    assert_eq!(helper.line(), json!({"event":"revoked"}));
    assert!(helper.finished().success());
}

#[test]
fn the_end_of_input_revokes() {
    let server = Xvfb::start("1280x900x24");
    let mut helper = Helper::start(&server.name, 60_000, None);
    helper.input.take();
    assert_eq!(helper.line(), json!({"event":"revoked"}));
    assert!(helper.finished().success());
}

#[test]
fn a_malformed_or_oversized_command_revokes() {
    let server = Xvfb::start("1280x900x24");
    for line in ["pas du json".to_owned(), "x".repeat(40_000)] {
        let mut helper = Helper::start(&server.name, 60_000, None);
        writeln!(helper.input.as_mut().unwrap(), "{line}").unwrap();
        assert_eq!(helper.line(), json!({"event":"revoked"}));
        assert!(helper.finished().success());
    }
}

#[test]
fn the_lease_expires_by_itself() {
    let server = Xvfb::start("1280x900x24");
    let mut helper = Helper::start(&server.name, 3_000, None);
    assert_eq!(helper.line(), json!({"event":"revoked"}));
    assert!(helper.finished().success());
}

#[test]
fn a_second_request_during_a_capture_is_refused_without_ending_the_lease() {
    let server = Xvfb::start("1280x900x24");
    let mut helper = Helper::start(&server.name, 60_000, Some(600));
    let shot =
        |id: u64| json!({"command":"action","id":id,"name":"computer_screenshot","arguments":{}});
    helper.send(shot(1));
    std::thread::sleep(Duration::from_millis(200));
    helper.send(shot(2));
    helper.send(shot(3));
    let mut replies: Vec<Value> = (0..3).map(|_| helper.line()).collect();
    replies.sort_by_key(|reply| reply["id"].as_u64());
    assert!(replies[0]["image"].is_string());
    assert!(replies[1]["image"].is_string());
    assert!(replies[2]["error"].is_string());
}

#[test]
fn points_map_through_the_last_capture_and_no_gesture_is_sent() {
    let server = Xvfb::start("1280x900x24");
    let mut helper = Helper::start(&server.name, 60_000, None);
    let click = json!({"x":640,"y":400,"button":"left"});
    let before = helper.act(1, "computer_click", click.clone());
    assert!(before["error"]
        .as_str()
        .unwrap()
        .contains("nouvelle capture"));
    helper.screenshot(2);
    let inside = helper.act(3, "computer_click", click);
    assert!(
        inside["error"].as_str().unwrap().contains("(640, 400)"),
        "{inside}"
    );
    assert!(inside["error"].as_str().unwrap().contains("aucun geste"));
    let outside = helper.act(4, "computer_move", json!({"x":1280,"y":10}));
    assert!(outside["error"]
        .as_str()
        .unwrap()
        .contains("hors de la dernière capture"));
    let typing = helper.act(5, "computer_type", json!({"text":"bonjour"}));
    assert!(typing["error"].as_str().unwrap().contains("aucun geste"));
    assert!(helper.act(6, "computer_inconnu", json!({}))["error"].is_string());
}

#[test]
fn the_overlay_is_visible_and_its_buttons_pause_and_stop_the_lease() {
    let server = Xvfb::start("1280x900x24");
    let viewer = Viewer::on(&server.name);
    let mut helper = Helper::start(&server.name, 60_000, None);
    let (x, y, width, height) = viewer.visible_overlay();
    assert_eq!(x, (1280 - OVERLAY_WIDTH as i32) / 2);
    assert!(y < 40, "top center, at {y}");
    let row = y + height as i32 / 2;
    let right = x + width as i32;
    let (pause, stop) = (right - 6 - 96 - 6 - 48, right - 6 - 48);
    for button in [pause, stop] {
        let [red, green, blue] = viewer.pixel(button - 40, row);
        assert!(
            red < 128 && green < 128 && blue < 128,
            "white labels need a dark face"
        );
    }

    viewer.click(pause, row);
    assert_eq!(helper.line(), json!({"event":"paused"}));
    let refused = helper.screenshot(1);
    assert!(refused["error"].is_string() && refused.get("image").is_none());
    helper.send(json!({"command":"resume"}));
    assert_eq!(helper.line(), json!({"event":"resumed"}));
    assert!(helper.screenshot(2)["image"].is_string());

    viewer.click(stop, row);
    assert_eq!(helper.line(), json!({"event":"revoked"}));
    assert!(helper.finished().success());
    assert_eq!(viewer.overlay(), None);
}

#[test]
fn closing_the_overlay_revokes() {
    let server = Xvfb::start("1280x900x24");
    let viewer = Viewer::on(&server.name);
    let mut helper = Helper::start(&server.name, 60_000, None);
    viewer.visible_overlay();
    viewer.ask_to_close();
    assert_eq!(helper.line(), json!({"event":"revoked"}));
    assert!(helper.finished().success());
}

#[test]
fn minimizing_the_overlay_pauses_and_resuming_shows_it_again() {
    let server = Xvfb::start("1280x900x24");
    let viewer = Viewer::on(&server.name);
    let mut helper = Helper::start(&server.name, 60_000, None);
    viewer.visible_overlay();
    viewer.minimize();
    assert_eq!(helper.line(), json!({"event":"paused"}));
    assert_eq!(viewer.overlay(), None);
    helper.send(json!({"command":"resume"}));
    assert_eq!(helper.line(), json!({"event":"resumed"}));
    // The banner is still on its way back: a capture asked now must not happen.
    let early = helper.screenshot(1);
    assert!(
        early.get("image").is_none() && early["error"].as_str().unwrap().contains("bandeau"),
        "{early}"
    );
    viewer.visible_overlay();
    let start = Instant::now();
    let mut id = 2;
    while helper.screenshot(id).get("image").is_none() {
        assert!(
            start.elapsed() < WAIT,
            "the banner came back but no capture"
        );
        std::thread::sleep(Duration::from_millis(20));
        id += 1;
    }
}

#[test]
fn a_window_over_the_banner_pauses_and_nothing_is_captured_until_it_is_seen_again() {
    let server = Xvfb::start("1280x900x24");
    let viewer = Viewer::on(&server.name);
    let mut helper = Helper::start(&server.name, 60_000, None);
    let area = viewer.visible_overlay();
    assert!(helper.screenshot(1)["image"].is_string());
    let cover = viewer.cover(area);
    assert_eq!(helper.line(), json!({"event":"paused"}));
    assert!(helper.screenshot(2).get("image").is_none());
    helper.send(json!({"command":"resume"}));
    assert_eq!(helper.line(), json!({"event":"resumed"}));
    let hidden = helper.screenshot(3);
    assert!(
        hidden["error"].as_str().unwrap().contains("bandeau"),
        "{hidden}"
    );
    viewer.uncover(cover);
    let start = Instant::now();
    let mut id = 4;
    while helper.screenshot(id).get("image").is_none() {
        assert!(
            start.elapsed() < WAIT,
            "the banner was seen again but no capture"
        );
        std::thread::sleep(Duration::from_millis(20));
        id += 1;
    }
}

#[test]
fn the_debug_binary_holds_no_capture_code() {
    let binary = std::fs::read(HELPER).unwrap();
    for name in ["org.freedesktop.portal.Screenshot", "handle_token"] {
        assert!(
            !binary
                .windows(name.len())
                .any(|bytes| bytes == name.as_bytes()),
            "{name}"
        );
    }
}

#[test]
fn under_a_compositing_manager_the_lease_starts_paused_and_captures_nothing() {
    let server = Xvfb::start("1280x900x24");
    let viewer = Viewer::on(&server.name);
    let _compositor = viewer.compositor();
    let mut helper = Helper::start_paused(&server.name);
    viewer.visible_overlay();
    assert!(helper.screenshot(1).get("image").is_none());
    helper.send(json!({"command":"resume"}));
    assert_eq!(helper.line(), json!({"event":"resumed"}));
    let refused = helper.screenshot(2);
    assert!(
        refused["error"].as_str().unwrap().contains("bandeau"),
        "{refused}"
    );
}
