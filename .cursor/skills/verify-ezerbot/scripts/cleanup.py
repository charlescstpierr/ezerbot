#!/usr/bin/env python3
"""Stop only processes this run started, delete the throwaway HOME, keep evidence."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import (
    LOGIN_HOME,
    MAC_AXI,
    assert_created_home,
    assert_created_run_root,
    assert_not_under,
    identity_matches,
    load_browser,
    load_run,
    owns_axi_session,
    pid_alive,
    require_absolute,
    run_path,
    run_lock,
    save_json,
    save_run,
    terminate_recorded,
)


def axi_binary() -> str | None:
    override = os.environ.get("EZERBOT_VERIFY_AXI")
    if override:
        path = str(Path(override).expanduser())
        return path if os.access(path, os.X_OK) else None
    found = shutil.which("chrome-devtools-axi")
    if found:
        return found
    default = Path(LOGIN_HOME) / MAC_AXI
    if default.is_file() and os.access(default, os.X_OK):
        return str(default)
    return None


def stop_run_axi_session(session: str | None, run: dict) -> dict:
    if session is None:
        return {"result": "non-attribuee"}
    binary = axi_binary()
    if not binary:
        return {"session": session, "result": "absent"}
    env = os.environ.copy()
    env["CHROME_DEVTOOLS_AXI_SESSION"] = session
    env["HOME"] = run["axi_home"]
    try:
        result = subprocess.run(
            [binary, "stop"],
            env=env,
            text=True,
            capture_output=True,
            timeout=20,
        )
    except (OSError, subprocess.TimeoutExpired):
        return {"session": session, "result": "echec"}
    return {
        "session": session,
        "result": "arrete" if result.returncode == 0 else "echec",
        "code": result.returncode,
    }


def kill_chrome(state: dict | None) -> str:
    if state is None:
        return "absent"
    if state.get("started_by") != "verify-ezerbot":
        return "non-lance-par-nous"
    pid = int(state.get("chrome_pid") or 0)
    identity = state.get("identity")
    if identity is not None:
        identity = dict(identity)
        identity["pgid"] = state.get("pgid")
    return terminate_recorded(pid, identity)


def main() -> None:
    parser = argparse.ArgumentParser(description="Nettoie l'instance de vérification")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    evidence = args.evidence or (Path(os.environ["EZERBOT_VERIFY_EVIDENCE_DIR"]) if os.environ.get("EZERBOT_VERIFY_EVIDENCE_DIR") else None)
    if evidence is None or not evidence.is_dir():
        print("cleanup: rien à arrêter")
        return
    with run_lock(evidence):
        if not run_path(evidence).is_file():
            print("cleanup: rien à arrêter")
            return
        cleanup_run(evidence)


def cleanup_run(evidence: Path) -> None:
    run = load_run(evidence)
    if run.get("cleaned"):
        print("cleanup: déjà fait")
        return
    home = require_absolute(run.get("home"), "HOME")
    run_root = require_absolute(run.get("run_root"), "run_root")
    verified_root = assert_created_run_root(run_root, run)
    verified_home = assert_created_home(home, verified_root)
    assert_not_under(evidence, verified_home, "preuves")
    assert_not_under(evidence, verified_root, "preuves")
    try:
        browser = load_browser(run)
    except SystemExit:
        browser = None
    axi_session = run["session"] if browser is not None and owns_axi_session(run, browser) else None
    run["cleaning"] = True
    save_run(evidence, run)
    results = {}
    for name in ("wrapper", "fake"):
        pid = int(run.get(f"{name}_pid") or 0)
        identity = run.get(f"{name}_identity")
        if name == "wrapper" and identity and run.get("wrapper_pgid"):
            identity = dict(identity)
            identity["pgid"] = run["wrapper_pgid"]
        if name == "fake" and identity:
            identity = dict(identity)
            identity["pgid"] = identity.get("pgid") or pid
        results[name] = terminate_recorded(pid, identity) if pid else "absent"
    results["chrome"] = kill_chrome(browser)
    results["axi"] = stop_run_axi_session(axi_session, run)
    if results["axi"]["result"] in ("absent", "echec"):
        save_json(evidence / "cleanup.json", {"cleaned": False, "killed": results})
        raise SystemExit("cleanup: arrêt axi incomplet; réessayer le nettoyage")
    removed_home = False
    if verified_home.is_dir():
        shutil.rmtree(verified_home)
        removed_home = True
    for name in ("wrapper.log", "chrome.log"):
        log = verified_root / name
        if log.is_file():
            tail = log.read_text(errors="replace").splitlines()[-200:]
            (evidence / name).write_text("\n".join(tail) + ("\n" if tail else ""))
    shutil.rmtree(verified_root)
    still = {
        name: pid_alive(int(run.get(f"{name}_pid") or 0)) and identity_matches(int(run.get(f"{name}_pid") or 0), run.get(f"{name}_identity"))
        for name in ("wrapper", "fake")
    }
    tombstone = {
        "cleaned": True,
        "killed": results,
        "removed_home": removed_home,
        "home_still_present": verified_home.exists(),
        "processes_still_ours": still,
        "evidence": str(evidence),
        "build_sha": run.get("build_sha"),
        "url": run.get("url"),
    }
    save_run(evidence, tombstone)
    save_json(evidence / "cleanup.json", tombstone)
    print("cleanup: ok")


if __name__ == "__main__":
    main()
