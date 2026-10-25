#!/usr/bin/env python3
"""Exercise browser and cleanup CLIs against isolated runs and a recording axi."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import launch
import verify_lib
from process_test_support import recorded_child

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
        self.fake.write_text(self.fake.read_text().replace(
            "if sys.argv[1:] == ['stop']:",
            "barrier = os.environ.get('AXI_BARRIER')\n"
            "def pause(phase):\n"
            "    if barrier and os.environ.get('AXI_PHASE') == phase:\n"
            "        import time\n"
            "        Path(barrier + '.ready').touch()\n"
            "        while not Path(barrier + '.release').exists(): time.sleep(0.01)\n"
            "pause('before')\n"
            "if sys.argv[1:] == ['stop'] and os.environ.get('AXI_FAIL_STOP'):\n"
            "    sys.exit(7)\n"
            "if sys.argv[1:] != ['stop']:\n"
            "    path = Path(os.environ['AXI_SESSIONS'])\n"
            "    sessions = json.loads(path.read_text())\n"
            "    sessions[session] = True\n"
            "    path.write_text(json.dumps(sessions))\n"
            "pause('after')\n"
            "if os.environ.get('AXI_FAIL_COMMAND'): sys.exit(8)\n"
            "if sys.argv[1:] == ['stop']:"
        ))
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

    def start_cli(self, script: str, *arguments: str, **environment: str) -> subprocess.Popen:
        process = subprocess.Popen(
            ["bash", str(SCRIPTS / script), "--evidence", str(self.evidence), *arguments],
            env={**self.env, **environment}, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        with (self.directory / "cli-started.jsonl").open("a") as log:
            log.write(json.dumps({"pid": process.pid, "script": script, "arguments": arguments}) + "\n")
        def finish():
            try:
                process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate(timeout=5)
        self.addCleanup(finish)
        for name in ("AXI_BARRIER", "WRITE_BARRIER", "BUILD_BARRIER"):
            if name in environment:
                self.addCleanup(Path(environment[name] + ".release").touch)
        return process

    def wait_for(self, predicate) -> None:
        deadline = time.monotonic() + 10
        while not predicate():
            self.assertLess(time.monotonic(), deadline, "barrière non atteinte")
            time.sleep(0.01)

    def install_barriers(self) -> None:
        hooks = self.directory / "hooks"
        hooks.mkdir()
        (hooks / "sitecustomize.py").write_text('''import fcntl, os, time
from pathlib import Path
original_write = Path.write_text
original_flock = fcntl.flock
original_replace = Path.replace
def write(path, *args, **kwargs):
    barrier = os.environ.get("WRITE_BARRIER")
    if barrier and path.name.startswith("browser.json"):
        with path.open("w"):
            Path(barrier + ".ready").touch()
            while not Path(barrier + ".release").exists(): time.sleep(0.01)
    return original_write(path, *args, **kwargs)
def flock(fd, operation):
    if operation == fcntl.LOCK_EX:
        try:
            return original_flock(fd, operation | fcntl.LOCK_NB)
        except BlockingIOError:
            marker = os.environ.get("LOCK_BLOCKED")
            if marker: Path(marker).touch()
    return original_flock(fd, operation)
def replace(path, target):
    if os.environ.get("FAIL_REPLACE") == Path(target).name:
        raise OSError("publication refusée par le test")
    return original_replace(path, target)
Path.write_text = write
Path.replace = replace
fcntl.flock = flock
''')
        self.env["PYTHONPATH"] = str(hooks)

    def assert_session_stopped(self) -> None:
        self.assertEqual(json.loads(self.sessions.read_text()),
                         {"pm-i545": False, "verify-ezerbot": True, "pm-other": True})
        self.assertEqual([call["session"] for call in self.calls() if call["args"] == ["stop"]], ["pm-i545"])
        self.assertEqual((self.evidence / "kept.txt").read_text(), "preuve\n")

    def test_cleanup_during_browser_metadata_publication(self) -> None:
        self.open_browser()
        self.install_barriers()
        barrier = str(self.directory / "write")
        browser = self.start_cli("browser.sh", "pages", WRITE_BARRIER=barrier)
        self.wait_for(lambda: Path(barrier + ".ready").exists())
        self.assertEqual(json.loads((self.run_root / "browser.json").read_text())["session"], "pm-i545")
        blocked = self.directory / "cleanup-blocked"
        cleanup = self.start_cli("cleanup.sh", LOCK_BLOCKED=str(blocked))
        try:
            self.wait_for(lambda: blocked.exists() or cleanup.poll() is not None)
        finally:
            Path(barrier + ".release").touch()
        out, err = browser.communicate(timeout=10)
        self.assertEqual(browser.returncode, 0, out + err)
        out, err = cleanup.communicate(timeout=10)
        self.assertEqual(cleanup.returncode, 0, out + err)
        self.assert_session_stopped()

    def cleanup_during_axi(self, command: str, phase: str) -> None:
        if command != "open":
            self.open_browser()
        self.install_barriers()
        barrier = str(self.directory / "axi-barrier")
        browser = self.start_cli("browser.sh", command, AXI_BARRIER=barrier, AXI_PHASE=phase)
        self.wait_for(lambda: Path(barrier + ".ready").exists())
        blocked = self.directory / "cleanup-blocked"
        cleanup = self.start_cli("cleanup.sh", LOCK_BLOCKED=str(blocked))
        try:
            self.wait_for(lambda: blocked.exists() or cleanup.poll() is not None)
        finally:
            Path(barrier + ".release").touch()
        out, err = browser.communicate(timeout=10)
        self.assertEqual(browser.returncode, 0, out + err)
        out, err = cleanup.communicate(timeout=10)
        self.assertEqual(cleanup.returncode, 0, out + err)
        self.assert_session_stopped()
        before = self.calls()
        self.assertNotEqual(self.cli("browser.sh", "open").returncode, 0)
        self.cleanup()
        self.assertEqual(self.calls(), before)

    def test_cleanup_before_axi_open_effect(self) -> None:
        self.cleanup_during_axi("open", "before")

    def test_cleanup_after_axi_open_effect(self) -> None:
        self.cleanup_during_axi("open", "after")

    def test_cleanup_before_axi_command_effect(self) -> None:
        self.cleanup_during_axi("pages", "before")

    def test_cleanup_after_axi_command_effect(self) -> None:
        self.cleanup_during_axi("pages", "after")

    def test_cleanup_blocks_waiting_browser_and_second_cleanup(self) -> None:
        self.open_browser()
        self.install_barriers()
        barrier = str(self.directory / "stop")
        cleanup = self.start_cli("cleanup.sh", AXI_BARRIER=barrier, AXI_PHASE="before")
        self.wait_for(lambda: Path(barrier + ".ready").exists())
        blocked = self.directory / "browser-blocked"
        browser = self.start_cli("browser.sh", "open", LOCK_BLOCKED=str(blocked))
        second_blocked = self.directory / "second-blocked"
        second = self.start_cli("cleanup.sh", LOCK_BLOCKED=str(second_blocked))
        try:
            self.wait_for(lambda: blocked.exists() or browser.poll() is not None)
            self.wait_for(lambda: second_blocked.exists() or second.poll() is not None)
        finally:
            Path(barrier + ".release").touch()
        cleanup.communicate(timeout=10)
        self.assertEqual(cleanup.returncode, 0)
        browser.communicate(timeout=10)
        self.assertNotEqual(browser.returncode, 0)
        second.communicate(timeout=10)
        self.assertEqual(second.returncode, 0)
        self.assert_session_stopped()

    def test_axi_command_failure_remains_attributed(self) -> None:
        self.assertEqual(self.cli("browser.sh", "open", AXI_FAIL_COMMAND="1").returncode, 8)
        self.cleanup()
        self.assert_session_stopped()

    def test_axi_stop_failure_is_retryable_and_blocks_reopening(self) -> None:
        self.open_browser()
        result = self.cli("cleanup.sh", AXI_FAIL_STOP="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.run_root.exists())
        self.assertFalse(json.loads((self.evidence / "run.json").read_text()).get("cleaned", False))
        self.assertNotEqual(self.cli("browser.sh", "pages").returncode, 0)
        self.cleanup()
        self.assertEqual(json.loads(self.sessions.read_text()),
                         {"pm-i545": False, "verify-ezerbot": True, "pm-other": True})

    def test_failed_browser_publication_preserves_previous_attribution(self) -> None:
        self.open_browser()
        self.install_barriers()
        before = (self.run_root / "browser.json").read_bytes()
        result = self.cli("browser.sh", "pages", FAIL_REPLACE="browser.json")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.run_root / "browser.json").read_bytes(), before)
        self.assertEqual(len(self.calls()), 1)
        self.cleanup()
        self.assert_session_stopped()

    def test_failed_cleanup_publication_has_no_external_effect(self) -> None:
        self.open_browser()
        self.install_barriers()
        result = self.cli("cleanup.sh", FAIL_REPLACE="run.json")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.calls()), 1)
        self.assertTrue(self.home.exists())
        self.assertFalse(json.loads((self.evidence / "run.json").read_text()).get("cleaning", False))
        self.cleanup()
        self.assert_session_stopped()

    def test_missing_axi_binary_preserves_retry(self) -> None:
        self.open_browser()
        result = self.cli("cleanup.sh", EZERBOT_VERIFY_AXI=str(self.directory / "absent"))
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.run_root.exists())
        self.assertNotEqual(self.cli("browser.sh", "open").returncode, 0)
        self.cleanup()
        self.assert_session_stopped()

    def test_launch_refuses_active_and_cleaned_evidence(self) -> None:
        original = (self.evidence / "run.json").read_bytes()
        self.assertNotEqual(self.cli("launch.sh").returncode, 0)
        self.assertEqual((self.evidence / "run.json").read_bytes(), original)
        self.cleanup()
        original = (self.evidence / "run.json").read_bytes()
        self.assertNotEqual(self.cli("launch.sh").returncode, 0)
        self.assertEqual((self.evidence / "run.json").read_bytes(), original)
        self.assertEqual(self.calls(), [])

    def test_cleanup_waits_for_launch_before_first_run_publication(self) -> None:
        self.install_barriers()
        self.evidence = self.directory / "new-evidence"
        self.evidence.mkdir()
        barrier = str(self.directory / "build")
        build = self.directory / "fake-build"
        build.write_text(
            f"#!{sys.executable}\n"
            "import os, time\nfrom pathlib import Path\n"
            "barrier = os.environ['BUILD_BARRIER']\n"
            "Path(barrier + '.ready').touch()\n"
            "while not Path(barrier + '.release').exists(): time.sleep(0.01)\n"
            "raise SystemExit(1)\n"
        )
        build.chmod(0o700)
        launch_process = self.start_cli("launch.sh", EZERBOT_VERIFY_CARGO=str(build), BUILD_BARRIER=barrier)
        self.wait_for(lambda: Path(barrier + ".ready").exists())
        blocked = self.directory / "cleanup-blocked"
        cleanup = self.start_cli("cleanup.sh", LOCK_BLOCKED=str(blocked))
        try:
            self.wait_for(lambda: blocked.exists() or cleanup.poll() is not None)
            self.assertTrue(blocked.exists(), "cleanup doit attendre launch, même sans run.json")
        finally:
            Path(barrier + ".release").touch()
        launch_process.communicate(timeout=10)
        self.assertNotEqual(launch_process.returncode, 0)
        out, err = cleanup.communicate(timeout=10)
        self.assertEqual(cleanup.returncode, 0, out + err)
        self.assertFalse((self.evidence / "run.json").exists())
        self.assertEqual(self.calls(), [])

    def test_headless_mode_cannot_replace_an_axi_attribution(self) -> None:
        self.open_browser()
        result = self.cli("browser.sh", "open", EZERBOT_VERIFY_BROWSER="chrome-headless",
                          EZERBOT_VERIFY_CHROME=str(self.fake))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.calls()), 1)
        self.cleanup()
        self.assert_session_stopped()

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
        with recorded_child(["sleep", "30"], start_new_session=True) as (owned, identity), \
                recorded_child(["sleep", "30"], start_new_session=True) as (foreign, _):
            self.record.update({"wrapper_pid": owned.pid, "wrapper_identity": identity,
                                "fake_pid": foreign.pid, "fake_identity": {"platform": "foreign"}})
            self.write_run()
            cleanup = self.cleanup()
            owned.wait(timeout=5)
            self.assertIsNone(foreign.poll())
            self.assertEqual(cleanup["killed"]["wrapper"], "arrete")
            self.assertEqual(cleanup["killed"]["fake"], "identite-differente-non-tue")

    def test_headless_cleanup_stops_only_recorded_chrome_without_axi(self) -> None:
        identity_reader = verify_lib.proc_identity
        observations = []
        def startup_identity(pid):
            identity = identity_reader(pid)
            observations.append(identity)
            if len(observations) == 1:
                return {**(identity or {}), "command": "", "marker": ""}
            return identity
        with patch.object(verify_lib, "proc_identity", side_effect=startup_identity):
            with recorded_child(["sleep", "30"], start_new_session=True) as (chrome, identity):
                self.assertGreaterEqual(len(observations), 2)
                (self.run_root / "browser.json").write_text(json.dumps({
                    "mode": "chrome-headless", "session": "pm-i545", "started_by": "verify-ezerbot",
                    "chrome_pid": chrome.pid, "identity": identity, "pgid": chrome.pid,
                }))
                cleanup = self.cleanup()
                chrome.wait(timeout=5)
                self.assertEqual(cleanup["killed"]["chrome"], "arrete")
                self.assert_no_stop()


if __name__ == "__main__":
    unittest.main()
