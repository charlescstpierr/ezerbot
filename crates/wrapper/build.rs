use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    embed_commit();
    let panels = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .join("../../app/ui/panels");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    println!("cargo:rerun-if-changed={}", panels.display());
    let mut sources: Vec<_> = match fs::read_dir(&panels) {
        Ok(entries) => entries
            .map(|entry| entry.expect("panel directory entry").path())
            .filter(|path| path.is_file())
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("cannot read {}: {error}", panels.display()),
    };
    sources.sort();
    for extension in ["js", "css"] {
        let mut bundle = String::new();
        for path in &sources {
            if path.extension().and_then(|value| value.to_str()) == Some(extension) {
                println!("cargo:rerun-if-changed={}", path.display());
                let source = fs::read_to_string(path).expect("UTF-8 panel source");
                if extension == "js" {
                    // Each panel has its own declarations and strict-mode scope.
                    bundle.push_str(";(function () {\n");
                    bundle.push_str(&source);
                    bundle.push_str("\n})();\n");
                } else {
                    bundle.push_str(&source);
                    bundle.push('\n');
                }
            }
        }
        fs::write(output.join(format!("panels.{extension}")), bundle).expect("write panel bundle");
    }
}

fn embed_commit() {
    println!("cargo:rerun-if-env-changed=EZERBOT_GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=EZERBOT_GIT_COMMIT_UNIX");
    let root =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory")).join("../..");
    let head = root.join(".git/HEAD");
    if head.exists() {
        println!("cargo:rerun-if-changed={}", head.display());
    }
    let sha = env::var("EZERBOT_GIT_COMMIT")
        .ok()
        .filter(|value| is_sha(value))
        .or_else(|| git(&root, &["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "inconnu".into());
    let unix = env::var("EZERBOT_GIT_COMMIT_UNIX")
        .ok()
        .filter(|value| {
            !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) && value != "0"
        })
        .or_else(|| git(&root, &["log", "-1", "--format=%ct"]))
        .unwrap_or_else(|| "0".into());
    println!("cargo:rustc-env=EZERBOT_GIT_COMMIT={sha}");
    println!("cargo:rustc-env=EZERBOT_GIT_COMMIT_UNIX={unix}");
}

fn is_sha(value: &str) -> bool {
    (7..=40).contains(&value.len()) && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn git(root: &std::path::Path, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(root);
    command.args(args);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
