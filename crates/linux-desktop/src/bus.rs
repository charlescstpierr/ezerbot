use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use zbus::blocking::{connection::Builder, Connection};

pub const APPLICATION_ID: &str = "ca.ezerbot.Ezerbot";
const CONNECT_LIMIT: Duration = Duration::from_secs(5);
const MAX_RUNNING: usize = 8;
static RUNNING: AtomicUsize = AtomicUsize::new(0);

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
        let bus = self.clone();
        bounded(CONNECT_LIMIT, move || {
            let builder = match &bus {
                #[cfg(not(debug_assertions))]
                Self::Session => Builder::session(),
                #[cfg(debug_assertions)]
                Self::Private(address) => Builder::address(address.as_str()),
                #[cfg(debug_assertions)]
                Self::Refused => return Err(Unavailable),
            };
            builder
                .and_then(|builder| builder.method_timeout(CONNECT_LIMIT).build())
                .map_err(|_| Unavailable)
        })
    }
}

struct Running;
impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Bounds what `method_timeout` does not cover: the bus authentication, and the wait for a
/// prompt's `Completed` signal. Neither can be cancelled from here, so a stuck thread stays
/// until its bus answers; the vault retries on every poll, hence the cap on such threads.
pub(crate) fn bounded<T: Send + 'static>(
    limit: Duration,
    work: impl FnOnce() -> Result<T, Unavailable> + Send + 'static,
) -> Result<T, Unavailable> {
    if RUNNING.fetch_add(1, Ordering::SeqCst) >= MAX_RUNNING {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
        return Err(Unavailable);
    }
    let running = Running;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _running = running;
        let _ = sender.send(work());
    });
    receiver.recv_timeout(limit).map_err(|_| Unavailable)?
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
    // The socket itself is resolved: a link under HOME could lead to the login session bus.
    let socket = std::fs::canonicalize(socket).ok()?;
    if !socket.starts_with(&home) {
        return None;
    }
    Some(format!("unix:path={}", socket.to_str()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stuck_work_is_abandoned_at_the_limit_and_never_piles_up() {
        let mut releases = Vec::new();
        for _ in 0..MAX_RUNNING {
            let (release, stuck) = mpsc::channel::<()>();
            releases.push(release);
            let outcome = bounded(Duration::from_millis(5), move || {
                let _ = stuck.recv();
                Ok(())
            });
            assert_eq!(outcome, Err(Unavailable));
        }
        let refused = std::time::Instant::now();
        assert_eq!(bounded(CONNECT_LIMIT, || Ok(7)), Err(Unavailable));
        assert!(refused.elapsed() < Duration::from_millis(500));
        drop(releases);
        while RUNNING.load(Ordering::SeqCst) != 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(bounded(CONNECT_LIMIT, || Ok(7)), Ok(7));
    }
}
