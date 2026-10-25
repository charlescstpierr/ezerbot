#!/usr/bin/env python3
"""Build grokbot-wrapper and start an isolated instance.

Build command, never with CARGO_TARGET_DIR set:

    ~/.cargo/bin/cargo build --locked -p grokbot-wrapper

Run command, with HOME and CFFIXED_USER_HOME on a fresh throwaway directory:

    target/debug/grokbot-wrapper --port 0 --db <throwaway>/workspace.sqlite
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import selectors
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
from local_api import LaunchIdentity

from verify_lib import (
    assert_not_under,
    assert_outside_repo,
    assert_throwaway,
    build_env,
    cargo_binary,
    commit_exists,
    compiled_sources_clean,
    evidence_dir,
    proc_identity,
    repo_root,
    resolve_commit,
    save_run,
    session_id,
    sha256_file,
    sign_run_directory,
)


def fake_jwt() -> str:
    def part(data: dict) -> str:
        return base64.urlsafe_b64encode(json.dumps(data, separators=(",", ":")).encode()).rstrip(b"=").decode()

    claims = {
        "email": "vous@exemple.com",
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "account-fake",
            "chatgpt_plan_type": "plus",
            "chatgpt_data_residency": "eu",
        },
    }
    return f"{part({'alg': 'none', 'typ': 'JWT'})}.{part(claims)}.fake"


def write_fake_token(db: Path) -> None:
    payload = {
        "host_id": "urn:uuid:00000000-0000-4000-8000-0000000000e2",
        "account_id": "account-fake",
        "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
        "access_token": fake_jwt(),
        "refresh_token": "fake-refresh-verify",
        "expires_at": int(time.time()) + 86400,
        "scopes": "openid profile email offline_access",
        "email": "vous@exemple.com",
        "plan": "plus",
        "welcomed": True,
    }
    encoded = json.dumps(payload)
    if "sk-" in encoded or "sess-" in encoded:
        raise SystemExit("refus: le jeton fictif ressemble à un identifiant réel")
    path = db.with_name(f"{db.stem}.codex-auth.json")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as handle:
        handle.write(encoded)
    os.chmod(path, 0o600)


def read_line(process: subprocess.Popen[str], timeout: float) -> str:
    if process.stdout is None:
        raise SystemExit("stdout du processus indisponible")
    waiter = selectors.DefaultSelector()
    waiter.register(process.stdout, selectors.EVENT_READ)
    if not waiter.select(timeout):
        raise SystemExit("délai dépassé en attendant l'annonce du processus")
    return process.stdout.readline().strip()


def allocate_run_root(evidence: Path) -> tuple[Path, Path, dict[str, str]]:
    """Validate the temp parent before mkdtemp. A refusal must not create a directory."""
    parent = Path(tempfile.gettempdir())
    assert_throwaway(parent, "TMPDIR")
    assert_outside_repo(parent / "verify-ezerbot.preview", "répertoire jetable")
    run_root = Path(tempfile.mkdtemp(prefix="verify-ezerbot."))
    try:
        assert_throwaway(run_root, "run_root")
        home = run_root / "home"
        home.mkdir()
        (home / "tmp").mkdir()
        assert_throwaway(home, "HOME")
        assert_outside_repo(run_root, "répertoire jetable")
        assert_not_under(evidence, home, "preuves")
        assert_not_under(evidence, run_root, "preuves")
        signature = sign_run_directory(run_root)
    except BaseException:
        shutil.rmtree(run_root, ignore_errors=True)
        raise
    return run_root, home, signature


def main() -> None:
    parser = argparse.ArgumentParser(description="Lance un Ezerbot isolé pour la vérification")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--sphere", default="accueil")
    args = parser.parse_args()
    repo = repo_root()
    session = session_id()
    evidence = evidence_dir(session, str(args.evidence) if args.evidence else None)
    if not compiled_sources_clean(repo):
        raise SystemExit(
            "refus: crates/, app/ui/, app/web/ ou Cargo.lock diffèrent du commit. "
            "Le binaire doit correspondre au commit extrait."
        )
    cargo = cargo_binary()
    if not cargo.is_file():
        raise SystemExit(f"cargo introuvable: {cargo}")
    print(f"build: {cargo} build --locked -p grokbot-wrapper", flush=True)
    gate = os.environ.get("EZERBOT_VERIFY_CARGO_GATE")
    compile_env = build_env()
    compile_env["EZERBOT_GIT_COMMIT"] = resolve_commit(repo, "HEAD")
    subprocess.run(
        ([gate] if gate else []) + [str(cargo), "build", "--locked", "-p", "grokbot-wrapper"],
        cwd=repo,
        env=compile_env,
        check=True,
    )
    sha = resolve_commit(repo, "HEAD")
    if not sha or not commit_exists(repo, sha):
        raise SystemExit("refus: le commit extrait n'est pas un objet commit")
    binary = repo / "target/debug/grokbot-wrapper"
    if not binary.is_file() or "Ezerbot.app" in str(binary):
        raise SystemExit(f"binaire de vérification introuvable: {binary}")
    run_root, home, signature = allocate_run_root(evidence)
    db = home / "workspace.sqlite"
    assert_throwaway(db, "db")
    write_fake_token(db)
    fake_log = open(run_root / "fake.err", "wb")
    fake_env = os.environ.copy()
    work = None
    if args.sphere == "mode-yolo":
        # Le Mode YOLO écrit dans un dossier de travail à côté du HOME : la base vit dans le HOME et ses données restent hors de portée des outils.
        work = run_root / "Documents"
        work.mkdir()
        for name in ("Budget.csv", "Brouillon.md", "Courriel.txt"):
            (work / name).write_text("exemple\n")
        (work / "Photos").mkdir()
        fake_env["EZERBOT_VERIFY_FAKE_WORK"] = str(work)
    fake = subprocess.Popen(
        [sys.executable, "-u", str(Path(__file__).with_name("fake_openai.py"))],
        env=fake_env,
        stdout=subprocess.PIPE,
        stderr=fake_log,
        stdin=subprocess.DEVNULL,
        text=True,
        start_new_session=True,
        cwd=repo,
    )
    record = {
        "session": session,
        "repo": str(repo),
        "run_root": str(run_root),
        "home": str(home),
        "db": str(db),
        "evidence": str(evidence),
        "build_sha": sha,
        "sources_clean": True,
        "binary": str(binary),
        "binary_sha256": sha256_file(binary),
        "fake_pid": fake.pid,
        "fake_identity": proc_identity(fake.pid),
        "wrapper_pid": None,
        "work": str(work) if work else None,
        **signature,
    }
    save_run(evidence, record)
    try:
        announced = read_line(fake, 10)
        fake_info = json.loads(announced)
        base = fake_info["base"]
        if not base.startswith("http://127.0.0.1:"):
            raise SystemExit(f"faux OpenAI hors boucle locale: {base}")
        record["fake_base"] = base
        save_run(evidence, record)
        wrapper_log = open(run_root / "wrapper.log", "wb")
        child_env = {
            "PATH": "/usr/bin:/bin:/usr/local/bin",
            "HOME": str(home),
            "CFFIXED_USER_HOME": str(home),
            "TMPDIR": str(home / "tmp"),
            "LANG": os.environ.get("LANG", "C.UTF-8"),
            "GROKBOT_FAKE_OPENAI": base,
        }
        if args.sphere not in ("images", "images-reference", "pages", "reponse-fil", "taches-de-fond", "approbation-tardive", "approbation-rappel", "mode-yolo", "sources-citees", "souvenirs-rappeles", "rien-a-signaler"):
            # Les builds de debug ouvrent la session factice eux-mêmes (le jeton écrit ci-dessus n'est plus lu).
            # Les sphères images, images-reference, pages, reponse-fil, taches-de-fond, approbation-tardive, approbation-rappel et sources-citees gardent le faux OpenAI du run : lui seul sait répondre à un dessin,
            # écrire une Page, citer le message parent d'un sous-fil, lancer une tâche de fond, demander une commande protégée puis en rendre le résultat, ou citer des sources numérotées.
            child_env["EZERBOT_FAKE_SIGNIN"] = "1"
        if args.sphere == "iphone-jumelage":
            # Faux Tailscale dans le HOME jetable (le seul que le wrapper de debug accepte) et port privé libre : 47863 peut être tenu par l'app installée.
            tailscale = home / "tailscale-fake"
            tailscale.write_text("#!/bin/sh\nprintf '%s\\n' '{\"BackendState\":\"Running\",\"Self\":{\"DNSName\":\"mac-exemple.tailnet-exemple.ts.net.\"},\"CurrentTailnet\":{\"Name\":\"tailnet-exemple\"}}'\n")
            tailscale.chmod(0o700)
            child_env["GROKBOT_FAKE_TAILSCALE"] = str(tailscale)
            with socket.socket() as probe:
                probe.bind(("127.0.0.1", 0))
                child_env["GROKBOT_IPHONE_PORT"] = str(probe.getsockname()[1])
        print(
            f"run: {binary} --port 0 --db {db}",
            flush=True,
        )
        identity = LaunchIdentity()
        wrapper = identity.spawn(
            [str(binary), "--port", "0", "--db", str(db)],
            stdout=subprocess.PIPE,
            stderr=wrapper_log,
            text=True,
            env=child_env,
            cwd=repo,
            start_new_session=True,
        )
        record["wrapper_pid"] = wrapper.pid
        record["wrapper_identity"] = proc_identity(wrapper.pid)
        record["wrapper_pgid"] = wrapper.pid
        if record["fake_identity"]:
            record["fake_identity"]["pgid"] = fake.pid
        save_run(evidence, record)
        line = read_line(wrapper, 20)
        info = json.loads(line)
        url = info["url"]
        if not url.startswith("http://127.0.0.1:") or url.endswith(":0"):
            raise SystemExit(f"URL de wrapper refusée: {url}")
        port = int(url.rsplit(":", 1)[1])
        record["url"] = url
        record["port"] = port
        record["wrapper_identity"] = proc_identity(wrapper.pid)
        save_run(evidence, record)
        from browser import ensure_headless, http_json
        state = ensure_headless(record, headed=sys.platform == "darwin" and os.environ.get("EZERBOT_VERIFY_BROWSER") != "chrome-headless")
        version = http_json(f"http://127.0.0.1:{state['debug_port']}/json/version")
        auth = identity.spawn(
            [sys.executable, str(repo / "tools/local_api.py"), "--browser-auth", version["webSocketDebuggerUrl"], url],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
            cwd=repo, start_new_session=True,
        )
        record["auth_pid"] = auth.pid
        record["auth_identity"] = proc_identity(auth.pid)
        save_run(evidence, record)
        if read_line(auth, 20) != "ready":
            raise SystemExit("le transport du navigateur n'est pas prêt")
    except Exception:
        save_run(evidence, record)
        raise
    print(json.dumps({"url": url, "evidence": str(evidence), "sha": sha, "port": port}))


if __name__ == "__main__":
    main()
