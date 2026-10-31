//! Desktop services of the Linux shell: the `ezerbot://` scheme with its single instance, and
//! `xdg-open`. D-Bus stays inside this crate: callers only see byte slices, strings and
//! `Unavailable`.
#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]

pub mod application;
mod bus;
pub mod open;
pub mod scheme;
#[cfg(all(feature = "test-support", debug_assertions))]
pub mod testing;

pub use bus::{Bus, Unavailable, APPLICATION_ID};
