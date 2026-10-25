"""Private Chrome paths and refusal before process creation."""

import io
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import verify_lib
import browser

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
import local_api


class ChromeEnvironmentTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="chrome-environment-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_browser_receives_only_private_data_paths_and_allowed_environment(self):
        home = self.root / "home"
        with patch.dict(os.environ, {"PARENT_PRIVATE_MARKER": "parent-only",
                                     "XDG_CONFIG_HOME": str(self.root / "foreign")}, clear=False):
            environment = verify_lib.chrome_environment(home)
        self.assertNotIn("PARENT_PRIVATE_MARKER", environment)
        self.assertEqual(environment["HOME"], str(home))
        self.assertEqual(environment["CFFIXED_USER_HOME"], str(home))
        for key in ("TMPDIR", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME"):
            directory = Path(environment[key])
            self.assertEqual(directory.parent, home)
            self.assertTrue(directory.is_dir())
            self.assertEqual(stat.S_IMODE(directory.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(home.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((home / "crashpad").stat().st_mode), 0o700)

    def test_operator_home_is_refused_before_any_directory_mutation(self):
        with patch.object(Path, "mkdir") as create, patch.object(Path, "chmod") as chmod:
            with self.assertRaises(SystemExit):
                verify_lib.chrome_environment(Path(verify_lib.LOGIN_HOME))
        create.assert_not_called()
        chmod.assert_not_called()

    def test_temp_parent_itself_is_not_a_browser_home(self):
        with patch.object(Path, "mkdir") as create, patch.object(Path, "chmod") as chmod:
            with self.assertRaises(SystemExit):
                verify_lib.chrome_environment(Path(tempfile.gettempdir()))
        create.assert_not_called()
        chmod.assert_not_called()

    def test_relative_home_is_refused(self):
        with patch.object(Path, "mkdir") as create:
            with self.assertRaises(SystemExit):
                verify_lib.chrome_environment(Path("relative-home"))
        create.assert_not_called()

    def test_existing_data_symlink_is_refused_before_other_paths_are_created(self):
        home = self.root / "home"
        home.mkdir(mode=0o755)
        foreign = self.root / "foreign"
        foreign.mkdir()
        (home / "xdg-config").symlink_to(foreign, target_is_directory=True)
        with self.assertRaises(SystemExit):
            verify_lib.chrome_environment(home)
        self.assertFalse((home / "tmp").exists())
        self.assertEqual(stat.S_IMODE(home.stat().st_mode), 0o755)
        self.assertEqual(list(foreign.iterdir()), [])

    def test_existing_non_directory_is_refused_before_mutation(self):
        home = self.root / "home"
        home.mkdir()
        (home / "xdg-cache").write_text("keep")
        with self.assertRaises(SystemExit):
            verify_lib.chrome_environment(home)
        self.assertEqual((home / "xdg-cache").read_text(), "keep")
        self.assertFalse((home / "tmp").exists())

    def test_home_symlink_is_refused(self):
        target = self.root / "target"
        target.mkdir()
        home = self.root / "home"
        home.symlink_to(target, target_is_directory=True)
        with self.assertRaises(SystemExit):
            verify_lib.chrome_environment(home)
        self.assertEqual(list(target.iterdir()), [])

    def test_auxiliary_launcher_uses_private_home_and_crashpad(self):
        profile = self.root / "profile"
        calls = []
        child = SimpleNamespace(poll=lambda: None)

        def start(command, **kwargs):
            calls.append((command, kwargs))
            (profile / "DevToolsActivePort").write_text("34567\n/devtools/browser/owned\n")
            return child

        driver = SimpleNamespace(chrome_binary=lambda: "/owned/chrome",
                                 chrome_environment=verify_lib.chrome_environment,
                                 initial_page=lambda endpoint: None)
        response = io.BytesIO(json.dumps({"webSocketDebuggerUrl":
                                         "ws://127.0.0.1:34567/devtools/browser/owned"}).encode())
        opener = SimpleNamespace(open=lambda *args, **kwargs: response)
        with patch.object(local_api, "_browser_module", return_value=driver), \
                patch.object(local_api.subprocess, "Popen", side_effect=start), \
                patch.object(local_api.urllib.request, "build_opener", return_value=opener), \
                patch.dict(os.environ, {"PARENT_PRIVATE_MARKER": "parent-only"}):
            process, endpoint, _ = local_api.launch_isolated_chrome(profile, io.BytesIO())
        self.assertIs(process, child)
        self.assertEqual(endpoint, "http://127.0.0.1:34567")
        command, options = calls[0]
        home = self.root / "chrome-home"
        self.assertEqual(options["env"]["HOME"], str(home))
        self.assertNotIn("PARENT_PRIVATE_MARKER", options["env"])
        self.assertIn(f"--crash-dumps-dir={home / 'crashpad'}", command)
        self.assertIn(f"--user-data-dir={profile}", command)

    def test_auxiliary_profile_symlink_is_refused_without_process_or_home(self):
        target = self.root / "target"
        target.mkdir()
        profile = self.root / "profile"
        profile.symlink_to(target, target_is_directory=True)
        with patch.object(local_api.subprocess, "Popen") as start:
            with self.assertRaises(local_api.LocalApiError):
                local_api.launch_isolated_chrome(profile, io.BytesIO())
        start.assert_not_called()
        self.assertFalse((self.root / "chrome-home").exists())

    def test_primary_profile_symlink_is_refused_before_process_or_data_creation(self):
        home = self.root / "home"
        home.mkdir()
        foreign = self.root / "foreign"
        foreign.mkdir()
        (home / "chrome-profile").symlink_to(foreign, target_is_directory=True)
        run = {"home": str(home), "run_root": str(self.root)}
        with patch.dict(os.environ, {"CHROME_DEVTOOLS_AXI_SESSION": "owned"}), \
                patch.object(browser, "load_browser", return_value=None), \
                patch.object(browser.subprocess, "Popen") as start:
            with self.assertRaises(SystemExit):
                browser.ensure_headless(run)
        start.assert_not_called()
        self.assertFalse((home / "tmp").exists())
        self.assertEqual(list(foreign.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
