//! Registration of `ezerbot://` through a desktop entry and `xdg-mime`.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{Unavailable, APPLICATION_ID};

const SCHEME_MIME: &str = "x-scheme-handler/ezerbot";

/// The Exec path is quoted only when it holds a reserved character: the generic path of
/// `xdg-open` (no desktop environment) does not unquote Exec, so a plain path keeps working
/// there. Quoting escapes `"`, `` ` ``, `$` and `\` with a backslash, and the file format then
/// doubles every backslash. `%` is doubled to stay literal.
pub fn desktop_entry(exec: &Path, icon: Option<&Path>) -> String {
    let exec = exec.to_string_lossy();
    let reserved = exec.chars().any(|c| " \t\n\"'\\><~|&;$*?#()`".contains(c));
    let mut value = String::new();
    for c in exec.chars() {
        match c {
            '"' | '`' | '$' => value.push_str(&format!("\\\\{c}")),
            '\\' => value.push_str("\\\\\\\\"),
            '%' => value.push_str("%%"),
            _ => value.push_str(&escape_value(c)),
        }
    }
    if reserved {
        value = format!("\"{value}\"");
    }
    let mut entry = format!(
        "[Desktop Entry]\nType=Application\nName=Ezerbot\nExec={value} %u\nTerminal=false\nMimeType={SCHEME_MIME};\nStartupWMClass=Ezerbot\n"
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

/// Writes the desktop entry and makes it the default handler of `ezerbot://`, then asks
/// `xdg-mime` whether it took. Safe to run again.
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
    let target = directory.join(&name);
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&temporary, desktop_entry(exec, icon))
        .and_then(|()| fs::rename(&temporary, &target))
        .map_err(|_| {
            let _ = fs::remove_file(&temporary);
            Unavailable
        })?;
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
