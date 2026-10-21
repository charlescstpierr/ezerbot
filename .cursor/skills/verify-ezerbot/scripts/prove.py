#!/usr/bin/env python3
"""End-to-end proof: launch, doctor, one sphere, verdict, cleanup, evidence remains."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import evidence_dir, png_size, run_path, session_id

MAC_COMMAND = (
    "CHROME_DEVTOOLS_AXI_SESSION=verify-ezerbot "
    "bash .cursor/skills/verify-ezerbot/scripts/prove.sh --sphere accueil"
)


def script(name: str) -> Path:
    return Path(__file__).with_name(name)


def run_script(name: str, *args: str) -> None:
    result = subprocess.run([sys.executable, str(script(name)), *args], env=os.environ.copy())
    if result.returncode != 0:
        raise SystemExit(result.returncode)


def note_unclassified_drive_failure(evidence: Path) -> None:
    """A drive crash with no product finding must still show up as a tool error."""
    if (evidence / "tool-error.json").is_file():
        return
    findings_path = evidence / "findings.json"
    findings = json.loads(findings_path.read_text()) if findings_path.is_file() else []
    if any(item.get("severity") == "bloquant" for item in findings):
        return
    payload = {
        "kind": "outil",
        "message": "le pilotage s'est arrêté sans écrire de défaut d'Ezerbot",
    }
    (evidence / "tool-error.json").write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description="Preuve de bout en bout verify-ezerbot")
    parser.add_argument("--sphere", default="accueil")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--patch", default=os.environ.get("EZERBOT_VERIFY_PATCH_ID"))
    args = parser.parse_args()
    os.environ.setdefault("CHROME_DEVTOOLS_AXI_SESSION", "verify-ezerbot")
    if sys.platform != "darwin" and not os.environ.get("EZERBOT_VERIFY_BROWSER"):
        os.environ["EZERBOT_VERIFY_BROWSER"] = "chrome-headless"
    session = session_id()
    evidence = evidence_dir(session, str(args.evidence) if args.evidence else None)
    os.environ["EZERBOT_VERIFY_EVIDENCE_DIR"] = str(evidence)
    if os.environ.get("EZERBOT_VERIFY_BROWSER") == "chrome-headless":
        (evidence / "stand-in.md").write_text(
            "\n".join(
                [
                    "# Substitut Linux",
                    "",
                    "Cette exécution n'est pas la preuve macOS.",
                    "",
                    "- Chrome a été lancé en headless (`--headless=new --no-sandbox --disable-dev-shm-usage`) au lieu de `chrome-devtools-axi`.",
                    "- Le binaire est `google-chrome-stable` (ou `/opt/google/chrome/chrome`), pas un wrapper qui épingle un profil dans le home réel.",
                    "- Aucune URL initiale : Chrome 156 headless quitte si une URL s'ajoute à un `--user-data-dir` séparé. Le profil est `--user-data-dir=<HOME jetable>/chrome-profile`.",
                    "- Aucune fenêtre WKWebView et aucun `Ezerbot.app` n'ont été ouverts.",
                    "- Les polices ne sont pas celles d'-apple-system.",
                    "- Le trousseau macOS n'est pas exercé : le coffre retombe sur un fichier du HOME jetable.",
                    "- `CARGO_TARGET_DIR` n'est pas défini. Un `CARGO_HOME` étranger est ignoré pour que `~/.cargo/bin/cargo` utilise son propre toolchain.",
                    "- Les corpus absents (captures `data/gb-visual-qa/shots`, `artifacts/formatted`) restent non comparés.",
                    "",
                    "Commande unique qui rejoue la preuve sur le Mac, depuis la racine du dépôt :",
                    "",
                    "```bash",
                    MAC_COMMAND,
                    "```",
                    "",
                    "Elle enchaîne le build, le lancement isolé, le doctor, la sphère accueil en clair et en sombre aux deux tailles, le verdict et le nettoyage.",
                    "",
                ]
            )
        )
    failed = None
    try:
        run_script("verify_lib.py", "self-check")
        run_script("launch.py", "--evidence", str(evidence))
        run_script("doctor.py", "--evidence", str(evidence))
        try:
            run_script("drive.py", "--evidence", str(evidence), "--sphere", args.sphere)
        except SystemExit as error:
            failed = error.code or 1
            note_unclassified_drive_failure(evidence)
        run_script("compare.py", "--evidence", str(evidence))
        verdict_args = ["--evidence", str(evidence), "--ref", "HEAD", "--status", "fix" if failed else "pass"]
        if args.patch:
            verdict_args.extend(["--patch", args.patch])
        run_script("verdict.py", *verdict_args)
    finally:
        subprocess.run([sys.executable, str(script("cleanup.py")), "--evidence", str(evidence)], env=os.environ.copy())
    if not run_path(evidence).is_file():
        raise SystemExit("les preuves ont disparu avec le nettoyage")
    report = evidence / "report.md"
    if not report.is_file():
        raise SystemExit("report.md absent après nettoyage")
    shots = list((evidence / "spheres").rglob("*.png")) if (evidence / "spheres").is_dir() else []
    if not shots:
        raise SystemExit("aucune capture n'a survécu au nettoyage")
    for shot in shots:
        png_size(shot)
    home_line = (evidence / "cleanup.json").read_text() if (evidence / "cleanup.json").is_file() else ""
    if '"removed_home": true' not in home_line and '"home_still_present": false' not in home_line:
        raise SystemExit("le HOME jetable n'est pas confirmé supprimé")
    if '"home_still_present": true' in home_line:
        raise SystemExit("le HOME jetable est encore là")
    verdict = [line for line in report.read_text().splitlines() if line.startswith("PASS ") or line.startswith("FIX ")]
    if not verdict:
        raise SystemExit("ligne de verdict absente")
    print(verdict[-1])
    print(f"preuves: {evidence}")
    if failed or verdict[-1].startswith("FIX "):
        raise SystemExit(failed or 1)


if __name__ == "__main__":
    main()
