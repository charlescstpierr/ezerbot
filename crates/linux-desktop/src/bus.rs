use std::time::Duration;

use zbus::blocking::{connection::Builder, Connection};

pub const APPLICATION_ID: &str = "ca.ezerbot.Ezerbot";

/// Carries no OS text, so neither a secret nor a path can leak through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unavailable;

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("service du bureau indisponible")
    }
}
impl std::error::Error for Unavailable {}

/// Which bus a service talks to. The login session bus only exists in release builds:
/// a debug or test binary can reach nothing but a throwaway bus named explicitly.
#[derive(Clone)]
pub enum Bus {
    #[cfg(not(debug_assertions))]
    Session,
    #[cfg(debug_assertions)]
    Private(String),
    /// A malformed override must never fall through to another bus.
    #[cfg(debug_assertions)]
    Refused,
}

impl Bus {
    /// Release: the session bus. Debug: nothing without `GROKBOT_TEST_SESSION_BUS`; with it,
    /// only `unix:path=<socket>` under a HOME that holds `.ezerbot-test-home`.
    pub fn for_app() -> Option<Self> {
        #[cfg(not(debug_assertions))]
        return Some(Self::Session);
        #[cfg(debug_assertions)]
        {
            let raw = std::env::var_os("GROKBOT_TEST_SESSION_BUS")?;
            Some(checked_override(&raw).map_or(Self::Refused, Self::Private))
        }
    }

    #[cfg(debug_assertions)]
    pub fn private(address: impl Into<String>) -> Self {
        Self::Private(address.into())
    }

    pub(crate) fn connect(&self) -> Result<Connection, Unavailable> {
        let builder = match self {
            #[cfg(not(debug_assertions))]
            Self::Session => Builder::session(),
            #[cfg(debug_assertions)]
            Self::Private(address) => Builder::address(address.as_str()),
            #[cfg(debug_assertions)]
            Self::Refused => return Err(Unavailable),
        };
        builder
            .and_then(|builder| builder.method_timeout(Duration::from_secs(5)).build())
            .map_err(|_| Unavailable)
    }
}

#[cfg(debug_assertions)]
fn checked_override(raw: &std::ffi::OsStr) -> Option<String> {
    use std::path::Path;
    let home = std::fs::canonicalize(std::env::var_os("HOME")?).ok()?;
    if !home.join(".ezerbot-test-home").is_file() {
        return None;
    }
    let socket = Path::new(raw.to_str()?.strip_prefix("unix:path=")?);
    // A comma or semicolon would append transport options or another address.
    if !socket.is_absolute() || raw.to_str()?.contains([',', ';']) {
        return None;
    }
    let parent = std::fs::canonicalize(socket.parent()?).ok()?;
    let socket = parent.join(socket.file_name()?);
    if !socket.starts_with(&home) {
        return None;
    }
    Some(format!("unix:path={}", socket.to_str()?))
}
