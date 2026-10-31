//! `xdg-open` without a shell. Wrapper requests are handled one at a time and a generic
//! `xdg-open` can run as long as the application it starts, so `run` waits only briefly.
use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use crate::Unavailable;

const GRACE: Duration = Duration::from_millis(1500);

pub fn command(target: &OsStr) -> Command {
    let mut command = Command::new("xdg-open");
    // A relative name starting with `-` would be read as an option.
    if target.as_bytes().first() == Some(&b'-') {
        let mut path = OsString::from("./").into_vec();
        path.extend_from_slice(target.as_bytes());
        command.arg(OsString::from_vec(path));
    } else {
        command.arg(target);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Ok once the handler exited successfully or is still running after the grace period.
pub fn run(mut command: Command) -> Result<(), Unavailable> {
    let mut child = command.spawn().map_err(|_| Unavailable)?;
    let deadline = Instant::now() + GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success().then_some(()).ok_or(Unavailable),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                reap(child);
                return Ok(());
            }
            Err(_) => return Err(Unavailable),
        }
    }
}

pub fn launch(target: &OsStr) -> Result<(), Unavailable> {
    run(command(target))
}

fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}
