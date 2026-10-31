//! Bac à sable du shell YOLO sous Linux : bubblewrap recouvre les racines protégées.
use crate::ApiError;
use landlock::{CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetStatus, Scope};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs, io,
    os::unix::{
        ffi::OsStringExt,
        fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::OnceLock,
};

const BWRAP: &str = "/usr/bin/bwrap"; // chemin absolu : un PATH inscriptible ne doit pas pouvoir substituer un faux lanceur

pub const UNAVAILABLE: &str = "Mode YOLO indisponible : le terminal ne peut pas démarrer sans son bac à sable. Installez bubblewrap (/usr/bin/bwrap), autorisez les espaces de noms utilisateur non privilégiés et utilisez un noyau Linux 6.12 ou plus récent (Landlock isole les sockets abstraits), puis relancez Ezerbot. En attendant, chaque action demande votre accord.";

/// Une ligne de `/proc/self/mountinfo` : `device` est `maj:min`, `root` le chemin du
/// montage dans son système de fichiers, `point` l'endroit où il apparaît.
struct Mount {
    device: Vec<u8>,
    root: PathBuf,
    point: PathBuf,
}

fn unescape(bytes: &[u8]) -> PathBuf {
    let mut raw = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let octal = bytes
            .get(at + 1..at + 4)
            .filter(|digits| bytes[at] == b'\\' && digits.iter().all(|d| (b'0'..=b'7').contains(d)))
            .and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok());
        match octal {
            Some(byte) => {
                raw.push(byte);
                at += 4;
            }
            None => {
                raw.push(bytes[at]);
                at += 1;
            }
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(raw))
}

fn lines(table: &[u8]) -> impl Iterator<Item = &[u8]> {
    table.split(|byte| *byte == b'\n')
}

fn mounts(table: &[u8]) -> Vec<Mount> {
    lines(table)
        .filter_map(|line| {
            let mut fields = line.split(|byte| *byte == b' ');
            let device = fields.nth(2)?;
            let root = fields.next()?;
            let point = fields.next()?;
            // Un montage dont le dossier source a été supprimé ne montre plus rien.
            (!root.ends_with(b"//deleted")).then(|| Mount {
                device: device.to_vec(),
                root: unescape(root),
                point: unescape(point),
            })
        })
        .collect()
}

fn join(base: &Path, rest: &Path) -> PathBuf {
    if rest.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(rest)
    }
}

/// Les autres chemins qui montrent `path` (ou un de ses sous-dossiers) : un montage du même
/// système de fichiers qui en expose la même partie. Chaque paire est (vue, original).
fn views(path: &Path, mounts: &[Mount]) -> Vec<(PathBuf, PathBuf)> {
    let Some(owner) = mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.point))
        .max_by_key(|mount| mount.point.components().count())
    else {
        return Vec::new();
    };
    let Ok(below) = path.strip_prefix(&owner.point) else {
        return Vec::new();
    };
    let file = join(&owner.root, below);
    let mut found = Vec::new();
    for mount in mounts.iter().filter(|mount| mount.device == owner.device) {
        let pair = if let Ok(rest) = file.strip_prefix(&mount.root) {
            (join(&mount.point, rest), path.to_path_buf())
        } else if let Ok(rest) = mount.root.strip_prefix(&file) {
            (mount.point.clone(), join(path, rest))
        } else {
            continue;
        };
        if pair.0 != pair.1 {
            found.push(pair);
        }
    }
    found
}

/// Les chemins liés à un socket Unix qui écoute (`SO_ACCEPTCON`, drapeau `00010000`).
/// Les sockets abstraits (`@…`) n'ont pas de fichier : Landlock s'en charge.
fn listening_sockets(table: &[u8]) -> Vec<PathBuf> {
    // Le noyau imprime le chemin brut : une ligne qui n'ouvre pas une entrée (`<hex>: …`)
    // continue le chemin de la précédente.
    let mut entries: Vec<Vec<u8>> = Vec::new();
    for line in lines(table.strip_suffix(b"\n").unwrap_or(table)) {
        let opens = line
            .iter()
            .position(|byte| *byte == b':')
            .is_some_and(|colon| colon >= 8 && line[..colon].iter().all(u8::is_ascii_hexdigit));
        match entries.last_mut() {
            Some(entry) if !opens => {
                entry.push(b'\n');
                entry.extend_from_slice(line);
            }
            _ if opens => entries.push(line.to_vec()),
            _ => {}
        }
    }
    entries
        .iter()
        .filter_map(|entry| {
            let mut rest = entry.as_slice();
            let mut fields: [&[u8]; 7] = [b""; 7];
            for field in &mut fields {
                rest = rest.trim_ascii_start();
                let end = rest.iter().position(u8::is_ascii_whitespace)?;
                (*field, rest) = rest.split_at(end);
            }
            let path = rest.strip_prefix(b" ")?;
            (fields[3] == b"00010000" && path.starts_with(b"/"))
                .then(|| PathBuf::from(std::ffi::OsString::from_vec(path.to_vec())))
        })
        .collect()
}

/// Des octets, pas du texte : un chemin peut ne pas être de l'UTF-8, et `/proc/net/unix`
/// liste aussi les sockets des autres utilisateurs. Une table illisible éteint le mode.
fn read_table(path: &str) -> Result<Vec<u8>, ApiError> {
    fs::read(path).map_err(|_| ApiError::new(503, UNAVAILABLE))
}

fn refused(path: &Path, error: &io::Error) -> ApiError {
    ApiError::new(
        403,
        format!(
            "Mode YOLO : la donnée protégée {} n’a pas pu être vérifiée ({error}). La commande n’est pas lancée.",
            path.display()
        ),
    )
}

/// Un lien dur est un second nom du même contenu : le masque d'un chemin ne le couvre pas.
/// Chaque nom d'un objet à plusieurs liens doit se trouver dans une racine protégée. Un nom
/// est une entrée de dossier réelle : deux vues d'un même dossier n'en font pas deux, et un
/// fichier monté sur un autre n'en fait aucun.
fn refuse_outside_links(roots: &[PathBuf], mounts: &[Mount]) -> Result<(), ApiError> {
    type Entry = ((u64, u64), OsString);
    let mut seen: HashMap<(u64, u64), (u64, HashSet<Entry>, PathBuf)> = HashMap::new();
    let mut pending = roots.to_vec();
    while let Some(path) = pending.pop() {
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(refused(&path, &error)),
        };
        if meta.is_dir() {
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(refused(&path, &error)),
            };
            for entry in entries {
                match entry {
                    Ok(entry) => pending.push(entry.path()),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(refused(&path, &error)),
                }
            }
        } else if !meta.file_type().is_symlink() && meta.nlink() > 1 {
            let entry = path.parent().and_then(identity).zip(path.file_name());
            let entry = entry
                .filter(|_| !mounts.iter().any(|mount| mount.point == path))
                .map(|(folder, name)| (folder, name.to_owned()));
            let names = &mut seen
                .entry((meta.dev(), meta.ino()))
                .or_insert_with(|| (meta.nlink(), HashSet::new(), path.clone()))
                .1;
            names.extend(entry);
        }
    }
    let outside = seen
        .into_values()
        .filter(|(links, names, _)| (names.len() as u64) < *links)
        .map(|(_, _, path)| path)
        .min();
    match outside {
        None => Ok(()),
        Some(path) => Err(ApiError::new(
            403,
            format!(
                "Mode YOLO : la donnée protégée {path} a un lien dur hors de sa racine, que le bac à sable ne peut pas masquer. Retrouvez-le avec `find / -samefile '{path}'` puis supprimez-le, et relancez la commande.",
                path = path.display()
            ),
        )),
    }
}

fn identity(path: &Path) -> Option<(u64, u64)> {
    fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()))
}

/// Les chemins qui montrent le contenu de `roots` sans en faire partie. Ce qui est monté
/// dans une racine y vit aussi : sa source en est une autre vue.
fn aliases(roots: &[PathBuf], mounts: &[Mount]) -> Vec<PathBuf> {
    let inner = mounts
        .iter()
        .filter(|mount| roots.iter().any(|root| mount.point.starts_with(root)))
        .map(|mount| &mount.point);
    let mut found = Vec::new();
    for root in roots.iter().chain(inner) {
        for (view, original) in views(root, mounts) {
            let shown = identity(&view);
            if shown.is_some() && shown == identity(&original) {
                found.push(view);
            }
        }
    }
    found
}

/// Les outils fichiers ne passent pas par le bac à sable : ce chemin montre-t-il une racine
/// protégée par un autre montage ? Une table illisible répond oui.
pub fn shows_protected(path: &Path, protected: &[PathBuf]) -> bool {
    let Ok(table) = read_table("/proc/self/mountinfo") else {
        return true;
    };
    let roots: Vec<PathBuf> = protected
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .collect();
    aliases(&roots, &mounts(&table))
        .iter()
        .any(|view| path.starts_with(view))
}

/// Tout ce que le shell ne doit pas atteindre : les racines protégées, chaque socket qui
/// écoute sous le même uid (un bus de session peut être n'importe où) et chaque autre chemin
/// qui montre ces racines.
fn masks(protected: &[PathBuf]) -> Result<Vec<PathBuf>, ApiError> {
    let sockets = read_table("/proc/net/unix")?;
    let table = read_table("/proc/self/mountinfo")?;
    let uid = nix::unistd::Uid::effective().as_raw();
    let mut roots = Vec::with_capacity(protected.len());
    for root in protected {
        match root.canonicalize() {
            Ok(real) => roots.push(real),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            // Illisible n'est pas absent : un `chmod` depuis le shell la rendrait lisible, sans masque.
            Err(error) => return Err(refused(root, &error)),
        }
    }
    for socket in listening_sockets(&sockets) {
        let ours = fs::symlink_metadata(&socket)
            .is_ok_and(|meta| meta.file_type().is_socket() && meta.uid() == uid);
        if let Some(real) = socket.canonicalize().ok().filter(|_| ours) {
            roots.push(real);
        }
    }
    roots.sort();
    roots.dedup_by(|inner, outer| inner.starts_with(&*outer));
    let mounts = mounts(&table);
    refuse_outside_links(&roots, &mounts)?;
    let mut masks = aliases(&roots, &mounts);
    masks.extend(roots);
    masks.sort();
    masks.dedup_by(|inner, outer| inner.starts_with(&*outer));
    Ok(masks)
}

fn launcher(bwrap: &Path, masks: &[PathBuf], cwd: &Path) -> Command {
    // --unshare-user est explicite : sans lui, bwrap lancé en root (uid 0, même dans un
    // espace de noms) ne crée pas d'espace utilisateur et le shell garde de quoi démonter.
    // L'espace utilisateur refuse aussi /proc/<pid>/root et fd vers les processus de l'hôte.
    let mut command = Command::new(bwrap);
    command.args(["--unshare-user", "--bind", "/", "/", "--dev", "/dev"]);
    for mask in masks {
        if mask.is_dir() {
            command
                .args(["--perms", "0000", "--tmpfs"])
                .arg(mask)
                .arg("--remount-ro")
                .arg(mask);
        } else {
            command.args(["--ro-bind", "/dev/null"]).arg(mask);
        }
    }
    command.arg("--chdir").arg(cwd);
    command
}

/// Lance la commande dans un domaine Landlock qui ferme les sockets abstraits créés hors de
/// lui : bubblewrap ne sépare pas l'espace de noms réseau, et un bus de session peut y vivre
/// sans fichier à masquer. Landlock s'applique au fil appelant et le fork en hérite ; un fil
/// jetable le porte, le reste du wrapper garde ses droits.
pub fn spawn(command: &mut Command) -> io::Result<Child> {
    fn confine() -> io::Result<()> {
        let status = Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .scope(Scope::AbstractUnixSocket)
            .and_then(|ruleset| ruleset.create())
            .and_then(|ruleset| ruleset.restrict_self())
            .map_err(io::Error::other)?;
        match status.ruleset {
            RulesetStatus::FullyEnforced => Ok(()),
            _ => Err(io::Error::other(
                "Landlock n’isole pas les sockets abstraits",
            )),
        }
    }
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                confine()?;
                command.spawn()
            })
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("le fil de lancement a paniqué")))
    })
}

fn probe(bwrap: &Path) -> bool {
    let mut command = launcher(bwrap, &[PathBuf::from("/tmp")], Path::new("/"));
    command
        .args(["/bin/sh", "-c", ":"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    spawn(&mut command)
        .and_then(|mut child| child.wait())
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
    let mut command = launcher(Path::new(BWRAP), &masks(protected)?, cwd);
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
        let mut command = launcher(Path::new(BWRAP), &masks(protected).unwrap(), cwd);
        command
            .args(["/bin/sh", "-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        spawn(&mut command).unwrap().wait_with_output().unwrap()
    }

    /// Relance un test dans un espace de montage où il peut lier des dossiers, et vérifie
    /// qu'il y a tourné jusqu'au bout.
    fn in_a_mount_namespace(test: &str, inside: &str, dir: &Path) {
        let out = Command::new("/usr/bin/unshare")
            .args(["--user", "--map-root-user", "--mount"])
            .arg(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(format!("features::yolo::sandbox::tests::{test}"))
            .arg("--nocapture")
            .env(inside, dir)
            .output()
            .unwrap();
        assert!(
            out.status.success() && stdout(&out).contains("refus-vu"),
            "{out:?}"
        );
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
    fn no_host_block_device_is_visible_and_the_usual_nodes_work() {
        let dir = TestDir::new("ezerbot-bwrap");
        let out = run(
            &[],
            &dir,
            "echo \"blocs=$(find /dev -type b | wc -l)\"; echo \"nul=$(echo x > /dev/null; echo $?)\"; echo \"octets=$(head -c 4 /dev/urandom | wc -c)\"",
        );
        let shown = stdout(&out);
        assert!(shown.contains("blocs=0"), "{shown}");
        assert!(shown.contains("nul=0"), "{shown}");
        assert!(shown.contains("octets=4"), "{shown}");
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

    fn shown(pairs: Vec<(PathBuf, PathBuf)>) -> Vec<(String, String)> {
        pairs
            .into_iter()
            .map(|(view, original)| (view.display().to_string(), original.display().to_string()))
            .collect()
    }

    fn pair(view: &str, original: &str) -> (String, String) {
        (view.to_owned(), original.to_owned())
    }

    #[test]
    fn the_mount_table_decodes_octal_escapes_and_drops_deleted_sources() {
        let table = "\
36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw\n\
40 36 8:1 /dos\\040espace /point\\040ici rw - ext4 /dev/sda1 rw\n\
41 36 8:1 /disparu//deleted /x rw - ext4 /dev/sda1 rw\n\
42 36 8:1 /tab\\011nl\\012bs\\134 /y rw - ext4 /dev/sda1 rw\n\
43 36 8:1 /plein\\9 /z rw - ext4 /dev/sda1 rw\n";
        let found = mounts(table.as_bytes());
        let rows: Vec<(&str, String, String)> = found
            .iter()
            .map(|m| {
                (
                    std::str::from_utf8(&m.device).unwrap(),
                    m.root.display().to_string(),
                    m.point.display().to_string(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("98:0", "/mnt1".to_owned(), "/mnt2".to_owned()),
                ("8:1", "/dos espace".to_owned(), "/point ici".to_owned()),
                ("8:1", "/tab\tnl\nbs\\".to_owned(), "/y".to_owned()),
                ("8:1", "/plein\\9".to_owned(), "/z".to_owned()),
            ]
        );
    }

    #[test]
    fn a_second_mount_of_the_same_device_is_a_view_and_a_sibling_folder_is_not() {
        let table = mounts(
            "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
2 1 8:1 / /mnt/copie rw - ext4 /dev/sda1 rw\n\
3 1 8:2 / /autre rw - ext4 /dev/sdb1 rw\n\
4 1 8:1 /a/data2 /elsewhere rw - ext4 /dev/sda1 rw\n"
                .as_bytes(),
        );
        assert_eq!(
            shown(views(Path::new("/a/data"), &table)),
            [pair("/mnt/copie/a/data", "/a/data")]
        );
    }

    #[test]
    fn a_bind_of_a_subfolder_and_a_bind_of_the_folder_itself_are_views() {
        let table = mounts(
            "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
2 1 8:1 /a/data /vue rw - ext4 /dev/sda1 rw\n\
3 1 8:1 /a/data/sous /sous rw - ext4 /dev/sda1 rw\n\
4 1 8:1 /a/data/sous/profond /profond rw - ext4 /dev/sda1 rw\n\
5 1 8:1 /a /parent rw - ext4 /dev/sda1 rw\n"
                .as_bytes(),
        );
        assert_eq!(
            shown(views(Path::new("/a/data"), &table)),
            [
                pair("/vue", "/a/data"),
                pair("/sous", "/a/data/sous"),
                pair("/profond", "/a/data/sous/profond"),
                pair("/parent/data", "/a/data"),
            ]
        );
    }

    #[test]
    fn a_view_of_a_path_on_a_subvolume_follows_the_root_of_its_mount() {
        let table = mounts(
            "1 0 0:30 /@home /home rw - btrfs /dev/sda2 rw\n\
2 1 0:30 / /mnt/racine rw - btrfs /dev/sda2 rw\n\
3 1 0:30 /@home /mnt/maison rw - btrfs /dev/sda2 rw\n"
                .as_bytes(),
        );
        assert_eq!(
            shown(views(Path::new("/home/u/data"), &table)),
            [
                pair("/mnt/racine/@home/u/data", "/home/u/data"),
                pair("/mnt/maison/u/data", "/home/u/data"),
            ]
        );
        assert!(views(Path::new("/ailleurs"), &table).is_empty());
    }

    #[test]
    fn the_deepest_mount_owns_a_path_and_the_last_wins_a_tie() {
        let table = mounts(
            "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
2 1 8:3 / /a rw - ext4 /dev/sda3 rw\n\
3 1 8:3 / /copie rw - ext4 /dev/sda3 rw\n\
4 1 8:4 / /a rw - ext4 /dev/sda4 rw\n\
5 1 8:4 / /dernier rw - ext4 /dev/sda4 rw\n"
                .as_bytes(),
        );
        assert_eq!(
            shown(views(Path::new("/a/data"), &table)),
            [pair("/dernier/data", "/a/data")]
        );
    }

    #[test]
    fn only_listening_sockets_with_an_absolute_path_are_kept() {
        let table = "\
Num       RefCount Protocol Flags    Type St Inode Path
0000000000000000: 00000002 00000000 00010000 0001 01  4242 /run/user/1000/bus
0000000000000000: 00000002 00000000 00010000 0001 01 12345 /tmp/un dossier/sock et.sock
0000000000000000: 00000003 00000000 00000000 0001 03  4243 /run/user/1000/bus
0000000000000000: 00000002 00000000 00010000 0001 01  4244 @/tmp/dbus-abstrait
0000000000000000: 00000002 00000000 00010000 0001 01  4245 relatif/sock
0000000000000000: 00000002 00000000 00010000 0001 01  4246
";
        let found: Vec<String> = listening_sockets(table.as_bytes())
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        assert_eq!(
            found,
            ["/run/user/1000/bus", "/tmp/un dossier/sock et.sock"]
        );
    }

    #[test]
    fn a_path_that_is_not_utf8_stays_a_path_in_both_tables() {
        use std::os::unix::ffi::OsStrExt;
        let socket = listening_sockets(
            b"0000000000000000: 00000002 00000000 00010000 0001 01 7 /tmp/caf\xe9.sock\n",
        );
        assert_eq!(socket[0].as_os_str().as_bytes(), b"/tmp/caf\xe9.sock");
        let mount = mounts(b"1 0 8:1 / /caf\xe9 rw - ext4 /dev/sda1 rw\n");
        assert_eq!(mount[0].point.as_os_str().as_bytes(), b"/caf\xe9");
    }

    #[test]
    fn a_hard_link_between_two_names_inside_a_root_does_not_refuse_but_one_outside_does() {
        let dir = TestDir::new("ezerbot-bwrap");
        let data = dir.join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("a.txt"), "jeton").unwrap();
        std::fs::hard_link(data.join("a.txt"), data.join("b.txt")).unwrap();
        let real = data.canonicalize().unwrap();
        assert!(masks(std::slice::from_ref(&data)).unwrap().contains(&real));

        let outside = dir.join("dehors.txt");
        std::fs::hard_link(data.join("a.txt"), &outside).unwrap();
        let error = masks(std::slice::from_ref(&data)).unwrap_err();
        assert_eq!(error.status, 403);
        assert!(error.message.contains("lien dur"), "{}", error.message);
        assert!(
            error.message.contains("find / -samefile"),
            "{}",
            error.message
        );
        let named = real.display().to_string();
        assert!(error.message.contains(&named), "{}", error.message);
        assert!(!error.message.contains("jeton"), "{}", error.message);

        std::fs::remove_file(&outside).unwrap();
        assert!(masks(std::slice::from_ref(&data)).is_ok());
    }

    #[test]
    fn a_socket_of_ours_is_masked_only_while_it_listens_and_a_plain_file_never_is() {
        let dir = TestDir::new("ezerbot-bwrap");
        let path = dir.join("bus-inconnu");
        let real = dir.canonicalize().unwrap().join("bus-inconnu");
        let listener = UnixListener::bind(&path).unwrap();
        assert!(masks(&[]).unwrap().contains(&real));
        drop(listener);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, "pas un socket").unwrap();
        assert!(!masks(&[]).unwrap().contains(&real));
        let closed = dir.join("ferme");
        drop(UnixListener::bind(&closed).unwrap());
        assert!(!masks(&[])
            .unwrap()
            .contains(&dir.canonicalize().unwrap().join("ferme")));
    }

    #[test]
    fn one_entry_seen_through_two_roots_stays_one_name() {
        const INSIDE: &str = "EZERBOT_TEST_DEUX_RACINES";
        if let Some(dir) = std::env::var_os(INSIDE) {
            let dir = PathBuf::from(dir);
            let (data, support) = (dir.join("data"), dir.join("support"));
            let bound = host(&dir, "mount --bind data support");
            assert!(bound.status.success(), "{bound:?}");
            let roots = [data.clone(), support.clone()];
            assert!(
                masks(&roots).is_err(),
                "dossier lié : le lien externe passe"
            );
            let unbound = host(&dir, "umount support && touch support/fichier");
            assert!(unbound.status.success(), "{unbound:?}");
            let bound = host(&dir, "mount --bind data/secret.txt support/fichier");
            assert!(bound.status.success(), "{bound:?}");
            assert!(
                masks(&roots).is_err(),
                "fichier lié : le lien externe passe"
            );
            println!("refus-vu");
            return;
        }
        let dir = TestDir::new("ezerbot-bwrap");
        for sub in ["data", "support"] {
            std::fs::create_dir(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("data/secret.txt"), "jeton").unwrap();
        std::fs::hard_link(dir.join("data/secret.txt"), dir.join("dehors.txt")).unwrap();
        in_a_mount_namespace(
            "one_entry_seen_through_two_roots_stays_one_name",
            INSIDE,
            &dir,
        );
    }

    #[test]
    fn the_source_of_a_mount_inside_a_root_is_masked_too() {
        const INSIDE: &str = "EZERBOT_TEST_MONTAGE_INTERIEUR";
        if let Some(dir) = std::env::var_os(INSIDE) {
            let dir = PathBuf::from(dir);
            let bound = host(
                &dir,
                "mount --bind stockage/jeton.txt data/jeton && mount --bind stockage/coffre data/coffre",
            );
            assert!(bound.status.success(), "{bound:?}");
            let data = dir.join("data");
            let out = run(
                std::slice::from_ref(&data),
                &dir,
                "cat stockage/jeton.txt stockage/coffre/cle.txt; cat stockage/voisin.txt",
            );
            let shown = stdout(&out);
            assert!(!shown.contains("jeton-monte"), "{shown}");
            assert!(!shown.contains("cle-montee"), "{shown}");
            assert!(shown.contains("utile"), "{shown}");
            println!("refus-vu");
            return;
        }
        let dir = TestDir::new("ezerbot-bwrap");
        std::fs::create_dir_all(dir.join("stockage/coffre")).unwrap();
        std::fs::create_dir_all(dir.join("data/coffre")).unwrap();
        std::fs::write(dir.join("data/jeton"), "").unwrap();
        std::fs::write(dir.join("stockage/jeton.txt"), "jeton-monte").unwrap();
        std::fs::write(dir.join("stockage/coffre/cle.txt"), "cle-montee").unwrap();
        std::fs::write(dir.join("stockage/voisin.txt"), "utile").unwrap();
        in_a_mount_namespace(
            "the_source_of_a_mount_inside_a_root_is_masked_too",
            INSIDE,
            &dir,
        );
    }

    #[test]
    fn a_protected_root_behind_an_unreadable_folder_stops_the_launch() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("ezerbot-bwrap");
        let parent = dir.join("share");
        let root = parent.join("keyrings");
        std::fs::create_dir_all(&root).unwrap();
        let mode = |bits| std::fs::Permissions::from_mode(bits);
        std::fs::set_permissions(&parent, mode(0o000)).unwrap();
        let hidden = masks(std::slice::from_ref(&root));
        std::fs::set_permissions(&parent, mode(0o700)).unwrap();
        let error = hidden.expect_err("un chmod rendrait la racine lisible, sans masque");
        assert_eq!(error.status, 403);
        assert!(masks(std::slice::from_ref(&root)).is_ok());
    }

    #[test]
    fn a_socket_with_a_line_break_in_its_path_is_masked() {
        let dir = TestDir::new("ezerbot-bwrap");
        let socket = dir.join("session\nbus.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        let masked = masks(&[]).unwrap();
        assert!(
            masked.contains(&socket.canonicalize().unwrap()),
            "{masked:?}"
        );
    }
}
