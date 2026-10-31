#![cfg(debug_assertions)]
use grokbot_linux_desktop::scheme::desktop_entry;
use std::path::Path;

/// Reads an `Exec` value the way the desktop entry specification says: string escapes first,
/// then the quoting rules of the Exec key.
fn exec_arguments(entry: &str) -> Vec<String> {
    let raw = entry
        .lines()
        .find_map(|line| line.strip_prefix("Exec="))
        .unwrap();
    let mut unescaped = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            unescaped.push(c);
            continue;
        }
        unescaped.push(match chars.next().unwrap() {
            's' => ' ',
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '\\' => '\\',
            other => panic!("invalid string escape \\{other}"),
        });
    }
    let (mut arguments, mut current, mut quoted, mut started) =
        (vec![], String::new(), false, false);
    let mut chars = unescaped.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => (quoted, started) = (!quoted, true),
            '\\' if quoted => current.push(chars.next().unwrap()),
            '%' if chars.peek() == Some(&'%') => {
                chars.next();
                current.push('%');
            }
            ' ' if !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        arguments.push(current);
    }
    arguments
}

#[test]
fn the_exec_path_survives_every_reserved_character_as_one_argument() {
    let path = "/opt/Mon Ezerbot/\"bin\" `x` $HOME 100%\\z\n\tend/ezerbot";
    let entry = desktop_entry(Path::new(path), None);
    assert_eq!(exec_arguments(&entry), [path, "%u"]);
    assert!(entry.contains("MimeType=x-scheme-handler/ezerbot;\n"));
    assert!(entry.contains("Terminal=false\n"));
    assert!(entry.contains("StartupWMClass=Ezerbot\n"));
    assert!(entry.contains("Name=Ezerbot\n"));
    assert!(!entry.contains("NoDisplay"));
}

#[test]
fn the_percent_sign_is_literal_the_icon_is_plain_and_only_reserved_paths_are_quoted() {
    let entry = desktop_entry(
        Path::new("/opt/100%/ezerbot"),
        Some(Path::new("/icônes/a\\b.png")),
    );
    assert!(entry.contains("Exec=/opt/100%%/ezerbot %u\n"));
    assert!(entry.contains("Icon=/icônes/a\\\\b.png\n"));
    assert!(!desktop_entry(Path::new("/x"), None).contains("Icon="));
    let spaced = desktop_entry(Path::new("/opt/Mon Ezerbot/ezerbot"), None);
    assert!(spaced.contains("Exec=\"/opt/Mon Ezerbot/ezerbot\" %u\n"));
}
