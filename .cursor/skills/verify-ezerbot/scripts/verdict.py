#!/usr/bin/env python3
"""Emit PASS <sha> or FIX <sha>, or refuse when the reviewed commit is not real."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import commit_exists, evidence_dir, load_run, repo_root, resolve_commit, session_id, validate_patch


def verdict_line(status: str, sha: str, patch: str | None) -> str:
    line = f"{status} {sha}"
    if patch:
        line += f" patch={patch}"
    return line


def load_tool_error(evidence: Path) -> str | None:
    path = evidence / "tool-error.json"
    if not path.is_file():
        return None
    payload = json.loads(path.read_text())
    if not isinstance(payload, dict):
        raise SystemExit("refus: tool-error.json illisible")
    message = payload.get("message") or "erreur d'outil"
    if not isinstance(message, str) or not message.strip():
        raise SystemExit("refus: erreur d'outil vide")
    return " ".join(message.split())


def main() -> None:
    parser = argparse.ArgumentParser(description="Verdict de vérification Ezerbot")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--ref", default="HEAD")
    parser.add_argument("--status", choices=("pass", "fix"), default="pass")
    parser.add_argument("--patch", default=os.environ.get("EZERBOT_VERIFY_PATCH_ID"))
    args = parser.parse_args()
    patch = validate_patch(args.patch if args.patch else None)
    evidence = args.evidence or evidence_dir(session_id())
    try:
        run = load_run(evidence)
        repo = Path(run.get("repo") or repo_root())
    except SystemExit:
        repo = repo_root()
    sha = resolve_commit(repo, args.ref)
    if not sha or not commit_exists(repo, sha):
        print(
            f"refus: git rev-parse --verify {args.ref}^{{commit}} ou git cat-file -e <sha>^{{commit}} a échoué",
            file=sys.stderr,
        )
        raise SystemExit(2)
    findings_path = evidence / "findings.json"
    findings = json.loads(findings_path.read_text()) if findings_path.is_file() else []
    tool_message = load_tool_error(evidence)
    blocking = any(item.get("severity") == "bloquant" for item in findings)
    status = "FIX" if args.status == "fix" or blocking or tool_message else "PASS"
    line = verdict_line(status, sha, patch)
    deviations = evidence / "deviations.md"
    deviation_text = deviations.read_text() if deviations.is_file() else "Comparaison non exécutée.\n"
    lines = deviation_text.splitlines()
    if lines and lines[0].startswith("# "):
        deviation_text = "\n".join(lines[1:]).lstrip("\n") + "\n"
    report = ["# Verdict", "", f"Commit revu : `{sha}`", ""]
    if findings:
        for index, item in enumerate(findings, start=1):
            report.extend(
                [
                    f"{index}. Capture : `{item.get('screenshot') or 'aucune'}`",
                    f"   Défaut : {item.get('defect') or 'non décrit'}",
                    f"   Gravité : {item.get('severity') or 'mineur'}",
                    f"   Correctif : {item.get('fix') or 'à préciser'}",
                    "",
                ]
            )
    elif tool_message:
        report.extend(["Aucun défaut d'Ezerbot.", ""])
    else:
        report.extend(["Aucun défaut numéroté.", ""])
    if tool_message:
        report.extend(
            [
                "## Erreur d'outil",
                "",
                tool_message,
                "",
                "Ce plantage vient de l'outil de vérification. Ce n'est pas un défaut d'Ezerbot.",
                "",
            ]
        )
    report.extend(["## Écarts de référence", "", deviation_text.rstrip(), "", line, ""])
    (evidence / "report.md").write_text("\n".join(report))
    print(line)


if __name__ == "__main__":
    main()
