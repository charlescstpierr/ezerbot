use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    embed_commit();
    build_panels();
    compress_static_files();
}

fn build_panels() {
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

/// Gzip and Brotli variants of the served text files, keyed `ui/<name>` for
/// app/ui and `public/<name>` for app/web/public.
/// An empty `.gz` means gzip did not make the file smaller than the identity
/// bytes. An empty `.br` means Brotli did not beat the smallest other variant.
/// The host then serves what is left.
fn compress_static_files() {
    let app = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .join("../../app");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    for (directory, key) in [("ui", "ui"), ("web/public", "public")] {
        let source = app.join(directory);
        println!("cargo:rerun-if-changed={}", source.display());
        let mut files: Vec<_> = match fs::read_dir(&source) {
            Ok(entries) => entries
                .map(|entry| entry.expect("static directory entry").path())
                .filter(|path| path.is_file() && is_text_asset(path))
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("cannot read {}: {error}", source.display()),
        };
        files.sort();
        for path in files {
            println!("cargo:rerun-if-changed={}", path.display());
            let name = path.file_name().expect("file name").to_string_lossy();
            write_variants(
                &output,
                &format!("{key}/{name}"),
                &fs::read(&path).expect("static file"),
            );
        }
    }
    for name in ["panels.js", "panels.css"] {
        let bundle = fs::read(output.join(name)).expect("panel bundle");
        write_variants(&output, name, &bundle);
    }
}

fn is_text_asset(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("js" | "css" | "html" | "webmanifest")
    )
}

fn write_variants(output: &std::path::Path, relative: &str, bytes: &[u8]) {
    let mut gzip = gzip(bytes);
    let mut smallest = bytes.len();
    if gzip.len() < smallest {
        smallest = gzip.len();
    } else {
        gzip.clear();
    }
    let mut brotli = brotli(bytes);
    if brotli.len() >= smallest {
        brotli.clear();
    }
    write_variant(output, "gzip", "gz", relative, &gzip);
    write_variant(output, "brotli", "br", relative, &brotli);
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes).expect("gzip static file");
    encoder.finish().expect("finish gzip")
}

fn brotli(bytes: &[u8]) -> Vec<u8> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: 24,
        mode: brotli::enc::backward_references::BrotliEncoderMode::BROTLI_MODE_TEXT,
        size_hint: bytes.len(),
        ..Default::default()
    };
    let mut compressed = Vec::new();
    brotli::BrotliCompress(&mut &bytes[..], &mut compressed, &params).expect("brotli static file");
    compressed
}

fn write_variant(
    output: &std::path::Path,
    directory: &str,
    extension: &str,
    relative: &str,
    bytes: &[u8],
) {
    let target = output
        .join(directory)
        .join(format!("{relative}.{extension}"));
    fs::create_dir_all(target.parent().expect("variant directory"))
        .expect("create variant directory");
    fs::write(target, bytes).expect("write variant file");
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
