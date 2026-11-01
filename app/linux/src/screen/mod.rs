//! `ezerbot-linux --screen-helper`: the Linux side of the lease in `screen_native.rs`.
#[cfg(any(test, not(debug_assertions)))]
pub mod capture;
#[cfg(all(test, debug_assertions))]
mod fake_portal;
pub mod gate;
pub mod image;
mod overlay;
#[cfg(test)]
#[path = "../../tests/support/xvfb.rs"]
mod xvfb;

use gate::{Gate, Reply, Stop, Ticket, View};
use gtk::glib;
use image::{encode, image_point, Bounds, Shot};
use overlay::Overlay;
use serde_json::Value;
use std::{
    ffi::OsString,
    io::{stdin, stdout, BufRead, BufReader, Read, Stdout},
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender},
        Arc,
    },
    time::{Duration, Instant},
};

const DURATIONS: std::ops::RangeInclusive<u64> = 100..=900_000;
const MAX_COMMAND: u64 = 32_768;
const USAGE: i32 = 64;
#[cfg(not(debug_assertions))]
const FOREIGN_PARENT: i32 = 77;

#[derive(Debug, PartialEq)]
pub enum Invocation {
    Helper { duration: Duration },
    Permissions,
    Usage,
}

impl Invocation {
    /// `None` leaves the arguments to the shell.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Option<Self> {
        let mut arguments = arguments.into_iter();
        let first = arguments.next()?;
        if first == "--screen-permissions" {
            return Some(match arguments.next() {
                None => Self::Permissions,
                Some(_) => Self::Usage,
            });
        }
        if first != "--screen-helper" {
            return None;
        }
        let duration = arguments
            .next()
            .and_then(|value| value.to_str()?.parse::<u64>().ok())
            .filter(|duration| DURATIONS.contains(duration));
        Some(match (duration, arguments.next()) {
            (Some(duration), None) => Self::Helper {
                duration: Duration::from_millis(duration),
            },
            _ => Self::Usage,
        })
    }
}

pub fn run(invocation: Invocation) -> i32 {
    if invocation == Invocation::Usage {
        return USAGE;
    }
    #[cfg(not(debug_assertions))]
    if !parent_is_wrapper() {
        return FOREIGN_PARENT;
    }
    match invocation {
        Invocation::Helper { duration } => helper(duration),
        _ => permissions(),
    }
}

#[cfg(debug_assertions)]
fn permissions() -> i32 {
    0
}

#[cfg(not(debug_assertions))]
fn permissions() -> i32 {
    i32::from(capture::open(&capture::Env::read()).is_err())
}

/// The child's parent must be the wrapper shipped beside it, under the same user.
#[cfg(any(test, not(debug_assertions)))]
fn parent_matches(
    parent: &std::path::Path,
    expected: &std::path::Path,
    parent_uid: u32,
    uid: u32,
) -> bool {
    parent == expected && parent_uid == uid
}

#[cfg(not(debug_assertions))]
fn parent_is_wrapper() -> bool {
    use std::os::unix::fs::MetadataExt;
    let parent = nix::unistd::getppid();
    let proc = std::path::PathBuf::from(format!("/proc/{parent}"));
    let (Ok(exe), Ok(own), Ok(meta)) = (
        std::fs::read_link(proc.join("exe")),
        std::env::current_exe(),
        std::fs::metadata(&proc),
    ) else {
        return false;
    };
    let expected = own.with_file_name("grokbot-wrapper");
    parent_matches(&exe, &expected, meta.uid(), nix::unistd::getuid().as_raw())
}

#[cfg(debug_assertions)]
fn opened() -> Result<Source, Refusal> {
    let delay = std::env::var("EZERBOT_SCREEN_FIXTURE_DELAY_MS")
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
        .unwrap_or(0)
        .min(2000);
    Ok(Source::Fixture {
        delay: Duration::from_millis(delay),
    })
}

#[cfg(not(debug_assertions))]
fn opened() -> Result<Source, Refusal> {
    capture::open(&capture::Env::read())
}

const BANNER_WAIT: Duration = Duration::from_secs(2);

fn helper(duration: Duration) -> i32 {
    let gate = Arc::new(Gate::new(stdout(), Instant::now() + duration));
    #[cfg(not(debug_assertions))]
    capture::init_x_threads();
    if gtk::init().is_err() {
        gate.abort(Refusal::NoDisplay.message());
        return 1;
    }
    let overlay = Overlay::build(&gate);
    // `ready` waits for the banner, so a lease never opens on a hidden one. A banner
    // that cannot be proven (Wayland) gives up here and stays paused.
    let shown = Instant::now() + BANNER_WAIT;
    while !gate.showing() && Instant::now() < shown {
        if gtk::events_pending() {
            gtk::main_iteration();
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    overlay.render(gate.tick());
    while gtk::events_pending() {
        gtk::main_iteration();
    }
    let (jobs, queue) = sync_channel(1);
    std::thread::spawn({
        let gate = gate.clone();
        move || worker(&queue, &gate)
    });
    std::thread::spawn({
        let gate = gate.clone();
        move || read_commands(&gate, &jobs)
    });
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let view = gate.tick();
        overlay.render(view);
        if view == View::Closed {
            gtk::main_quit();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    gtk::main();
    0
}

enum Action {
    Screenshot,
    Point { x: f64, y: f64 },
    Gesture,
    Unknown,
}

impl Action {
    fn parse(name: &str, arguments: &Value) -> Self {
        let coordinate = |key: &str| arguments[key].as_f64().unwrap_or(f64::NAN);
        match name {
            "computer_screenshot" => Self::Screenshot,
            "computer_move" | "computer_click" => Self::Point {
                x: coordinate("x"),
                y: coordinate("y"),
            },
            "computer_scroll" | "computer_type" | "computer_key" => Self::Gesture,
            _ => Self::Unknown,
        }
    }
}

enum Command {
    Action { id: u64, action: Action },
    Resume,
    Revoke,
}

fn parse_command(line: &[u8]) -> Option<Command> {
    let request: Value = serde_json::from_slice(line).ok()?;
    match request["command"].as_str()? {
        "revoke" => Some(Command::Revoke),
        "resume" => Some(Command::Resume),
        "action" => Some(Command::Action {
            id: request["id"].as_u64()?,
            action: Action::parse(request["name"].as_str()?, &request["arguments"]),
        }),
        _ => None,
    }
}

/// End of input, a malformed line and an oversized one all revoke: a lost wrapper
/// never leaves a lease running.
fn read_commands(gate: &Gate<Stdout>, jobs: &SyncSender<(Ticket, Action)>) {
    let mut input = BufReader::new(stdin());
    loop {
        let mut line = Vec::new();
        let read = input
            .by_ref()
            .take(MAX_COMMAND)
            .read_until(b'\n', &mut line);
        if !matches!(read, Ok(1..)) || line.last() != Some(&b'\n') {
            break;
        }
        match parse_command(&line) {
            Some(Command::Resume) => gate.resume(),
            Some(Command::Action { id, action }) => {
                if let Some(ticket) = gate.admit(id) {
                    if jobs.try_send((ticket, action)).is_err() {
                        let busy = "Une capture est déjà en cours.".to_owned();
                        gate.deliver(ticket, Reply::Refused(busy));
                    }
                }
            }
            Some(Command::Revoke) | None => break,
        }
    }
    gate.stop(Stop::Revoke);
}

#[derive(Clone, Copy)]
struct LastCapture {
    width: i32,
    height: i32,
    bounds: Bounds,
}

const SEE_ONLY: &str = "Capture seule sous Linux : aucun geste envoyé.";

fn answer(
    action: &Action,
    source: &mut Source,
    last: &mut Option<LastCapture>,
    give_up: &dyn Fn() -> bool,
) -> Reply {
    match *action {
        Action::Screenshot => {
            let captured = source.shot(give_up).and_then(|shot| {
                let image = encode(&shot)?;
                *last = Some(LastCapture {
                    width: image.width,
                    height: image.height,
                    bounds: shot.bounds,
                });
                Ok(image)
            });
            match captured {
                Ok(image) => Reply::Image(image),
                Err(refusal) => Reply::Refused(refusal.message().to_owned()),
            }
        }
        Action::Point { x, y } => Reply::Refused(match *last {
            None => "Prenez une nouvelle capture avant de déplacer la souris ou cliquer.".into(),
            Some(capture) => {
                match image_point(x, y, capture.width, capture.height, capture.bounds) {
                    None => "Coordonnées hors de la dernière capture. Utilisez ses dimensions en pixels.".into(),
                    Some(point) => format!(
                        "{SEE_ONLY} Point d’écran ({}, {}).",
                        point.x.round(),
                        point.y.round()
                    ),
                }
            }
        }),
        Action::Gesture => Reply::Refused(SEE_ONLY.into()),
        Action::Unknown => Reply::Refused("Outil d’écran inconnu.".into()),
    }
}

/// The source opens here and stays here: an X display and a D-Bus connection
/// belong to one thread, and a slow portal must not stall the overlay's buttons.
fn worker(queue: &Receiver<(Ticket, Action)>, gate: &Gate<Stdout>) {
    let mut source = match opened() {
        Ok(source) => source,
        Err(refusal) => {
            gate.abort(refusal.message());
            std::process::exit(1);
        }
    };
    gate.ready();
    let mut last = None;
    while let Ok((ticket, action)) = queue.recv() {
        let give_up = || !gate.current(ticket);
        gate.deliver(ticket, answer(&action, &mut source, &mut last, &give_up));
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refusal {
    NoDisplay,
    #[cfg(any(test, not(debug_assertions)))]
    WaylandNeedsPortal,
    #[cfg(any(test, not(debug_assertions)))]
    Portal,
    #[cfg(any(test, not(debug_assertions)))]
    Denied,
    Capture,
    TooLarge,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::NoDisplay => "Aucun affichage disponible pour partager l’écran.",
            #[cfg(any(test, not(debug_assertions)))]
            Self::WaylandNeedsPortal => {
                "Le partage d’écran fonctionne sous X11 sans compositeur, pas sous Wayland."
            }
            #[cfg(any(test, not(debug_assertions)))]
            Self::Portal => "Le portail du bureau n’a pas fourni de capture.",
            #[cfg(any(test, not(debug_assertions)))]
            Self::Denied => "Capture refusée ou annulée dans la fenêtre du système.",
            Self::Capture => "La capture de l’écran est indisponible.",
            Self::TooLarge => {
                "La capture dépasse la taille permise. Réduisez la résolution de l’écran."
            }
        }
    }
}

pub enum Source {
    #[cfg(debug_assertions)]
    Fixture { delay: Duration },
    #[cfg(any(test, not(debug_assertions)))]
    Portal(capture::Portal),
    #[cfg(any(test, not(debug_assertions)))]
    X11(capture::X11Root),
}

impl Source {
    fn shot(&mut self, give_up: &dyn Fn() -> bool) -> Result<Shot, Refusal> {
        if give_up() {
            return Err(Refusal::Capture);
        }
        match self {
            #[cfg(debug_assertions)]
            Self::Fixture { delay } => {
                let waited = Instant::now();
                while waited.elapsed() < *delay {
                    if give_up() {
                        return Err(Refusal::Capture);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                fixture()
            }
            #[cfg(any(test, not(debug_assertions)))]
            Self::Portal(portal) => portal.shot(give_up),
            #[cfg(any(test, not(debug_assertions)))]
            Self::X11(root) => root.shot(),
        }
    }
}

/// A drawn image, never a screen capture.
#[cfg(debug_assertions)]
fn fixture() -> Result<Shot, Refusal> {
    let png = include_bytes!("../../../native/screen-fixture.png");
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(png))
        .map_err(|_| Refusal::Capture)?;
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        w: f64::from(pixbuf.width()),
        h: f64::from(pixbuf.height()),
    };
    Ok(Shot { pixbuf, bounds })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parsed(arguments: &[&str]) -> Option<Invocation> {
        Invocation::parse(arguments.iter().map(OsString::from))
    }

    #[test]
    fn only_the_two_screen_options_leave_the_shell_path() {
        assert_eq!(parsed(&[]), None);
        assert_eq!(parsed(&["--data-dir", "/tmp/d"]), None);
        assert_eq!(
            parsed(&["--screen-permissions"]),
            Some(Invocation::Permissions)
        );
        assert_eq!(
            parsed(&["--screen-helper", "100"]),
            Some(Invocation::Helper {
                duration: Duration::from_millis(100)
            })
        );
        assert_eq!(
            parsed(&["--screen-helper", "900000"]),
            Some(Invocation::Helper {
                duration: Duration::from_secs(900)
            })
        );
    }

    #[test]
    fn a_duration_outside_the_lease_limits_is_a_usage_error() {
        for arguments in [
            &["--screen-helper"][..],
            &["--screen-helper", "99"],
            &["--screen-helper", "900001"],
            &["--screen-helper", "soon"],
            &["--screen-helper", "1000", "more"],
            &["--screen-permissions", "more"],
        ] {
            assert_eq!(parsed(arguments), Some(Invocation::Usage));
        }
    }

    #[test]
    fn only_the_sibling_wrapper_of_the_same_user_may_start_a_lease() {
        let wrapper = Path::new("/opt/ezerbot/grokbot-wrapper");
        assert!(parent_matches(wrapper, wrapper, 1000, 1000));
        assert!(!parent_matches(
            Path::new("/usr/bin/sh"),
            wrapper,
            1000,
            1000
        ));
        assert!(!parent_matches(wrapper, wrapper, 0, 1000));
    }
}
