#!/usr/bin/env python3
"""Linux proof: verdict rules, prerequisites report, cleanup of the run folder."""

from __future__ import annotations

import hashlib
import json
import os
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import doctor  # noqa: E402
import linux_verdict  # noqa: E402

SCRIPTS = Path(__file__).resolve().parent
SHA = "a" * 40


def png(width: int = 4, height: int = 3) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    raw = b"".join(b"\x00" + b"\x00" * width * 3 for _ in range(height))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


class VerdictTests(unittest.TestCase):
    def evidence(self, *, error: str | None = None, failing: str | None = None, manifest: bool = True) -> Path:
        root = Path(tempfile.mkdtemp(prefix="linux-verdict-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(root)], check=True))
        captures = root / "captures"
        captures.mkdir()
        features = []
        views = []
        for feature, view in (("accueil", "home-dark"), ("reponse-fil", "conversation-dark"), ("parametres-connecteurs", "settings-dark")):
            status = "fail" if view == failing else "pass"
            if status == "pass":
                (captures / f"{view}.png").write_bytes(png())
                views.append({"id": view, "file": f"{view}.png", "sha256": hashlib.sha256(png()).hexdigest()})
            features.append({"feature": feature, "view": view, "file": f"{view}.png", "status": status})
        (captures / "result.json").write_text(json.dumps({"sha": SHA, "error": error, "features": features}))
        if manifest:
            (captures / "manifest.json").write_text(json.dumps({"sha": SHA, "renderer": "r", "chrome_version": "c", "views": views}))
        return root

    def test_all_features_captured_is_pass(self) -> None:
        status, lines = linux_verdict.verdict(self.evidence(), SHA)
        self.assertEqual(status, "PASS")
        self.assertEqual(lines[-1], f"PASS {SHA}")

    def test_broken_feature_is_fix_and_keeps_earlier_captures(self) -> None:
        evidence = self.evidence(error="settings-dark readiness failed.", failing="settings-dark", manifest=False)
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertEqual(lines[-1], f"FIX {SHA}")
        self.assertIn("| `accueil` | `home-dark` | `captures/home-dark.png` | ok |", lines)
        self.assertIn("| `parametres-connecteurs` | `settings-dark` | `captures/settings-dark.png` | ÉCHEC |", lines)

    def test_tampered_capture_is_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "home-dark.png").write_bytes(png(5, 3))
        self.assertEqual(linux_verdict.verdict(evidence, SHA)[0], "FIX")

    def test_other_commit_is_refused(self) -> None:
        with self.assertRaises(SystemExit):
            linux_verdict.verdict(self.evidence(), "b" * 40)

    def test_mapped_features_exist_in_the_feature_map(self) -> None:
        for name in ("accueil", "reponse-fil", "parametres-connecteurs"):
            self.assertTrue((linux_verdict.FEATURES_DIR / f"{name}.md").is_file(), name)


@unittest.skipUnless(sys.platform.startswith("linux"), "prérequis Linux")
class LinuxPrerequisitesTests(unittest.TestCase):
    def test_missing_tool_is_reported(self) -> None:
        env = {**os.environ, "PATH": "/nonexistent", "EZERBOT_VERIFY_CARGO": "/nonexistent/cargo"}
        result = subprocess.run([sys.executable, str(SCRIPTS / "doctor.py"), "--prerequisites"], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("echec  xvfb-run", result.stdout)
        self.assertIn("echec  cargo: /nonexistent/cargo", result.stdout)
        self.assertEqual(result.stdout.strip().splitlines()[-1], "doctor: echec")

    def test_no_mac_path_is_needed(self) -> None:
        text = (SCRIPTS / "prove-linux.sh").read_text() + (SCRIPTS / "linux_drive.cjs").read_text()
        self.assertNotIn("/Users", text)
        self.assertNotIn("/Applications", text)

    def test_doctor_lists_prerequisites(self) -> None:
        names = {line.split(":")[0].split()[-1] for line in subprocess.run(
            [sys.executable, str(SCRIPTS / "doctor.py"), "--prerequisites"], capture_output=True, text=True
        ).stdout.splitlines() if ":" in line}
        self.assertTrue({"xvfb-run", "Xvfb", "xauth", "chrome", "node-websocket", "cargo", "disque"} <= names)

    def test_refused_run_still_removes_its_run_folder(self) -> None:
        before = set(Path("/tmp").glob("ezv.*"))
        result = subprocess.run(["bash", str(SCRIPTS / "prove-linux.sh"), "--evidence", str(SCRIPTS.parents[3] / "inside")], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("hors du dépôt", result.stderr)
        self.assertEqual(set(Path("/tmp").glob("ezv.*")), before)


if __name__ == "__main__":
    unittest.main()
