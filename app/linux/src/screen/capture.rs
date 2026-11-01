//! Real screen sources. Compiled in release and in the unit test harness only:
//! a debug binary holds no capture code, and only `Env::read` touches the session.
use super::{
    image::{Bounds, Shot},
    Refusal, Source,
};
use dbus::{
    arg::{PropMap, RefArg, Variant},
    blocking::{stdintf::org_freedesktop_dbus::Properties, Connection},
    channel::Channel,
    message::MatchRule,
    Message, Path,
};
use gtk::{
    gdk_pixbuf::{Colorspace, Pixbuf},
    glib::Bytes,
};
use nix::fcntl::OFlag;
use std::{
    ffi::CString,
    fs::{self, OpenOptions},
    io::{Cursor, Read},
    os::{raw::c_ulong, unix::fs::OpenOptionsExt},
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver},
        Once,
    },
    time::{Duration, Instant},
};
use x11_dl::xlib::{self, Xlib};

pub(super) const DESKTOP: &str = "org.freedesktop.portal.Desktop";
pub(super) const OBJECT: &str = "/org/freedesktop/portal/desktop";
pub(super) const REQUEST: &str = "org.freedesktop.portal.Request";
const SCREENSHOT: &str = "org.freedesktop.portal.Screenshot";
const PROBE_LIMIT: Duration = Duration::from_secs(1);
const CALL_LIMIT: Duration = Duration::from_secs(5);
// The whole request, its call and the closing PROBE_LIMIT included, stays under
// the 20 s of Client::action: a slow first consent fails one call, not the lease.
const ANSWER_LIMIT: Duration = Duration::from_secs(17);
const FILE_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Display {
    Wayland,
    X11,
    None,
}

impl Display {
    pub fn of_session(wayland: bool, session_type: Option<&str>, x11: bool) -> Self {
        if wayland || session_type == Some("wayland") {
            Self::Wayland
        } else if x11 {
            Self::X11
        } else {
            Self::None
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Route {
    Portal,
    X11Root,
}

/// X11 needs no portal, and no portal file to read and remove from the person's
/// folders. XWayland's root shows only X clients, so Wayland never falls back to
/// it: the model would receive a false desktop.
pub fn route(display: Display, portal: bool) -> Result<Route, Refusal> {
    match (display, portal) {
        (Display::None, _) => Err(Refusal::NoDisplay),
        (Display::X11, _) => Ok(Route::X11Root),
        (Display::Wayland, true) => Ok(Route::Portal),
        (Display::Wayland, false) => Err(Refusal::WaylandNeedsPortal),
    }
}

pub struct BusAddress(String);
pub struct XDisplay(CString);

impl BusAddress {
    #[cfg(not(debug_assertions))]
    fn session() -> Option<Self> {
        let runtime = std::env::var("XDG_RUNTIME_DIR").ok();
        Self::of_session(std::env::var("DBUS_SESSION_BUS_ADDRESS").ok(), runtime)
    }

    /// The same order as libdbus: a session started by systemd may leave only
    /// its socket, and a missing address must not read as a missing portal.
    fn of_session(listed: Option<String>, runtime: Option<String>) -> Option<Self> {
        let socket = || {
            let path = std::path::Path::new(&runtime?).join("bus");
            path.exists()
                .then(|| format!("unix:path={}", path.display()))
        };
        listed
            .filter(|address| !address.is_empty())
            .or_else(socket)
            .map(Self)
    }

    #[cfg(test)]
    pub fn private(address: String) -> Self {
        Self(address)
    }
}

impl XDisplay {
    #[cfg(not(debug_assertions))]
    fn session() -> Option<Self> {
        let name = std::env::var("DISPLAY").ok()?;
        CString::new(name).ok().map(Self)
    }

    #[cfg(test)]
    pub fn private(name: &str) -> Self {
        Self(CString::new(name).unwrap())
    }
}

pub struct Env {
    pub display: Display,
    pub bus: Option<BusAddress>,
    pub x: Option<XDisplay>,
}

impl Env {
    #[cfg(not(debug_assertions))]
    pub fn read() -> Self {
        let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        let session = std::env::var("XDG_SESSION_TYPE").ok();
        let x = XDisplay::session();
        Self {
            display: Display::of_session(set("WAYLAND_DISPLAY"), session.as_deref(), x.is_some()),
            bus: BusAddress::session(),
            x,
        }
    }
}

/// Connects without capturing. It never opens a dialog, so the settings route
/// can call it too.
pub fn open(env: &Env) -> Result<Source, Refusal> {
    let portal = env
        .bus
        .as_ref()
        .and_then(|bus| Portal::open(bus).ok())
        .filter(Portal::available);
    match (route(env.display, portal.is_some())?, portal) {
        (Route::Portal, Some(portal)) => Ok(Source::Portal(portal)),
        _ => X11Root::open(env.x.as_ref().ok_or(Refusal::NoDisplay)?).map(Source::X11),
    }
}

pub struct Portal {
    conn: Connection,
    sender: String,
    next: u64,
    pub(super) limit: Duration,
    /// The only folders a portal answer may name: the backends save there, and
    /// nothing outside them is ever read or removed.
    pub(super) captures: Vec<PathBuf>,
}

impl Portal {
    pub fn open(address: &BusAddress) -> Result<Self, Refusal> {
        let mut channel = Channel::open_private(&address.0).map_err(|_| Refusal::Portal)?;
        channel.register().map_err(|_| Refusal::Portal)?;
        let conn = Connection::from(channel);
        let sender = conn.unique_name().replace(':', "").replace('.', "_");
        Ok(Self {
            conn,
            sender,
            next: 0,
            limit: ANSWER_LIMIT,
            captures: capture_folders(),
        })
    }

    fn available(&self) -> bool {
        self.conn
            .with_proxy(DESKTOP, OBJECT, PROBE_LIMIT)
            .get::<u32>(SCREENSHOT, "version")
            .is_ok()
    }

    fn owner(&self) -> Result<String, Refusal> {
        let bus = "org.freedesktop.DBus";
        let (owner,): (String,) = self
            .conn
            .with_proxy(bus, "/org/freedesktop/DBus", PROBE_LIMIT)
            .method_call(bus, "GetNameOwner", (DESKTOP,))
            .map_err(|_| Refusal::Portal)?;
        Ok(owner)
    }

    /// The handle is computed, not received: the match exists before the call,
    /// so a portal that answers before its method return is still heard.
    pub fn shot(&mut self, give_up: &dyn Fn() -> bool) -> Result<Shot, Refusal> {
        self.next += 1;
        let token = format!("ezerbot{}", self.next);
        let handle = format!("{OBJECT}/request/{}/{token}", self.sender);
        let path = Path::new(handle).map_err(|_| Refusal::Portal)?;
        let (answer, answers) = mpsc::channel();
        // Any client can address a signal to us, and a well-known sender name
        // matches them all: only the name's current owner is the portal.
        let rule = MatchRule::new_signal(REQUEST, "Response")
            .with_path(path.clone())
            .with_strict_sender(self.owner()?);
        let listening = self
            .conn
            .add_match(
                rule,
                move |(code, results): (u32, PropMap), _: &Connection, _: &Message| {
                    let uri = results.get("uri").and_then(|value| value.0.as_str());
                    let _ = answer.send((code, uri.map(str::to_owned)));
                    true
                },
            )
            .map_err(|_| Refusal::Portal)?;
        let outcome = self.ask(&token, &path, &answers, give_up);
        let _ = self.conn.remove_match(listening);
        let pixbuf = take_image(&outcome?, &self.captures)?;
        let bounds = Bounds {
            x: 0.0,
            y: 0.0,
            w: f64::from(pixbuf.width()),
            h: f64::from(pixbuf.height()),
        };
        Ok(Shot { pixbuf, bounds })
    }

    fn ask(
        &self,
        token: &str,
        path: &Path<'static>,
        answers: &Receiver<(u32, Option<String>)>,
        give_up: &dyn Fn() -> bool,
    ) -> Result<String, Refusal> {
        let deadline = Instant::now() + self.limit;
        let mut options = PropMap::new();
        options.insert("handle_token".into(), Variant(Box::new(token.to_owned())));
        options.insert("interactive".into(), Variant(Box::new(false)));
        options.insert("modal".into(), Variant(Box::new(false)));
        let (handle,): (Path,) = self
            .conn
            .with_proxy(DESKTOP, OBJECT, CALL_LIMIT)
            .method_call(SCREENSHOT, "Screenshot", ("", options))
            .map_err(|_| Refusal::Portal)?;
        if handle != *path {
            return Err(Refusal::Portal);
        }
        loop {
            match answers.try_recv() {
                Ok((0, Some(uri))) => return Ok(uri),
                Ok((0, None)) => return Err(Refusal::Portal),
                Ok(_) => return Err(Refusal::Denied),
                Err(_) => {}
            }
            if give_up() || Instant::now() >= deadline {
                let _: Result<(), _> = self
                    .conn
                    .with_proxy(DESKTOP, path.clone(), PROBE_LIMIT)
                    .method_call(REQUEST, "Close", ());
                return Err(Refusal::Portal);
            }
            self.conn
                .process(Duration::from_millis(100))
                .map_err(|_| Refusal::Portal)?;
        }
    }
}

/// Where the screenshot backends save: Pictures, and the Screenshots folder
/// GNOME uses inside it.
fn capture_folders() -> Vec<PathBuf> {
    gtk::glib::user_special_dir(gtk::glib::UserDirectory::Pictures)
        .map(|pictures| vec![pictures.join("Screenshots"), pictures])
        .unwrap_or_default()
}

/// True when `path` names an entry directly inside one of `folders`, links resolved
/// on the folder side: the answer of whoever owns the portal name is not trusted.
fn inside(path: &std::path::Path, folders: &[PathBuf]) -> bool {
    let (Some(parent), Some(_)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let Ok(parent) = fs::canonicalize(parent) else {
        return false;
    };
    folders
        .iter()
        .filter_map(|folder| fs::canonicalize(folder).ok())
        .any(|folder| folder == parent)
}

/// Backends write into the person's folders, and a lease leaves no desktop
/// pixels behind. Only an image inside a capture folder is read, then removed.
fn take_image(uri: &str, captures: &[PathBuf]) -> Result<Pixbuf, Refusal> {
    let path = url::Url::parse(uri)
        .ok()
        .filter(|url| url.scheme() == "file")
        .and_then(|url| url.to_file_path().ok())
        .filter(|path| inside(path, captures))
        .ok_or(Refusal::Portal)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(&path)
        .map_err(|_| Refusal::Portal)?;
    if !file.metadata().is_ok_and(|meta| meta.is_file()) {
        return Err(Refusal::Portal);
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Refusal::Capture)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(Refusal::TooLarge);
    }
    let pixbuf = Pixbuf::from_read(Cursor::new(bytes)).map_err(|_| Refusal::Capture)?;
    fs::remove_file(&path).map_err(|_| Refusal::Capture)?;
    Ok(pixbuf)
}

pub struct X11Root {
    xlib: Box<Xlib>,
    display: *mut xlib::Display,
}

/// Must run before any other Xlib call of the process, GTK's included.
pub fn init_x_threads() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Ok(xlib) = Xlib::open() {
            unsafe { (xlib.XInitThreads)() };
        }
    });
}

impl X11Root {
    pub fn open(address: &XDisplay) -> Result<Self, Refusal> {
        init_x_threads();
        let xlib = Box::new(Xlib::open().map_err(|_| Refusal::NoDisplay)?);
        let display = unsafe { (xlib.XOpenDisplay)(address.0.as_ptr()) };
        if display.is_null() {
            return Err(Refusal::NoDisplay);
        }
        Ok(Self { xlib, display })
    }

    /// Asks the server: this connection's cached size goes stale when the
    /// resolution changes during a lease.
    unsafe fn size(&self, root: xlib::Window) -> Result<(i32, i32), Refusal> {
        let (mut parent, mut x, mut y) = (0, 0, 0);
        let (mut width, mut height, mut border, mut depth) = (0, 0, 0, 0);
        let known = (self.xlib.XGetGeometry)(
            self.display,
            root,
            &mut parent,
            &mut x,
            &mut y,
            &mut width,
            &mut height,
            &mut border,
            &mut depth,
        );
        if known == 0 {
            return Err(Refusal::Capture);
        }
        Ok((width as i32, height as i32))
    }

    pub fn shot(&mut self) -> Result<Shot, Refusal> {
        let (width, height, rgb) = unsafe {
            let x = &self.xlib;
            let root = (x.XDefaultRootWindow)(self.display);
            // A resize between these two requests would be a fatal BadMatch.
            (x.XGrabServer)(self.display);
            let size = self.size(root);
            let image = size.map(|(width, height)| {
                (x.XGetImage)(
                    self.display,
                    root,
                    0,
                    0,
                    width as u32,
                    height as u32,
                    !0,
                    xlib::ZPixmap,
                )
            });
            (x.XUngrabServer)(self.display);
            (x.XFlush)(self.display);
            let (width, height) = size?;
            let image = image?;
            if image.is_null() {
                return Err(Refusal::Capture);
            }
            let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
            let masks = [(*image).red_mask, (*image).green_mask, (*image).blue_mask];
            for y in 0..height {
                for x_position in 0..width {
                    let pixel = (x.XGetPixel)(image, x_position, y);
                    rgb.extend(masks.map(|mask| channel(pixel, mask)));
                }
            }
            (x.XDestroyImage)(image);
            (width, height, rgb)
        };
        let pixbuf = Pixbuf::from_bytes(
            &Bytes::from_owned(rgb),
            Colorspace::Rgb,
            false,
            8,
            width,
            height,
            width * 3,
        );
        let bounds = Bounds {
            x: 0.0,
            y: 0.0,
            w: f64::from(width),
            h: f64::from(height),
        };
        Ok(Shot { pixbuf, bounds })
    }
}

impl Drop for X11Root {
    fn drop(&mut self) {
        unsafe { (self.xlib.XCloseDisplay)(self.display) };
    }
}

fn channel(pixel: c_ulong, mask: c_ulong) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    (((pixel & mask) >> shift) * 255 / (mask >> shift)) as u8
}

#[cfg(test)]
mod tests {
    use super::super::{image::encode, xvfb::Xvfb};
    use super::*;
    use gtk::glib;

    struct Painter {
        xlib: Xlib,
        display: *mut xlib::Display,
    }

    impl Painter {
        fn on(name: &str) -> Self {
            let xlib = Xlib::open().unwrap();
            let address = CString::new(name).unwrap();
            let display = unsafe { (xlib.XOpenDisplay)(address.as_ptr()) };
            assert!(!display.is_null());
            Self { xlib, display }
        }

        fn shrink_screen(&self, width: i32, height: i32) {
            let randr = x11_dl::xrandr::Xrandr::open().unwrap();
            unsafe {
                let root = (self.xlib.XDefaultRootWindow)(self.display);
                let resources = (randr.XRRGetScreenResourcesCurrent)(self.display, root);
                let crtc = *(*resources).crtcs;
                let none = std::ptr::null_mut();
                (randr.XRRSetCrtcConfig)(self.display, resources, crtc, 0, 0, 0, 0, 1, none, 0);
                (randr.XRRSetScreenSize)(self.display, root, width, height, 100, 100);
                (randr.XRRFreeScreenResources)(resources);
                (self.xlib.XSync)(self.display, 0);
            }
        }

        /// The server paints a window's background by itself when it is mapped.
        fn rectangle(&self, x: i32, y: i32, size: u32, [red, green, blue]: [u8; 3]) {
            let color = u64::from(u32::from_be_bytes([0, red, green, blue]));
            unsafe {
                let root = (self.xlib.XDefaultRootWindow)(self.display);
                let window = (self.xlib.XCreateSimpleWindow)(
                    self.display,
                    root,
                    x,
                    y,
                    size,
                    size,
                    0,
                    0,
                    color,
                );
                (self.xlib.XMapWindow)(self.display, window);
                (self.xlib.XSync)(self.display, 0);
            }
        }
    }

    impl Drop for Painter {
        fn drop(&mut self) {
            unsafe { (self.xlib.XCloseDisplay)(self.display) };
        }
    }

    fn pixel(image: &Pixbuf, x: usize, y: usize) -> [u8; 3] {
        let bytes = image.read_pixel_bytes();
        let start = y * image.rowstride() as usize + x * image.n_channels() as usize;
        [bytes[start], bytes[start + 1], bytes[start + 2]]
    }

    fn decoded(data_url: &str) -> Pixbuf {
        let png = data_url.strip_prefix("data:image/png;base64,").unwrap();
        Pixbuf::from_read(Cursor::new(glib::base64_decode(png).to_vec())).unwrap()
    }

    #[test]
    fn xgetimage_reads_a_known_pattern() {
        const RED: [u8; 3] = [255, 0, 0];
        const GREEN: [u8; 3] = [0, 255, 0];
        const BLUE: [u8; 3] = [0, 0, 255];
        const PLUM: [u8; 3] = [18, 52, 86];
        let server = Xvfb::start("640x480x24");
        let painter = Painter::on(&server.name);
        painter.rectangle(10, 10, 100, RED);
        painter.rectangle(200, 10, 100, GREEN);
        painter.rectangle(10, 200, 100, BLUE);
        painter.rectangle(200, 200, 100, PLUM);
        let mut root = X11Root::open(&XDisplay::private(&server.name)).unwrap();
        let shot = root.shot().unwrap();
        assert_eq!((shot.pixbuf.width(), shot.pixbuf.height()), (640, 480));
        let encoded = encode(&shot).unwrap();
        for image in [shot.pixbuf, decoded(&encoded.data_url)] {
            assert_eq!(pixel(&image, 60, 60), RED);
            assert_eq!(pixel(&image, 250, 60), GREEN);
            assert_eq!(pixel(&image, 60, 250), BLUE);
            assert_eq!(pixel(&image, 250, 250), PLUM);
        }
    }

    #[test]
    fn a_large_screen_is_sent_at_1280_wide_and_keeps_its_bounds() {
        let server = Xvfb::start("2560x1440x24");
        let mut root = X11Root::open(&XDisplay::private(&server.name)).unwrap();
        let shot = root.shot().unwrap();
        let encoded = encode(&shot).unwrap();
        assert_eq!((encoded.width, encoded.height), (1280, 720));
        assert_eq!(shot.bounds.w, 2560.0);
        assert_eq!(shot.bounds.h, 1440.0);
        let image = decoded(&encoded.data_url);
        assert_eq!((image.width(), image.height()), (1280, 720));
    }

    #[test]
    fn a_resolution_change_during_a_lease_is_followed() {
        let server = Xvfb::start("1280x800x24");
        let mut root = X11Root::open(&XDisplay::private(&server.name)).unwrap();
        assert_eq!(root.shot().unwrap().bounds.w, 1280.0);
        Painter::on(&server.name).shrink_screen(640, 480);
        let shot = root.shot().unwrap();
        assert_eq!((shot.pixbuf.width(), shot.pixbuf.height()), (640, 480));
        assert_eq!((shot.bounds.w, shot.bounds.h), (640.0, 480.0));
    }

    #[test]
    fn a_display_that_does_not_exist_is_refused() {
        let missing = XDisplay::private(":59999");
        assert!(matches!(X11Root::open(&missing), Err(Refusal::NoDisplay)));
    }

    #[test]
    fn the_session_picks_the_display_and_wayland_wins() {
        for (wayland, kind, x11, expected) in [
            (true, None, true, Display::Wayland),
            (false, Some("wayland"), true, Display::Wayland),
            (false, Some("x11"), true, Display::X11),
            (false, None, true, Display::X11),
            (false, Some("tty"), false, Display::None),
        ] {
            assert_eq!(Display::of_session(wayland, kind, x11), expected);
        }
    }

    #[test]
    fn the_bus_falls_back_to_the_runtime_socket_like_libdbus() {
        let found = |listed: Option<&str>, runtime: Option<&std::path::Path>| {
            let runtime = runtime.map(|path| path.display().to_string());
            BusAddress::of_session(listed.map(str::to_owned), runtime).map(|bus| bus.0)
        };
        let home = grokbot_test_support::TestDir::new("ezerbot-bus");
        let directory = home.path();
        assert_eq!(
            found(Some("unix:path=/x"), None).as_deref(),
            Some("unix:path=/x")
        );
        assert_eq!(found(None, Some(directory)), None);
        assert_eq!(found(Some(""), None), None);
        std::fs::write(directory.join("bus"), "").unwrap();
        let socket = format!("unix:path={}", directory.join("bus").display());
        assert_eq!(found(None, Some(directory)), Some(socket.clone()));
        assert_eq!(found(Some(""), Some(directory)), Some(socket));
    }

    #[test]
    fn the_route_never_shows_a_false_desktop() {
        assert_eq!(route(Display::Wayland, true), Ok(Route::Portal));
        assert_eq!(route(Display::X11, true), Ok(Route::X11Root));
        assert_eq!(route(Display::X11, false), Ok(Route::X11Root));
        assert_eq!(
            route(Display::Wayland, false),
            Err(Refusal::WaylandNeedsPortal)
        );
        assert_eq!(route(Display::None, true), Err(Refusal::NoDisplay));
        assert_eq!(route(Display::None, false), Err(Refusal::NoDisplay));
    }

    #[test]
    fn a_pixel_scales_whatever_the_channel_depth() {
        assert_eq!(channel(0xff0000, 0xff0000), 255);
        assert_eq!(channel(0x123456, 0xff00), 0x34);
        assert_eq!(channel(0xf800, 0xf800), 255);
        assert_eq!(channel(0x1234, 0), 0);
    }
}
