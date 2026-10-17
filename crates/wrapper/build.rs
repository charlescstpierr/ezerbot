use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
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
    for (extension, separator) in [("js", "\n;\n"), ("css", "\n")] {
        let mut bundle = String::new();
        for path in &sources {
            if path.extension().and_then(|value| value.to_str()) == Some(extension) {
                println!("cargo:rerun-if-changed={}", path.display());
                bundle.push_str(&fs::read_to_string(path).expect("UTF-8 panel source"));
                // Keep trailing comments and adjacent JS expressions in separate scripts.
                bundle.push_str(separator);
            }
        }
        fs::write(output.join(format!("panels.{extension}")), bundle).expect("write panel bundle");
    }
}
