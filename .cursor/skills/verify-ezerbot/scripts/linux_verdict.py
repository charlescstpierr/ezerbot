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


def verdict(evidence: Path, sha: str) -> tuple[str, list[str]]:
    captures = evidence / "captures"
    result = json.loads((captures / "result.json").read_text())
    if result.get("sha") != sha:
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
    ok = result.get("error") is None and manifest is not None and manifest.get("sha") == sha
    for item in result["features"]:
        shot = captures / item["file"]
        passed = item["status"] == "pass" and (FEATURES_DIR / f"{item['feature']}.md").is_file()
        if passed:
            png_size(shot)
            digest = hashlib.sha256(shot.read_bytes()).hexdigest()
            # Sans manifeste (vue suivante en échec), l'empreinte n'est pas comparée ; le verdict reste FIX.
            passed = manifest is None or hashes.get(item["file"]) == digest
        ok = ok and passed
        lines.append(f"| `{item['feature']}` | `{item['view']}` | `captures/{item['file']}` | {'ok' if passed else 'ÉCHEC'} |")
    if result.get("error"):
        lines += ["", "## Échec", "", f"`{result['error']}`"]
    status = "PASS" if ok else "FIX"
    lines += ["", f"{status} {sha}"]
    return status, lines


def main() -> None:
    parser = argparse.ArgumentParser(description="Verdict de la preuve Linux")
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--sha", required=True)
    args = parser.parse_args()
    if not commit_exists(repo_root(), args.sha):
        raise SystemExit(f"commit introuvable : {args.sha}")
    status, lines = verdict(args.evidence, args.sha)
    (args.evidence / "report.md").write_text("\n".join(lines) + "\n")
    print(f"preuves: {args.evidence}")
    print("\n".join(lines))
    raise SystemExit(0 if status == "PASS" else 1)


if __name__ == "__main__":
    main()
