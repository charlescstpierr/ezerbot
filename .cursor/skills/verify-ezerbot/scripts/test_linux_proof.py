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
    def evidence(self, *, error: str | None = None, failing: str | None = None, manifest: bool = True, omit: str | None = None) -> Path:
        root = Path(tempfile.mkdtemp(prefix="linux-verdict-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(root)], check=True))
        captures = root / "captures"
        captures.mkdir()
        features = []
        views = []
        for feature, view in linux_verdict.EXPECTED:
            status = "fail" if view == failing else "pass"
            if status == "pass":
                (captures / f"{view}.png").write_bytes(png())
                views.append({"id": view, "file": f"{view}.png", "sha256": hashlib.sha256(png()).hexdigest()})
            features.append({"feature": feature, "view": view, "file": f"{view}.png", "status": status})
        features = [item for item in features if item["view"] != omit]
        views = [item for item in views if item["id"] != omit]
        (captures / "result.json").write_text(json.dumps({"sha": SHA, "error": error, "features": features}))
        if manifest:
            (captures / "manifest.json").write_text(json.dumps({"sha": SHA, "renderer": "r", "chrome_version": "c", "views": views}))
        return root

    def test_all_features_captured_is_pass(self) -> None:
        status, lines = linux_verdict.verdict(self.evidence(), SHA)
        self.assertEqual(status, "PASS")
        self.assertEqual(lines[-1], f"PASS {SHA}")

    def test_broken_feature_is_fix_and_keeps_earlier_captures(self) -> None:
        evidence = self.evidence(error="settings-connectors readiness failed.", failing="settings-connectors", manifest=False)
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertEqual(lines[-1], f"FIX {SHA}")
        self.assertIn("| `accueil` | `home-dark` | `captures/home-dark.png` | ok |", lines)
        self.assertIn("| `parametres-connecteurs` | `settings-connectors` | `captures/settings-connectors.png` | ÉCHEC |", lines)
        self.assertIn("Entrées : `accueil` ok, `reponse-fil` ok, `parametres-connecteurs` ÉCHEC", lines)

    def test_failed_driver_exit_is_fix_even_with_complete_captures(self) -> None:
        status, lines = linux_verdict.verdict(self.evidence(), SHA, drive_status=134)
        self.assertEqual(status, "FIX")
        self.assertIn("Le pilote a quitté avec le code 134.", lines)

    def test_missing_result_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "result.json").unlink()
        status, lines = linux_verdict.verdict(evidence, SHA, drive_status=1)
        self.assertEqual(status, "FIX")
        self.assertIn("## Erreur d’outil", lines)

    def test_each_omitted_view_is_fix(self) -> None:
        for _, view in linux_verdict.EXPECTED:
            with self.subTest(view=view):
                status, lines = linux_verdict.verdict(self.evidence(omit=view), SHA)
                self.assertEqual(status, "FIX")
                self.assertTrue(any(line.endswith(f"`captures/{view}.png` | ÉCHEC |") for line in lines))

    def test_registry_matches_the_driver(self) -> None:
        listed = subprocess.run(
            ["node", "-e", "const m=require(process.argv[1]); console.log(JSON.stringify(m.selectedViews().map(v => [v.feature, v.id])))", str(SCRIPTS / "linux_drive.cjs")],
            capture_output=True, text=True, check=True,
        )
        self.assertEqual([tuple(item) for item in json.loads(listed.stdout)], list(linux_verdict.EXPECTED))

    def test_truncated_result_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "result.json").write_text('{"sha": "' + SHA[:10])
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("## Erreur d’outil", lines)
        self.assertIn("`result.json` illisible (JSONDecodeError).", lines)

    def test_truncated_manifest_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "manifest.json").write_text('{"views": [')
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("`manifest.json` illisible (JSONDecodeError).", lines)

    def test_malformed_manifest_views_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "manifest.json").write_text(json.dumps({"sha": SHA, "views": [1]}))
        self.assertEqual(linux_verdict.verdict(evidence, SHA)[0], "FIX")

    def test_patch_is_copied_on_the_verdict_line(self) -> None:
        status, lines = linux_verdict.verdict(self.evidence(), SHA, patch="c" * 40)
        self.assertEqual(lines[-1], f"PASS {SHA} patch={'c' * 40}")

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

    def test_evidence_inside_repo_is_refused(self) -> None:
        result = subprocess.run(["bash", str(SCRIPTS / "prove-linux.sh"), "--evidence", str(SCRIPTS.parents[3] / "inside")], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("hors du dépôt", result.stderr)

    def test_all_and_named_spheres_keep_the_prove_py_path(self) -> None:
        # `all` (and every existing sphere) must behave as on main: prove.py, not the Xvfb subset.
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        env["EZERBOT_VERIFY_CARGO"] = "/nonexistent/cargo"
        for sphere in ("all", "accueil"):
            with self.subTest(sphere=sphere):
                scratch = Path(tempfile.mkdtemp(prefix="linux-all-"))
                self.addCleanup(lambda path=scratch: subprocess.run(["rm", "-rf", str(path)], check=True))
                result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), "--sphere", sphere, "--evidence", str(scratch / "ev")], env=env, capture_output=True, text=True, timeout=120)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("cargo introuvable: /nonexistent/cargo", result.stdout + result.stderr)
                self.assertTrue((scratch / "ev" / "stand-in.md").is_file())

    def test_sphere_linux_and_default_use_the_xvfb_path(self) -> None:
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        for extra in (["--sphere", "linux"], []):
            with self.subTest(extra=extra):
                result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), *extra, "--evidence", str(SCRIPTS.parents[3] / "inside")], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)
                self.assertIn("hors du dépôt", result.stderr)

    def test_uncovered_sphere_is_refused_by_the_xvfb_path(self) -> None:
        for sphere in ("modeles", "all"):
            result = subprocess.run(["bash", str(SCRIPTS / "prove-linux.sh"), "--sphere", sphere], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn("seul « linux »", result.stderr)

    def test_explicit_patch_uses_the_linux_path(self) -> None:
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), "--patch", "d" * 40, "--evidence", str(SCRIPTS.parents[3] / "inside")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("hors du dépôt", result.stderr)

    def test_patch_from_environment_is_honored(self) -> None:
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        env["EZERBOT_VERIFY_PATCH_ID"] = "pas-un-patch"
        result = subprocess.run(["bash", str(SCRIPTS / "prove.sh")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("patch-id invalide", result.stderr)

    def failing_build(self, tool: str, extra_env: dict[str, str]) -> tuple[subprocess.CompletedProcess[str], Path]:
        repo = SCRIPTS.parents[3]
        dirty = subprocess.run(["git", "-C", str(repo), "status", "--porcelain", "--", "crates", "app/ui", "app/web", "Cargo.toml", "Cargo.lock", "vendor", "tools/linux-captures.cjs", ".cursor/skills/verify-ezerbot/scripts"], capture_output=True, text=True, check=True).stdout
        if dirty:
            self.skipTest("checkout modifié : prove-linux.sh refuse avant le build")
        scratch = Path(tempfile.mkdtemp(prefix="linux-build-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        fake = scratch / "bin"
        fake.mkdir()
        (fake / tool).write_text("#!/bin/sh\nexit 7\n")
        (fake / tool).chmod(0o755)
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        env.update(extra_env)
        env["PATH"] = f"{fake}:{env['PATH']}"
        evidence = scratch / "evidence"
        result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), "--evidence", str(evidence)], env=env, capture_output=True, text=True, timeout=600)
        return result, evidence

    def test_failed_web_build_writes_a_fix_report_with_the_patch(self) -> None:
        result, evidence = self.failing_build("npm", {"EZERBOT_VERIFY_PATCH_ID": "e" * 40})
        self.assertEqual(result.returncode, 1, result.stderr)
        report = (evidence / "report.md").read_text().splitlines()
        sha = subprocess.run(["git", "-C", str(SCRIPTS), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
        self.assertEqual(report[-1], f"FIX {sha} patch={'e' * 40}")
        self.assertIn("L’étape `npm ci` a échoué (code 7) ; aucune vue n’a été pilotée.", report)

    def test_failed_cargo_build_writes_a_fix_report(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-cargo-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        cargo = scratch / "cargo"
        cargo.write_text("#!/bin/sh\nexit 9\n")
        cargo.chmod(0o755)
        result, evidence = self.failing_build("unused-tool", {"EZERBOT_VERIFY_CARGO": str(cargo)})
        self.assertEqual(result.returncode, 1, result.stderr)
        report = (evidence / "report.md").read_text().splitlines()
        self.assertTrue(report[-1].startswith("FIX "))
        self.assertIn("L’étape `cargo build` a échoué (code 9) ; aucune vue n’a été pilotée.", report)

    def test_used_evidence_folder_is_refused(self) -> None:
        used = Path(tempfile.mkdtemp(prefix="linux-used-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(used)], check=True))
        (used / "captures").mkdir()
        (used / "captures" / "result.json").write_text("{}")
        result = subprocess.run(["bash", str(SCRIPTS / "prove-linux.sh"), "--evidence", str(used)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("déjà utilisé", result.stderr)

    def assert_dead(self, pid: int) -> None:
        state = Path(f"/proc/{pid}/stat")
        alive = state.exists() and state.read_text().split(") ")[-1][0] != "Z"
        self.assertFalse(alive, f"pid {pid} vivant")

    def test_failed_step_kills_its_descendants_and_removes_the_run_folder(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-lib-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        script = (f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\necho "$run" > {scratch}/run\n'
                  f"step . bash -c 'sleep 300 & echo $! > {scratch}/pid; exit 7'\n")
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 7)
        self.assert_dead(int((scratch / "pid").read_text()))
        run = Path((scratch / "run").read_text().strip())
        self.assertTrue(run.name.startswith("ezv."))
        self.assertFalse(run.exists())
        self.assertIn(f"nettoyage: {run} supprimé", result.stdout)

    def test_detached_run_process_is_killed_after_the_step(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-lib-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        spawn = ("import subprocess,sys; p=subprocess.Popen(['sleep','300'], start_new_session=True); "
                 "open(sys.argv[1],'w').write(str(p.pid))")
        script = (f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\necho "$run" > {scratch}/run\n'
                  f'step . env TMPDIR="$run/tmp" python3 -c "{spawn}" {scratch}/pid\n'
                  f'cat /proc/$(cat {scratch}/pid)/stat > {scratch}/after-step 2>/dev/null || true\n')
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        pid = int((scratch / "pid").read_text())
        self.assert_dead(pid)
        after = (scratch / "after-step").read_text()
        self.assertTrue(after == "" or after.split(") ")[-1][0] == "Z", "le processus détaché a survécu à l'étape")

    def test_unrelated_process_is_left_alone(self) -> None:
        other = subprocess.Popen(["sleep", "300"], start_new_session=True)
        self.addCleanup(lambda: (other.kill(), other.wait()))
        script = f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\nstep . true\n'
        subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60, check=True)
        self.assertIsNone(other.poll())

    def test_interrupted_step_is_killed_by_the_trap(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-lib-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        script = (f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\necho "$run" > {scratch}/run\n'
                  f"(sleep 1; kill -TERM $$) &\nstep . bash -c 'sleep 300 & echo $! > {scratch}/pid; wait'\n")
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 143)
        self.assert_dead(int((scratch / "pid").read_text()))
        self.assertFalse(Path((scratch / "run").read_text().strip()).exists())


if __name__ == "__main__":
    unittest.main()
