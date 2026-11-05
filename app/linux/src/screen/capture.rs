//! Real screen sources. Compiled in release and in the unit test harness only:
//! a debug binary holds no capture code, and only `Env::read` touches the session.
use super::{
    image::{Bounds, Shot},
    Refusal, Source,
};
use gtk::{
    gdk_pixbuf::{Colorspace, Pixbuf},
    glib::Bytes,
};
use std::{ffi::CString, os::raw::c_ulong, sync::Once};
use x11_dl::xlib::{self, Xlib};

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

pub struct XDisplay(CString);

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
            x,
        }
    }
}

/// Connects without capturing. XWayland's root shows only X clients, so Wayland
/// never falls back to it: the model would receive a false desktop.
pub fn open(env: &Env) -> Result<Source, Refusal> {
    match env.display {
        Display::None => Err(Refusal::NoDisplay),
        Display::Wayland => Err(Refusal::Wayland),
        Display::X11 => X11Root::open(env.x.as_ref().ok_or(Refusal::NoDisplay)?).map(Source::X11),
    }
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
    use std::io::Cursor;

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
    fn only_a_bare_x11_session_opens_a_source() {
        let server = Xvfb::start("640x480x24");
        let env = |display| Env {
            display,
            x: Some(XDisplay::private(&server.name)),
        };
        assert!(matches!(open(&env(Display::X11)), Ok(Source::X11(_))));
        assert!(matches!(
            open(&env(Display::Wayland)),
            Err(Refusal::Wayland)
        ));
        assert!(matches!(open(&env(Display::None)), Err(Refusal::NoDisplay)));
        let without_x = Env {
            display: Display::X11,
            x: None,
        };
        assert!(matches!(open(&without_x), Err(Refusal::NoDisplay)));
    }

    #[test]
    fn a_pixel_scales_whatever_the_channel_depth() {
        assert_eq!(channel(0xff0000, 0xff0000), 255);
        assert_eq!(channel(0x123456, 0xff00), 0x34);
        assert_eq!(channel(0xf800, 0xf800), 255);
        assert_eq!(channel(0x1234, 0), 0);
    }
}
