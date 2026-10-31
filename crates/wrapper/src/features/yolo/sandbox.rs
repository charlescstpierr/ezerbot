//! Bac à sable du shell YOLO sous Linux : bubblewrap recouvre les racines protégées.
use crate::ApiError;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

const BWRAP: &str = "/usr/bin/bwrap"; // chemin absolu : un PATH inscriptible ne doit pas pouvoir substituer un faux lanceur

pub const UNAVAILABLE: &str = "Mode YOLO indisponible : le terminal ne peut pas démarrer sans son bac à sable. Installez bubblewrap (/usr/bin/bwrap), autorisez les espaces de noms utilisateur non privilégiés, puis relancez Ezerbot. En attendant, chaque action demande votre accord.";

fn launcher(bwrap: &Path, protected: &[PathBuf], cwd: &Path) -> Command {
    // --unshare-user est explicite : sans lui, bwrap lancé en root (uid 0, même dans un
    // espace de noms) ne crée pas d'espace utilisateur et le shell garde de quoi démonter.
    // L'espace utilisateur refuse aussi /proc/<pid>/root et fd vers les processus de l'hôte.
    let mut command = Command::new(bwrap);
    command.args(["--unshare-user", "--dev-bind", "/", "/"]);
    let mut roots: Vec<PathBuf> = protected
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .collect();
    roots.sort();
    roots.dedup_by(|inner, outer| inner.starts_with(&*outer));
    for root in &roots {
        if root.is_dir() {
            command
                .args(["--perms", "0000", "--tmpfs"])
                .arg(root)
                .arg("--remount-ro")
                .arg(root);
        } else {
            command.args(["--ro-bind", "/dev/null"]).arg(root);
        }
    }
    command.arg("--chdir").arg(cwd);
    command
}

fn probe(bwrap: &Path) -> bool {
    launcher(bwrap, &[PathBuf::from("/tmp")], Path::new("/"))
        .args(["/bin/sh", "-c", ":"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| probe(Path::new(BWRAP)))
}

pub fn command(protected: &[PathBuf], cwd: &Path) -> Result<Command, ApiError> {
    if !available() {
        return Err(ApiError::new(503, UNAVAILABLE));
    }
    let mut command = launcher(Path::new(BWRAP), protected, cwd);
    command.arg("/bin/sh");
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokbot_test_support::TestDir;
    use std::os::unix::net::UnixListener;
    use std::process::Output;

    fn run(protected: &[PathBuf], cwd: &Path, script: &str) -> Output {
        launcher(Path::new(BWRAP), protected, cwd)
            .args(["/bin/sh", "-c", script])
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn host(cwd: &Path, script: &str) -> Output {
        Command::new("/bin/sh")
            .args(["-c", script])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn stdout(out: &Output) -> String {
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn a_protected_folder_and_a_protected_socket_are_unreachable_in_the_sandbox() {
        let dir = TestDir::new("ezerbot-bwrap");
        let data = dir.join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("secret.txt"), "jeton-secret").unwrap();
        let socket = dir.join("bus");
        let _listener = UnixListener::bind(&socket).unwrap();
        std::fs::write(dir.join("voisin.txt"), "utile").unwrap();
        let roots = [data.clone(), socket.clone()];

        let probes = r#"cat data/secret.txt; echo "ecriture=$(echo x > data/intrus 2>&1; echo $?)"; echo "dossier=$(mkdir data/sous >/dev/null 2>&1; echo $?)"; echo "liste=$(ls data 2>&1)"; echo "socket=$(cat bus 2>&1)"; cat voisin.txt"#;
        let reference = host(&dir, probes);
        assert!(
            stdout(&reference).contains("jeton-secret"),
            "la référence hors bac doit lire le secret : {reference:?}"
        );
        assert!(
            stdout(&reference).contains("socket=cat: bus: No such device"),
            "la référence hors bac doit atteindre le socket : {reference:?}"
        );
        assert!(data.join("intrus").exists());
        std::fs::remove_file(data.join("intrus")).unwrap();
        std::fs::remove_dir(data.join("sous")).unwrap();

        let sandboxed = run(&roots, &dir, probes);
        let shown = stdout(&sandboxed);
        assert!(!shown.contains("jeton-secret"), "{shown}");
        assert!(!shown.contains("ecriture=0"), "{shown}");
        assert!(!shown.contains("dossier=0"), "{shown}");
        assert!(shown.contains("Permission denied"), "{shown}");
        assert!(!shown.contains("secret.txt"), "{shown}");
        assert!(!shown.contains("No such device"), "{shown}");
        assert!(shown.contains("utile"), "{shown}");
        assert!(!data.join("intrus").exists());
        assert!(!data.join("sous").exists());
        assert_eq!(
            std::fs::read_to_string(data.join("secret.txt")).unwrap(),
            "jeton-secret"
        );
    }

    #[test]
    fn a_nested_root_and_a_missing_path_still_start_and_the_missing_path_is_not_created() {
        let dir = TestDir::new("ezerbot-bwrap");
        let outer = dir.join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("secret.txt"), "profond").unwrap();
        let missing = dir.join("absent/profond");
        let out = run(
            &[inner.clone(), outer.clone(), missing.clone()],
            &dir,
            "echo demarre; ls outer 2>&1; cat outer/inner/secret.txt 2>&1",
        );
        let shown = stdout(&out);
        assert!(shown.contains("demarre"), "{shown}");
        assert!(!shown.contains("profond"), "{shown}");
        assert!(!dir.join("absent").exists());
        assert!(!missing.exists());
    }

    #[test]
    fn the_launcher_probe_tells_a_missing_bwrap_from_the_real_one() {
        assert!(!probe(Path::new("/nonexistent/bwrap")));
        assert!(probe(Path::new(BWRAP)));
    }

    #[test]
    fn a_working_folder_under_a_protected_root_stops_the_launch() {
        let dir = TestDir::new("ezerbot-bwrap");
        let data = dir.join("data");
        std::fs::create_dir(&data).unwrap();
        let witness = dir.join("temoin.txt");
        let touch = format!("touch '{}'", witness.display());
        let out = run(std::slice::from_ref(&data), &data, &touch);
        assert!(!out.status.success(), "{out:?}");
        assert!(!witness.exists());
        let beside = run(std::slice::from_ref(&data), &dir, &touch);
        assert!(beside.status.success(), "{beside:?}");
        assert!(witness.exists());
    }
}
