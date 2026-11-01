#!/usr/bin/env python3
"""Verdict of the Linux proof: report.md ending with PASS <sha> or FIX <sha>."""

from __future__ import annotations

import argparse
import hashlib
import re
import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import PATCH_ID, commit_exists, repo_root

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
    ("parametres-connecteurs", "settings-connectors-add"),
)


def read_json(path: Path, problems: list[str]) -> dict | None:
    """Absent -> None; unreadable, truncated or not an object -> None plus a tool-error line."""
    if not path.is_file():
        return None
    try:
        value = json.loads(path.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        problems.append(f"`{path.name}` illisible ({type(error).__name__}).")
        return None
    if not isinstance(value, dict):
        problems.append(f"`{path.name}` n’est pas un objet JSON.")
        return None
    return value


def tool_error(sha: str, message: str, patch: str | None = None) -> tuple[str, list[str]]:
    """No trustworthy capture: FIX with a tool error, never a silent exit or a traceback."""
    lines = ["# verify-ezerbot (Linux, Xvfb)", "", f"Commit : `{sha}`", "", "## Erreur d’outil", "", message]
    return "FIX", lines + ["", verdict_line("FIX", sha, patch)]


def build_failure(sha: str, step: str, code: int, patch: str | None = None) -> tuple[str, list[str]]:
    """A build step failed before any capture."""
    return tool_error(sha, f"L’étape `{step}` a échoué (code {code}) ; aucune vue n’a été pilotée.", patch)


def safe_verdict(evidence: Path, sha: str, drive_status: int = 0, patch: str | None = None) -> tuple[str, list[str]]:
    """Any unexpected failure while reading this run's evidence is a tool error, not a crash.

    SystemExit stays: it is the deliberate refusal of evidence produced for another commit.
    """
    try:
        return verdict(evidence, sha, drive_status, patch)
    except SystemExit:
        raise
    except Exception as error:  # noqa: BLE001 - every unreadable artifact must end in FIX
        return tool_error(sha, f"Preuves illisibles ({type(error).__name__}) ; verdict impossible.", patch)


def png_loads(path: Path) -> None:
    """Sanity check only; integrity comes from the sha256 and size recorded by the capture step."""
    from PIL import Image  # noqa: PLC0415 - checked by doctor.py --prerequisites

    with Image.open(path) as image:
        image.load()


def valid_features(items: list) -> bool:
    """Strict schema: string fields, status in {pass, fail}, (feature, view) and view ids unique."""
    if not all(isinstance(item, dict) and set(item) == {"feature", "view", "file", "status"} and all(isinstance(item[key], str) for key in item) and item["status"] in ("pass", "fail") for item in items):
        return False
    return len({(item["feature"], item["view"]) for item in items}) == len(items) and len({item["view"] for item in items}) == len(items)


def write_report(path: Path, lines: list[str]) -> bytes:
    """UTF-8 bytes; characters that cannot be encoded (lone surrogates) are escaped, not fatal."""
    data = ("\n".join(lines) + "\n").encode("utf-8", "backslashreplace")
    path.write_bytes(data)
    return data


def verdict_line(status: str, sha: str, patch: str | None) -> str:
    return f"{status} {sha}" + (f" patch={patch}" if patch else "")


def verdict(evidence: Path, sha: str, drive_status: int = 0, patch: str | None = None) -> tuple[str, list[str]]:
    """PASS only when this run's driver exited 0 and every mapped view has its PNG in the manifest."""
    captures = evidence / "captures"
    problems: list[str] = []
    result = read_json(captures / "result.json", problems)
    if result is not None and not (isinstance(result.get("sha"), str) and PATCH_ID.fullmatch(result["sha"])):
        problems.append("`result.json` : `sha` absent ou invalide.")
        result = None
    if result is not None and result["sha"] != sha:
        raise SystemExit("result.json ne correspond pas au commit demandé")
    manifest = read_json(captures / "manifest.json", problems)
    views = manifest.get("views") if manifest else []
    if manifest is not None and not (isinstance(views, list) and all(isinstance(view, dict) and isinstance(view.get("file"), str) and isinstance(view.get("sha256"), str) and re.fullmatch(r"[0-9a-f]{64}", view["sha256"]) and type(view.get("size")) is int for view in views) and len({view["file"] for view in views}) == len(views)):
        problems.append("`manifest.json` : liste `views` invalide.")
        manifest, views = None, []
    if result is not None and not (isinstance(result.get("features"), list) and valid_features(result["features"])):
        problems.append("`result.json` : liste `features` invalide.")
        result = None
    if manifest is None:
        # Sans manifeste aucune empreinte n'est vérifiable : aucune vue ne peut être déclarée ok.
        problems.append("`manifest.json` absent ou illisible : captures non vérifiées.")
    recorded = {view["file"]: (view["sha256"], view["size"]) for view in views}
    lines = [
        "# verify-ezerbot (Linux, Xvfb)",
        "",
        f"Commit : `{sha}`",
        f"Rendu : {manifest.get('renderer')} {manifest.get('chrome_version')}" if manifest else "Rendu : manifeste absent",
        "",
        "| Entrée de la carte | Vue | Capture | Résultat |",
        "|---|---|---|---|",
    ]
    ok = not problems and drive_status == 0 and result is not None and result.get("error") is None
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
            data = shot.read_bytes()
            if recorded.get(f"{view}.png") != (hashlib.sha256(data).hexdigest(), len(data)):
                problems.append(f"`captures/{view}.png` ne correspond pas à l’empreinte du manifeste.")
                passed = False
        if passed:
            try:
                png_loads(shot)
            except Exception as error:  # noqa: BLE001 - any decoder failure is a tool error
                problems.append(f"`captures/{view}.png` illisible ({type(error).__name__}).")
                passed = False
        entries[feature] = entries.get(feature, True) and passed
        ok = ok and passed
        mark = "non vérifié" if manifest is None else "ok" if passed else "ÉCHEC"
        lines.append(f"| `{feature}` | `{view}` | `captures/{view}.png` | {mark} |")
    if lines_extra:
        lines += ["", "Vues inattendues ou manquantes dans result.json : " + ", ".join(f"`{name}`" for name in lines_extra)]
    lines += ["", "Entrées : " + (", ".join(f"`{name}` {'non vérifié' if manifest is None else 'ok' if good else 'ÉCHEC'}" for name, good in entries.items()) or "aucune")]
    if problems:
        lines += ["", "## Erreur d’outil", "", *problems]
    elif result is None:
        lines += ["", "## Erreur d’outil", "", f"Le pilote n’a écrit aucun résultat (code {drive_status})."]
    elif result.get("error"):
        lines += ["", "## Échec", "", f"`{result['error']}`"]
    elif drive_status:
        lines += ["", "## Erreur d’outil", "", f"Le pilote a quitté avec le code {drive_status}."]
    status = "PASS" if ok else "FIX"
    lines += ["", verdict_line(status, sha, patch)]
    return status, lines


def main() -> None:
    parser = argparse.ArgumentParser(description="Verdict de la preuve Linux")
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--drive-status", type=int, default=1)
    parser.add_argument("--patch", help="patch-id git (40 hexadécimaux minuscules), recopié en patch=<id>")
    parser.add_argument("--build-failed", metavar="ÉTAPE", help="étape de build en échec : FIX sans pilotage")
    parser.add_argument("--build-status", type=int, default=1)
    args = parser.parse_args()
    if not commit_exists(repo_root(), args.sha):
        raise SystemExit(f"commit introuvable : {args.sha}")
    if args.patch is not None and not PATCH_ID.fullmatch(args.patch):
        raise SystemExit("patch-id invalide : 40 hexadécimaux minuscules attendus")
    if args.build_failed:
        status, lines = build_failure(args.sha, args.build_failed, args.build_status, args.patch)
    else:
        status, lines = safe_verdict(args.evidence, args.sha, args.drive_status, args.patch)
    data = write_report(args.evidence / "report.md", lines)
    print(f"preuves: {args.evidence}", flush=True)
    sys.stdout.buffer.write(data)
    sys.stdout.flush()
    raise SystemExit(0 if status == "PASS" else 1)


if __name__ == "__main__":
    main()
