import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location("rust_ci", Path(__file__).with_name("run-rust-ci.py"))
CI = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CI)
HEAD = "a" * 40
TREE = "b" * 40
EVENT = "c" * 40
WORKFLOW = "d" * 40


def summary(passed=1, failed=0, ignored=0, filtered=0):
    status = "FAILED" if failed else "ok"
    return "test result: {}. {} passed; {} failed; {} ignored; 0 measured; {} filtered out; finished in 0.01s\n".format(status, passed, failed, ignored, filtered)


def test_output(passed=1, failed=0, ignored=0, filtered=0):
    return ("Running unittests src/lib.rs (/isolated/target/debug/deps/sample_crate-1234567)\n"
            "running {} tests\n".format(passed + failed + ignored) +
            ("test tests::fails_case ... FAILED\n" if failed else "") +
            summary(passed, failed, ignored, filtered)).encode()


MULTI = (test_output(2) +
         b"Running tests/integration.rs (/isolated/target/debug/deps/integration-abcdef1)\nrunning 1 test\n" +
         summary().encode() + b"Doc-tests sample_crate\nrunning 2 tests\n" + summary(2).encode())


class MockedRustSubprocess:
    def __init__(self, root):
        self.root = root
        self.commands = []
        self.phase_codes = {}
        self.phase_reasons = {}
        self.test_raw = MULTI
        self.head = HEAD
        self.tree = TREE
        self.dirty = False
        self.after_tests = None

    def __call__(self, command, root, env, seconds, output_limit=CI.MAX_OUTPUT, disk_root=None):
        command = tuple(command)
        self.commands.append(command)
        code, raw, reasons = 0, b"", []
        if command == ("git", "rev-parse", "HEAD"):
            raw = self.head.encode()
        elif command == ("git", "rev-parse", "HEAD^{tree}"):
            raw = self.tree.encode()
        elif command[:3] == ("git", "diff", "--quiet"):
            code = int(self.dirty)
        elif command[:3] == ("git", "ls-files", "--error-unmatch"):
            raw = b"Cargo.lock\nrust-toolchain.toml\n"
        elif command == ("git", "ls-files", "-z"):
            raw = b"Cargo.lock\0rust-toolchain.toml\0Cargo.toml\0crates/sample/Cargo.toml\0crates/sample/src/lib.rs\0crates/sample/tests/integration.rs\0"
        elif command == ("uname", "-m"):
            raw = b"arm64"
        elif command == ("sw_vers", "-productVersion"):
            raw = b"15.7.1"
        elif command == ("rustc", "-Vv"):
            raw = ("rustc 1.99.1\nrelease: 1.99.1\ncommit-hash: {}\nhost: aarch64-apple-darwin\n".format("e" * 40)).encode()
        elif command == ("cargo", "-V"):
            raw = b"cargo 1.99.1 (123456789 2026-10-01)"
        elif command == ("rustup", "component", "list", "--installed"):
            raw = b"rustfmt-aarch64-apple-darwin\nclippy-aarch64-apple-darwin"
        else:
            phase = next((name for name, _, argv, _ in CI.PHASES if command == argv), None)
            if phase is None:
                raise AssertionError("Unexpected subprocess boundary")
            code = self.phase_codes.get(phase, 0)
            reasons = self.phase_reasons.get(phase, [])
            raw = self.test_raw if phase == "test_workspace" else b""
            if phase == "test_workspace" and self.after_tests:
                self.after_tests()
        state = "not_run" if "launch_error" in reasons else "executed"
        return {"state": state, "exit_code": code if state == "executed" else None,
                "duration_ms": 7 if state == "executed" else None, "raw": raw, "reasons": list(reasons)}

    def phases(self):
        return [command for command in self.commands if command in [phase[2] for phase in CI.PHASES]]


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        base = os.environ.get("RUST_CI_TEST_ROOT")
        if not base or not Path(base).is_dir():
            raise RuntimeError("Set RUST_CI_TEST_ROOT to an existing test evidence directory")
        self.temporary = tempfile.TemporaryDirectory(prefix="rust-ci-tests-", dir=base)
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "checkout"
        (self.root / "crates/sample/src").mkdir(parents=True)
        (self.root / "crates/sample/tests").mkdir()
        (self.root / "Cargo.lock").write_text("version = 4\n")
        (self.root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/sample"]\n')
        (self.root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.99.1"\ncomponents = ["rustfmt", "clippy"]\n')
        (self.root / "crates/sample/Cargo.toml").write_text('[package]\nname = "sample-crate"\n')
        (self.root / "crates/sample/src/lib.rs").write_text("fn passes_case() {}\nfn fails_case() {}\n")
        (self.root / "crates/sample/tests/integration.rs").write_text("fn integration_case() {}\n")
        self.event = self.base / "event.json"
        self.event.write_text(json.dumps({"repository": {"full_name": "owner/project"}, "number": 42,
                                         "pull_request": {"head": {"sha": HEAD, "repo": {"full_name": "fork/project"}}}}))
        self.runner_temp = self.base / "runner"
        self.runner_temp.mkdir()
        self.env = {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted", "RUNNER_OS": "macOS",
                    "RUNNER_ARCH": "ARM64", "GITHUB_JOB": "rust-macos", "GITHUB_WORKSPACE": str(self.root),
                    "GITHUB_EVENT_NAME": "pull_request", "GITHUB_EVENT_PATH": str(self.event),
                    "GITHUB_REPOSITORY": "owner/project", "GITHUB_REF": "refs/pull/42/merge",
                    "GITHUB_SHA": EVENT, "GITHUB_WORKFLOW_SHA": WORKFLOW,
                    "GITHUB_WORKFLOW_REF": "owner/project/.github/workflows/rust-macos.yml@refs/pull/42/merge",
                    "GITHUB_RUN_ID": "1234", "GITHUB_RUN_ATTEMPT": "1", "RUNNER_TEMP": str(self.runner_temp),
                    "GITHUB_OUTPUT": str(self.base / "outputs"), "ImageOS": "macos15", "ImageVersion": "20261001.1"}
        self.output = self.base / "receipt/result.json"
        self.backend = MockedRustSubprocess(self.root)
        self.console = io.StringIO()
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(contextlib.redirect_stdout(self.console))
        self.stack.enter_context(contextlib.redirect_stderr(self.console))
        self.stack.enter_context(mock.patch.object(CI.platform, "system", return_value="Darwin"))
        self.stack.enter_context(mock.patch.object(CI.platform, "machine", return_value="arm64"))
        self.mock_execute = self.stack.enter_context(mock.patch.object(CI, "execute", side_effect=self.backend))
        self.stack.enter_context(mock.patch.object(CI.shutil, "disk_usage", return_value=type("Disk", (), {"free": 100 * 1024**3})()))

    def prepare(self):
        return CI.prepare(self.output, self.env, self.root)

    def run_receipt(self, outcome="success", hit="false"):
        self.assertEqual(self.prepare(), 0)
        result = CI.run(self.output, self.env, outcome, hit, self.root)
        return result, json.loads(self.output.read_text())

    def test_complete_workspace_binary_and_doctest_receipt(self):
        result, report = self.run_receipt()
        self.assertEqual(result, 0)
        self.assertEqual(report["status"], "vert")
        self.assertEqual(report["provenance"]["source_sha"], HEAD)
        self.assertEqual(report["provenance"]["checkout_sha"], HEAD)
        self.assertEqual(report["provenance"]["event_sha"], EVENT)
        self.assertEqual(report["provenance"]["workflow_sha"], WORKFLOW)
        self.assertEqual(report["provenance"]["source_repository"], "fork/project")
        self.assertEqual(report["provenance"]["architecture"]["machine"], "arm64")
        test = report["phases"][3]
        self.assertEqual(test["counts"], {"passed": 5, "failed": 0, "ignored": 0, "filtered": 0})
        self.assertEqual([row["kind"] for row in test["summaries"]], ["binary", "binary", "doctest"])
        self.assertTrue(test["summary_complete"])
        self.assertEqual(self.backend.phases(), [
            ("cargo", "fmt", "--all", "--check"),
            ("cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"),
            ("cargo", "clippy", "--workspace", "--all-targets", "--release", "--locked", "--", "-D", "warnings"),
            ("cargo", "test", "--workspace", "--locked", "--no-fail-fast"),
        ])
        self.assertTrue(all(phase["duration_ms"] == 7 for phase in report["phases"]))
        self.assertTrue(all(phase["counts"] is None for phase in report["phases"][:3]))
        self.assertIsNone(report["cache"]["saved"])
        self.assertEqual(set(phase["id"] for phase in report["phases"]), {"fmt", "clippy_debug", "clippy_release", "test_workspace"})

    def test_earlier_failure_keeps_all_four_phases_and_real_codes(self):
        self.backend.phase_codes = {"fmt": 1, "clippy_debug": 101, "clippy_release": 2}
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(len(self.backend.phases()), 4)
        self.assertEqual([phase["exit_code"] for phase in report["phases"]], [1, 101, 2, 0])
        self.assertEqual(report["phases"][3]["counts"]["passed"], 5)

    def test_compilation_error_has_null_test_counts(self):
        self.backend.phase_codes["test_workspace"] = 101
        self.backend.test_raw = b"compilation diagnostic omitted\n"
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertIsNone(report["phases"][3]["counts"])
        self.assertFalse(report["phases"][3]["summary_complete"])

    def test_zero_tests_is_red(self):
        self.backend.test_raw = test_output(0)
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertIn("empty_tests", report["phases"][3]["reasons"])

    def test_missing_one_binary_summary_is_red_with_observed_totals(self):
        self.backend.test_raw = test_output(2) + b"Running tests/integration.rs (/isolated/target/debug/deps/integration-abcdef1)\nrunning 1 test\n"
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"][3]["counts"]["passed"], 2)
        self.assertFalse(report["phases"][3]["summary_complete"])

    def test_ignored_and_filtered_are_blocking(self):
        for kwargs, reason in (({"ignored": 1}, "ignored_tests"), ({"filtered": 1}, "filtered_tests")):
            with self.subTest(reason=reason):
                self.backend.test_raw = test_output(**kwargs)
                self.assertEqual(self.prepare(), 0)
                result = CI.run(self.output, self.env, "success", "false", self.root)
                report = json.loads(self.output.read_text())
                self.assertEqual(result, 1)
                self.assertIn(reason, report["phases"][3]["reasons"])
                self.env["GITHUB_RUN_ATTEMPT"] = str(int(self.env["GITHUB_RUN_ATTEMPT"]) + 1)

    def test_failure_identifiers_come_from_tracked_source(self):
        self.backend.phase_codes["test_workspace"] = 101
        self.backend.test_raw = test_output(0, failed=1)
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"][3]["failures"], ["crates/sample/src/lib.rs::fails_case"])
        self.assertEqual(report["phases"][3]["unresolved_failures"], 0)

    def test_diagnostics_and_untrusted_names_never_reach_public_outputs(self):
        marker = "UNPUBLISHABLE_DIAGNOSTIC_SENTINEL"
        self.backend.phase_codes["test_workspace"] = 101
        self.backend.test_raw = ("diagnostic " + marker + "\n").encode() + test_output(0, failed=1).replace(b"tests::fails_case", marker.encode())
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertTrue(marker not in self.output.read_text())
        self.assertTrue(marker not in self.console.getvalue())
        self.assertEqual(report["phases"][3]["failures"], [])
        self.assertEqual(report["phases"][3]["unresolved_failures"], 1)

    def test_doctest_failure_uses_a_tracked_location(self):
        self.backend.phase_codes["test_workspace"] = 101
        self.backend.test_raw = (b"Doc-tests sample_crate\nrunning 1 test\n"
                                 b"test crates/sample/src/lib.rs - fails_case (line 2) ... FAILED\n" + summary(0, 1).encode())
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"][3]["failures"], ["crates/sample/src/lib.rs:2"])

    def test_panic_coordinates_are_tracked_without_publishing_diagnostics(self):
        self.backend.phase_codes["test_workspace"] = 101
        self.backend.test_raw = (test_output(0, failed=1) +
                                b"thread 'UNPUBLISHABLE_THREAD' panicked at crates/sample/src/lib.rs:2:5:\n"
                                b"UNPUBLISHABLE_DIAGNOSTIC\n"
                                b"thread 'UNPUBLISHABLE_THREAD' panicked at untracked/private.rs:1:1:\n")
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"][3]["failures"],
                         ["crates/sample/src/lib.rs:2", "crates/sample/src/lib.rs::fails_case"])
        self.assertNotIn("UNPUBLISHABLE", self.output.read_text() + self.console.getvalue())
        self.assertNotIn("untracked/private", self.output.read_text())

    def test_failure_line_cannot_be_overruled_by_a_green_summary(self):
        self.backend.test_raw = test_output() + b"test tests::fails_case ... FAILED\n"
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"][3]["exit_code"], 0)
        self.assertIn("failed_tests", report["phases"][3]["reasons"])
        self.assertFalse(report["phases"][3]["summary_complete"])

    def test_host_guard_refuses_cargo_before_any_subprocess(self):
        for key, value in (("GITHUB_ACTIONS", "false"), ("RUNNER_ENVIRONMENT", "self-hosted"), ("RUNNER_OS", "Linux")):
            with self.subTest(key=key):
                self.backend.commands.clear()
                env = dict(self.env, **{key: value})
                self.assertEqual(CI.prepare(self.output, env, self.root), 1)
                report = json.loads(self.output.read_text())
                self.assertIsNone(report["provenance"])
                self.assertEqual(self.backend.commands, [])
                self.assertTrue(all(phase["state"] == "not_run" and phase["exit_code"] is None and phase["counts"] is None for phase in report["phases"]))

    def test_wrong_head_and_invalid_metadata_have_no_provenance(self):
        for key, value in (("GITHUB_SHA", "invalid"), ("GITHUB_WORKFLOW_SHA", "invalid"), ("GITHUB_RUN_ID", "0"), ("RUNNER_ARCH", "X64")):
            with self.subTest(key=key):
                self.env["GITHUB_RUN_ATTEMPT"] = str(int(self.env["GITHUB_RUN_ATTEMPT"]) + 1)
                self.assertEqual(CI.prepare(self.output, dict(self.env, **{key: value}), self.root), 1)
                self.assertIsNone(json.loads(self.output.read_text())["provenance"])
        self.backend.head = "f" * 40
        self.env["GITHUB_RUN_ATTEMPT"] = "99"
        self.assertEqual(self.prepare(), 1)
        self.assertIsNone(json.loads(self.output.read_text())["provenance"])
        self.assertEqual(self.backend.phases(), [])

    def test_push_is_main_only(self):
        self.env.update({"GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/main", "GITHUB_SHA": HEAD,
                         "GITHUB_WORKFLOW_REF": "owner/project/.github/workflows/rust-macos.yml@refs/heads/main"})
        self.event.write_text(json.dumps({"repository": {"full_name": "owner/project"}, "ref": "refs/heads/main", "after": HEAD}))
        result, report = self.run_receipt()
        self.assertEqual(result, 0)
        self.assertEqual(report["provenance"]["event"], "push")
        self.env["GITHUB_REF"] = "refs/heads/feature"
        self.assertEqual(self.prepare(), 1)
        self.assertIsNone(json.loads(self.output.read_text())["provenance"])

    def test_cache_is_fresh_exact_and_attempt_independent(self):
        self.assertEqual(self.prepare(), 0)
        first = json.loads(self.output.read_text())["cache"]["key"]
        outputs = (self.base / "outputs").read_text()
        self.assertTrue("cargo-home" in outputs)
        self.assertEqual(self.prepare(), 1)
        self.env["GITHUB_RUN_ATTEMPT"] = "2"
        self.assertEqual(self.prepare(), 0)
        self.assertEqual(json.loads(self.output.read_text())["cache"]["key"], first)
        self.env["GITHUB_RUN_ID"] = "1235"
        self.assertEqual(self.prepare(), 0)
        self.assertNotEqual(json.loads(self.output.read_text())["cache"]["key"], first)
        self.assertTrue(HEAD in first)

    def test_cache_failure_still_runs_four_checks_and_stays_red(self):
        result, report = self.run_receipt("failure", "")
        self.assertEqual(result, 1)
        self.assertEqual(len(self.backend.phases()), 4)
        self.assertIn("cache_restore_failed", report["reasons"])
        self.assertIsNone(report["cache"]["hit"])
        self.assertIsNone(report["cache"]["saved"])

    def test_cache_hit_requires_exact_matched_key_and_discards_prefix_archives(self):
        self.assertEqual(self.prepare(), 0)
        key = json.loads(self.output.read_text())["cache"]["key"]
        home = self.runner_temp / "rust-macos-1234-1/cargo-home"
        archives = home / "registry/cache/public-registry"
        archives.mkdir(parents=True)
        (archives / "sample.crate").write_bytes(b"archive fixture")
        self.assertEqual(CI.run(self.output, self.env, "success", "false", self.root,
                                cache_matched_key=key + "-other"), 1)
        report = json.loads(self.output.read_text())
        self.assertIn("cache_key_mismatch", report["reasons"])
        self.assertIsNone(report["cache"]["hit"])
        self.assertEqual(report["cache"]["bytes_before"], 0)
        self.assertFalse(archives.exists())
        self.assertEqual(len(self.backend.phases()), 4)
        self.env["GITHUB_RUN_ATTEMPT"] = "2"
        self.assertEqual(self.prepare(), 0)
        self.assertEqual(CI.run(self.output, self.env, "success", "true", self.root,
                                cache_matched_key=key), 0)
        report = json.loads(self.output.read_text())
        self.assertTrue(report["cache"]["hit"])
        self.assertIsNone(report["cache"]["saved"])

    def test_head_tree_and_lock_mutations_are_blocking(self):
        mutations = (
            lambda: setattr(self.backend, "head", "f" * 40),
            lambda: setattr(self.backend, "tree", "f" * 40),
            lambda: (self.root / "Cargo.lock").write_text("version = 3\n"),
            lambda: setattr(self.backend, "dirty", True),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                self.backend.head, self.backend.tree, self.backend.dirty = HEAD, TREE, False
                (self.root / "Cargo.lock").write_text("version = 4\n")
                self.env["GITHUB_RUN_ATTEMPT"] = str(index + 1)
                self.backend.after_tests = mutate
                result, report = self.run_receipt()
                self.assertEqual(result, 1)
                self.assertIn("checkout_changed", report["reasons"])

    def test_timeout_continues_and_interruption_leaves_remaining_not_run(self):
        self.backend.phase_codes["fmt"] = -signal.SIGTERM
        self.backend.phase_reasons["fmt"] = ["timeout"]
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(len(self.backend.phases()), 4)
        self.assertEqual(report["phases"][0]["exit_code"], -signal.SIGTERM)
        self.backend.commands.clear()
        self.env["GITHUB_RUN_ATTEMPT"] = "2"
        self.backend.phase_reasons["fmt"] = ["interrupted"]
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(len(self.backend.phases()), 1)
        self.assertTrue(all(phase["state"] == "not_run" and phase["duration_ms"] is None for phase in report["phases"][1:]))

    def test_budget_and_launch_errors_never_claim_execution(self):
        self.assertEqual(self.prepare(), 0)
        with mock.patch.object(CI, "TOTAL_SECONDS", -1):
            self.assertEqual(CI.run(self.output, self.env, "success", "false", self.root), 1)
        report = json.loads(self.output.read_text())
        self.assertEqual(self.backend.phases(), [])
        self.assertTrue(all(phase["state"] == "not_run" and phase["exit_code"] is None for phase in report["phases"]))
        self.env["GITHUB_RUN_ATTEMPT"] = "2"
        self.backend.phase_reasons["fmt"] = ["launch_error"]
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(report["phases"][0]["state"], "not_run")
        self.assertIsNone(report["phases"][0]["duration_ms"])
        self.assertEqual(len(self.backend.phases()), 4)

    def test_inventory_failure_does_not_suppress_workspace_commands(self):
        with mock.patch.object(CI, "source_inventory", side_effect=CI.InvalidReceipt()):
            result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertEqual(len(self.backend.phases()), 4)
        self.assertFalse(report["inventory_complete"])

    def test_output_and_report_limits_stay_red_and_bounded(self):
        self.backend.phase_reasons["test_workspace"] = ["output_limit"]
        result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertFalse(report["inventory_complete"])
        self.env["GITHUB_RUN_ATTEMPT"] = "2"
        self.backend.phase_reasons.clear()
        self.backend.test_raw = MULTI * 100
        with mock.patch.object(CI, "MAX_REPORT", 12000):
            result, report = self.run_receipt()
        self.assertEqual(result, 1)
        self.assertIn("report_limit", report["reasons"])
        self.assertFalse(report["inventory_complete"])
        self.assertFalse(report["phases"][3]["summary_complete"])
        self.assertLessEqual(self.output.stat().st_size, 12000)

    def test_unbound_summary_and_count_mismatch_are_red(self):
        inventory = CI.source_inventory(self.root, self.env)
        for raw in (summary().encode(), test_output(1).replace(b"running 1 tests", b"running 2 tests")):
            parsed = CI.parse_test_stream(raw, inventory)
            self.assertFalse(parsed["summary_complete"])
            self.assertIn("missing_summary", parsed["reasons"])

    def test_subprocess_bounds_use_only_an_explicit_python_backend(self):
        backend = self.base / "fake_rust_backend.py"
        with mock.patch.object(CI, "execute", self.real_execute):
            backend.write_text("import sys\nsys.exit(7)\n")
            result = CI.execute([sys.executable, str(backend)], self.root, dict(os.environ), 2)
            self.assertEqual(result["exit_code"], 7)
            backend.write_text("import time\ntime.sleep(30)\n")
            result = CI.execute([sys.executable, str(backend)], self.root, dict(os.environ), 0.05)
            self.assertIn("timeout", result["reasons"])
            self.assertIsNotNone(result["exit_code"])
            backend.write_text("import sys\nsys.stdout.write('x' * 200000)\n")
            result = CI.execute([sys.executable, str(backend)], self.root, dict(os.environ), 2, 1024)
            self.assertIn("output_limit", result["reasons"])
            self.assertLessEqual(len(result["raw"]), 1024)
            result = CI.execute([str(self.base / "missing_python_backend")], self.root, dict(os.environ), 2)
            self.assertEqual(result["state"], "not_run")
            self.assertIsNone(result["exit_code"])

    real_execute = staticmethod(CI.execute)


if __name__ == "__main__":
    unittest.main()
