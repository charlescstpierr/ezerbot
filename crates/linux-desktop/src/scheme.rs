//! Registration of `ezerbot://` through a desktop entry and `xdg-mime`.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{Unavailable, APPLICATION_ID};

const SCHEME_MIME: &str = "x-scheme-handler/ezerbot";

/// Whether a path can stand as the first word of `Exec` exactly as written. The specification
/// wants a reserved character quoted and `%` doubled, but the generic path of `xdg-open` undoes
/// neither: no spelling suits both, so any other path goes through a launcher. It also starts
/// the program with `env`, which takes a word holding `=` for a variable.
fn plain(path: &Path) -> bool {
    path.to_str().is_some_and(|text| {
        !text
            .chars()
            .any(|c| c.is_control() || " \"'\\><~|&;$*?#()`%=".contains(c))
    })
}

fn launcher(exec: &Path) -> String {
    let quoted = exec.to_string_lossy().replace('\'', "'\\''");
    format!("#!/bin/sh\nexec '{quoted}' \"$@\"\n")
}

/// `exec` must be [`plain`], which is why it needs no escaping here. The entry only carries the
/// link association: the menu shortcut is the installer's, and one written by a build would point
/// at the checkout.
pub fn desktop_entry(exec: &Path, icon: Option<&Path>) -> String {
    let mut entry = format!(
        "[Desktop Entry]\nType=Application\nName=Ezerbot\nExec={} %u\nTerminal=false\nNoDisplay=true\nMimeType={SCHEME_MIME};\nStartupWMClass=Ezerbot\n",
        exec.display()
    );
    if let Some(icon) = icon {
        let value: String = icon.to_string_lossy().chars().map(escape_value).collect();
        entry.push_str(&format!("Icon={value}\n"));
    }
    entry
}

fn escape_value(c: char) -> String {
    match c {
        '\\' => "\\\\".into(),
        '\n' => "\\n".into(),
        '\t' => "\\t".into(),
        '\r' => "\\r".into(),
        c if c.is_control() => String::new(),
        c => c.into(),
    }
}

fn data_home() -> Option<PathBuf> {
    let set = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    set("XDG_DATA_HOME").or_else(|| set("HOME").map(|home| home.join(".local/share")))
}

fn install(target: &Path, content: &str, mode: u32) -> Result<(), Unavailable> {
    let name = target.file_name().ok_or(Unavailable)?.to_string_lossy();
    let temporary = target.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&temporary, content)
        .and_then(|()| fs::set_permissions(&temporary, fs::Permissions::from_mode(mode)))
        .and_then(|()| fs::rename(&temporary, target))
        .map_err(|_| {
            let _ = fs::remove_file(&temporary);
            Unavailable
        })
}

/// Writes the desktop entry and makes it the default handler of `ezerbot://`, then asks
/// `xdg-mime` whether it took. An `exec` that is not [`plain`] gets a launcher next to the entry;
/// it is refused when that folder is not plain either. Safe to run again.
pub fn register(exec: &Path, icon: Option<&Path>) -> Result<PathBuf, Unavailable> {
    let controls = |path: &Path| {
        path.to_str()
            .is_none_or(|text| text.chars().any(char::is_control))
    };
    if !exec.is_absolute() || controls(exec) || icon.is_some_and(controls) {
        return Err(Unavailable);
    }
    let directory = data_home().ok_or(Unavailable)?.join("applications");
    fs::create_dir_all(&directory).map_err(|_| Unavailable)?;
    let name = format!("{APPLICATION_ID}.desktop");
    let script = directory.join(APPLICATION_ID);
    let program = if plain(exec) {
        exec
    } else if plain(&script) {
        install(&script, &launcher(exec), 0o755)?;
        &script
    } else {
        return Err(Unavailable);
    };
    let target = directory.join(&name);
    install(&target, &desktop_entry(program, icon), 0o644)?;
    // Only refreshes the MIME cache: the default below does not depend on it.
    let _ = tool("update-desktop-database")
        .arg(&directory)
        .stdout(Stdio::null())
        .status();
    let set = tool("xdg-mime")
        .args(["default", &name, SCHEME_MIME])
        .stdout(Stdio::null())
        .status();
    if !set.is_ok_and(|status| status.success()) {
        return Err(Unavailable);
    }
    let answer = tool("xdg-mime")
        .args(["query", "default", SCHEME_MIME])
        .output()
        .map_err(|_| Unavailable)?;
    (String::from_utf8_lossy(&answer.stdout).trim() == name)
        .then_some(target)
        .ok_or(Unavailable)
}

fn tool(name: &str) -> Command {
    let mut command = Command::new(name);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    command
}
