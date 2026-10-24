//! Le seul mécanisme de dossier temporaire des tests du dépôt.
//!
//! Chaque test reçoit un sous-dossier unique de `std::env::temp_dir()` et le
//! supprime en entier dans `Drop`, y compris quand le test échoue : l'unwinding
//! exécute `Drop`. Rien n'est donc posé directement dans `$TMPDIR` (une base
//! posée là ferait balayer tout `$TMPDIR` par `Vault::open`) et rien n'y reste,
//! fichiers frères d'une base compris (`-wal`, `-shm`, `.secrets.json`, `.lock`).

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Dossier unique sous `$TMPDIR`, supprimé récursivement à la destruction.
#[derive(Debug)]
pub struct TestDir {
    path: PathBuf,
    quiet: Duration,
}

impl TestDir {
    /// Crée `$TMPDIR/<label>-<pid>-<n>-<nanos>`. `label` garde le nom lisible
    /// de la fixture (`ezerbot-pages`, `grokbot-wrapper`...).
    pub fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "{label}-{}-{}-{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create the test directory");
        Self { path, quiet: QUIET }
    }

    /// Allonge la fenêtre calme de la suppression pour une fixture dont un thread de fond du produit
    /// (sonde de connexion, rafraîchissement du catalogue) écrit encore une à deux secondes après
    /// la fin du test.
    pub fn settling_for(mut self, quiet: Duration) -> Self {
        self.quiet = quiet;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Chemin d'une entrée du dossier (la base d'un test s'appelle ainsi).
    pub fn join(&self, entry: impl AsRef<Path>) -> PathBuf {
        self.path.join(entry)
    }
}

/// Un thread de fond du produit (rafraîchissement du catalogue, journal, verrou) peut survivre
/// au test et recréer des fichiers après la suppression, jusqu'à recréer le dossier lui-même.
/// La suppression recommence donc tant que le dossier réapparaît, et ne s'arrête qu'après une
/// fenêtre calme (ou la limite, pour ne jamais bloquer un test).
const POLL: Duration = Duration::from_millis(25);
const QUIET: Duration = Duration::from_millis(250);
const LIMIT: Duration = Duration::from_secs(3);

impl Drop for TestDir {
    fn drop(&mut self) {
        let started = Instant::now();
        let mut last_removal = Instant::now();
        let _ = std::fs::remove_dir_all(&self.path);
        while last_removal.elapsed() < self.quiet && started.elapsed() < LIMIT {
            std::thread::sleep(POLL);
            if self.path.exists() {
                let _ = std::fs::remove_dir_all(&self.path);
                last_removal = Instant::now();
            }
        }
    }
}

impl Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_is_unique_and_removed_with_its_contents() {
        let first = TestDir::new("test-support");
        let second = TestDir::new("test-support");
        assert_ne!(first.path(), second.path());
        assert_eq!(first.path().parent(), Some(std::env::temp_dir().as_path()));
        let nested = first.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("db.sqlite-wal"), b"x").unwrap();
        let kept = first.path().to_path_buf();
        drop(first);
        assert!(!kept.exists());
        assert!(second.path().is_dir());
    }

    #[test]
    fn directory_is_removed_when_the_test_panics() {
        let seen = std::sync::Mutex::new(None);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let dir = TestDir::new("test-support-panic");
            *seen.lock().unwrap() = Some(dir.path().to_path_buf());
            std::fs::write(dir.join("db.sqlite"), b"x").unwrap();
            panic!("test failure");
        }));
        assert!(outcome.is_err());
        let path = seen.lock().unwrap().take().unwrap();
        assert!(!path.exists());
    }
}
