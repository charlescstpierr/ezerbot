//! Garde-fou de #443 : un test qui pose un dossier ou une base directement sous `$TMPDIR` la
//! fait fuir au moindre échec, et fait balayer tout `$TMPDIR` par `Vault::open`. Tout passe par
//! `grokbot_test_support::TestDir`.

use std::fs;
use std::path::{Path, PathBuf};

/// Fichiers dont un autre ticket est propriétaire ; à retirer d'ici quand ils passent à `TestDir`.
const PENDING: &[&str] = &[
    "crates/wrapper/tests/connectors.rs",
    "crates/wrapper/tests/state_scale.rs",
    "crates/wrapper/tests/chat_reliability.rs",
];

/// Racines parcourues, relatives au dépôt.
const ROOTS: &[&str] = &["crates", "app/linux"];

fn rust_files(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn no_test_builds_a_path_directly_under_the_system_temp_dir() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for root in ROOTS {
        rust_files(&repository.join(root), &mut files);
    }
    let mut offenders = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&repository)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative.starts_with("crates/test-support/") || PENDING.contains(&relative.as_str()) {
            continue;
        }
        let source: String = fs::read_to_string(&file)
            .unwrap()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if source.contains("temp_dir().join(") {
            offenders.push(relative);
        }
    }
    assert!(
        offenders.is_empty(),
        "use grokbot_test_support::TestDir instead of std::env::temp_dir().join(..) in: {offenders:?}"
    );
}
