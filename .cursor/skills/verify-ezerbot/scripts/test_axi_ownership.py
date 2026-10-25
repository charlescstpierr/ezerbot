#!/usr/bin/env python3
"""Exercise browser and cleanup CLIs against isolated runs and a recording axi."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import launch
from verify_lib import proc_identity

SCRIPTS = Path(__file__).resolve().parent


class AxiOwnershipTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = Path(tempfile.mkdtemp(prefix=f"{self._testMethodName}-"))
        self.evidence = self.directory / "evidence"
        self.evidence.mkdir()
        self.log = self.directory / "axi.jsonl"
        self.sessions = self.directory / "sessions.json"
        self.sessions.write_text(json.dumps({"pm-i545": True, "verify-ezerbot": True, "pm-other": True}))
        self.fake = self.directory / "axi"
        self.fake.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "session = os.environ.get('CHROME_DEVTOOLS_AXI_SESSION')\n"
            "with open(os.environ['AXI_LOG'], 'a') as log:\n"
            "    log.write(json.dumps({'session': session, 'home': os.environ.get('HOME'), 'args': sys.argv[1:]}) + '\\n')\n"
            "if sys.argv[1:] == ['stop']:\n"
            "    path = Path(os.environ['AXI_SESSIONS'])\n"
            "    sessions = json.loads(path.read_text())\n"
            "    sessions[session] = False\n"
            "    path.write_text(json.dumps(sessions))\n"
        )
        self.fake.chmod(0o700)
        self.env = os.environ.copy()
        self.env.update({
            "HOME": str(self.directory),
            "CHROME_DEVTOOLS_AXI_SESSION": "pm-i545",
            "EZERBOT_VERIFY_AXI": str(self.fake),
            "EZERBOT_VERIFY_BROWSER": "chrome-devtools-axi",
            "AXI_LOG": str(self.log),
            "AXI_SESSIONS": str(self.sessions),
        })
        with patch.dict(os.environ, self.env):
            self.run_root, self.home, signature = launch.allocate_run_root(self.evidence)
        self.addCleanup(shutil.rmtree, self.run_root, True)
        self.record = {"session": "pm-i545", "run_root": str(self.run_root), "home": str(self.home), **signature}
        self.write_run()
        (self.evidence / "kept.txt").write_text("preuve\n")

    def write_run(self) -> None:
        (self.evidence / "run.json").write_text(json.dumps(self.record))

    def cli(self, script: str, *arguments: str, **environment: str) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            ["bash", str(SCRIPTS / script), "--evidence", str(self.evidence), *arguments],
            env={**self.env, **environment}, text=True, capture_output=True, timeout=30,
        )
        with (self.directory / "cli.jsonl").open("a") as log:
            log.write(json.dumps({"script": script, "code": result.returncode,
                                  "stdout": result.stdout, "stderr": result.stderr}) + "\n")
        return result

    def calls(self) -> list[dict]:
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def open_browser(self) -> None:
        result = self.cli("browser.sh", "open", "http://127.0.0.1:45678")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def cleanup(self, **environment: str) -> dict:
        result = self.cli("cleanup.sh", **environment)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads((self.evidence / "cleanup.json").read_text())

    def assert_no_stop(self) -> None:
        self.assertFalse(any(call["args"] == ["stop"] for call in self.calls()))
        self.assertTrue(all(json.loads(self.sessions.read_text()).values()))

    def test_cleanup_uses_owned_session_after_environment_changes_and_is_idempotent(self) -> None:
        self.open_browser()
        cleanup = self.cleanup(CHROME_DEVTOOLS_AXI_SESSION="pm-other", HOME=str(self.directory / "foreign-home"))
        self.assertEqual(self.calls(), [
            {"session": "pm-i545", "home": self.env["HOME"], "args": ["open", "http://127.0.0.1:45678"]},
            {"session": "pm-i545", "home": self.env["HOME"], "args": ["stop"]},
        ])
        self.assertEqual(json.loads(self.sessions.read_text()),
                         {"pm-i545": False, "verify-ezerbot": True, "pm-other": True})
        self.assertEqual(cleanup["killed"]["axi"]["session"], "pm-i545")
        self.assertFalse(self.run_root.exists())
        before = (self.evidence / "cleanup.json").read_bytes()
        self.cleanup(CHROME_DEVTOOLS_AXI_SESSION="verify-ezerbot")
        self.assertEqual(len(self.calls()), 2)
        self.assertEqual((self.evidence / "cleanup.json").read_bytes(), before)
        self.assertEqual((self.evidence / "kept.txt").read_text(), "preuve\n")

    def test_browser_refuses_environment_session_changed_since_launch(self) -> None:
        result = self.cli("browser.sh", "pages", CHROME_DEVTOOLS_AXI_SESSION="pm-other")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_browser_refuses_environment_home_changed_since_launch(self) -> None:
        result = self.cli("browser.sh", "pages", HOME=str(self.directory / "foreign-home"))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_changed_axi_home_does_not_authorize_foreign_stop(self) -> None:
        self.open_browser()
        self.record["axi_home"] = str(self.directory / "foreign-home")
        self.write_run()
        self.cleanup()
        self.assert_no_stop()

    def test_cleanup_without_browser_use_does_not_stop_a_session(self) -> None:
        self.cleanup()
        self.assert_no_stop()

    def test_browser_metadata_write_failure_does_not_call_axi(self) -> None:
        (self.run_root / "browser.json").mkdir()
        result = self.cli("browser.sh", "pages")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])
        self.cleanup()
        self.assert_no_stop()

    def test_legacy_run_without_session_binding_does_not_stop_axi(self) -> None:
        self.open_browser()
        self.record.pop("session_signature", None)
        self.write_run()
        self.cleanup()
        self.assert_no_stop()

    def test_changed_run_session_does_not_authorize_foreign_stop(self) -> None:
        self.open_browser()
        self.record["session"] = "verify-ezerbot"
        self.write_run()
        self.cleanup()
        self.assert_no_stop()

    def test_foreign_browser_record_does_not_authorize_stop(self) -> None:
        self.open_browser()
        other = self.directory / "other-evidence"
        other.mkdir()
        with patch.dict(os.environ, self.env):
            foreign_root, _, signature = launch.allocate_run_root(other)
        self.addCleanup(shutil.rmtree, foreign_root, True)
        (self.run_root / "browser.json").write_text(json.dumps({
            "mode": "chrome-devtools-axi", "session": "pm-i545",
            "started_by": "verify-ezerbot", "session_signature": signature.get("session_signature"),
        }))
        self.cleanup()
        self.assert_no_stop()

    def test_invalid_browser_records_never_stop_axi(self) -> None:
        self.open_browser()
        path = self.run_root / "browser.json"
        for state in (None, {}, {"mode": "chrome-devtools-axi", "session": "pm-other"},
                      {"mode": "chrome-headless", "started_by": "verify-ezerbot"}):
            with self.subTest(state=state):
                path.write_text(json.dumps(state))
                result = self.cli("browser.sh", "pages")
                self.assertNotEqual(result.returncode, 0)
        path.write_text("broken json")
        self.cleanup()
        self.assert_no_stop()

    def test_refused_directory_does_not_stop_axi_or_remove_home(self) -> None:
        self.open_browser()
        self.record["marker"] = "invalid"
        self.write_run()
        result = self.cli("cleanup.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.home.is_dir())
        self.assert_no_stop()

    def test_cleanup_preserves_pid_identity_guards(self) -> None:
        self.open_browser()
        owned = subprocess.Popen(["sleep", "30"], start_new_session=True)
        foreign = subprocess.Popen(["sleep", "30"], start_new_session=True)
        try:
            self.record.update({"wrapper_pid": owned.pid, "wrapper_identity": proc_identity(owned.pid),
                                "fake_pid": foreign.pid, "fake_identity": {"platform": "foreign"}})
            self.write_run()
            cleanup = self.cleanup()
            owned.wait(timeout=5)
            self.assertIsNone(foreign.poll())
            self.assertEqual(cleanup["killed"]["wrapper"], "arrete")
            self.assertEqual(cleanup["killed"]["fake"], "identite-differente-non-tue")
        finally:
            for process in (owned, foreign):
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=5)

    def test_headless_cleanup_stops_only_recorded_chrome_without_axi(self) -> None:
        chrome = subprocess.Popen(["sleep", "30"], start_new_session=True)
        try:
            (self.run_root / "browser.json").write_text(json.dumps({
                "mode": "chrome-headless", "session": "pm-i545", "started_by": "verify-ezerbot",
                "chrome_pid": chrome.pid, "identity": proc_identity(chrome.pid), "pgid": chrome.pid,
            }))
            cleanup = self.cleanup()
            chrome.wait(timeout=5)
            self.assertEqual(cleanup["killed"]["chrome"], "arrete")
            self.assert_no_stop()
        finally:
            if chrome.poll() is None:
                chrome.kill()
            chrome.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
