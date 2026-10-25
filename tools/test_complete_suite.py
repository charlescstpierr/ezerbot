import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


TOOLS = Path(__file__).resolve().parent
LOGS = Path(os.environ.get("SUITE_TEST_TMPDIR", TOOLS.parent / "target/suite-verification"))
FAKE = '''#!/usr/bin/env python3
import os, pathlib, sys
phase = pathlib.Path(sys.argv[0]).name
case = os.environ.get("SUITE_CASE", "green")
with open("commands.jsonl", "a") as f:
    import json
    f.write(json.dumps(sys.argv) + "\\n")
if phase == "npm":
    sys.exit(case == "npm-error")
if phase == "cargo":
    if case == "compile-error":
        print("PRIVATE_SENTINEL /Users/private/runtime")
        sys.exit(101)
    if case == "dirty-lock":
        pathlib.Path("Cargo.lock").write_text("modified")
    failed = int(case == "rust-error")
    print("Running tests/yolo.rs (target/debug/deps/yolo-abcdef1234)")
    print("test sample ... " + ("FAILED" if failed else "ok"))
    print(f"test result: {'FAILED' if failed else 'ok'}. {1-failed} passed; {failed} failed; 0 ignored; 0 measured; 1 filtered out;")
    sys.exit(failed)
web = any("app/web/" in arg for arg in sys.argv)
failed = case == "node-error" and web
print("TAP version 13")
if failed:
    print("not ok 1 - PRIVATE_SENTINEL /Users/private/runtime")
    print("  ---\\n  location: '" + str(pathlib.Path("app/web/src/sample.test.js").resolve()) + ":1:1'\\n  message: PRIVATE_SENTINEL\\n  ...")
else:
    print("ok 1 - sample")
skips = ["the Cocoa shell compiles on macOS", "build signing diagnostic keeps the explicit absent-identity path ad hoc without a real keychain or a build"] if web else []
if case == "unexpected-skip":
    skips.append("unapproved")
for i, title in enumerate(skips, 2):
    print(f"ok {i} - {title} # SKIP")
print(f"1..{1+len(skips)}\\n# tests {1+len(skips)}\\n# pass {int(not failed)}\\n# fail {int(failed)}\\n# cancelled 0\\n# skipped {len(skips)}\\n# todo 0")
sys.exit(failed)
'''


class CompleteSuiteTests(unittest.TestCase):
    def test_cli_reports_failures_and_continues_without_publishing_diagnostics(self):
        LOGS.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=LOGS) as directory:
            root = Path(directory)
            for folder in ("tools", "docs", "crates/sample/src", "app/web/src", "app/ui", "bin"):
                (root / folder).mkdir(parents=True)
            (root / "probe.py").write_text("""import importlib.util, os, sys
from types import SimpleNamespace
from unittest.mock import patch
spec = importlib.util.spec_from_file_location('runner', 'tools/run-complete-suite.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
free = 19 if os.environ.get('SUITE_CASE') == 'low-disk' else 40
with patch.object(runner.shutil, 'disk_usage', return_value=SimpleNamespace(free=free * 1024**3)):
    sys.exit(runner.main())
""")
            for name in ("run-complete-suite.py", "suite-platform-inventory.py"):
                shutil.copy(TOOLS / name, root / "tools" / name)
            (root / "Cargo.lock").write_text("fixture")
            for name in ("app/web/src/sample.test.js", "app/ui/sample.test.cjs"):
                (root / name).write_text("fixture\n")
            (root / "crates/sample/src/lib.rs").write_text('#[cfg(target_os = "macos")] mod platform;\n#[cfg(not(target_os = "macos"))] mod outer;\n#[test] fn sample() {}')
            (root / "crates/sample/src/platform.rs").write_text("#[test] fn covered() {}")
            (root / "crates/sample/src/outer.rs").write_text('#[cfg(target_os = "macos")] mod child;')
            (root / "crates/sample/src/outer").mkdir()
            (root / "crates/sample/src/outer/child.rs").write_text("#[test] fn unreachable() {}")
            baseline = dict.fromkeys(("absent", "partial", "runtime_branches", "ignored"), [])
            baseline["absent"] = ["crates/sample/src/platform.rs::covered"]
            (root / "docs/suite-platform-baseline.json").write_text(json.dumps(baseline))
            for tool in ("cargo", "npm", "node"):
                (root / "bin" / tool).write_text(FAKE)
                (root / "bin" / tool).chmod(0o755)
            for args in (("init", "--quiet"), ("add", "."), ("-c", "user.name=Suite fixture", "-c", "user.email=suite@example.invalid", "commit", "--quiet", "-m", "fixture")):
                subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
            for case in ("green", "rust-error", "node-error", "compile-error", "npm-error", "unexpected-skip", "low-disk", "bad-metadata", "dirty-lock"):
                with self.subTest(case=case):
                    output = root / "reports" / case / "result.json"
                    env = {**os.environ, "PATH": str(root / "bin") + os.pathsep + os.environ["PATH"], "SUITE_CASE": case,
                           "GITHUB_SHA": "", "GITHUB_RUN_ID": "PRIVATE_SENTINEL" if case == "bad-metadata" else "123", "GITHUB_RUN_ATTEMPT": "1", "RUSTC_WRAPPER": ""}
                    process = subprocess.run([sys.executable, str(root / "probe.py"), "--output", str(output)], cwd=root, env=env, capture_output=True, text=True)
                    report = json.loads(output.read_text())
                    shutil.copy(output, LOGS / (case + ".json"))
                    self.assertEqual(report["status"], "vert" if case == "green" else "rouge")
                    self.assertEqual(process.returncode, 0 if case == "green" else 1)
                    self.assertEqual(set(report["phases"]), {"npm", "rust", "web", "ui"})
                    self.assertEqual(report["counts"]["excluded"], 1)
                    self.assertEqual(report["counts"]["absent_linux"], 1)
                    self.assertNotIn("PRIVATE_SENTINEL", output.read_text() + process.stdout + process.stderr)
                    self.assertNotIn("/Users/private", output.read_text())
                    if case == "rust-error":
                        self.assertIn("rust:yolo:sample", report["failures"])
                    if case == "node-error":
                        self.assertIn("web:app/web/src/sample.test.js:1", report["failures"])
                    if case == "dirty-lock":
                        self.assertIn("runner:tracked-tree-changed", report["failures"])
                    if case == "low-disk":
                        self.assertIn("rust:insufficient-disk", report["failures"])
                        self.assertIsNone(report["phases"]["rust"]["exit_code"])

    def test_inventory_tracks_ancestry_and_ignores_literals(self):
        spec = importlib.util.spec_from_file_location("inventory", TOOLS / "suite-platform-inventory.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        source = '''/* outer /* #[test] fn fake() {} */ comment */
#[cfg(target_os = "macos")] mod mac {
    #[test] fn whole() { let s = r###"#[test] fn fake() { }"###; }
}
#[test] fn partial() { #[cfg(target_os = "macos")] { check(); } }
#[test] fn runtime() { if cfg!(target_os = "macos") { check(); } }
'''
        result, external = module.source_inventory("fixture.rs", source)
        self.assertEqual(external, [])
        self.assertEqual(result["absent"], ["fixture.rs::mac::whole"])
        self.assertEqual(result["partial"], ["fixture.rs::partial"])
        self.assertEqual(result["runtime_branches"], ["fixture.rs::runtime"])
        unreachable, children = module.source_inventory("outer.rs", '#[cfg(target_os = "macos")] mod child;', (True, False))
        self.assertEqual(children, [(["child"], (False, False))])
        self.assertEqual(unreachable["absent"], [])
        with self.assertRaises(ValueError):
            module.source_inventory("macro.rs", '#[cfg(target_os = "macos")] #[tokio::test] async fn hidden() {}')

    def test_inventory_follows_transitive_module_platforms(self):
        spec = importlib.util.spec_from_file_location("inventory", TOOLS / "suite-platform-inventory.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for directory in ("src", "tests/support"):
            for predicate, expected in (('target_os="macos"', 1), ('not(target_os="linux")', 1), ('not(target_os="macos")', 0)):
                with self.subTest(directory=directory, predicate=predicate):
                    prefix = "crates/sample/" + directory + "/"
                    sources = {
                        prefix + ("lib.rs" if directory == "src" else "mod.rs"): "#[cfg(" + predicate + ")] mod platform;",
                        prefix + "platform.rs": '#[cfg(target_os="macos")] mod child;',
                        prefix + "platform/child.rs": "mod leaf;",
                        prefix + "platform/child/leaf.rs": "#[test] fn covered() {}",
                    }
                    def read(command, **kwargs):
                        return "\n".join(sources) if command[1] == "ls-tree" else sources[command[-1].split(":", 1)[1]]
                    with patch.object(module.subprocess, "check_output", side_effect=read):
                        self.assertEqual(len(module.inventory()["absent"]), expected)


if __name__ == "__main__":
    unittest.main()
