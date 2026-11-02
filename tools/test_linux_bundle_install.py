import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest


TOOLS = Path(__file__).resolve().parent


@unittest.skipUnless(sys.platform == "linux", "Linux desktop launcher required")
class LinuxBundleInstallTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ezerbot-install-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.home = self.root / "home with spaces"
        self.home.mkdir()
        self.repo = self.root / "build checkout"
        for directory in ("tools", "app/ui", "target/release"):
            (self.repo / directory).mkdir(parents=True)
        for name in ("build-app-linux.sh", "install-app-linux.py"):
            if (TOOLS / name).exists():
                shutil.copyfile(TOOLS / name, self.repo / "tools" / name)
        (self.repo / "app/ui/icon-192.png").write_bytes(b"fixture icon")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.script("df", "#!/bin/sh\necho 'Filesystem Blocks Used Available'\necho 'fixture 99999999999 0 99999999999'\n")
        self.script("pkg-config", "#!/bin/sh\nexit 0\n")
        self.script("cargo", '#!/bin/sh\nif [ "$1" = metadata ]; then\n  printf \'{"target_directory":"%s/target"}\\n\' "$PWD"\nfi\n')
        self.log = self.root / "launch.json"
        self.env = {
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "HOME": str(self.home),
            "XDG_DATA_HOME": str(self.home / "desktop data"),
            "CARGO": str(self.bin / "cargo"),
            "LAUNCH_LOG": str(self.log),
        }
        self.executable = (
            "#!/usr/bin/python3\n"
            "import json, os, pathlib, sys\n"
            "pathlib.Path(os.environ['LAUNCH_LOG']).write_text(json.dumps({"
            "'executable': sys.argv[0], 'args': sys.argv[1:], "
            "'wrapper': pathlib.Path(sys.argv[0]).with_name('grokbot-wrapper').read_text()}))\n"
        )
        for name, content in (("ezerbot-linux", self.executable), ("grokbot-wrapper", "fixture wrapper v1")):
            path = self.repo / "target/release" / name
            path.write_text(content)
            path.chmod(0o755)
        self.run_command(["git", "init", "-q", str(self.repo)])
        self.run_command(["git", "-C", str(self.repo), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-qm", "fixture"])
        self.run_command(["bash", str(self.repo / "tools/build-app-linux.sh")])
        self.bundle = self.root / "download moved bundle"
        (self.repo / "app/build/linux").rename(self.bundle)
        shutil.rmtree(self.repo)

    def script(self, name, content):
        path = self.bin / name
        path.write_text(content)
        path.chmod(0o755)

    def run_command(self, command, *, success=True, env=None):
        result = subprocess.run(command, env=env or self.env, cwd=self.root,
                                text=True, capture_output=True, timeout=15)
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def install(self, destination=None, *, success=True):
        args = ["python3", str(self.bundle / "install-app-linux.py")]
        if destination is not None:
            args += ["--install-dir", str(destination)]
        return self.run_command(args, success=success)

    def launch(self, desktop, destination):
        self.log.unlink(missing_ok=True)
        self.run_command(["desktop-file-validate", str(desktop)])
        self.run_command(["gio", "launch", str(desktop)])
        deadline = time.monotonic() + 5
        while not self.log.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(self.log.exists(), "Desktop launcher did not execute the fixture")
        launched = json.loads(self.log.read_text())
        self.assertEqual(launched, {"executable": str(destination / "ezerbot-linux"),
                                    "args": [], "wrapper": "fixture wrapper v1"})
        self.assertIn(str(destination / "ezerbot.png"), desktop.read_text())

    def test_moved_bundle_installs_and_launches_without_build_tools(self):
        self.script("cargo", "#!/bin/sh\necho 'Rust must not run during installation' >&2\nexit 99\n")
        destination = self.home / "Applications with spaces/Ezerbot"
        self.install(destination)
        shutil.rmtree(self.bundle)
        self.launch(self.home / "desktop data/applications/ezerbot.desktop", destination)

    def test_reinstall_updates_payload_and_preserves_user_files(self):
        destination = self.home / ".local/opt/ezerbot"
        data = self.home / "desktop data/ezerbot/conversations.db"
        data.parent.mkdir(parents=True)
        data.write_text("user conversations")
        self.install()
        foreign = destination / "keep.txt"
        foreign.write_text("user file")
        desktop = self.home / "desktop data/applications/ezerbot.desktop"
        original_desktop = desktop.read_bytes()
        (self.bundle / "grokbot-wrapper").write_text("fixture wrapper v2")
        self.install()
        self.install()
        self.assertEqual((destination / "grokbot-wrapper").read_text(), "fixture wrapper v2")
        self.assertEqual(foreign.read_text(), "user file")
        self.assertEqual(data.read_text(), "user conversations")
        self.assertEqual(desktop.read_bytes(), original_desktop)

    def test_relative_xdg_path_uses_default_desktop_directory(self):
        self.env["XDG_DATA_HOME"] = "relative-path-is-ignored"
        self.install()
        self.launch(self.home / ".local/share/applications/ezerbot.desktop", self.home / ".local/opt/ezerbot")
        self.assertFalse((self.root / "relative-path-is-ignored").exists())

    def test_invalid_bundle_leaves_existing_installation_unchanged(self):
        destination = self.home / ".local/opt/ezerbot"
        self.install()
        (self.bundle / "grokbot-wrapper").unlink()
        self.install(success=False)
        self.launch(self.home / "desktop data/applications/ezerbot.desktop", destination)


if __name__ == "__main__":
    unittest.main()
