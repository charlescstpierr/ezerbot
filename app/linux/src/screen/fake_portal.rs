//! A private dbus-daemon with a Rust fake of the portal. The temporary config
//! has no service directory, so nothing real can activate on this bus.
use super::{
    capture::{open, BusAddress, Display, Env, Portal, XDisplay, DESKTOP, OBJECT, REQUEST},
    gate::{sink::Sink, Gate, Reply, Stop},
    image::encode,
    xvfb::Xvfb,
    Refusal, Source,
};
use dbus::{
    arg::{PropMap, RefArg, Variant},
    blocking::Connection,
    channel::{Channel, MatchingReceiver, Sender},
    message::MatchRule,
    Message,
};
use grokbot_test_support::TestDir;
use gtk::gdk_pixbuf::Pixbuf;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Cursor},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const FIXTURE: &[u8] = include_bytes!("../../../native/screen-fixture.png");

struct Bus {
    daemon: Child,
    address: String,
    directory: TestDir,
}

impl Bus {
    fn start() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = TestDir::new("ezerbot-fake-portal");
        let socket = format!(
            "ezerbot-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        );
        let config = directory.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:abstract={socket}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#
            ),
        )
        .unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .arg("--nofork")
            .arg("--print-address")
            .arg(format!("--config-file={}", config.display()))
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
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
            directory,
        }
    }

    fn client(&self) -> BusAddress {
        BusAddress::private(self.address.clone())
    }

    fn picture(&self) -> (PathBuf, String) {
        let path = self.directory.join("Capture d’écran.png");
        std::fs::write(&path, FIXTURE).unwrap();
        let uri = url::Url::from_file_path(&path).unwrap().to_string();
        (path, uri)
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Order {
    SignalFirst,
    ReplyFirst,
}

#[derive(Clone)]
struct Script {
    order: Order,
    response: u32,
    uri: Option<String>,
    hold: bool,
    stall: Duration,
    wrong_path: bool,
}

impl Script {
    fn answering(order: Order, uri: String) -> Self {
        Self {
            order,
            response: 0,
            uri: Some(uri),
            hold: false,
            stall: Duration::ZERO,
            wrong_path: false,
        }
    }

    fn refusing(response: u32) -> Self {
        Self {
            order: Order::SignalFirst,
            response,
            uri: None,
            hold: false,
            stall: Duration::ZERO,
            wrong_path: false,
        }
    }
}

#[derive(Default)]
struct Seen {
    screenshots: AtomicUsize,
    closed: AtomicBool,
    interactive: AtomicBool,
    request: Mutex<Option<(String, String)>>,
}

struct FakePortal {
    stop: Arc<AtomicBool>,
    seen: Arc<Seen>,
    thread: Option<JoinHandle<()>>,
}

fn answer(message: &Message, conn: &Connection, script: &Script, seen: &Seen) {
    let reply = match message.member().as_deref() {
        Some("Get") => message.method_return().append1(Variant(2_u32)),
        Some("Close") => {
            seen.closed.store(true, Ordering::SeqCst);
            message.method_return()
        }
        Some("Screenshot") => {
            seen.screenshots.fetch_add(1, Ordering::SeqCst);
            let (_, options): (&str, PropMap) = message.read2().unwrap();
            let token = options["handle_token"].0.as_str().unwrap();
            seen.interactive.store(
                options["interactive"].0.as_i64() == Some(1),
                Ordering::SeqCst,
            );
            let sender = message.sender().unwrap().replace(':', "").replace('.', "_");
            let handle = format!("{OBJECT}/request/{sender}/{token}");
            *seen.request.lock().unwrap() =
                Some((message.sender().unwrap().to_string(), handle.clone()));
            let mut results = HashMap::new();
            if let Some(uri) = &script.uri {
                results.insert("uri".to_owned(), Variant(uri.clone()));
            }
            let signal = Message::new_signal(handle.as_str(), REQUEST, "Response")
                .unwrap()
                .append2(script.response, results);
            let returned = dbus::Path::new(if script.wrong_path {
                format!("{OBJECT}/request/other/path")
            } else {
                handle
            })
            .unwrap();
            let method_return = message.method_return().append1(returned);
            std::thread::sleep(script.stall);
            if script.order == Order::ReplyFirst || script.hold {
                conn.channel().send(method_return).unwrap();
                if !script.hold {
                    conn.channel().send(signal).unwrap();
                }
            } else {
                conn.channel().send(signal).unwrap();
                conn.channel().send(method_return).unwrap();
            }
            return;
        }
        _ => return,
    };
    let _ = conn.send(reply);
}

impl FakePortal {
    fn serve(bus: &Bus, script: Script) -> Self {
        let address = bus.address.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Seen::default());
        let (running, on_bus) = mpsc::channel();
        let thread = std::thread::spawn({
            let (stop, seen) = (stop.clone(), seen.clone());
            move || {
                let mut channel = Channel::open_private(&address).unwrap();
                channel.register().unwrap();
                let conn = Connection::from(channel);
                conn.request_name(DESKTOP, false, true, false).unwrap();
                conn.start_receive(
                    MatchRule::new_method_call(),
                    Box::new({
                        let seen = seen.clone();
                        move |message, conn| {
                            answer(&message, conn, &script, &seen);
                            true
                        }
                    }),
                );
                running.send(()).unwrap();
                while !stop.load(Ordering::SeqCst) {
                    conn.process(Duration::from_millis(20)).unwrap();
                }
            }
        });
        on_bus.recv().unwrap();
        Self {
            stop,
            seen,
            thread: Some(thread),
        }
    }
}

impl Drop for FakePortal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.thread.take().unwrap().join();
    }
}

fn the_fixture() -> Pixbuf {
    Pixbuf::from_read(Cursor::new(FIXTURE)).unwrap()
}

fn portal(bus: &Bus) -> Portal {
    Portal::open(&bus.client()).unwrap()
}

#[test]
fn a_response_sent_before_the_method_return_delivers_the_fixture_and_removes_the_file() {
    let bus = Bus::start();
    let (path, uri) = bus.picture();
    let fake = FakePortal::serve(&bus, Script::answering(Order::SignalFirst, uri));
    let shot = portal(&bus).shot(&|| false).unwrap();
    assert_eq!(
        shot.pixbuf.read_pixel_bytes(),
        the_fixture().read_pixel_bytes()
    );
    assert_eq!(shot.bounds.w, 1280.0);
    assert_eq!(encode(&shot).unwrap().width, 1280);
    assert!(!path.exists(), "the portal's file stays on the desktop");
    assert!(!fake.seen.interactive.load(Ordering::SeqCst));
}

#[test]
fn a_response_sent_after_the_method_return_delivers_the_fixture_twice_in_a_row() {
    let bus = Bus::start();
    let (path, uri) = bus.picture();
    let _fake = FakePortal::serve(&bus, Script::answering(Order::ReplyFirst, uri));
    let mut client = portal(&bus);
    assert_eq!(client.shot(&|| false).unwrap().pixbuf.width(), 1280);
    assert!(!path.exists());
    bus.picture();
    assert_eq!(client.shot(&|| false).unwrap().pixbuf.height(), 800);
    assert!(!path.exists());
}

#[test]
fn a_person_who_cancels_or_a_backend_that_fails_is_a_refusal() {
    for response in [1, 2] {
        let bus = Bus::start();
        let _fake = FakePortal::serve(&bus, Script::refusing(response));
        assert_eq!(
            portal(&bus).shot(&|| false).err(),
            Some(Refusal::Denied),
            "response {response}"
        );
    }
}

#[test]
fn a_handle_other_than_the_predicted_one_fails_closed() {
    let bus = Bus::start();
    let (path, uri) = bus.picture();
    let mut script = Script::answering(Order::ReplyFirst, uri);
    script.wrong_path = true;
    let _fake = FakePortal::serve(&bus, script);
    assert_eq!(portal(&bus).shot(&|| false).err(), Some(Refusal::Portal));
    assert!(path.exists(), "no answer was read");
}

#[test]
fn a_success_without_a_file_or_with_a_symlink_delivers_nothing() {
    let bus = Bus::start();
    let (picture, _) = bus.picture();
    let link = bus.directory.join("lien.png");
    std::os::unix::fs::symlink(&picture, &link).unwrap();
    let uri = url::Url::from_file_path(&link).unwrap().to_string();
    let _fake = FakePortal::serve(&bus, Script::answering(Order::SignalFirst, uri));
    assert_eq!(portal(&bus).shot(&|| false).err(), Some(Refusal::Portal));
    assert!(picture.exists());
    assert!(link.exists());
}

#[test]
fn a_revocation_during_a_portal_wait_closes_the_request_and_delivers_no_image() {
    let bus = Bus::start();
    let (_, uri) = bus.picture();
    let mut script = Script::answering(Order::ReplyFirst, uri);
    script.hold = true;
    let fake = FakePortal::serve(&bus, script);
    let sink = Sink::default();
    let deadline = Instant::now() + Duration::from_secs(60);
    let gate = Arc::new(Gate::new(sink.clone(), deadline));
    let ticket = gate.admit(1).unwrap();
    let worker = std::thread::spawn({
        let (gate, address) = (gate.clone(), bus.client());
        move || {
            let shot = Portal::open(&address)
                .unwrap()
                .shot(&|| !gate.current(ticket));
            let reply = match shot.as_ref().map(encode) {
                Ok(Ok(image)) => Reply::Image(image),
                _ => Reply::Refused("Capture impossible.".into()),
            };
            gate.deliver(ticket, reply);
        }
    });
    while fake.seen.screenshots.load(Ordering::SeqCst) == 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
    gate.stop(Stop::Revoke);
    worker.join().unwrap();
    assert_eq!(sink.lines(), [serde_json::json!({"event":"revoked"})]);
    assert!(fake.seen.closed.load(Ordering::SeqCst));
}

#[test]
fn a_response_from_another_client_is_ignored_and_touches_no_file() {
    let bus = Bus::start();
    let (victim, uri) = bus.picture();
    let mut script = Script::refusing(0);
    script.hold = true;
    let fake = FakePortal::serve(&bus, script);
    let rogue = std::thread::spawn({
        let (address, seen) = (bus.address.clone(), fake.seen.clone());
        move || {
            let mut channel = Channel::open_private(&address).unwrap();
            channel.register().unwrap();
            let (client, handle) = loop {
                if let Some(request) = seen.request.lock().unwrap().clone() {
                    break request;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            let results = HashMap::from([("uri".to_owned(), Variant(uri))]);
            let mut forged = Message::new_signal(handle.as_str(), REQUEST, "Response")
                .unwrap()
                .append2(0_u32, results);
            forged.set_destination(Some(client.into()));
            channel.send(forged).unwrap();
            channel.flush();
        }
    });
    let mut portal = portal(&bus);
    portal.limit = Duration::from_millis(400);
    assert!(matches!(portal.shot(&|| false), Err(Refusal::Portal)));
    rogue.join().unwrap();
    assert!(victim.exists());
}

#[test]
fn a_file_that_is_not_an_image_is_refused_and_stays() {
    let bus = Bus::start();
    let (path, uri) = bus.picture();
    std::fs::write(&path, "notes personnelles").unwrap();
    let _fake = FakePortal::serve(&bus, Script::answering(Order::ReplyFirst, uri));
    assert!(matches!(
        portal(&bus).shot(&|| false),
        Err(Refusal::Capture)
    ));
    assert!(path.exists());
}

#[test]
fn a_slow_method_call_counts_against_the_time_limit_of_the_request() {
    let bus = Bus::start();
    let mut script = Script::refusing(0);
    script.hold = true;
    script.stall = Duration::from_millis(600);
    let fake = FakePortal::serve(&bus, script);
    let mut portal = portal(&bus);
    portal.limit = Duration::from_millis(600);
    let started = Instant::now();
    assert!(matches!(portal.shot(&|| false), Err(Refusal::Portal)));
    assert!(started.elapsed() < Duration::from_millis(1100));
    assert!(fake.seen.closed.load(Ordering::SeqCst));
}

#[test]
fn a_bus_without_a_portal_refuses_on_wayland_and_uses_the_root_on_x11() {
    let bus = Bus::start();
    let wayland = Env {
        display: Display::Wayland,
        bus: Some(bus.client()),
        x: None,
    };
    assert_eq!(open(&wayland).err(), Some(Refusal::WaylandNeedsPortal));
    let server = Xvfb::start("320x200x24");
    let x11 = Env {
        display: Display::X11,
        bus: Some(bus.client()),
        x: Some(XDisplay::private(&server.name)),
    };
    let mut source = open(&x11).unwrap();
    assert!(matches!(source, Source::X11(_)));
    assert_eq!(source.shot(&|| false).unwrap().pixbuf.width(), 320);
}

#[test]
fn a_bus_with_a_portal_wins_on_x11_and_serves_wayland() {
    let bus = Bus::start();
    let (_, uri) = bus.picture();
    let _fake = FakePortal::serve(&bus, Script::answering(Order::SignalFirst, uri));
    for display in [Display::Wayland, Display::X11] {
        bus.picture();
        let env = Env {
            display,
            bus: Some(bus.client()),
            x: None,
        };
        let mut source = open(&env).unwrap();
        assert!(matches!(source, Source::Portal(_)));
        assert_eq!(source.shot(&|| false).unwrap().pixbuf.width(), 1280);
    }
}

#[test]
fn no_session_at_all_is_a_refusal() {
    let env = Env {
        display: Display::None,
        bus: None,
        x: None,
    };
    assert_eq!(open(&env).err(), Some(Refusal::NoDisplay));
}
