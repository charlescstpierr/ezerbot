//! Mise à jour locale d’Ezerbot depuis la branche `main`.
//!
//! Le clone et la compilation vivent dans un dossier de travail jetable
//! (`~/Library/Caches/Ezerbot/mise-a-jour`). Le checkout de développement n’est
//! jamais un dépôt git de cette opération, et le dossier des conversations
//! n’est ni lu ni écrit. L’application en cours n’est remplacée qu’après un
//! bundle vérifié ; un échec de compilation la laisse intacte.

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

pub const RELAUNCH_EXIT_CODE: i32 = 86;
pub const REPO_URL: &str = "https://github.com/charlescstpierr/ezerbot.git";
pub const GITHUB_COMMIT_URL: &str =
    "https://api.github.com/repos/charlescstpierr/ezerbot/commits/main";
/// Dépôt git local (`file://` ou chemin absolu) à la place de GitHub.
/// Réservé à une copie isolée : aucun jeton n’est lu.
pub const UPDATE_REPO_ENV: &str = "EZERBOT_UPDATE_REPO";
/// `hold` : armer le redémarrage sans quitter le processus, pour l’observer.
pub const UPDATE_RELAUNCH_ENV: &str = "EZERBOT_UPDATE_RELAUNCH";

const LOG_LIMIT: usize = 8_000;

thread_local! {
    static TOUCHED: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitRef {
    pub sha: String,
    pub unix: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Availability {
    UpToDate,
    Available(CommitRef),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Telechargement,
    Compilation,
    Installation,
    Redemarrage,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Telechargement => "Téléchargement…",
            Self::Compilation => "Compilation…",
            Self::Installation => "Installation…",
            Self::Redemarrage => "Redémarrage…",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Telechargement => "telechargement",
            Self::Compilation => "compilation",
            Self::Installation => "installation",
            Self::Redemarrage => "redemarrage",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    None,
    /// Le dernier renommage échoue. L’application précédente est remise en place.
    FinalRename,
}

#[derive(Clone, Debug)]
pub struct UpdatePaths {
    pub work_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub bundle: PathBuf,
    pub dev_root: PathBuf,
    pub user_data: PathBuf,
}

#[derive(Clone, Debug)]
pub struct CommandRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner {
    fn run(&mut self, request: &CommandRequest) -> Result<CommandOutput, UpdateError>;
}

#[derive(Debug)]
pub struct UpdateError {
    pub message: String,
    pub log: String,
}

impl UpdateError {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            log: String::new(),
        }
    }

    pub fn with_log(message: impl Into<String>, log: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            log: log.into(),
        }
    }
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for UpdateError {}

pub struct ApplyInput<'a> {
    pub paths: &'a UpdatePaths,
    pub remote: &'a CommitRef,
    /// Commit installé. Présent, un `main` qui a avancé est adopté ;
    /// un `main` qui n’est plus une mise à jour est refusé.
    pub installed: Option<&'a CommitRef>,
    /// Dépôt explicite. Sinon, [`configured_repo`] (variable d’environnement ou GitHub).
    pub repo: Option<&'a str>,
    pub fault: Fault,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refresh {
    /// `main` est encore une mise à jour. C’est ce commit qu’il faut construire.
    Ready(CommitRef),
    /// `main` ne propose plus rien de plus récent.
    Current,
}

pub fn installed_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn installed_commit() -> CommitRef {
    let unix = env!("EZERBOT_GIT_COMMIT_UNIX");
    CommitRef {
        sha: env!("EZERBOT_GIT_COMMIT").to_string(),
        unix: unix.parse::<i64>().ok().filter(|value| *value > 0),
    }
}

pub fn short_commit(sha: &str) -> String {
    let sha = sha.trim();
    if sha.chars().count() <= 7 {
        sha.to_string()
    } else {
        sha.chars().take(7).collect()
    }
}

pub fn paths_for(home: &Path, bundle: &Path, dev_root: &Path) -> UpdatePaths {
    UpdatePaths {
        work_dir: home.join("Library/Caches/Ezerbot/mise-a-jour/depot"),
        backup_dir: home.join("Library/Caches/Ezerbot/mise-a-jour/sauvegarde"),
        bundle: bundle.to_path_buf(),
        dev_root: dev_root.to_path_buf(),
        user_data: home.join("Library/Application Support/Grokbot"),
    }
}

pub fn bundle_from_executable(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    (bundle.file_name()? == "Ezerbot.app").then(|| bundle.to_path_buf())
}

pub fn running_bundle() -> Option<PathBuf> {
    bundle_from_executable(&std::env::current_exe().ok()?)
}

/// Racine git au-dessus du bundle, s’il vit dans un checkout. Ne sert qu’à
/// interdire ce dossier comme dépôt de travail.
pub fn detect_dev_root(bundle: &Path) -> Option<PathBuf> {
    let mut cursor = bundle.parent()?;
    loop {
        if cursor.join(".git").exists() {
            return Some(cursor.to_path_buf());
        }
        cursor = cursor.parent()?;
    }
}

pub fn production_paths() -> Result<UpdatePaths, UpdateError> {
    let bundle = std::env::var_os("EZERBOT_APP_BUNDLE")
        .map(PathBuf::from)
        .or_else(running_bundle)
        .ok_or_else(|| {
            UpdateError::message(
                "Ouvrez Ezerbot.app pour le mettre à jour. Le binaire de développement ne se remplace pas.",
            )
        })?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| UpdateError::message("Le dossier personnel est introuvable."))?;
    let dev_root = std::env::var_os("EZERBOT_DEV_ROOT")
        .map(PathBuf::from)
        .or_else(|| detect_dev_root(&bundle))
        .unwrap_or_else(|| home.join("Desktop/ezerbot-rust"));
    Ok(paths_for(&home, &bundle, &dev_root))
}

pub fn canonical_sha(input: &str) -> Result<String, UpdateError> {
    let sha = input.trim().to_ascii_lowercase();
    if sha.is_empty() || sha == "inconnu" {
        return Err(UpdateError::message(
            "Le commit installé est inconnu. Reconstruisez Ezerbot avec tools/build-app.sh.",
        ));
    }
    if !(7..=40).contains(&sha.len()) || !sha.chars().all(|character| character.is_ascii_hexdigit())
    {
        return Err(UpdateError::message("L’empreinte du commit est illisible."));
    }
    Ok(sha)
}

pub fn same_commit(left: &str, right: &str) -> Result<bool, UpdateError> {
    let left = canonical_sha(left)?;
    let right = canonical_sha(right)?;
    Ok(left == right || left.starts_with(&right) || right.starts_with(&left))
}

pub fn availability(
    installed: &CommitRef,
    remote: &CommitRef,
) -> Result<Availability, UpdateError> {
    let local = canonical_sha(&installed.sha)?;
    let distant = canonical_sha(&remote.sha)?;
    if same_commit(&local, &distant)? {
        return Ok(Availability::UpToDate);
    }
    if let (Some(local_unix), Some(remote_unix)) = (installed.unix, remote.unix) {
        if remote_unix <= local_unix {
            return Ok(Availability::UpToDate);
        }
    }
    Ok(Availability::Available(CommitRef {
        sha: distant,
        unix: remote.unix,
    }))
}

pub fn parse_github_commit(body: &str) -> Result<CommitRef, UpdateError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| UpdateError::message("La réponse GitHub est illisible. Réessayez."))?;
    let sha = value["sha"].as_str().ok_or_else(|| {
        UpdateError::message("La réponse GitHub ne contient pas le commit de main.")
    })?;
    let date = value["commit"]["committer"]["date"].as_str();
    Ok(CommitRef {
        sha: canonical_sha(sha)?,
        unix: date.and_then(parse_rfc3339_utc),
    })
}

pub fn parse_ls_remote(body: &str) -> Result<String, UpdateError> {
    for line in body.lines() {
        let mut parts = line.split_whitespace();
        let Some(sha) = parts.next() else { continue };
        if parts.next() == Some("refs/heads/main") {
            return canonical_sha(sha);
        }
    }
    Err(UpdateError::message(
        "git ls-remote n’a pas renvoyé la branche main.",
    ))
}

/// Préfère le corps JSON de l’API publique. Sinon, la ligne `ls-remote`.
pub fn resolve_remote(
    github_body: Result<String, UpdateError>,
    ls_remote: Option<&str>,
) -> Result<CommitRef, UpdateError> {
    match github_body {
        Ok(body) => parse_github_commit(&body),
        Err(error) => {
            let text = ls_remote.ok_or(error)?;
            Ok(CommitRef {
                sha: parse_ls_remote(text)?,
                unix: None,
            })
        }
    }
}

pub fn repo_from_setting(value: Option<&str>) -> Result<String, UpdateError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(REPO_URL.to_string());
    };
    local_repo_url(value)
}

pub fn configured_repo() -> Result<String, UpdateError> {
    repo_from_setting(std::env::var(UPDATE_REPO_ENV).ok().as_deref())
}

fn local_repo_url(value: &str) -> Result<String, UpdateError> {
    if value
        .chars()
        .any(|character| character.is_whitespace() || character == '\0' || character == '@')
    {
        return Err(UpdateError::message(
            "EZERBOT_UPDATE_REPO attend un dépôt local (chemin absolu ou file://).",
        ));
    }
    let path = if let Some(rest) = value.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        if !rest.starts_with('/') {
            return Err(UpdateError::message(
                "EZERBOT_UPDATE_REPO attend un dépôt local (chemin absolu ou file://).",
            ));
        }
        rest.to_string()
    } else if value.starts_with('/') {
        value.to_string()
    } else {
        return Err(UpdateError::message(
            "EZERBOT_UPDATE_REPO attend un dépôt local (chemin absolu ou file://). Aucun identifiant GitHub n’est utilisé.",
        ));
    };
    Ok(format!("file://{path}"))
}

pub fn fetch_remote(runner: &mut dyn CommandRunner) -> Result<CommitRef, UpdateError> {
    fetch_from(runner, &configured_repo()?)
}

pub fn fetch_from(runner: &mut dyn CommandRunner, repo: &str) -> Result<CommitRef, UpdateError> {
    let repo = if repo == REPO_URL {
        REPO_URL.to_string()
    } else {
        local_repo_url(repo)?
    };
    if repo != REPO_URL {
        return fetch_local(runner, &repo);
    }
    let github = github_commit_body();
    if let Ok(body) = github {
        return parse_github_commit(&body);
    }
    let output = ls_remote(runner, &repo)?;
    resolve_remote(
        Err(UpdateError::message("API GitHub indisponible.")),
        Some(&output),
    )
}

/// Relit `main` juste avant l’installation. Un échec réseau laisse l’appelant
/// garder le commit déjà annoncé : le clone adoptera ensuite la pointe réelle.
pub fn refresh_target(
    installed: &CommitRef,
    runner: &mut dyn CommandRunner,
) -> Result<Refresh, UpdateError> {
    match availability(installed, &fetch_remote(runner)?)? {
        Availability::UpToDate => Ok(Refresh::Current),
        Availability::Available(commit) => Ok(Refresh::Ready(commit)),
    }
}

fn fetch_local(runner: &mut dyn CommandRunner, repo: &str) -> Result<CommitRef, UpdateError> {
    let stdout = ls_remote(runner, repo)?;
    let sha = parse_ls_remote(&stdout)?;
    Ok(CommitRef {
        sha,
        unix: local_commit_unix(runner, repo),
    })
}

fn ls_remote(runner: &mut dyn CommandRunner, repo: &str) -> Result<String, UpdateError> {
    let output = runner.run(&CommandRequest {
        program: "git".into(),
        args: vec!["ls-remote".into(), repo.into(), "refs/heads/main".into()],
        cwd: std::env::temp_dir(),
        env: git_env(),
    })?;
    if output.status != 0 {
        return Err(UpdateError::with_log(
            "Impossible de lire la branche main. Réessayez.",
            command_log(&output),
        ));
    }
    Ok(output.stdout)
}

fn local_commit_unix(runner: &mut dyn CommandRunner, repo: &str) -> Option<i64> {
    let path = repo.strip_prefix("file://")?;
    let output = runner
        .run(&CommandRequest {
            program: "git".into(),
            args: vec![
                "-C".into(),
                path.into(),
                "log".into(),
                "-1".into(),
                "--format=%ct".into(),
                "refs/heads/main".into(),
            ],
            cwd: std::env::temp_dir(),
            env: git_env(),
        })
        .ok()?;
    if output.status != 0 {
        return None;
    }
    output.stdout.trim().parse().ok()
}

fn git_env() -> Vec<(String, String)> {
    vec![("GIT_TERMINAL_PROMPT".into(), "0".into())]
}

pub fn reconcile_checked_out(
    installed: &CommitRef,
    requested: &CommitRef,
    head: &CommitRef,
) -> Result<CommitRef, UpdateError> {
    if same_commit(&requested.sha, &head.sha)? {
        return Ok(CommitRef {
            sha: canonical_sha(&requested.sha)?,
            unix: requested.unix.or(head.unix),
        });
    }
    match availability(installed, head)? {
        Availability::Available(commit) => Ok(commit),
        Availability::UpToDate => Err(UpdateError::message(
            "La branche main ne propose plus de mise à jour. L’application actuelle est restée en place.",
        )),
    }
}

pub fn validate_paths(paths: &UpdatePaths) -> Result<(), UpdateError> {
    for path in [
        &paths.work_dir,
        &paths.backup_dir,
        &paths.bundle,
        &paths.dev_root,
        &paths.user_data,
    ] {
        if !path.is_absolute() {
            return Err(UpdateError::message(
                "Les chemins de mise à jour doivent être absolus.",
            ));
        }
    }
    if normalize(&paths.dev_root).components().count() < 3
        || normalize(&paths.user_data).components().count() < 3
    {
        return Err(UpdateError::message(
            "Le dossier de développement indiqué est trop large.",
        ));
    }
    if paths.bundle.file_name().and_then(|name| name.to_str()) != Some("Ezerbot.app") {
        return Err(UpdateError::message(
            "La mise à jour vise uniquement Ezerbot.app.",
        ));
    }
    if is_within(&paths.work_dir, &paths.dev_root)
        || is_within(&paths.backup_dir, &paths.dev_root)
        || is_within(&paths.dev_root, &paths.work_dir)
    {
        return Err(UpdateError::message(
            "Le dossier de développement n’est pas utilisé pour la mise à jour.",
        ));
    }
    if is_within(&paths.work_dir, &paths.user_data)
        || is_within(&paths.backup_dir, &paths.user_data)
        || is_within(&paths.bundle, &paths.user_data)
        || is_within(&paths.user_data, &paths.work_dir)
    {
        return Err(UpdateError::message(
            "Les conversations et les sessions ne sont pas modifiées par la mise à jour.",
        ));
    }
    if is_within(&paths.work_dir, &paths.bundle) || is_within(&paths.backup_dir, &paths.bundle) {
        return Err(UpdateError::message(
            "Le dossier de travail ne peut pas être l’application elle-même.",
        ));
    }
    Ok(())
}

pub fn apply(
    input: &ApplyInput<'_>,
    runner: &mut dyn CommandRunner,
    on_progress: &mut dyn FnMut(Phase),
) -> Result<(), UpdateError> {
    clear_touches();
    validate_paths(input.paths)?;
    // Un lien symbolique vers le checkout de développement est refusé avant
    // toute commande : `git -C` suivrait la cible.
    authorize(&input.paths.work_dir, input.paths)?;
    authorize(&input.paths.backup_dir, input.paths)?;
    let _ = canonical_sha(&input.remote.sha)?;
    let repo = match input.repo {
        Some(value) => repo_from_setting(Some(value))?,
        None => configured_repo()?,
    };
    on_progress(Phase::Telechargement);
    let target = sync_main(input.paths, input.remote, input.installed, &repo, runner)?;
    on_progress(Phase::Compilation);
    compile(input.paths, &target, runner)?;
    let built = input.paths.work_dir.join("app/build/Ezerbot.app");
    verify_bundle(&built, &target, input.paths)?;
    on_progress(Phase::Installation);
    install_bundle(&built, &target, input.paths, input.fault)?;
    on_progress(Phase::Redemarrage);
    Ok(())
}

fn sync_main(
    paths: &UpdatePaths,
    remote: &CommitRef,
    installed: Option<&CommitRef>,
    repo: &str,
    runner: &mut dyn CommandRunner,
) -> Result<CommitRef, UpdateError> {
    let parent = paths
        .work_dir
        .parent()
        .ok_or_else(|| UpdateError::message("Le dossier de travail n’a pas de dossier parent."))?;
    ensure_dir(parent, paths)?;
    let git_dir = paths.work_dir.join(".git");
    if git_dir.is_dir() {
        run(
            runner,
            paths,
            "git",
            vec![
                "-C".into(),
                path_arg(&paths.work_dir),
                "remote".into(),
                "set-url".into(),
                "origin".into(),
                repo.into(),
            ],
            &paths.work_dir,
            git_env(),
        )?;
        run(
            runner,
            paths,
            "git",
            vec![
                "-C".into(),
                path_arg(&paths.work_dir),
                "fetch".into(),
                "--depth".into(),
                "1".into(),
                "origin".into(),
                "main".into(),
            ],
            &paths.work_dir,
            git_env(),
        )?;
        run(
            runner,
            paths,
            "git",
            vec![
                "-C".into(),
                path_arg(&paths.work_dir),
                "checkout".into(),
                "--detach".into(),
                "--force".into(),
                "FETCH_HEAD".into(),
            ],
            &paths.work_dir,
            git_env(),
        )?;
    } else if path_occupied(&paths.work_dir)? {
        return Err(UpdateError::message(
            "Le dossier de travail existe déjà et n’est pas un clone Ezerbot. Rien n’a été effacé.",
        ));
    } else {
        run(
            runner,
            paths,
            "git",
            vec![
                "clone".into(),
                "--depth".into(),
                "1".into(),
                "--branch".into(),
                "main".into(),
                "--single-branch".into(),
                repo.into(),
                path_arg(&paths.work_dir),
            ],
            parent,
            git_env(),
        )?;
    }
    let head = run(
        runner,
        paths,
        "git",
        vec![
            "-C".into(),
            path_arg(&paths.work_dir),
            "rev-parse".into(),
            "HEAD".into(),
        ],
        &paths.work_dir,
        git_env(),
    )?;
    if head.status != 0 {
        return Err(UpdateError::with_log(
            "Le dépôt de travail ne correspond pas au commit de main. L’application actuelle est restée en place.",
            command_log(&head),
        ));
    }
    let sha = canonical_sha(head.stdout.trim())?;
    if same_commit(&sha, &remote.sha)? {
        return Ok(CommitRef {
            sha: canonical_sha(&remote.sha)?,
            unix: remote.unix,
        });
    }
    let Some(installed) = installed else {
        return Err(UpdateError::with_log(
            "Le dépôt de travail ne correspond pas au commit de main. L’application actuelle est restée en place.",
            command_log(&head),
        ));
    };
    let unix = commit_unix(runner, paths)?;
    reconcile_checked_out(installed, remote, &CommitRef { sha, unix })
}

fn commit_unix(
    runner: &mut dyn CommandRunner,
    paths: &UpdatePaths,
) -> Result<Option<i64>, UpdateError> {
    let output = run(
        runner,
        paths,
        "git",
        vec![
            "-C".into(),
            path_arg(&paths.work_dir),
            "log".into(),
            "-1".into(),
            "--format=%ct".into(),
            "HEAD".into(),
        ],
        &paths.work_dir,
        git_env(),
    )?;
    if output.status != 0 {
        return Ok(None);
    }
    Ok(output.stdout.trim().parse().ok())
}

fn compile(
    paths: &UpdatePaths,
    remote: &CommitRef,
    runner: &mut dyn CommandRunner,
) -> Result<(), UpdateError> {
    let script = paths.work_dir.join("tools/build-app.sh");
    authorize(&script, paths)?;
    if !script.is_file() {
        return Err(UpdateError::message(
            "Le script tools/build-app.sh est absent du dépôt. L’application actuelle est restée en place.",
        ));
    }
    let unix = remote
        .unix
        .map(|value| value.to_string())
        .unwrap_or_default();
    let output = run(
        runner,
        paths,
        "bash",
        vec![path_arg(&script)],
        &paths.work_dir,
        vec![
            ("EZERBOT_GIT_COMMIT".into(), remote.sha.clone()),
            ("EZERBOT_GIT_COMMIT_UNIX".into(), unix),
        ],
    )?;
    if output.status != 0 {
        return Err(UpdateError::with_log(
            "La compilation a échoué. L’application actuelle est restée en place.",
            command_log(&output),
        ));
    }
    Ok(())
}

fn install_bundle(
    source: &Path,
    remote: &CommitRef,
    paths: &UpdatePaths,
    fault: Fault,
) -> Result<(), UpdateError> {
    verify_bundle(source, remote, paths)?;
    ensure_dir(&paths.backup_dir, paths)?;
    let backup = paths.backup_dir.join("Ezerbot.app");
    if exists(&backup) {
        remove_tree(&backup, paths)?;
    }
    copy_path(&paths.bundle, &backup, paths)?;
    let incoming = paths.bundle.with_file_name("Ezerbot.app.incoming");
    let previous = paths.bundle.with_file_name("Ezerbot.app.previous");
    if exists(&incoming) {
        remove_tree(&incoming, paths)?;
    }
    if exists(&previous) {
        remove_tree(&previous, paths)?;
    }
    copy_path(source, &incoming, paths)?;
    verify_bundle(&incoming, remote, paths)?;
    rename_path(&paths.bundle, &previous, paths, Fault::None)?;
    if let Err(error) = rename_path(&incoming, &paths.bundle, paths, fault) {
        let restored = rename_path(&previous, &paths.bundle, paths, Fault::None);
        if exists(&incoming) {
            let _ = remove_tree(&incoming, paths);
        }
        if restored.is_err() {
            return Err(UpdateError::with_log(
                "Le remplacement a échoué et l’application n’a pas pu être remise en place. La sauvegarde est dans le dossier de mise à jour.",
                error.log,
            ));
        }
        return Err(UpdateError::message(
            "Le remplacement n’a pas pu être terminé. L’application actuelle a été remise en place.",
        ));
    }
    if exists(&previous) {
        let _ = remove_tree(&previous, paths);
    }
    Ok(())
}

fn verify_bundle(
    bundle: &Path,
    remote: &CommitRef,
    paths: &UpdatePaths,
) -> Result<(), UpdateError> {
    let executable = bundle.join("Contents/MacOS/Ezerbot");
    let wrapper = bundle.join("Contents/MacOS/grokbot-wrapper");
    let plist_path = bundle.join("Contents/Info.plist");
    for file in [&executable, &wrapper, &plist_path] {
        authorize(file, paths)?;
        if !file.is_file() {
            return Err(UpdateError::message(format!(
                "Le nouveau bundle est incomplet ({}). L’application actuelle est restée en place.",
                file.display()
            )));
        }
    }
    let plist = fs::read_to_string(&plist_path).map_err(|_| {
        UpdateError::message(
            "Le fichier Info.plist du nouveau bundle est illisible. L’application actuelle est restée en place.",
        )
    })?;
    if plist_value(&plist, "CFBundleExecutable").as_deref() != Some("Ezerbot") {
        return Err(UpdateError::message(
            "Le nouveau bundle n’est pas Ezerbot. L’application actuelle est restée en place.",
        ));
    }
    let Some(found) = plist_value(&plist, "EzerbotGitCommit") else {
        return Err(UpdateError::message(
            "Le bundle compilé n’indique pas son commit. L’application actuelle est restée en place.",
        ));
    };
    if !same_commit(&found, &remote.sha).unwrap_or(false) {
        return Err(UpdateError::message(
            "Le bundle compilé ne correspond pas au commit de main. L’application actuelle est restée en place.",
        ));
    }
    Ok(())
}

fn run(
    runner: &mut dyn CommandRunner,
    paths: &UpdatePaths,
    program: &str,
    args: Vec<String>,
    cwd: &Path,
    env: Vec<(String, String)>,
) -> Result<CommandOutput, UpdateError> {
    authorize(cwd, paths)?;
    let dev = normalize(&paths.dev_root);
    let user = normalize(&paths.user_data);
    let dev_text = dev.to_string_lossy();
    let user_text = user.to_string_lossy();
    for arg in &args {
        if arg == "reset" || arg == "--hard" {
            return Err(UpdateError::message(
                "La mise à jour n’utilise pas git reset.",
            ));
        }
        let candidate = Path::new(arg);
        if candidate.is_absolute() {
            authorize(candidate, paths)?;
        } else if arg.contains(dev_text.as_ref()) || arg.contains(user_text.as_ref()) {
            return Err(UpdateError::message(
                "Le dossier de développement n’est pas utilisé pour la mise à jour.",
            ));
        }
    }
    runner.run(&CommandRequest {
        program: program.into(),
        args,
        cwd: cwd.to_path_buf(),
        env,
    })
}

fn rename_path(
    from: &Path,
    to: &Path,
    paths: &UpdatePaths,
    fault: Fault,
) -> Result<(), UpdateError> {
    authorize(from, paths)?;
    authorize(to, paths)?;
    if fault == Fault::FinalRename && normalize(to) == normalize(&paths.bundle) {
        return Err(UpdateError::message("Le renommage final a été interrompu."));
    }
    fs::rename(from, to).map_err(|error| io_error("Déplacement impossible", to, error))
}

/// Fichiers de session réels, à côté de `workspace.sqlite`.
/// `workspace.*-auth.json`, `workspace.provider.json`, et tout nom `*.sqlite*`.
fn preserved_session_name(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    (name.starts_with("workspace.") && name.ends_with("-auth.json"))
        || name == "workspace.provider.json"
        || name.contains(".sqlite")
}

fn copy_path(src: &Path, dst: &Path, paths: &UpdatePaths) -> Result<(), UpdateError> {
    if src.file_name().is_some_and(preserved_session_name) {
        return Ok(());
    }
    authorize(src, paths)?;
    authorize(dst, paths)?;
    let metadata =
        fs::symlink_metadata(src).map_err(|error| io_error("Lecture impossible", src, error))?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(src).map_err(|error| io_error("Lien illisible", src, error))?;
        if exists(dst) {
            let _ = fs::remove_file(dst);
        }
        std::os::unix::fs::symlink(&target, dst)
            .map_err(|error| io_error("Lien impossible", dst, error))?;
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir_all(dst).map_err(|error| io_error("Dossier impossible", dst, error))?;
        for entry in
            fs::read_dir(src).map_err(|error| io_error("Lecture impossible", src, error))?
        {
            let entry = entry.map_err(|error| io_error("Lecture impossible", src, error))?;
            copy_path(&entry.path(), &dst.join(entry.file_name()), paths)?;
        }
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("Dossier impossible", parent, error))?;
    }
    fs::copy(src, dst).map_err(|error| io_error("Copie impossible", dst, error))?;
    fs::set_permissions(dst, metadata.permissions())
        .map_err(|error| io_error("Droits impossibles", dst, error))?;
    Ok(())
}

fn remove_tree(path: &Path, paths: &UpdatePaths) -> Result<(), UpdateError> {
    authorize(path, paths)?;
    if exists(path) {
        fs::remove_dir_all(path)
            .map_err(|error| io_error("Suppression impossible", path, error))?;
    }
    Ok(())
}

fn ensure_dir(path: &Path, paths: &UpdatePaths) -> Result<(), UpdateError> {
    authorize(path, paths)?;
    fs::create_dir_all(path).map_err(|error| io_error("Dossier impossible", path, error))
}

fn authorize(path: &Path, paths: &UpdatePaths) -> Result<(), UpdateError> {
    let path = normalize(path);
    note(&path);
    if !path.is_absolute() {
        return Err(UpdateError::message(
            "Les chemins de mise à jour doivent être absolus.",
        ));
    }
    let resolved = resolve_path(&path);
    if resolved != path {
        note(&resolved);
    }
    let dev = resolve_path(&paths.dev_root);
    let user = resolve_path(&paths.user_data);
    let in_dev = is_within(&path, &paths.dev_root) || is_within(&resolved, &dev);
    if in_dev && !bundle_operation(&path, paths) && !bundle_operation(&resolved, paths) {
        return Err(UpdateError::message(
            "Le dossier de développement n’est pas utilisé pour la mise à jour.",
        ));
    }
    if is_within(&path, &paths.user_data) || is_within(&resolved, &user) {
        return Err(UpdateError::message(
            "Les conversations et les sessions ne sont pas modifiées par la mise à jour.",
        ));
    }
    let work = resolve_path(&paths.work_dir);
    let backup = resolve_path(&paths.backup_dir);
    if is_within(&work, &dev)
        || is_within(&backup, &dev)
        || is_within(&work, &user)
        || is_within(&backup, &user)
    {
        return Err(UpdateError::message(
            "Le dossier de développement n’est pas utilisé pour la mise à jour.",
        ));
    }
    if !allowed(&path, paths) || !cache_allows(&path, &resolved, paths, &work, &backup) {
        return Err(UpdateError::message(
            "La mise à jour a refusé un chemin hors du dossier de travail.",
        ));
    }
    Ok(())
}

/// Résout les liens existants sans exiger que le dernier composant existe.
fn resolve_path(path: &Path) -> PathBuf {
    let lexical = normalize(path);
    let mut pending = Vec::new();
    let mut cursor = lexical.clone();
    loop {
        if let Ok(real) = fs::canonicalize(&cursor) {
            let mut out = real;
            for part in pending.iter().rev() {
                out.push(part);
            }
            return normalize(&out);
        }
        let Some(name) = cursor.file_name().map(|name| name.to_os_string()) else {
            return lexical;
        };
        pending.push(name);
        if !cursor.pop() {
            return lexical;
        }
    }
}

fn cache_allows(
    lexical: &Path,
    resolved: &Path,
    paths: &UpdatePaths,
    work: &Path,
    backup: &Path,
) -> bool {
    let in_work = (is_within(lexical, &paths.work_dir) || parent_of(lexical, &paths.work_dir))
        && (is_within(resolved, work) || parent_of(resolved, work));
    let in_backup = (is_within(lexical, &paths.backup_dir)
        || parent_of(lexical, &paths.backup_dir))
        && (is_within(resolved, backup) || parent_of(resolved, backup));
    in_work || in_backup || bundle_operation(lexical, paths) || bundle_operation(resolved, paths)
}

fn allowed(path: &Path, paths: &UpdatePaths) -> bool {
    is_within(path, &paths.work_dir)
        || is_within(path, &paths.backup_dir)
        || bundle_operation(path, paths)
        || parent_of(path, &paths.work_dir)
        || parent_of(path, &paths.backup_dir)
}

fn bundle_operation(path: &Path, paths: &UpdatePaths) -> bool {
    let path = normalize(path);
    let bundle = normalize(&paths.bundle);
    if path == bundle || path.starts_with(&bundle) {
        return true;
    }
    let Some(parent) = bundle.parent() else {
        return false;
    };
    path.ancestors().any(|ancestor| {
        let Some(name) = ancestor.file_name().and_then(|value| value.to_str()) else {
            return false;
        };
        ancestor.parent() == Some(parent) && is_staging_name(name)
    })
}

fn is_staging_name(name: &str) -> bool {
    name == "Ezerbot.app.incoming" || name == "Ezerbot.app.previous"
}

fn parent_of(path: &Path, child: &Path) -> bool {
    normalize(child).parent() == Some(path)
}

fn is_within(path: &Path, root: &Path) -> bool {
    let path = normalize(path);
    let root = normalize(root);
    path == root || path.starts_with(&root)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir => out.push(component.as_os_str()),
            Component::Normal(value) => out.push(value),
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
        }
    }
    out
}

fn path_occupied(path: &Path) -> Result<bool, UpdateError> {
    match fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("Lecture impossible", path, error)),
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn note(path: &Path) {
    let path = path.to_path_buf();
    TOUCHED.with(|touched| touched.borrow_mut().push(path));
}

fn clear_touches() {
    TOUCHED.with(|touched| touched.borrow_mut().clear());
}

#[cfg(test)]
fn take_touches() -> Vec<PathBuf> {
    TOUCHED.with(|touched| std::mem::take(&mut *touched.borrow_mut()))
}

fn io_error(action: &str, path: &Path, error: io::Error) -> UpdateError {
    UpdateError::message(format!("{action} ({}) : {error}", path.display()))
}

fn command_log(output: &CommandOutput) -> String {
    let mut combined = String::new();
    if !output.stdout.trim().is_empty() {
        combined.push_str(output.stdout.trim());
        combined.push('\n');
    }
    if !output.stderr.trim().is_empty() {
        combined.push_str(output.stderr.trim());
    }
    tail(combined.trim(), LOG_LIMIT)
}

fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    text.chars().skip(count - max).collect()
}

fn plist_value(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let index = xml.find(&needle)?;
    let rest = &xml[index + needle.len()..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = start + rest[start..].find("</string>")?;
    Some(rest[start..end].trim().to_string())
}

fn github_commit_body() -> Result<String, UpdateError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    match agent
        .get(GITHUB_COMMIT_URL)
        .set("User-Agent", "ezerbot")
        .set("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => response
            .into_string()
            .map_err(|_| UpdateError::message("La réponse GitHub est illisible. Réessayez.")),
        Err(ureq::Error::Status(code, _)) => Err(UpdateError::message(format!(
            "L’API GitHub a répondu {code}."
        ))),
        Err(_) => Err(UpdateError::message(
            "L’API GitHub est injoignable. Réessayez.",
        )),
    }
}

fn parse_rfc3339_utc(value: &str) -> Option<i64> {
    let value = value.trim().strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;
    let mut dates = date.split('-');
    let year: i32 = dates.next()?.parse().ok()?;
    let month: u32 = dates.next()?.parse().ok()?;
    let day: u32 = dates.next()?.parse().ok()?;
    if dates.next().is_some() {
        return None;
    }
    let mut times = time.split(':');
    let hour: u32 = times.next()?.parse().ok()?;
    let minute: u32 = times.next()?.parse().ok()?;
    let second: u32 = times.next()?.parse().ok()?;
    if times.next().is_some()
        || !(1..=12).contains(&month)
        || day == 0
        || day > 31
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let year_adjusted = if month <= 2 { year - 1 } else { year };
    let era = if year_adjusted >= 0 {
        year_adjusted
    } else {
        year_adjusted - 399
    } / 400;
    let year_of_era = (year_adjusted - era * 400) as u32;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era as i64 * 146_097 + day_of_era as i64 - 719_468;
    Some(days * 86_400 + hour as i64 * 3_600 + minute as i64 * 60 + second as i64)
}

pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&mut self, request: &CommandRequest) -> Result<CommandOutput, UpdateError> {
        let mut command = Command::new(&request.program);
        command.args(&request.args).current_dir(&request.cwd).envs(
            request
                .env
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
        let output = command.output().map_err(|_| {
            UpdateError::message(format!(
                "La commande {} est introuvable. Installez-la, puis réessayez.",
                request.program
            ))
        })?;
        Ok(CommandOutput {
            status: output.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn hex_sha(fill: char) -> String {
        fill.to_string().repeat(40)
    }

    fn commit(sha: &str, unix: Option<i64>) -> CommitRef {
        CommitRef {
            sha: sha.to_string(),
            unix,
        }
    }

    struct Layout {
        root: PathBuf,
        paths: UpdatePaths,
    }

    fn layout(name: &str, bundle_in_dev: bool) -> Layout {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ezerbot-maj-{}-{}-{}",
            name,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let dev = root.join("dev");
        fs::create_dir_all(dev.join(".git")).unwrap();
        fs::write(dev.join("canary.txt"), "ne-pas-toucher").unwrap();
        fs::write(dev.join("src-secret.txt"), "code-local").unwrap();
        let user = root.join("user-data");
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("workspace.sqlite"), "sessions-intactes").unwrap();
        for name in [
            "workspace.codex-auth.json",
            "workspace.chatgpt-auth.json",
            "workspace.provider.json",
            "workspace.sqlite-wal",
            "workspace.sqlite-shm",
            "workspace.sqlite-journal",
        ] {
            fs::write(user.join(name), format!("kept-{name}")).unwrap();
        }
        let bundle = if bundle_in_dev {
            dev.join("app/build/Ezerbot.app")
        } else {
            root.join("Applications/Ezerbot.app")
        };
        write_app(&bundle, "ancienne-version", &hex_sha('1'));
        let paths = UpdatePaths {
            work_dir: root.join("caches/depot"),
            backup_dir: root.join("caches/sauvegarde"),
            bundle,
            dev_root: dev,
            user_data: user,
        };
        Layout { root, paths }
    }

    impl Drop for Layout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_app(bundle: &Path, marker: &str, sha: &str) {
        let macos = bundle.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("Ezerbot"), marker).unwrap();
        fs::write(macos.join("grokbot-wrapper"), format!("wrapper-{marker}")).unwrap();
        fs::write(
            bundle.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>Ezerbot</string>
  <key>EzerbotGitCommit</key><string>{sha}</string>
</dict></plist>
"#
            ),
        )
        .unwrap();
    }

    fn marker(bundle: &Path) -> String {
        fs::read_to_string(bundle.join("Contents/MacOS/Ezerbot")).unwrap()
    }

    struct Scripted {
        remote: CommitRef,
        work_dir: PathBuf,
        build_status: i32,
        log: String,
        calls: Vec<CommandRequest>,
        head_unix: Option<i64>,
    }

    impl CommandRunner for Scripted {
        fn run(&mut self, request: &CommandRequest) -> Result<CommandOutput, UpdateError> {
            self.calls.push(request.clone());
            let git =
                |name: &str| request.program == "git" && request.args.iter().any(|arg| arg == name);
            if git("rev-parse") {
                return Ok(CommandOutput {
                    status: 0,
                    stdout: format!("{}\n", self.remote.sha),
                    stderr: String::new(),
                });
            }
            if git("clone") || git("fetch") || git("checkout") {
                fs::create_dir_all(self.work_dir.join(".git")).unwrap();
                fs::create_dir_all(self.work_dir.join("tools")).unwrap();
                fs::write(
                    self.work_dir.join("tools/build-app.sh"),
                    "#!/bin/bash\nexit 0\n",
                )
                .unwrap();
                return Ok(CommandOutput {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            if git("log") {
                return Ok(CommandOutput {
                    status: 0,
                    stdout: self
                        .head_unix
                        .map(|value| format!("{value}\n"))
                        .unwrap_or_default(),
                    stderr: String::new(),
                });
            }
            if request.args.iter().any(|arg| arg.ends_with("build-app.sh")) {
                if self.build_status != 0 {
                    return Ok(CommandOutput {
                        status: self.build_status,
                        stdout: String::new(),
                        stderr: self.log.clone(),
                    });
                }
                write_app(
                    &self.work_dir.join("app/build/Ezerbot.app"),
                    "nouvelle-version",
                    &self.remote.sha,
                );
                return Ok(CommandOutput {
                    status: 0,
                    stdout: "Application créée\n".into(),
                    stderr: String::new(),
                });
            }
            Ok(CommandOutput {
                status: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn scripted(paths: &UpdatePaths, remote: CommitRef, build_status: i32, log: &str) -> Scripted {
        Scripted {
            remote,
            work_dir: paths.work_dir.clone(),
            build_status,
            log: log.into(),
            calls: Vec::new(),
            head_unix: None,
        }
    }

    fn applying<'a>(paths: &'a UpdatePaths, remote: &'a CommitRef, fault: Fault) -> ApplyInput<'a> {
        ApplyInput {
            paths,
            remote,
            installed: None,
            repo: None,
            fault,
        }
    }

    fn assert_outside_dev(runner: &Scripted, dev: &Path) {
        let dev_text = dev.to_string_lossy().into_owned();
        for call in &runner.calls {
            assert!(
                !call.cwd.starts_with(dev),
                "cwd dans le dépôt de développement : {}",
                call.cwd.display()
            );
            assert!(!call
                .args
                .iter()
                .any(|arg| arg == "reset" || arg == "--hard"));
            for arg in &call.args {
                assert!(
                    !arg.contains(&dev_text),
                    "argument dans le dépôt de développement : {arg}"
                );
            }
        }
    }

    #[test]
    fn meme_empreinte_est_a_jour() {
        let sha = hex_sha('a');
        let installed = commit(&sha, Some(10));
        let remote = commit(&sha.to_uppercase(), Some(99));
        assert_eq!(
            availability(&installed, &remote).unwrap(),
            Availability::UpToDate
        );
    }

    #[test]
    fn prefixe_court_designe_le_meme_commit() {
        let full = hex_sha('c');
        let installed = commit(&full[..12], Some(1));
        let remote = commit(&full.to_uppercase(), Some(50));
        assert_eq!(
            availability(&installed, &remote).unwrap(),
            Availability::UpToDate
        );
    }

    #[test]
    fn commit_distant_plus_recent_propose_la_mise_a_jour() {
        let installed = commit(&hex_sha('a'), Some(100));
        let remote = commit(&hex_sha('b'), Some(200));
        match availability(&installed, &remote).unwrap() {
            Availability::Available(commit) => assert_eq!(commit.sha, hex_sha('b')),
            Availability::UpToDate => panic!("main plus récent doit être proposé"),
        }
    }

    #[test]
    fn commit_distant_plus_ancien_ne_propose_pas_de_mise_a_jour() {
        let installed = commit(&hex_sha('a'), Some(300));
        let remote = commit(&hex_sha('b'), Some(200));
        assert_eq!(
            availability(&installed, &remote).unwrap(),
            Availability::UpToDate
        );
    }

    #[test]
    fn meme_instant_ne_declenche_pas_la_mise_a_jour() {
        let installed = commit(&hex_sha('a'), Some(50));
        let remote = commit(&hex_sha('b'), Some(50));
        assert_eq!(
            availability(&installed, &remote).unwrap(),
            Availability::UpToDate
        );
    }

    #[test]
    fn empreintes_differentes_sans_date_proposent_la_mise_a_jour() {
        let installed = commit(&hex_sha('a'), None);
        let remote = commit(&hex_sha('b'), None);
        assert!(matches!(
            availability(&installed, &remote).unwrap(),
            Availability::Available(_)
        ));
    }

    #[test]
    fn empreinte_illisible_ou_inconnue() {
        let remote = commit(&hex_sha('a'), Some(1));
        assert!(availability(&commit("pas-un-sha", None), &remote).is_err());
        assert!(availability(&commit("abc", None), &remote).is_err());
        let error = availability(&commit("inconnu", None), &remote).unwrap_err();
        assert!(error.message.contains("inconnu"));
    }

    #[test]
    fn api_github_puis_ls_remote() {
        let sha = hex_sha('d');
        let body = format!(
            r#"{{"sha":"{sha}","commit":{{"committer":{{"date":"2020-01-01T00:00:00Z"}}}}}}"#
        );
        let parsed = parse_github_commit(&body).unwrap();
        assert_eq!(parsed.sha, sha);
        assert_eq!(parsed.unix, Some(1_577_836_800));
        let other = hex_sha('e');
        let preferred =
            resolve_remote(Ok(body), Some(&format!("{other}\trefs/heads/main\n"))).unwrap();
        assert_eq!(preferred.sha, sha);
        assert!(preferred.unix.is_some());
        let fallback = resolve_remote(
            Err(UpdateError::message("réseau")),
            Some(&format!("{other}\trefs/heads/main\n")),
        )
        .unwrap();
        assert_eq!(fallback.sha, other);
        assert_eq!(fallback.unix, None);
        assert!(parse_ls_remote("abcdef0123456789\trefs/heads/other\n").is_err());
        assert!(parse_github_commit("non json").is_err());
    }

    #[test]
    fn chemins_par_defaut_hors_du_depot_et_des_sessions() {
        let home = Path::new("/Users/charles");
        let dev = home.join("Desktop/ezerbot-rust");
        let bundle = dev.join("app/build/Ezerbot.app");
        let paths = paths_for(home, &bundle, &dev);
        assert!(paths
            .work_dir
            .ends_with("Library/Caches/Ezerbot/mise-a-jour/depot"));
        assert!(paths
            .backup_dir
            .ends_with("Library/Caches/Ezerbot/mise-a-jour/sauvegarde"));
        assert_eq!(
            paths.user_data,
            home.join("Library/Application Support/Grokbot")
        );
        assert!(validate_paths(&paths).is_ok());
        assert!(!is_within(&paths.work_dir, &dev));
        assert!(!is_within(&paths.backup_dir, &paths.user_data));
        let executable = bundle.join("Contents/MacOS/grokbot-wrapper");
        assert_eq!(
            bundle_from_executable(&executable).as_deref(),
            Some(bundle.as_path())
        );
        assert_eq!(RELAUNCH_EXIT_CODE, 86);
    }

    #[test]
    fn dossier_de_travail_dans_le_depot_est_refuse_sans_commande() {
        let fixture = layout("refuse", false);
        let mut paths = fixture.paths.clone();
        paths.work_dir = paths.dev_root.join("depot");
        let remote = commit(&hex_sha('b'), Some(200));
        let mut runner = scripted(&paths, remote.clone(), 0, "");
        let mut phases = Vec::new();
        let error = apply(
            &applying(&paths, &remote, Fault::None),
            &mut runner,
            &mut |phase| {
                phases.push(phase);
            },
        )
        .unwrap_err();
        assert!(error.message.contains("développement"));
        assert!(runner.calls.is_empty());
        assert!(phases.is_empty());
        assert!(!paths.work_dir.exists());
        assert_eq!(
            fs::read_to_string(fixture.paths.dev_root.join("canary.txt")).unwrap(),
            "ne-pas-toucher"
        );
        assert!(take_touches().is_empty());
    }

    #[test]
    fn echec_de_compilation_conserve_l_application() {
        let fixture = layout("echec", false);
        let remote = commit(&hex_sha('b'), Some(200));
        let before = fs::read(fixture.paths.bundle.join("Contents/MacOS/Ezerbot")).unwrap();
        let mut runner = scripted(
            &fixture.paths,
            remote.clone(),
            1,
            "error[E0425]: impossible de compiler le crate\n",
        );
        let mut phases = Vec::new();
        let error = apply(
            &applying(&fixture.paths, &remote, Fault::None),
            &mut runner,
            &mut |phase| phases.push(phase),
        )
        .unwrap_err();
        assert!(error.message.contains("compilation"));
        assert!(error.message.contains("restée en place"));
        assert!(error.log.contains("E0425"));
        assert_eq!(
            fs::read(fixture.paths.bundle.join("Contents/MacOS/Ezerbot")).unwrap(),
            before
        );
        assert!(!fixture.paths.backup_dir.join("Ezerbot.app").exists());
        assert!(!fixture
            .paths
            .bundle
            .with_file_name("Ezerbot.app.incoming")
            .exists());
        assert!(!fixture
            .paths
            .bundle
            .with_file_name("Ezerbot.app.previous")
            .exists());
        assert_eq!(phases, vec![Phase::Telechargement, Phase::Compilation]);
        assert_sessions(&fixture.paths.user_data);
        assert_outside_dev(&runner, &fixture.paths.dev_root);
    }

    #[test]
    fn remplacement_atomique_installe_la_nouvelle_et_sauvegarde_l_ancienne() {
        let fixture = layout("atome", false);
        let remote = commit(&hex_sha('b'), Some(200));
        let mut runner = scripted(&fixture.paths, remote.clone(), 0, "");
        let mut phases = Vec::new();
        apply(
            &applying(&fixture.paths, &remote, Fault::None),
            &mut runner,
            &mut |phase| phases.push(phase),
        )
        .unwrap();
        assert_eq!(marker(&fixture.paths.bundle), "nouvelle-version");
        assert_eq!(
            marker(&fixture.paths.backup_dir.join("Ezerbot.app")),
            "ancienne-version"
        );
        assert_eq!(
            phases,
            vec![
                Phase::Telechargement,
                Phase::Compilation,
                Phase::Installation,
                Phase::Redemarrage,
            ]
        );
        assert_eq!(
            fs::read_to_string(fixture.paths.dev_root.join("canary.txt")).unwrap(),
            "ne-pas-toucher"
        );
        assert_eq!(
            fs::read_to_string(fixture.paths.dev_root.join("src-secret.txt")).unwrap(),
            "code-local"
        );
        assert_sessions(&fixture.paths.user_data);
        assert_outside_dev(&runner, &fixture.paths.dev_root);
        let dev = fixture.paths.dev_root.clone();
        for path in take_touches() {
            assert!(
                !is_within(&path, &dev),
                "accès au dossier de développement : {}",
                path.display()
            );
        }
    }

    #[test]
    fn echec_du_remplacement_restaure_l_application() {
        let fixture = layout("restauration", false);
        let remote = commit(&hex_sha('b'), Some(200));
        let mut runner = scripted(&fixture.paths, remote.clone(), 0, "");
        let error = apply(
            &applying(&fixture.paths, &remote, Fault::FinalRename),
            &mut runner,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.message.contains("remise en place"));
        assert_eq!(marker(&fixture.paths.bundle), "ancienne-version");
        assert!(!fixture
            .paths
            .bundle
            .with_file_name("Ezerbot.app.previous")
            .exists());
        assert!(!fixture
            .paths
            .bundle
            .with_file_name("Ezerbot.app.incoming")
            .exists());
        assert_eq!(
            marker(&fixture.paths.backup_dir.join("Ezerbot.app")),
            "ancienne-version"
        );
    }

    #[test]
    fn lien_vers_le_depot_de_developpement_est_refuse_sans_commande() {
        let fixture = layout("lien", false);
        let paths = fixture.paths.clone();
        fs::create_dir_all(paths.work_dir.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&paths.dev_root, &paths.work_dir).unwrap();
        let remote = commit(&hex_sha('b'), Some(200));
        let mut runner = scripted(&paths, remote.clone(), 0, "");
        let mut phases = Vec::new();
        let error = apply(
            &applying(&paths, &remote, Fault::None),
            &mut runner,
            &mut |phase| {
                phases.push(phase);
            },
        )
        .unwrap_err();
        assert!(error.message.contains("développement"), "{}", error.message);
        assert!(runner.calls.is_empty());
        assert!(phases.is_empty());
        assert_eq!(
            fs::read_to_string(paths.dev_root.join("canary.txt")).unwrap(),
            "ne-pas-toucher"
        );
        assert_eq!(
            fs::read_to_string(paths.dev_root.join("src-secret.txt")).unwrap(),
            "code-local"
        );
        assert_sessions(&paths.user_data);
    }

    fn assert_sessions(user: &Path) {
        assert_eq!(
            fs::read_to_string(user.join("workspace.sqlite")).unwrap(),
            "sessions-intactes"
        );
        for name in [
            "workspace.codex-auth.json",
            "workspace.chatgpt-auth.json",
            "workspace.provider.json",
            "workspace.sqlite-wal",
            "workspace.sqlite-shm",
            "workspace.sqlite-journal",
        ] {
            assert_eq!(
                fs::read_to_string(user.join(name)).unwrap(),
                format!("kept-{name}"),
                "{name}"
            );
        }
    }

    #[test]
    fn la_copie_ne_remplace_pas_les_noms_reels_de_session() {
        let fixture = layout("noms", false);
        let names = [
            "workspace.codex-auth.json",
            "workspace.chatgpt-auth.json",
            "workspace.provider.json",
            "workspace.sqlite",
            "workspace.sqlite-wal",
            "workspace.sqlite-shm",
            "workspace.sqlite-journal",
        ];
        for name in names {
            fs::write(fixture.paths.bundle.join(name), format!("bundle-{name}")).unwrap();
        }
        let staged = fixture.paths.work_dir.join("staged");
        write_app(&staged, "binaire-nouveau", &hex_sha('c'));
        fs::create_dir_all(staged.join("Contents/Resources")).unwrap();
        fs::write(staged.join("Contents/Resources/notes.txt"), "release-notes").unwrap();
        for name in names {
            fs::write(staged.join(name), "wiped").unwrap();
            fs::write(staged.join("Contents").join(name), "wiped").unwrap();
        }
        copy_path(&staged, &fixture.paths.bundle, &fixture.paths).unwrap();
        assert_eq!(marker(&fixture.paths.bundle), "binaire-nouveau");
        assert_eq!(
            fs::read(fixture.paths.bundle.join("Contents/Resources/notes.txt")).unwrap(),
            b"release-notes"
        );
        for name in names {
            assert_eq!(
                fs::read_to_string(fixture.paths.bundle.join(name)).unwrap(),
                format!("bundle-{name}"),
                "{name}"
            );
            assert!(
                !fixture.paths.bundle.join("Contents").join(name).exists(),
                "{name}"
            );
        }
        assert_sessions(&fixture.paths.user_data);
        let refused = copy_path(&staged, &fixture.paths.user_data, &fixture.paths);
        assert!(refused.is_err());
        assert_sessions(&fixture.paths.user_data);
    }

    #[test]
    fn aucun_acces_au_dossier_de_developpement_meme_si_l_app_y_est_installee() {
        let fixture = layout("checkout", true);
        let remote = commit(&hex_sha('b'), Some(200));
        let mut runner = scripted(&fixture.paths, remote.clone(), 0, "");
        apply(
            &applying(&fixture.paths, &remote, Fault::None),
            &mut runner,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(marker(&fixture.paths.bundle), "nouvelle-version");
        assert_eq!(
            fs::read_to_string(fixture.paths.dev_root.join("canary.txt")).unwrap(),
            "ne-pas-toucher"
        );
        assert_eq!(
            fs::read_to_string(fixture.paths.dev_root.join("src-secret.txt")).unwrap(),
            "code-local"
        );
        assert_outside_dev(&runner, &fixture.paths.dev_root);
        for path in take_touches() {
            if is_within(&path, &fixture.paths.dev_root) {
                assert!(
                    bundle_operation(&path, &fixture.paths),
                    "accès hors de l’application : {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn depot_local_accepte_seulement_un_chemin_ou_file() {
        assert_eq!(repo_from_setting(None).unwrap(), REPO_URL);
        assert_eq!(repo_from_setting(Some("  ")).unwrap(), REPO_URL);
        assert_eq!(
            repo_from_setting(Some("file:///tmp/ezerbot-fictif")).unwrap(),
            "file:///tmp/ezerbot-fictif"
        );
        assert_eq!(
            repo_from_setting(Some("/tmp/ezerbot-fictif")).unwrap(),
            "file:///tmp/ezerbot-fictif"
        );
        assert_eq!(
            repo_from_setting(Some("file://localhost/tmp/ezerbot-fictif")).unwrap(),
            "file:///tmp/ezerbot-fictif"
        );
        assert!(repo_from_setting(Some("https://github.com/autre/depot.git")).is_err());
        assert!(repo_from_setting(Some("git@github.com:charlescstpierr/ezerbot.git")).is_err());
        assert!(repo_from_setting(Some("depot-relatif")).is_err());
    }

    #[test]
    fn main_qui_avance_est_adopte_et_un_retour_arriere_est_refuse() {
        let installed = commit(&hex_sha('a'), Some(100));
        let advertised = commit(&hex_sha('b'), Some(200));
        let advanced = commit(&hex_sha('c'), Some(300));
        assert_eq!(
            reconcile_checked_out(&installed, &advertised, &advanced)
                .unwrap()
                .sha,
            advanced.sha
        );
        assert_eq!(
            reconcile_checked_out(&installed, &advanced, &advanced)
                .unwrap()
                .sha,
            advanced.sha
        );
        let rolled = commit(&hex_sha('d'), Some(50));
        let error = reconcile_checked_out(&installed, &advertised, &rolled).unwrap_err();
        assert!(
            error.message.contains("ne propose plus"),
            "{}",
            error.message
        );

        let fixture = layout("avance", false);
        let mut runner = scripted(&fixture.paths, advanced.clone(), 0, "");
        runner.head_unix = Some(300);
        let mut phases = Vec::new();
        apply(
            &ApplyInput {
                paths: &fixture.paths,
                remote: &advertised,
                installed: Some(&installed),
                repo: Some("file:///tmp/ezerbot-fictif"),
                fault: Fault::None,
            },
            &mut runner,
            &mut |phase| phases.push(phase),
        )
        .unwrap();
        let plist = fs::read_to_string(fixture.paths.bundle.join("Contents/Info.plist")).unwrap();
        assert!(plist.contains(&advanced.sha), "{plist}");
        assert!(phases.contains(&Phase::Redemarrage));
        assert!(runner.calls.iter().any(|call| {
            call.args
                .iter()
                .any(|arg| arg == "file:///tmp/ezerbot-fictif")
        }));
        assert!(runner
            .calls
            .iter()
            .all(|call| { call.args.iter().all(|arg| !arg.contains("github.com")) }));

        let before = fs::read(fixture.paths.bundle.join("Contents/MacOS/Ezerbot")).unwrap();
        let mut backward = scripted(&fixture.paths, rolled.clone(), 0, "");
        backward.head_unix = Some(50);
        let error = apply(
            &ApplyInput {
                paths: &fixture.paths,
                remote: &advertised,
                installed: Some(&installed),
                repo: None,
                fault: Fault::None,
            },
            &mut backward,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(
            error.message.contains("ne propose plus"),
            "{}",
            error.message
        );
        assert_eq!(
            fs::read(fixture.paths.bundle.join("Contents/MacOS/Ezerbot")).unwrap(),
            before
        );
    }

    #[test]
    fn depot_file_local_installe_sans_github() {
        let fixture = layout("fichier", false);
        let source = fixture.root.join("source");
        fs::create_dir_all(source.join("tools")).unwrap();
        fs::write(
            source.join("tools/build-app.sh"),
            r#"#!/bin/bash
set -euo pipefail
dest="app/build/Ezerbot.app/Contents/MacOS"
mkdir -p "$dest"
printf 'nouvelle-version\n' > "$dest/Ezerbot"
printf 'wrapper\n' > "$dest/grokbot-wrapper"
cat > "app/build/Ezerbot.app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>Ezerbot</string>
  <key>EzerbotGitCommit</key><string>${EZERBOT_GIT_COMMIT}</string>
</dict></plist>
EOF
"#,
        )
        .unwrap();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&source)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Ezerbot")
                .env("GIT_AUTHOR_EMAIL", "ezerbot@example.com")
                .env("GIT_COMMITTER_NAME", "Ezerbot")
                .env("GIT_COMMITTER_EMAIL", "ezerbot@example.com")
                .status()
                .expect("git");
            assert!(status.success(), "{args:?} a échoué");
        };
        git(&["init", "-b", "main"]);
        git(&["add", "tools/build-app.sh"]);
        git(&[
            "-c",
            "user.name=Ezerbot",
            "-c",
            "user.email=ezerbot@example.com",
            "commit",
            "-m",
            "annonce",
        ]);
        let first = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&source)
            .output()
            .unwrap();
        assert!(first.status.success());
        let advertised = String::from_utf8(first.stdout).unwrap();
        fs::write(source.join("tools/note.txt"), "plus récent\n").unwrap();
        git(&["add", "tools/note.txt"]);
        git(&[
            "-c",
            "user.name=Ezerbot",
            "-c",
            "user.email=ezerbot@example.com",
            "commit",
            "-m",
            "avance",
        ]);
        let head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&source)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        let head_sha = String::from_utf8(head.stdout).unwrap().trim().to_string();
        assert_ne!(head_sha, advertised.trim());
        let url = format!("file://{}", source.display());
        let fetched = fetch_from(&mut SystemRunner, &url).unwrap();
        assert_eq!(fetched.sha, head_sha);
        assert!(fetched.unix.is_some());
        let installed = commit(&hex_sha('1'), Some(1));
        let requested = commit(advertised.trim(), None);
        let mut phases = Vec::new();
        apply(
            &ApplyInput {
                paths: &fixture.paths,
                remote: &requested,
                installed: Some(&installed),
                repo: Some(&url),
                fault: Fault::None,
            },
            &mut SystemRunner,
            &mut |phase| phases.push(phase),
        )
        .unwrap();
        let plist = fs::read_to_string(fixture.paths.bundle.join("Contents/Info.plist")).unwrap();
        assert!(plist.contains(&head_sha), "{plist}");
        assert_eq!(
            phases,
            vec![
                Phase::Telechargement,
                Phase::Compilation,
                Phase::Installation,
                Phase::Redemarrage,
            ]
        );
        let origin = std::process::Command::new("git")
            .args([
                "-C",
                &path_arg(&fixture.paths.work_dir),
                "remote",
                "get-url",
                "origin",
            ])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        let origin = String::from_utf8(origin.stdout).unwrap();
        assert!(origin.contains(&url), "{origin}");
        assert!(!origin.contains("github.com"), "{origin}");
    }
}
