#!/usr/bin/env python3
"""Read-only doctor: our PID, our port, build SHA equals the checked-out commit."""

from __future__ import annotations

import argparse
import sys
import urllib.request
import urllib.error
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import (
    assert_throwaway,
    commit_exists,
    identity_matches,
    listening_pids,
    load_run,
    pid_alive,
    repo_root,
    resolve_commit,
    sha256_file,
)


def check(name: str, passed: bool, detail: str) -> str | None:
    print(f"{'ok' if passed else 'echec'}  {name}: {detail}")
    return None if passed else name


def main() -> None:
    parser = argparse.ArgumentParser(description="Contrôle en lecture seule de l'instance")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument(
        "--prerequisites",
        action="store_true",
        help="Linux : vérifie Xvfb, Chrome, Node, cargo, disque, sans lancer d'instance",
    )
    parser.add_argument("--chrome", action="store_true", help="Linux : imprime le Chrome retenu")
    args = parser.parse_args()
    if args.chrome:
        chrome = linux_chrome()
        if not chrome:
            raise SystemExit("chrome introuvable")
        print(chrome)
        raise SystemExit(0)
    if args.prerequisites:
        failed = [item for item in linux_prerequisites() if item]
        print("doctor: echec" if failed else "doctor: ok")
        raise SystemExit(1 if failed else 0)
    evidence = args.evidence or Path(os_evidence())
    run = load_run(evidence)
    failures: list[str] = []
    repo = Path(run.get("repo") or repo_root())
    pid = int(run.get("wrapper_pid") or 0)
    alive = pid_alive(pid) and identity_matches(pid, run.get("wrapper_identity"))
    failures.append(check("pid", alive, f"{pid} vivant et toujours le wrapper lancé") or "")
    port = int(run.get("port") or 0)
    owners = listening_pids(port) if port else set()
    failures.append(check("port", pid in owners, f"{port} écouté par {sorted(owners) or 'personne'}") or "")
    sha = str(run.get("build_sha") or "")
    head = resolve_commit(repo, "HEAD")
    sha_ok = bool(sha) and sha == head and commit_exists(repo, sha)
    failures.append(
        check("build-sha", sha_ok, f"binaire {sha or 'absent'} commit extrait {head or 'absent'}") or ""
    )
    binary = Path(str(run.get("binary") or ""))
    digest = sha256_file(binary) if binary.is_file() else ""
    binary_ok = bool(digest) and digest == run.get("binary_sha256")
    failures.append(check("binaire", binary_ok, "empreinte inchangée depuis le lancement") or "")
    sources_ok = bool(run.get("sources_clean"))
    failures.append(check("sources", sources_ok, "crates et interface identiques au commit") or "")
    home = Path(str(run.get("home") or "/"))
    try:
        assert_throwaway(home, "HOME")
        home_ok = home.is_dir()
        home_detail = str(home)
    except SystemExit as error:
        home_ok = False
        home_detail = str(error)
    failures.append(check("home", home_ok, home_detail) or "")
    url = str(run.get("url") or "")
    page_ok = False
    page_detail = url or "URL absente"
    if url.startswith("http://127.0.0.1:"):
        try:
            with urllib.request.urlopen(url + "/", timeout=5) as response:
                body = response.read(300_000)
            page_ok = response.status == 200 and b"<title>Ezerbot</title>" in body
            page_detail = f"{url} titre Ezerbot"
        except OSError as error:
            page_detail = str(error)
    failures.append(check("page", page_ok, page_detail) or "")
    auth_pid = int(run.get("auth_pid") or 0)
    auth_ok = pid_alive(auth_pid) and identity_matches(auth_pid, run.get("auth_identity"))
    failures.append(check("auth-navigateur", auth_ok, "transport en mémoire du run vivant") or "")
    api_denied = False
    try:
        with urllib.request.urlopen(url + "/api/state", timeout=5):
            pass
    except urllib.error.HTTPError as error:
        api_denied = error.code == 403
    except OSError:
        pass
    failures.append(check("api-sans-identite", api_denied, "GET externe refusé sans identité de lancement") or "")
    fake_pid = int(run.get("fake_pid") or 0)
    fake_ok = pid_alive(fake_pid) and identity_matches(fake_pid, run.get("fake_identity"))
    failures.append(check("faux-openai", fake_ok, run.get("fake_base", "")) or "")
    failed = [item for item in failures if item]
    if failed:
        print("doctor: echec")
        raise SystemExit(1)
    print("doctor: ok")


MIN_FREE_BYTES = 25 * 1024**3


def linux_chrome() -> str | None:
    """Single Chrome lookup for the Linux path: EZERBOT_VERIFY_CHROME, then CHROME_BIN, then PATH.

    An explicit variable that is not an executable is an error, never a silent fallback.
    """
    import os
    import shutil

    for variable in ("EZERBOT_VERIFY_CHROME", "CHROME_BIN"):
        value = os.environ.get(variable)
        if value:
            return os.path.realpath(value) if os.path.isfile(value) and os.access(value, os.X_OK) else None
    for name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
        if found := shutil.which(name):
            return os.path.realpath(found)
    return None


def linux_prerequisites(repo: Path | None = None) -> list[str]:
    """Read-only checks for the Xvfb path; each failing check returns its name."""
    import os
    import platform
    import shutil
    import subprocess

    repo = repo or repo_root()
    results: list[str | None] = []
    results.append(check("plateforme", sys.platform.startswith("linux"), platform.system()))
    for tool in ("git", "xvfb-run", "Xvfb", "xauth", "flock", "setsid", "nice", "node", "npm"):
        found = shutil.which(tool)
        results.append(check(tool, bool(found), found or "absent du PATH"))
    chrome = linux_chrome()
    results.append(check("chrome", bool(chrome), chrome or "EZERBOT_VERIFY_CHROME/CHROME_BIN invalide, ou google-chrome/chromium absent du PATH"))
    node_ok, node_detail = False, "node absent"
    if shutil.which("node"):
        probe = subprocess.run(
            ["node", "--experimental-websocket", "-e", "process.exit(typeof WebSocket === 'function' ? 0 : 1)"],
            capture_output=True,
            text=True,
        )
        version = subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip()
        node_ok, node_detail = probe.returncode == 0, f"{version} WebSocket {'présent' if probe.returncode == 0 else 'absent'}"
    results.append(check("node-websocket", node_ok, node_detail))
    cargo = Path(os.environ.get("EZERBOT_VERIFY_CARGO", Path.home() / ".cargo/bin/cargo")).expanduser()
    results.append(check("cargo", cargo.is_file() and os.access(cargo, os.X_OK), str(cargo)))
    free = shutil.disk_usage(repo).free
    results.append(check("disque", free >= MIN_FREE_BYTES, f"{free // 1024**3} Gio libres sous {repo} (minimum 25)"))
    return [item for item in results if item]


def os_evidence() -> str:
    import os

    value = os.environ.get("EZERBOT_VERIFY_EVIDENCE_DIR")
    if not value:
        raise SystemExit("--evidence ou EZERBOT_VERIFY_EVIDENCE_DIR est requis")
    return value


if __name__ == "__main__":
    main()
