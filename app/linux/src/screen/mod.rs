//! `ezerbot-linux --screen-helper`: the Linux side of the lease in `screen_native.rs`.
#[cfg(any(test, not(debug_assertions)))]
pub mod capture;
#[cfg(all(test, debug_assertions))]
mod fake_portal;
pub mod gate;
pub mod image;
#[cfg(test)]
#[path = "../../tests/support/xvfb.rs"]
mod xvfb;

use image::Shot;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refusal {
    NoDisplay,
    WaylandNeedsPortal,
    Portal,
    Denied,
    Capture,
    TooLarge,
}

pub enum Source {
    #[cfg(any(test, not(debug_assertions)))]
    Portal(capture::Portal),
    #[cfg(any(test, not(debug_assertions)))]
    X11(capture::X11Root),
}

impl Source {
    fn shot(&mut self, give_up: &dyn Fn() -> bool) -> Result<Shot, Refusal> {
        match self {
            #[cfg(any(test, not(debug_assertions)))]
            Self::Portal(portal) => portal.shot(give_up),
            #[cfg(any(test, not(debug_assertions)))]
            Self::X11(root) => root.shot(),
        }
    }
}
