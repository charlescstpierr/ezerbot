#![cfg(all(target_os = "linux", debug_assertions))]
use grokbot_linux_desktop::{
    scheme::{desktop_entry, register},
    Unavailable,
};
use std::path::Path;

#[test]
fn the_entry_names_the_application_and_escapes_the_icon() {
    let entry = desktop_entry(
        Path::new("/opt/ezerbot"),
        Some(Path::new("/icônes/a\\b.png")),
    );
    assert!(entry.contains("Exec=/opt/ezerbot %u\n"));
    assert!(entry.contains("Icon=/icônes/a\\\\b.png\n"));
    assert!(entry.contains("MimeType=x-scheme-handler/ezerbot;\n"));
    assert!(entry.contains("Terminal=false\n"));
    assert!(entry.contains("StartupWMClass=Ezerbot\n"));
    assert!(entry.contains("Name=Ezerbot\n"));
    assert!(!entry.contains("NoDisplay"));
    assert!(!desktop_entry(Path::new("/x"), None).contains("Icon="));
}

#[test]
fn a_relative_executable_is_refused_before_anything_is_written() {
    assert_eq!(register(Path::new("bin/ezerbot"), None), Err(Unavailable));
}
