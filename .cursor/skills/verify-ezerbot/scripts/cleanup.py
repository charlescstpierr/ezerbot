#!/usr/bin/env python3
"""Stop only processes this run started, delete the throwaway HOME, keep evidence."""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import (
    assert_created_home,
    assert_created_run_root,
    assert_not_under,
    identity_matches,
    load_run,
    pid_alive,
    require_absolute,
    run_path,
    save_run,
    terminate_recorded,
)


def kill_chrome(run: dict) -> str:
    browser_path = Path(run["run_root"]) / "browser.json"
    if not browser_path.is_file():
        return "absent"
    state = json.loads(browser_path.read_text())
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
    import os

    evidence = args.evidence or (Path(os.environ["EZERBOT_VERIFY_EVIDENCE_DIR"]) if os.environ.get("EZERBOT_VERIFY_EVIDENCE_DIR") else None)
    if evidence is None or not run_path(evidence).is_file():
        print("cleanup: rien à arrêter")
        return
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
    results["chrome"] = kill_chrome(run)
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
    (evidence / "cleanup.json").write_text(json.dumps(tombstone, indent=2, ensure_ascii=False) + "\n")
    print("cleanup: ok")


if __name__ == "__main__":
    main()
