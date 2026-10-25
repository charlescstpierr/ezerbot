import contextlib
from dataclasses import replace
import io
import json
import os
from pathlib import Path
import secrets
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from bundle_inspection import BundleInspection, BundleInspectionError, _ProcessIdentity


class BundleInspectionChecks(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="bundle-inspection-checks-")
        self.addCleanup(self.directory.cleanup)
        self.evidence = Path(self.directory.name).resolve()

    def bundle(self, *, ready=None, state=None, before_ready="", before_state="",
               after_eof="", cleanup_engine=True, bundled_engine=False, engine_code=None,
               engine_own_group=False):
        app = Path(tempfile.mkdtemp(prefix="fixture-", dir=self.evidence)) / "Fixture.app"
        executable = app / "Contents/MacOS/Ezerbot"
        executable.parent.mkdir(parents=True)
        if bundled_engine:
            engine_executable = str(executable.with_name("grokbot-wrapper"))
            Path(engine_executable).symlink_to("/bin/sleep")
            engine_command = [engine_executable, "120"]
        else:
            engine_command = [sys.executable, "-c", engine_code or "import time; print('ready', flush=True); time.sleep(120)"]
        script = "\n".join([
            f"#!{sys.executable}",
            "import json, os, pathlib, signal, subprocess, sys, time",
            "signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))",
            f"engine = subprocess.Popen({engine_command!r}, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, preexec_fn=os.setpgrp if {engine_own_group!r} else None)",
            "assert engine.stdout.readline() == b'ready\\n'" if not bundled_engine else "",
            "engine.stdout.close()" if not bundled_engine else "",
            "runtime = {'host_pid': os.getpid(), 'engine_pid': engine.pid, 'url': 'http://127.0.0.1:12345'}",
            "ready = {'ready': True, **runtime}",
            "state = {**runtime, 'status': 200, 'mode': 'simulated', 'journals_safe': True}",
            "def emit(value):",
            "    sys.stdout.buffer.write(json.dumps(value).encode() + b'\\n')",
            "    sys.stdout.buffer.flush()",
            "try:",
            "    " + before_ready.replace("\n", "\n    "),
            "    " + (ready or "emit(ready)").replace("\n", "\n    "),
            "    for command in sys.stdin.buffer:",
            "        if command != b'state\\n':",
            "            emit({'error': 'invalid_command'})",
            "            continue",
            "        " + before_state.replace("\n", "\n        "),
            "        " + (state or "emit(state)").replace("\n", "\n        "),
            "    " + after_eof.replace("\n", "\n    "),
            "finally:",
            "    engine.stdin.close()",
            "    engine.terminate(); engine.wait(timeout=2)" if cleanup_engine else "    pass",
            "",
        ])
        executable.write_text(script)
        executable.chmod(0o700)
        return app

    def inspect(self, app):
        return BundleInspection(app, self.evidence, timeout=0.5, startup_timeout=10, shutdown_timeout=3)

    def assert_safe_failure(self, operation, sensitive=""):
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            with self.assertRaises(BundleInspectionError) as caught:
                operation()
            output.write(str(caught.exception))
        if sensitive:
            self.assertTrue(sensitive not in output.getvalue(), "Invalid channel data was disclosed")

    def process_snapshot(self, pid):
        result = subprocess.run(
            ["/bin/ps", "-ww", "-p", str(pid), "-o", "ppid=", "-o", "stat=",
             "-o", "lstart=", "-o", "command="],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=2, check=False,
        )
        if result.returncode == 1 and not result.stdout:
            return None
        self.assertEqual(result.returncode, 0, "Process observation failed")
        fields = result.stdout.strip().split(None, 2)
        self.assertEqual(len(fields), 3, "Process observation failed")
        if fields[1].startswith(b"Z"):
            return None
        return int(fields[0]), fields[2]

    def owned_engine(self, inspected):
        pid = inspected.ready["engine_pid"]
        return self.owned_child(inspected, pid)

    def owned_child(self, inspected, pid):
        observed = self.process_snapshot(pid)
        self.assertTrue(observed is not None and observed[0] == inspected.pid,
                        "Fixture engine does not belong to fixture host")
        self.addCleanup(self.stop_fixture_engine, pid, observed[1])
        host = self.process_snapshot(inspected.pid)
        self.assertTrue(host is not None and host[0] == os.getpid(),
                        "Fixture host is not a child of the test process")
        self.addCleanup(self.stop_fixture_host, inspected.process, host[1])
        return pid

    def stop_fixture_host(self, process, identity):
        if process.poll() is None:
            observed = self.process_snapshot(process.pid)
            if observed is not None and observed[0] == os.getpid() and observed[1] == identity:
                process.kill()
                process.wait(timeout=2)

    def stop_fixture_engine(self, pid, identity):
        current = self.process_snapshot(pid)
        if current is not None and current[1] == identity:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            deadline = time.monotonic() + 2
            while self.process_snapshot(pid) is not None and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(self.process_snapshot(pid) is None, "Fixture engine cleanup failed")

    def assert_closed(self, inspected, engine):
        self.assertIsNotNone(inspected.process.returncode, "Fixture host survived cleanup")
        self.assertTrue(self.process_snapshot(engine) is None, "Fixture engine survived cleanup")
        self.assertTrue(inspected.process.stdin.closed)
        self.assertTrue(inspected.process.stdout.closed)
        self.assertFalse(inspected.home.exists())

    def test_bundle_process_and_home_are_owned_until_eof(self):
        app = self.bundle(before_ready="assert sys.argv[1:] == ['--inspect-private']; assert os.environ['HOME'] == os.environ['CFFIXED_USER_HOME'] == os.environ['EZERBOT_TEST_HOME']")
        with self.inspect(app) as inspected:
            home = inspected.home
            process = inspected.process
            engine = self.owned_engine(inspected)
            self.assertEqual(home.stat().st_mode & 0o777, 0o700)
            self.assertEqual((home / "shell.err").stat().st_mode & 0o777, 0o600)
            for stream in (process.stdin, process.stdout):
                self.assertTrue(stat.S_ISFIFO(os.fstat(stream.fileno()).st_mode))
                self.assertFalse(os.get_inheritable(stream.fileno()))
            self.assertEqual(inspected.ready["host_pid"], process.pid)
            ready = inspected.ready
            ready["host_pid"] = 1
            state = inspected.state(inspected.ready["engine_pid"], inspected.ready["url"])
            self.assertEqual(state, {key: value for key, value in inspected.ready.items() if key != "ready"}
                             | {"status": 200, "mode": "simulated", "journals_safe": True})
        self.assertEqual(process.returncode, 0)
        self.assertTrue(process.stdin.closed)
        self.assertTrue(process.stdout.closed)
        self.assertFalse(home.exists())
        self.assert_closed(inspected, engine)
        inspected.close()

    def test_host_ignoring_eof_and_term_cannot_leave_engine_alive(self):
        inspected = self.inspect(self.bundle(
            before_ready="signal.signal(signal.SIGTERM, signal.SIG_IGN)",
            after_eof="while True: time.sleep(1)",
        )).__enter__()
        engine = self.owned_engine(inspected)
        inspected.shutdown_timeout = 0.15
        started = time.monotonic()
        with self.assertRaisesRegex(BundleInspectionError, "^Bundle inspection did not close normally$"):
            inspected.close()
        self.assertLess(time.monotonic() - started, 2)
        self.assert_closed(inspected, engine)
        with patch("bundle_inspection.os.kill", side_effect=AssertionError("Cleanup was repeated")):
            inspected.close()
        self.assert_safe_failure(lambda: inspected.state(engine, inspected.ready["url"]))

    def test_stopped_host_cannot_leave_engine_alive(self):
        inspected = self.inspect(self.bundle()).__enter__()
        engine = self.owned_engine(inspected)
        inspected.shutdown_timeout = 0.15
        os.kill(inspected.pid, signal.SIGSTOP)
        with self.assertRaisesRegex(BundleInspectionError, "^Bundle inspection did not close normally$"):
            inspected.close()
        self.assert_closed(inspected, engine)
        inspected.close()

    def test_dead_host_cannot_leave_reparented_engine_alive(self):
        inspected = self.inspect(self.bundle()).__enter__()
        engine = self.owned_engine(inspected)
        inspected.process.kill()
        inspected.process.wait(timeout=2)
        observed = self.process_snapshot(engine)
        self.assertTrue(observed is not None and observed[0] != inspected.pid,
                        "Fixture engine was not reparented")
        with self.assertRaisesRegex(BundleInspectionError, "^Bundle inspection did not close normally$"):
            inspected.close()
        self.assert_closed(inspected, engine)
        inspected.close()

    def test_stopped_reparented_engine_requires_kill_escalation(self):
        inspected = self.inspect(self.bundle()).__enter__()
        engine = self.owned_engine(inspected)
        inspected.shutdown_timeout = 0.15
        os.kill(engine, signal.SIGSTOP)
        inspected.process.kill()
        inspected.process.wait(timeout=2)
        with self.assertRaisesRegex(BundleInspectionError, "^Bundle inspection did not close normally$"):
            inspected.close()
        self.assert_closed(inspected, engine)

    def test_normal_host_exit_requires_engine_exit_before_success(self):
        inspected = self.inspect(self.bundle(cleanup_engine=False)).__enter__()
        engine = self.owned_engine(inspected)
        with self.assertRaisesRegex(BundleInspectionError, "^Bundle inspection did not close normally$"):
            inspected.close()
        self.assertEqual(inspected.process.returncode, 0)
        self.assert_closed(inspected, engine)

    def test_ready_cannot_claim_a_foreign_process_as_engine(self):
        sensitive = secrets.token_urlsafe(24)
        foreign = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)", sensitive],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(foreign.wait, timeout=2)
        self.addCleanup(foreign.terminate)
        inspected = self.inspect(self.bundle(ready=f"emit(ready | {{'engine_pid': {foreign.pid}}})"))
        self.assert_safe_failure(inspected.__enter__, sensitive)
        self.assertIsNone(foreign.poll(), "Foreign process was signalled")
        self.assertFalse(inspected.home.exists())
        inspected.close()
        self.assertIsNone(foreign.poll(), "Foreign process was signalled during repeated cleanup")

    def test_cleanup_continues_after_a_signal_error(self):
        inspected = self.inspect(self.bundle(after_eof="while True: time.sleep(1)")).__enter__()
        engine = self.owned_engine(inspected)
        inspected.shutdown_timeout = 0.1
        original_kill = os.kill

        def fail_term(pid, number):
            if number == signal.SIGTERM:
                raise OSError("untrusted process detail")
            return original_kill(pid, number)

        with patch("bundle_inspection.os.kill", side_effect=fail_term):
            self.assert_safe_failure(inspected.close, "untrusted process detail")
        self.assert_closed(inspected, engine)

    def test_engine_identity_is_rechecked_before_each_signal(self):
        with self.inspect(self.bundle()) as inspected:
            engine = self.owned_engine(inspected)
            identity = _ProcessIdentity.read(engine, session_id=inspected.pid)
            for changed in (replace(identity, started=(identity.started[0] + 1, identity.started[1])),
                            replace(identity, command=identity.command + b" changed"),
                            replace(identity, executable=identity.executable + b" changed"),
                            replace(identity, session_id=identity.session_id + 1)):
                with patch("bundle_inspection.os.kill", wraps=os.kill) as send_signal:
                    changed.signal(signal.SIGKILL)
                    send_signal.assert_not_called()
                self.assertTrue(self.process_snapshot(engine) is not None,
                                "A changed identity was signalled")

    def test_changed_engine_command_is_not_signalled_or_reported_as_absent(self):
        release = self.evidence / "replace-engine-command"
        changed = self.evidence / "engine-command-replaced"
        replacement = f"import pathlib, time; pathlib.Path({str(changed)!r}).touch(); time.sleep(120)"
        engine_code = (
            "import os, pathlib, sys, time\nprint('ready', flush=True)\n"
            f"while not pathlib.Path({str(release)!r}).exists(): time.sleep(0.01)\n"
            f"os.execv(sys.executable, [sys.executable, '-c', {replacement!r}])"
        )
        inspected = self.inspect(self.bundle(engine_code=engine_code, cleanup_engine=False)).__enter__()
        engine = self.owned_engine(inspected)
        original = _ProcessIdentity.read(engine, session_id=inspected.pid)
        release.touch()
        deadline = time.monotonic() + 3
        while not changed.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(changed.exists(), "Fixture engine did not replace its command")
        replacement_identity = _ProcessIdentity.read(engine, session_id=inspected.pid)
        self.assertTrue(replacement_identity is not None
                        and replacement_identity.started == original.started
                        and replacement_identity.command != original.command,
                        "Fixture engine identity did not change")
        self.owned_child(inspected, engine)
        self.assert_safe_failure(inspected.close)
        self.assertEqual(inspected.process.returncode, 0)
        self.assertTrue(self.process_snapshot(engine) is not None,
                        "A changed engine identity was signalled")
        self.assertFalse(inspected.home.exists())
        inspected.close()

    def test_pre_ready_failure_cleans_only_verified_bundled_engine(self):
        for readiness in ("sys.stdout.buffer.write(b'{'); sys.stdout.buffer.flush(); sys.stdin.buffer.read()",
                          "emit({'invalid': True})"):
            with self.subTest(readiness=readiness):
                marker = self.evidence / "engine-created"
                marker.unlink(missing_ok=True)
                app = self.bundle(
                    bundled_engine=True,
                    before_ready=("signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                                  f"pathlib.Path({str(marker)!r}).write_text(str(engine.pid))"),
                    ready=readiness, after_eof="while True: time.sleep(1)",
                )
                inspected = self.inspect(app)
                inspected.startup_timeout = 0.3
                inspected.shutdown_timeout = 0.1
                original_read = inspected._read_message
                children = []

                def observe_readiness(timeout):
                    deadline = time.monotonic() + 5
                    while not marker.exists() and inspected.process.poll() is None and time.monotonic() < deadline:
                        time.sleep(0.01)
                    self.assertTrue(marker.exists(), "Fixture engine did not start")
                    children.append(self.owned_child(inspected, int(marker.read_text())))
                    return original_read(timeout)

                with patch.object(inspected, "_read_message", side_effect=observe_readiness):
                    self.assert_safe_failure(inspected.__enter__)
                self.assertEqual(len(children), 1)
                self.assert_closed(inspected, children[0])
                inspected.close()

    def test_host_exit_before_ready_cleans_engine_in_its_own_process_group(self):
        marker = self.evidence / "orphan-engine-created"
        app = self.bundle(
            bundled_engine=True, cleanup_engine=False, engine_own_group=True,
            before_ready=f"pathlib.Path({str(marker)!r}).write_text(str(engine.pid)); os._exit(23)",
        )
        foreign = subprocess.Popen(
            [str(app / "Contents/MacOS/grokbot-wrapper"), "120"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        self.addCleanup(foreign.wait, timeout=2)
        self.addCleanup(foreign.terminate)
        inspected = self.inspect(app)
        original_message = inspected._read_message
        original_identity = _ProcessIdentity.read
        children = []
        identity_reads = []

        def observe_host_exit(timeout):
            deadline = time.monotonic() + 5
            while (not marker.exists() or inspected.process.poll() is None) and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(marker.exists(), "Fixture engine did not start")
            self.assertEqual(inspected.process.poll(), 23, "Fixture host did not exit before readiness")
            engine = int(marker.read_text())
            observed = self.process_snapshot(engine)
            self.assertIsNotNone(observed, "Fixture engine exited with the host")
            self.addCleanup(self.stop_fixture_engine, engine, observed[1])
            self.assertNotEqual(observed[0], inspected.pid, "Fixture engine was not reparented")
            self.assertEqual(os.getpgid(engine), engine, "Fixture engine has no separate process group")
            self.assertEqual(os.getsid(engine), inspected.pid, "Fixture engine left the private session")
            children.append(engine)
            return original_message(timeout)

        def observe_identity(pid, **kwargs):
            identity_reads.append(pid)
            return original_identity(pid, **kwargs)

        with patch.object(inspected, "_read_message", side_effect=observe_host_exit), \
                patch.object(_ProcessIdentity, "read", side_effect=observe_identity):
            self.assert_safe_failure(inspected.__enter__)
        self.assertEqual(len(children), 1)
        self.assert_closed(inspected, children[0])
        self.assertNotEqual(os.getsid(foreign.pid), inspected.pid)
        self.assertNotIn(foreign.pid, identity_reads, "Foreign process identity was read")
        self.assertIsNone(foreign.poll(), "Foreign process in another session was signalled")
        inspected.close()
        self.assertIsNone(foreign.poll(), "Foreign process was signalled during repeated cleanup")

    def test_inherited_environment_is_restricted_to_the_allowlist(self):
        sentinel = secrets.token_urlsafe(24)
        app = self.bundle(before_ready=(
            "assert 'EZERBOT_INSPECTION_TEST_SENTINEL' not in os.environ; "
            "assert 'DYLD_INSERT_LIBRARIES' not in os.environ; "
            "assert set(os.environ) <= {'PATH', 'LANG', 'HOME', 'CFFIXED_USER_HOME', 'EZERBOT_TEST_HOME', 'TMPDIR', 'LC_CTYPE', '__CF_USER_TEXT_ENCODING'}"))
        with patch.dict(os.environ, {"EZERBOT_INSPECTION_TEST_SENTINEL": sentinel,
                                    "DYLD_INSERT_LIBRARIES": sentinel}):
            with self.inspect(app) as inspected:
                self.assertEqual(inspected.state(inspected.ready["engine_pid"], inspected.ready["url"])["status"], 200)

    def test_ready_host_must_be_the_launched_pid(self):
        for expression in ("ready | {'host_pid': os.getpid() + 1}", "ready | {'host_pid': True}",
                           "ready | {'engine_pid': os.getpid()}", "ready | {'ready': 1}"):
            with self.subTest(expression=expression):
                self.assert_safe_failure(lambda: self.inspect(self.bundle(ready=f"emit({expression})")).__enter__())

    def test_ready_origin_is_canonical_loopback_only(self):
        for url in ("https://127.0.0.1:12345", "http://localhost:12345", "http://127.0.0.1:0",
                    "http://127.0.0.1:65536", "http://127.0.0.1:0123", "http://127.0.0.1:12345/",
                    "http://127.0.0.1:12345?token=invalid", "http://user@127.0.0.1:12345"):
            with self.subTest(url=url):
                self.assert_safe_failure(lambda: self.inspect(self.bundle(ready=f"emit(ready | {{'url': {url!r}}})")).__enter__())

    def test_parse_errors_and_unknown_fields_never_disclose_received_data(self):
        sensitive = secrets.token_urlsafe(24)
        payloads = [sensitive.encode() + b"\n", json.dumps([sensitive]).encode() + b"\n",
                    b'{"ready":true,"ready":false}\n', b'{"ready":NaN}\n', b"\xff\n",
                    json.dumps({"error": sensitive}).encode() + b"\n"]
        for payload in payloads:
            with self.subTest(case=payloads.index(payload)):
                self.assert_safe_failure(lambda: self.inspect(self.bundle(
                    ready=f"sys.stdout.buffer.write({payload!r}); sys.stdout.buffer.flush()")).__enter__(), sensitive)
        self.assert_safe_failure(lambda: self.inspect(self.bundle(
            ready=f"emit(ready | {{'unexpected': {sensitive!r}}})")).__enter__(), sensitive)

    def test_state_rejects_changed_runtime_and_implicit_generation(self):
        for change in ("{'host_pid': os.getpid() + 1}", "{'engine_pid': engine.pid + 1}",
                       "{'url': 'http://127.0.0.1:12346'}", "{'generation': 2}"):
            with self.subTest(change=change):
                with self.inspect(self.bundle(state=f"emit(state | {change})")) as inspected:
                    self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))

    def test_state_requires_integer_200_and_a_known_mode(self):
        for change in ("{'status': 403}", "{'status': True}", "{'status': 200.0}",
                       "{'mode': 'unknown'}", "{'mode': 1}"):
            with self.subTest(change=change):
                with self.inspect(self.bundle(state=f"emit(state | {change})")) as inspected:
                    self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))

    def test_only_fixed_native_error_codes_are_accepted(self):
        for code in ("inspection_failed", "invalid_command", "state_failed", "stale_runtime"):
            with self.subTest(code=code):
                with self.inspect(self.bundle(state=f"emit({{'error': {code!r}}})")) as inspected:
                    with self.assertRaisesRegex(BundleInspectionError, "^Bundle inspection state unavailable$"):
                        inspected.state(inspected.ready["engine_pid"], inspected.ready["url"])

    def test_journal_safety_requires_explicit_boolean_true(self):
        for response in ("state | {'journals_safe': False}", "state | {'journals_safe': 1}",
                         "state | {'journals_safe': 'true'}", "{key: value for key, value in state.items() if key != 'journals_safe'}"):
            with self.subTest(response=response):
                with self.inspect(self.bundle(state=f"emit({response})")) as inspected:
                    self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))

    def test_target_parameters_are_checked_before_any_command(self):
        marker = self.evidence / "command-seen"
        sensitive = secrets.token_urlsafe(24)
        app = self.bundle(before_state=f"pathlib.Path({str(marker)!r}).write_text('seen')")
        with self.inspect(app) as inspected:
            for engine, url in ((inspected.ready["engine_pid"] + 1, inspected.ready["url"]),
                                (True, inspected.ready["url"]),
                                (inspected.ready["engine_pid"], sensitive)):
                self.assert_safe_failure(lambda: inspected.state(engine, url), sensitive)
                self.assertFalse(marker.exists())
            inspected.state(inspected.ready["engine_pid"], inspected.ready["url"])
            self.assertTrue(marker.exists())

    def test_unexpected_output_before_command_is_refused_without_echo(self):
        sensitive = secrets.token_urlsafe(24)
        release = self.evidence / "release-output"
        emitted = self.evidence / "output-emitted"
        app = self.bundle(before_ready="import threading", ready=(
            "emit(ready)\n"
            "def unexpected():\n"
            f"    while not pathlib.Path({str(release)!r}).exists(): time.sleep(0.001)\n"
            f"    sys.stdout.buffer.write({sensitive.encode()!r}); sys.stdout.buffer.flush()\n"
            f"    pathlib.Path({str(emitted)!r}).write_text('done')\n"
            "threading.Thread(target=unexpected, daemon=True).start()"))
        with self.inspect(app) as inspected:
            release.touch()
            deadline = time.monotonic() + 2
            while not emitted.exists() and time.monotonic() < deadline:
                time.sleep(0.001)
            self.assertTrue(emitted.exists())
            self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]), sensitive)

    def test_multiple_lines_or_unbounded_response_are_rejected(self):
        for response in (b"{}\n{}\n", b"x" * 4097):
            with self.subTest(case=len(response)):
                with self.inspect(self.bundle(state=f"sys.stdout.buffer.write({response!r}); sys.stdout.buffer.flush()")) as inspected:
                    self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))

    def test_partial_line_obeys_deadline_and_eof_fails(self):
        for behavior in ("sys.stdout.buffer.write(b'{'); sys.stdout.buffer.flush(); sys.stdin.buffer.read()",
                         "sys.stdout.close()", "sys.stdin.buffer.read()"):
            with self.subTest(behavior=behavior):
                inspected = self.inspect(self.bundle(state=behavior)).__enter__()
                try:
                    started = time.monotonic()
                    self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))
                    self.assertLess(time.monotonic() - started, 2)
                finally:
                    try:
                        inspected.close()
                    except BundleInspectionError:
                        self.assertIsNotNone(inspected.process.returncode)

    def test_readiness_partial_line_obeys_startup_deadline(self):
        inspected = self.inspect(self.bundle(ready="sys.stdout.buffer.write(b'{'); sys.stdout.buffer.flush(); sys.stdin.buffer.read()"))
        inspected.startup_timeout = 0.1
        started = time.monotonic()
        self.assert_safe_failure(inspected.__enter__)
        self.assertLess(time.monotonic() - started, 2)
        self.assertFalse(inspected.home.exists())

    def test_timed_out_state_cannot_be_reused_by_another_command(self):
        marker = self.evidence / "commands-observed"
        app = self.bundle(
            before_state=f"with pathlib.Path({str(marker)!r}).open('a') as stream: stream.write('seen\\n')",
            state="time.sleep(0.2); emit(state)",
        )
        with self.inspect(app) as inspected:
            inspected.timeout = 0.05
            self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))
            inspected.timeout = 0.5
            self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))
        self.assertEqual(marker.read_text(), "seen\n", "A failed channel accepted another command")

    def test_invalid_state_permanently_invalidates_the_channel(self):
        for response in ("emit(state | {'journals_safe': 1})", "emit({'error': 'state_failed'})",
                         "sys.stdout.buffer.write(b'{}\\n'); sys.stdout.buffer.flush()"):
            with self.subTest(response=response):
                marker = self.evidence / "commands-observed"
                marker.unlink(missing_ok=True)
                app = self.bundle(
                    before_state=f"with pathlib.Path({str(marker)!r}).open('a') as stream: stream.write('seen\\n')",
                    state=response,
                )
                with self.inspect(app) as inspected:
                    self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))
                    self.assert_safe_failure(lambda: inspected.state(inspected.ready["engine_pid"], inspected.ready["url"]))
                self.assertEqual(marker.read_text(), "seen\n", "A failed channel accepted another command")

    def test_healthy_separate_client_cannot_validate_an_invalid_target(self):
        valid = self.bundle()
        invalid = self.bundle(state="emit({'error': 'stale_runtime'})")
        with self.inspect(valid) as healthy, self.inspect(invalid) as target:
            self.assertEqual(healthy.state(healthy.ready["engine_pid"], healthy.ready["url"])["status"], 200)
            self.assert_safe_failure(lambda: target.state(target.ready["engine_pid"], target.ready["url"]))
            self.assert_safe_failure(lambda: target.state(healthy.ready["engine_pid"], healthy.ready["url"]))
            target.close()
            self.assertEqual(healthy.state(healthy.ready["engine_pid"], healthy.ready["url"])["status"], 200)


if __name__ == "__main__":
    unittest.main()
