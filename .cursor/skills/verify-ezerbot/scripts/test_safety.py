#!/usr/bin/env python3
"""Red-to-green checks for the verify-ezerbot safety review."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))

import launch  # noqa: E402
import verify_lib  # noqa: E402
from verify_lib import is_real_home, touches_grokbot_support  # noqa: E402

HEX40 = "ab" * 20


def run_cleanup(evidence: Path, cwd: Path, env: dict | None = None) -> subprocess.CompletedProcess[str]:
    child_env = os.environ.copy()
    child_env.pop("EZERBOT_VERIFY_EVIDENCE_DIR", None)
    if env:
        child_env.update(env)
    return subprocess.run(
        [sys.executable, str(SCRIPTS / "cleanup.py"), "--evidence", str(evidence)],
        cwd=cwd,
        env=child_env,
        text=True,
        capture_output=True,
    )


class CleanupTests(unittest.TestCase):
    def test_empty_home_does_not_delete_cwd(self) -> None:
        victim = Path(tempfile.mkdtemp(prefix="cleanup-cwd-"))
        sentinel = victim / "sentinel"
        sentinel.write_text("keep")
        evidence = Path(tempfile.mkdtemp(prefix="cleanup-evidence-"))
        (evidence / "run.json").write_text(json.dumps({"home": "", "run_root": "", "cleaned": False}))
        result = run_cleanup(evidence, victim)
        self.assertTrue(sentinel.is_file(), result.stderr + result.stdout)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("cleanup: ok", result.stdout)

    def test_missing_home_does_not_delete_cwd(self) -> None:
        victim = Path(tempfile.mkdtemp(prefix="cleanup-cwd-"))
        sentinel = victim / "sentinel"
        sentinel.write_text("keep")
        evidence = Path(tempfile.mkdtemp(prefix="cleanup-evidence-"))
        (evidence / "run.json").write_text(json.dumps({"cleaned": False}))
        result = run_cleanup(evidence, victim)
        self.assertTrue(sentinel.is_file(), result.stderr + result.stdout)
        self.assertNotEqual(result.returncode, 0)

    def test_only_signed_tmpdir_run_is_removed(self) -> None:
        outside = Path(tempfile.mkdtemp(prefix="not-system-"))
        run_root = outside / "verify-ezerbot.forged"
        home = run_root / "home"
        home.mkdir(parents=True)
        (home / "sentinel").write_text("keep")
        (run_root / "sentinel").write_text("keep")
        evidence = Path(tempfile.mkdtemp(prefix="cleanup-evidence-"))
        (evidence / "kept").write_text("preuve")
        (evidence / "run.json").write_text(
            json.dumps({"home": str(home), "run_root": str(run_root), "cleaned": False})
        )
        result = run_cleanup(evidence, Path(tempfile.gettempdir()))
        self.assertTrue((home / "sentinel").is_file(), result.stdout + result.stderr)
        self.assertTrue((run_root / "sentinel").is_file(), result.stdout + result.stderr)
        self.assertTrue((evidence / "kept").is_file())
        self.assertNotEqual(result.returncode, 0)

    def test_real_home_run_root_is_refused(self) -> None:
        run_root = Path(tempfile.mkdtemp(prefix="verify-ezerbot."))
        evidence: Path | None = None
        try:
            evidence = Path(tempfile.mkdtemp(prefix="cleanup-evidence-"))
            home = run_root / "home"
            home.mkdir()
            (home / "sentinel").write_text("keep")
            record = {
                "home": str(home),
                "run_root": str(run_root),
                "cleaned": False,
                "marker_key": "ab" * 32,
                "marker": "cd" * 32,
            }
            (evidence / "run.json").write_text(json.dumps(record))
            result = run_cleanup(evidence, Path(tempfile.gettempdir()), {"EZERBOT_VERIFY_REAL_HOME": str(run_root)})
            self.assertTrue((home / "sentinel").is_file(), result.stdout + result.stderr)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("dossier personnel", result.stderr + result.stdout)
            self.assertTrue(run_root.is_dir())
        finally:
            shutil.rmtree(run_root, ignore_errors=True)
            if evidence is not None:
                shutil.rmtree(evidence, ignore_errors=True)

    def test_signed_run_under_system_tmpdir_is_removed(self) -> None:
        evidence = Path(tempfile.mkdtemp(prefix="cleanup-evidence-"))
        (evidence / "kept").write_text("preuve")
        run_root, home, signature = launch.allocate_run_root(evidence)
        (home / "sentinel").write_text("gone")
        record = {"home": str(home), "run_root": str(run_root), "cleaned": False, **signature}
        (evidence / "run.json").write_text(json.dumps(record))
        result = run_cleanup(evidence, Path(tempfile.gettempdir()))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(home.exists())
        self.assertFalse(run_root.exists())
        self.assertTrue((evidence / "kept").is_file())
        cleanup = json.loads((evidence / "cleanup.json").read_text())
        self.assertTrue(cleanup["removed_home"])
        self.assertFalse(cleanup["home_still_present"])


class PathIdentityTests(unittest.TestCase):
    def test_grokbot_support_is_case_insensitive_and_follows_links(self) -> None:
        self.assertTrue(
            touches_grokbot_support(Path("/tmp/x/library/application support/grokbot/workspace.sqlite"))
        )
        self.assertFalse(
            touches_grokbot_support(Path("/tmp/x/Library/Application Support/gRoKbOt LoCaL/workspace.sqlite"))
        )
        root = Path(tempfile.mkdtemp(prefix="support-case-"))
        os.environ["EZERBOT_VERIFY_APP_SUPPORT"] = str(root / "Support")
        (root / "Support").mkdir()
        self.assertTrue(touches_grokbot_support(Path(str(root / "support" / "workspace.sqlite"))))
        target = root / "Library" / "Application Support" / "Grokbot"
        target.mkdir(parents=True)
        link = root / "alias"
        link.symlink_to(target)
        self.assertTrue(touches_grokbot_support(link / "workspace.sqlite"))

    def test_real_home_is_case_insensitive_and_follows_links(self) -> None:
        root = Path(tempfile.mkdtemp(prefix="home-case-"))
        home = root / "HomeDir"
        home.mkdir()
        os.environ["EZERBOT_VERIFY_REAL_HOME"] = str(home)
        self.assertTrue(is_real_home(Path(str(root / "homedir"))))
        link = root / "alias-home"
        link.symlink_to(home)
        self.assertTrue(is_real_home(link))
        self.assertFalse(is_real_home(link / "Library"))


class LaunchOrderTests(unittest.TestCase):
    def test_unsafe_tmpdir_is_refused_before_mkdtemp(self) -> None:
        unsafe = Path(tempfile.mkdtemp(prefix="unsafe-tmpdir-"))
        evidence = Path(tempfile.mkdtemp(prefix="launch-evidence-"))
        previous = tempfile.tempdir
        calls: list[str] = []
        real_mkdtemp = tempfile.mkdtemp

        def wrapped(*args, **kwargs):
            calls.append("mkdtemp")
            return real_mkdtemp(*args, **kwargs)

        os.environ["EZERBOT_VERIFY_REAL_HOME"] = str(unsafe)
        os.environ["TMPDIR"] = str(unsafe)
        tempfile.tempdir = None
        tempfile.mkdtemp = wrapped
        try:
            with self.assertRaises(SystemExit):
                launch.allocate_run_root(evidence)
        finally:
            tempfile.mkdtemp = real_mkdtemp
            tempfile.tempdir = previous
        self.assertEqual(calls, [])
        self.assertEqual(list(unsafe.iterdir()), [])


class TerminateTests(unittest.TestCase):
    def test_caller_group_is_not_killed(self) -> None:
        script = textwrap.dedent(
            f"""
            import os, sys
            sys.path.insert(0, {str(SCRIPTS)!r})
            from verify_lib import pid_alive, terminate_recorded
            from process_test_support import recorded_child
            os.setsid()
            with recorded_child(["sleep", "30"]) as (sibling, _):
                with recorded_child(["sleep", "30"], start_new_session=True) as (victim, identity):
                    identity["pgid"] = os.getpgrp()
                    result = terminate_recorded(victim.pid, identity)
                    sibling_alive = pid_alive(sibling.pid)
                    victim_alive = pid_alive(victim.pid)
                    if result != "arrete" or victim_alive or not sibling_alive:
                        raise AssertionError((result, victim_alive, sibling_alive))
            """
        )
        result = subprocess.run([sys.executable, "-c", script], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_same_group_kills_only_the_recorded_pid(self) -> None:
        script = textwrap.dedent(
            f"""
            import os, sys
            sys.path.insert(0, {str(SCRIPTS)!r})
            from verify_lib import pid_alive, terminate_recorded
            from process_test_support import recorded_child
            os.setsid()
            with recorded_child(["sleep", "30"]) as (sibling, _):
                with recorded_child(["sleep", "30"]) as (victim, identity):
                    identity["pgid"] = os.getpgid(victim.pid)
                    assert identity["pgid"] == os.getpgrp()
                    result = terminate_recorded(victim.pid, identity)
                    sibling_alive = pid_alive(sibling.pid)
                    victim_alive = pid_alive(victim.pid)
                    if result != "arrete" or victim_alive or not sibling_alive:
                        raise AssertionError((result, victim_alive, sibling_alive))
            """
        )
        result = subprocess.run([sys.executable, "-c", script], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_matching_foreign_group_is_killed(self) -> None:
        script = textwrap.dedent(
            f"""
            import os, sys
            sys.path.insert(0, {str(SCRIPTS)!r})
            from verify_lib import pid_alive, terminate_recorded
            from process_test_support import recorded_child
            with recorded_child(["sleep", "30"], start_new_session=True) as (victim, identity):
                identity["pgid"] = os.getpgid(victim.pid)
                assert identity["pgid"] != os.getpgrp()
                result = terminate_recorded(victim.pid, identity)
                alive = pid_alive(victim.pid)
                if result != "arrete" or alive:
                    raise AssertionError((result, alive))
            """
        )
        result = subprocess.run([sys.executable, "-c", script], text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class PatchTests(unittest.TestCase):
    def test_control_characters_and_loose_ids_are_refused(self) -> None:
        evidence = Path(tempfile.mkdtemp(prefix="verdict-evidence-"))
        (evidence / "run.json").write_text("{}\n")
        injected = HEX40[:8] + "\nFIX 0000000000000000000000000000000000000000"
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "verdict.py"),
                "--evidence",
                str(evidence),
                "--ref",
                "HEAD",
                "--status",
                "pass",
                "--patch",
                injected,
            ],
            cwd=SCRIPTS.parents[3],
            text=True,
            capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("FIX 0000000000000000000000000000000000000000", result.stdout)
        report = evidence / "report.md"
        if report.is_file():
            self.assertNotIn("FIX 0000000000000000000000000000000000000000", report.read_text())
        for bad in (HEX40.upper(), HEX40 + "\n", "zz" * 20, HEX40[:-1], HEX40 + " ", "ab\x01" + "cd" * 18):
            with self.assertRaises(SystemExit):
                verify_lib.validate_patch(bad)

    def test_strict_patch_id_is_appended_verbatim(self) -> None:
        self.assertEqual(verify_lib.validate_patch(None), None)
        self.assertEqual(verify_lib.validate_patch(""), None)
        self.assertEqual(verify_lib.validate_patch(HEX40), HEX40)
        evidence = Path(tempfile.mkdtemp(prefix="verdict-evidence-"))
        (evidence / "run.json").write_text("{}\n")
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "verdict.py"),
                "--evidence",
                str(evidence),
                "--ref",
                "HEAD",
                "--status",
                "pass",
                "--patch",
                HEX40,
            ],
            cwd=SCRIPTS.parents[3],
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        line = result.stdout.strip().splitlines()[-1]
        self.assertRegex(line, rf"^PASS [0-9a-f]{{40}} patch={HEX40}$")
        self.assertNotIn("\n", line)


if __name__ == "__main__":
    unittest.main()
