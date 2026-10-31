#!/usr/bin/env python3
"""Verdict of the Linux proof: report.md ending with PASS <sha> or FIX <sha>."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import commit_exists, png_size, repo_root

FEATURES_DIR = Path(__file__).resolve().parent.parent / "features"
# Same registry as FEATURES in linux_drive.cjs (a test keeps both equal): every view is required.
EXPECTED: tuple[tuple[str, str], ...] = (
    ("accueil", "home-dark"),
    ("reponse-fil", "conversation-dark"),
    ("reponse-fil", "thread-menu"),
    ("reponse-fil", "thread-open"),
    ("reponse-fil", "thread-reply"),
    ("reponse-fil", "thread-main"),
    ("parametres-connecteurs", "settings-dark"),
    ("parametres-connecteurs", "settings-connectors"),
)


def verdict(evidence: Path, sha: str, drive_status: int = 0) -> tuple[str, list[str]]:
    """PASS only when this run's driver exited 0 and every mapped view has its PNG in the manifest."""
    captures = evidence / "captures"
    result_path = captures / "result.json"
    result = json.loads(result_path.read_text()) if result_path.is_file() else None
    if result is not None and result.get("sha") != sha:
        raise SystemExit("result.json ne correspond pas au commit demandé")
    manifest_path = captures / "manifest.json"
    manifest = json.loads(manifest_path.read_text()) if manifest_path.is_file() else None
    hashes = {view["file"]: view["sha256"] for view in (manifest or {}).get("views", [])}
    lines = [
        "# verify-ezerbot (Linux, Xvfb)",
        "",
        f"Commit : `{sha}`",
        f"Rendu : {manifest['renderer']} {manifest['chrome_version']}" if manifest else "Rendu : manifeste absent",
        "",
        "| Entrée de la carte | Vue | Capture | Résultat |",
        "|---|---|---|---|",
    ]
    ok = drive_status == 0 and result is not None and result.get("error") is None
    ok = ok and manifest is not None and manifest.get("sha") == sha
    entries: dict[str, bool] = {}
    reported = {(item.get("feature"), item.get("view")): item for item in (result or {}).get("features", [])}
    if result is not None and set(reported) != set(EXPECTED):
        ok = False
        lines_extra = sorted(f"{feature}/{view}" for feature, view in set(reported) ^ set(EXPECTED))
    else:
        lines_extra = []
    for feature, view in EXPECTED:
        item = reported.get((feature, view), {"feature": feature, "view": view, "file": f"{view}.png", "status": "absent"})
        shot = captures / f"{view}.png"
        passed = item["status"] == "pass" and item.get("file") == f"{view}.png" and shot.is_file() and (FEATURES_DIR / f"{feature}.md").is_file()
        if passed:
            png_size(shot)
            digest = hashlib.sha256(shot.read_bytes()).hexdigest()
            # Sans manifeste (vue suivante en échec), l'empreinte n'est pas comparée ; le verdict reste FIX.
            passed = manifest is None or hashes.get(f"{view}.png") == digest
        entries[feature] = entries.get(feature, True) and passed
        ok = ok and passed
        lines.append(f"| `{feature}` | `{view}` | `captures/{view}.png` | {'ok' if passed else 'ÉCHEC'} |")
    if lines_extra:
        lines += ["", "Vues inattendues ou manquantes dans result.json : " + ", ".join(f"`{name}`" for name in lines_extra)]
    lines += ["", "Entrées : " + (", ".join(f"`{name}` {'ok' if good else 'ÉCHEC'}" for name, good in entries.items()) or "aucune")]
    if result is None:
        lines += ["", "## Erreur d’outil", "", f"Le pilote n’a écrit aucun résultat (code {drive_status})."]
    elif result.get("error"):
        lines += ["", "## Échec", "", f"`{result['error']}`"]
    elif drive_status:
        lines += ["", "## Erreur d’outil", "", f"Le pilote a quitté avec le code {drive_status}."]
    status = "PASS" if ok else "FIX"
    lines += ["", f"{status} {sha}"]
    return status, lines


def main() -> None:
    parser = argparse.ArgumentParser(description="Verdict de la preuve Linux")
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--drive-status", type=int, default=1)
    args = parser.parse_args()
    if not commit_exists(repo_root(), args.sha):
        raise SystemExit(f"commit introuvable : {args.sha}")
    status, lines = verdict(args.evidence, args.sha, args.drive_status)
    (args.evidence / "report.md").write_text("\n".join(lines) + "\n")
    print(f"preuves: {args.evidence}")
    print("\n".join(lines))
    raise SystemExit(0 if status == "PASS" else 1)


if __name__ == "__main__":
    main()
