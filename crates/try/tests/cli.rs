use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct DemoDatabase(PathBuf);

impl DemoDatabase {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("grokbot-cli-{name}-{}", std::process::id()));
        fs::create_dir(&path).expect("dossier temporaire distinct");
        Self(path)
    }

    fn run(&self, command: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_grokbot-try"))
            .arg(command)
            .arg("--db")
            .arg(self.0.join("session.sqlite"))
            .output()
            .expect("lancer le programme compilé")
    }
}

impl Drop for DemoDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn la_demo_se_relit_depuis_un_autre_processus() {
    let database = DemoDatabase::new("restart");
    let demo = database.run("demo");
    assert!(
        demo.status.success(),
        "{}",
        String::from_utf8_lossy(&demo.stderr)
    );
    let status = database.run("status");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let output = String::from_utf8(status.stdout).unwrap();
    assert!(output.contains("msg-repetition"));
    assert!(output.contains("msg-acceptation"));
    assert!(output.contains("couverture 2"));
}

#[test]
fn le_parcours_orchestre_reconcilie_sans_doublon_et_se_relit() {
    let database = DemoDatabase::new("orchestration");
    let flow = database.run("flow");
    assert!(
        flow.status.success(),
        "{}",
        String::from_utf8_lossy(&flow.stderr)
    );
    let output = String::from_utf8(flow.stdout).unwrap();
    assert!(output.contains("2 envois, 2 recherches"), "{output}");
    let status = database.run("status");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let output = String::from_utf8(status.stdout).unwrap();
    assert!(output.contains("msg-orchestration-loss"));
    assert!(output.contains("msg-orchestration-not-found"));
    assert!(output.contains("couverture 2"));
    assert_eq!(output.matches("écho reçu").count(), 2, "{output}");
    let path = database.0.join("session.sqlite");
    let before = fs::read(&path).unwrap();
    assert!(!database.run("flow").status.success());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn une_seconde_demo_refuse_de_remplacer_la_base() {
    let database = DemoDatabase::new("preserve");
    let first = database.run("demo");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let path = database.0.join("session.sqlite");
    let before = fs::read(&path).unwrap();
    let second = database.run("demo");
    assert!(
        !second.status.success(),
        "une base existante doit être préservée"
    );
    assert_eq!(fs::read(path).unwrap(), before);
}
