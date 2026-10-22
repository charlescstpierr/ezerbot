#!/usr/bin/env python3
"""Read-only comparison against Grok Bot reference trees. Lists deviations."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import MAC_REF_ANNEX, MAC_REF_SHOTS, evidence_dir, png_size, repo_root, session_id


def inventory(root: Path) -> dict:
    if not root.exists():
        return {"path": str(root), "exists": False, "files": 0, "pngs": []}
    files = 0
    pngs: list[str] = []
    for path in root.rglob("*"):
        if not path.is_file() or path.is_symlink():
            continue
        files += 1
        if path.suffix.lower() == ".png" and len(pngs) < 40:
            pngs.append(str(path.relative_to(root)))
    return {"path": str(root), "exists": True, "files": files, "pngs": pngs}


def pair_shots(evidence: Path, shots: Path) -> list[dict]:
    rows = []
    sphere_root = evidence / "spheres"
    if not sphere_root.is_dir():
        return rows
    references = []
    if shots.exists():
        references = [path for path in shots.rglob("*") if path.is_file() and path.suffix.lower() == ".png" and not path.is_symlink()]
    for shot in sorted(sphere_root.rglob("*.png")):
        stem = shot.stem.lower()
        match = None
        for candidate in references:
            name = candidate.stem.lower()
            if stem in name or name in stem:
                match = candidate
                break
        row = {"shot": str(shot), "reference": str(match) if match else None, "note": ""}
        if match is None:
            row["note"] = "aucune capture de référence appariée"
        else:
            try:
                left = png_size(shot)
                right = png_size(match)
            except ValueError as error:
                row["note"] = str(error)
            else:
                row["note"] = f"dimensions preuve {left[0]}x{left[1]}, référence {right[0]}x{right[1]}"
                compare = shutil.which("compare")
                if compare:
                    metric = subprocess.run(
                        [compare, "-metric", "AE", str(shot), str(match), "null:"],
                        capture_output=True,
                        text=True,
                        check=False,
                    )
                    row["pixel_ae"] = (metric.stderr or metric.stdout).strip()
        rows.append(row)
    return rows


def main() -> None:
    parser = argparse.ArgumentParser(description="Compare les captures aux références Grok Bot, en lecture seule")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    evidence = args.evidence or evidence_dir(session_id())
    repo = repo_root()
    roots = {
        "reports": Path(os.environ.get("EZERBOT_VERIFY_REA_REPORTS", repo / "reports")),
        "evidence": Path(os.environ.get("EZERBOT_VERIFY_REA_EVIDENCE", repo / "evidence")),
        "formatted": Path(os.environ.get("EZERBOT_VERIFY_REA_FORMATTED", repo / "artifacts/formatted")),
        "shots": Path(os.environ.get("EZERBOT_VERIFY_REF_SHOTS", MAC_REF_SHOTS)),
        "annex": Path(os.environ.get("EZERBOT_VERIFY_REF_ANNEX", MAC_REF_ANNEX)),
    }
    inventories = {name: inventory(path) if path.is_dir() or name != "annex" else {"path": str(path), "exists": path.is_file(), "files": int(path.is_file()), "pngs": []} for name, path in roots.items()}
    if roots["annex"].is_file():
        inventories["annex"] = {"path": str(roots["annex"]), "exists": True, "files": 1, "pngs": []}
    pairs = pair_shots(evidence, roots["shots"])
    gaps = [name for name, info in inventories.items() if not info.get("exists")]
    payload = {"inventories": inventories, "pairs": pairs, "missing_corpora": gaps}
    (evidence / "deviations.json").write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")
    lines = ["# Écarts de référence", ""]
    for name, info in inventories.items():
        state = "présent" if info.get("exists") else "absent"
        lines.append(f"- {name} : {state} ({info.get('path')}), {info.get('files', 0)} fichier(s)")
    lines.append("")
    if not pairs:
        lines.append("Aucune capture de sphère à comparer.")
    for row in pairs:
        lines.append(f"- `{row['shot']}` : {row['note']}")
    lines.append("")
    lines.append("Les corpus absents ne sont pas des défauts du produit. Ils restent non comparés.")
    (evidence / "deviations.md").write_text("\n".join(lines) + "\n")
    print(f"compare: ok corpus-absents={','.join(gaps) if gaps else 'aucun'}")


if __name__ == "__main__":
    main()
