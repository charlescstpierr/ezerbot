#!/usr/bin/env python3
"""Shared safety, run records, and git checks for verify-ezerbot.

The wrapper must never see the operator's real home or the captain's
Grokbot application-support directory. Evidence stays outside the repo.
"""

from __future__ import annotations

import hashlib
import json
import os
import pwd
import signal
import socket
import struct
import subprocess
import sys
import time
from pathlib import Path

MAC_REAL_HOME = "/Users/cstpierrecharles"
MAC_APP_SUPPORT = "/Users/cstpierrecharles/Library/Application Support/Grokbot"
MAC_DATA_ROOT = "/Users/cstpierrecharles/firstmate/data/gb-verify-skill"
MAC_REF_SHOTS = "/Users/cstpierrecharles/firstmate/data/gb-visual-qa/shots"
MAC_REF_ANNEX = "/Users/cstpierrecharles/firstmate/data/gb-visual-qa2/report.md"
MAC_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
MAC_AXI = ".local/bin/chrome-devtools-axi"

LOGIN_HOME = os.environ.setdefault("EZERBOT_VERIFY_LOGIN_HOME", os.environ.get("HOME", ""))


def repo_root() -> Path:
    start = Path(__file__).resolve()
    for parent in [start, *start.parents]:
        git = parent / ".git"
        if git.exists() and (parent / "Cargo.toml").is_file() and (parent / "crates").is_dir():
            return parent
    raise SystemExit("dépôt ezerbot-rust introuvable depuis les scripts de vérification")


def real_homes() -> list[Path]:
    values = [
        os.environ.get("EZERBOT_VERIFY_REAL_HOME", MAC_REAL_HOME),
        pwd.getpwuid(os.getuid()).pw_dir,
        LOGIN_HOME,
    ]
    homes: list[Path] = []
    for value in values:
        if not value:
            continue
        path = Path(value).expanduser()
        if path not in homes:
            homes.append(path)
    return homes


def app_support_root() -> Path:
    return Path(os.environ.get("EZERBOT_VERIFY_APP_SUPPORT", MAC_APP_SUPPORT)).expanduser()


def touches_grokbot_support(path: Path) -> bool:
    """True when a path is the captain's Grokbot support dir, not Grokbot Local."""
    candidates = [path]
    try:
        candidates.append(path.resolve())
    except OSError:
        pass
    forbidden = app_support_root()
    forbidden_resolved = forbidden
    try:
        forbidden_resolved = forbidden.resolve()
    except OSError:
        pass
    for candidate in candidates:
        parts = candidate.parts
        for index in range(len(parts) - 2):
            if (
                parts[index] == "Library"
                and parts[index + 1] == "Application Support"
                and parts[index + 2] == "Grokbot"
            ):
                return True
        for root in (forbidden, forbidden_resolved):
            try:
                candidate.resolve().relative_to(root)
                return True
            except (OSError, ValueError):
                try:
                    candidate.relative_to(root)
                    return True
                except ValueError:
                    continue
    raw = str(path)
    needle = f"{os.sep}Application Support{os.sep}Grokbot{os.sep}"
    alt = f"{os.sep}Application Support{os.sep}Grokbot"
    return needle in raw or raw.endswith(alt)


def is_real_home(path: Path) -> bool:
    try:
        resolved = path.resolve()
    except OSError:
        resolved = path
    for home in real_homes():
        try:
            if resolved == home.resolve():
                return True
        except OSError:
            if resolved == home:
                return True
    return False


def assert_throwaway(path: Path, label: str) -> Path:
    if is_real_home(path):
        raise SystemExit(f"refus: {label} résout vers le vrai dossier personnel ({path})")
    if touches_grokbot_support(path):
        raise SystemExit(f"refus: {label} touche ~/Library/Application Support/Grokbot ({path})")
    return path


def assert_outside_repo(path: Path, label: str) -> Path:
    repo = repo_root().resolve()
    try:
        path.resolve().relative_to(repo)
    except ValueError:
        return path
    raise SystemExit(f"refus: {label} est dans le dépôt ({path})")


def assert_not_under(path: Path, parent: Path, label: str) -> None:
    try:
        path.resolve().relative_to(parent.resolve())
    except ValueError:
        return
    raise SystemExit(f"refus: {label} est sous {parent}")


def evidence_dir(session: str, explicit: str | None = None) -> Path:
    if explicit:
        path = Path(explicit).expanduser()
    elif os.environ.get("EZERBOT_VERIFY_EVIDENCE_DIR"):
        path = Path(os.environ["EZERBOT_VERIFY_EVIDENCE_DIR"]).expanduser()
    elif Path(MAC_REAL_HOME).is_dir():
        root = Path(os.environ.get("EZERBOT_VERIFY_DATA_ROOT", MAC_DATA_ROOT)).expanduser()
        path = root / "evidence" / session
    else:
        path = Path(os.environ.get("TMPDIR", "/tmp")) / f"verify-ezerbot-evidence-{session}"
    assert_outside_repo(path, "dossier de preuves")
    if touches_grokbot_support(path):
        raise SystemExit(f"refus: les preuves touchent Application Support/Grokbot ({path})")
    path.mkdir(parents=True, exist_ok=True)
    return path


def session_id() -> str:
    return os.environ.get("CHROME_DEVTOOLS_AXI_SESSION") or "verify-ezerbot"


def run_path(evidence: Path) -> Path:
    return evidence / "run.json"


def load_run(evidence: Path) -> dict:
    path = run_path(evidence)
    if not path.is_file():
        raise SystemExit(f"aucun run.json dans {evidence}")
    return json.loads(path.read_text())


def save_run(evidence: Path, record: dict) -> None:
    path = run_path(evidence)
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(record, indent=2) + "\n")
    temporary.replace(path)


def git(repo: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-C", str(repo), *args],
        text=True,
        capture_output=True,
        check=check,
    )


def resolve_commit(repo: Path, ref: str) -> str:
    """Resolve with git rev-parse --verify <ref>^{commit}. Empty string on failure."""
    result = git(repo, "rev-parse", "--verify", f"{ref}^{{commit}}", check=False)
    if result.returncode != 0:
        return ""
    return result.stdout.strip()


def commit_exists(repo: Path, sha: str) -> bool:
    result = git(repo, "cat-file", "-e", f"{sha}^{{commit}}", check=False)
    return result.returncode == 0


def compiled_sources_clean(repo: Path) -> bool:
    paths = ["crates", "app/ui", "app/web", "Cargo.toml", "Cargo.lock"]
    status = git(repo, "status", "--porcelain", "--", *paths, check=False)
    return status.returncode == 0 and status.stdout.strip() == ""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pid_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    return True


def proc_identity(pid: int) -> dict | None:
    if not pid_alive(pid):
        return None
    if sys.platform == "darwin":
        result = subprocess.run(
            ["ps", "-p", str(pid), "-o", "lstart=,command="],
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            return None
        line = result.stdout.strip()
        return {"pid": pid, "platform": "darwin", "marker": line, "command": line}
    stat_path = Path(f"/proc/{pid}/stat")
    cmd_path = Path(f"/proc/{pid}/cmdline")
    try:
        stat = stat_path.read_text()
        command = cmd_path.read_bytes().replace(b"\x00", b" ").decode(errors="replace").strip()
    except OSError:
        return None
    end = stat.rfind(")")
    fields = stat[end + 2 :].split()
    start = fields[19] if len(fields) > 19 else ""
    return {"pid": pid, "platform": "linux", "start": start, "command": command, "marker": f"{start} {command}"}


def identity_matches(pid: int, recorded: dict | None) -> bool:
    if not recorded:
        return False
    current = proc_identity(pid)
    if current is None:
        return False
    if recorded.get("platform") != current.get("platform"):
        return False
    if sys.platform == "darwin":
        return recorded.get("marker") == current.get("marker")
    return recorded.get("start") == current.get("start") and recorded.get("command") == current.get("command")


def listening_pids(port: int) -> set[int]:
    if sys.platform == "darwin":
        result = subprocess.run(
            ["lsof", "-nP", f"-iTCP:{port}", "-sTCP:LISTEN", "-t"],
            text=True,
            capture_output=True,
            check=False,
        )
        owners: set[int] = set()
        for line in result.stdout.split():
            if line.strip().isdigit():
                owners.add(int(line.strip()))
        return owners
    inodes: set[str] = set()
    for name in ("/proc/net/tcp", "/proc/net/tcp6"):
        path = Path(name)
        if not path.is_file():
            continue
        lines = path.read_text().splitlines()[1:]
        for line in lines:
            fields = line.split()
            if len(fields) < 10:
                continue
            local, state, inode = fields[1], fields[3], fields[9]
            if state != "0A":
                continue
            try:
                local_port = int(local.rsplit(":", 1)[1], 16)
            except ValueError:
                continue
            if local_port == port:
                inodes.add(inode)
    owners: set[int] = set()
    proc = Path("/proc")
    for entry in proc.iterdir():
        if not entry.name.isdigit():
            continue
        fd_dir = entry / "fd"
        try:
            fds = list(fd_dir.iterdir())
        except OSError:
            continue
        for fd in fds:
            try:
                target = os.readlink(fd)
            except OSError:
                continue
            for inode in inodes:
                if target == f"socket:[{inode}]":
                    owners.add(int(entry.name))
    return owners


def terminate_recorded(pid: int, recorded: dict | None) -> str:
    """Kill one process we started, only if its identity still matches."""
    if not pid_alive(pid):
        return "deja-arrete"
    if not identity_matches(pid, recorded):
        return "identite-differente-non-tue"
    sigterm = signal.SIGTERM
    sigkill = signal.SIGKILL
    pgid = recorded.get("pgid") if recorded else None
    if pgid and sys.platform != "win32":
        try:
            os.killpg(int(pgid), sigterm)
        except OSError:
            os.kill(pid, sigterm)
    else:
        os.kill(pid, sigterm)
    deadline = time.time() + 3
    while time.time() < deadline and pid_alive(pid):
        time.sleep(0.1)
    if pid_alive(pid) and identity_matches(pid, recorded):
        if pgid and sys.platform != "win32":
            try:
                os.killpg(int(pgid), sigkill)
            except OSError:
                os.kill(pid, sigkill)
        else:
            os.kill(pid, sigkill)
        time.sleep(0.2)
    return "arrete" if not pid_alive(pid) else "toujours-vivant"


def png_size(path: Path) -> tuple[int, int]:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise ValueError(f"{path} n'est pas un PNG")
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def cargo_binary() -> Path:
    override = os.environ.get("EZERBOT_VERIFY_CARGO")
    if override:
        return Path(override).expanduser()
    return Path(LOGIN_HOME) / ".cargo/bin/cargo"


def build_env() -> dict[str, str]:
    env = os.environ.copy()
    env.pop("CARGO_TARGET_DIR", None)
    if not os.environ.get("EZERBOT_VERIFY_CARGO"):
        # A foreign CARGO_HOME would ignore ~/.cargo/bin/cargo's toolchain.
        env.pop("CARGO_HOME", None)
        env.pop("RUSTUP_HOME", None)
    return env


def self_check() -> None:
    repo = repo_root()
    throwaway = Path("/tmp/verify-ezerbot-self-check")
    assert_throwaway(throwaway, "HOME")
    for home in real_homes():
        try:
            assert_throwaway(home, "HOME")
        except SystemExit:
            pass
        else:
            raise SystemExit(f"le vrai home n'a pas été refusé: {home}")
    support = app_support_root() / "workspace.sqlite"
    try:
        assert_throwaway(support, "db")
    except SystemExit:
        pass
    else:
        raise SystemExit("Application Support/Grokbot n'a pas été refusé")
    local = Path("/tmp/verify-ezerbot-self-check/Library/Application Support/Grokbot Local/workspace.sqlite")
    if touches_grokbot_support(local):
        raise SystemExit("Grokbot Local a été confondu avec Grokbot")
    try:
        assert_outside_repo(repo / "target", "preuves")
    except SystemExit:
        pass
    else:
        raise SystemExit("une preuve dans le dépôt n'a pas été refusée")
    sha = resolve_commit(repo, "HEAD")
    if not sha or not commit_exists(repo, sha):
        raise SystemExit("HEAD n'est pas un commit vérifiable")
    missing = resolve_commit(repo, "refs/heads/this-ref-does-not-exist-verify-ezerbot")
    if missing:
        raise SystemExit("une ref absente a produit un commit")
    print("self-check: ok")


def main() -> None:
    if len(sys.argv) > 1 and sys.argv[1] == "self-check":
        self_check()
        return
    raise SystemExit("usage: verify_lib.py self-check")


if __name__ == "__main__":
    main()
