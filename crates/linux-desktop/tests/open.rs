#![cfg(debug_assertions)]
use grokbot_linux_desktop::{
    open,
    testing::{recording_handler, tools, wait_for, Home},
    Unavailable,
};
use std::{ffi::OsStr, fs, os::unix::fs::PermissionsExt, process::Command, time::Instant};

/// A home whose only handler of `x-scheme-handler/ezertest` is a script that records its argument.
fn handler_home(
    handler: impl Fn(&std::path::Path) -> std::path::PathBuf,
) -> Option<(Home, std::path::PathBuf)> {
    if !tools(&["xdg-open", "xdg-mime"]) {
        return None;
    }
    let home = Home::new("ezo");
    let log = home.path().join("received.log");
    let script = handler(home.path());
    let applications = home.path().join("data/applications");
    fs::create_dir_all(&applications).unwrap();
    fs::write(
        applications.join("ezertest.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Test\nExec={} %u\nMimeType=x-scheme-handler/ezertest;\n",
            script.display()
        ),
    )
    .unwrap();
    let mut mime = Command::new("xdg-mime");
    home.env(&mut mime)
        .args(["default", "ezertest.desktop", "x-scheme-handler/ezertest"]);
    assert!(mime.status().unwrap().success());
    Some((home, log))
}

#[test]
fn xdg_open_hands_the_exact_target_to_the_registered_handler() {
    let Some((home, log)) = handler_home(|dir| recording_handler(dir, &dir.join("received.log")))
    else {
        return;
    };
    let target = "ezertest://app/v1/thread?id=a%20b&x=1";
    let mut command = open::command(OsStr::new(target));
    home.env(&mut command);
    open::run(command).unwrap();
    wait_for("the handler to record the target", || {
        fs::read_to_string(&log).is_ok_and(|text| text == format!("{target}\n"))
    });
}

#[test]
fn a_handler_that_keeps_running_does_not_hold_the_caller() {
    let Some((home, _log)) = handler_home(|dir| {
        let script = dir.join("slow.sh");
        fs::write(&script, "#!/bin/sh\nsleep 6\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }) else {
        return;
    };
    let mut command = open::command(OsStr::new("ezertest://slow"));
    home.env(&mut command);
    let started = Instant::now();
    open::run(command).unwrap();
    assert!(started.elapsed().as_secs() < 4, "waited for the handler");
}

#[test]
fn a_target_without_a_handler_or_without_xdg_open_is_unavailable() {
    let Some((home, _log)) = handler_home(|dir| dir.join("unused.sh")) else {
        return;
    };
    let mut failing = open::command(OsStr::new("ezerother://nobody"));
    home.env(&mut failing);
    assert_eq!(open::run(failing), Err(Unavailable));
    assert_eq!(
        open::run(Command::new("/nonexistent/xdg-open")),
        Err(Unavailable)
    );
}

#[test]
fn a_target_starting_with_a_dash_is_a_path_never_an_option() {
    let command = open::command(OsStr::new("--manual"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [OsStr::new("./--manual")]
    );
}
