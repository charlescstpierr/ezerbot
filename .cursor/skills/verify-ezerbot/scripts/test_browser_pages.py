#!/usr/bin/env python3
"""Exercise run-scoped browser commands through a deterministic CDP transport."""

from __future__ import annotations

import base64
import copy
import io
import unittest
from contextlib import ExitStack, redirect_stdout
from pathlib import Path
from unittest.mock import call, patch

import browser


PORT = 43123
BROWSER_WS = f"ws://127.0.0.1:{PORT}/devtools/browser/generation-one"
URL = "http://127.0.0.1:43210"
COMMANDS = (
    ["open", URL], ["eval", "() => 42"], ["resize", "820", "560"],
    ["screenshot", "/unwritten/capture.png"], ["pages"], ["selectpage", "owned"],
    ["newpage", URL], ["closepage", "owned"],
)


def target(page_id: str, context: str | None = "run-context", *, kind: str = "page") -> dict:
    info = {"targetId": page_id, "type": kind, "url": f"http://127.0.0.1:43210/{page_id}"}
    if context is not None:
        info["browserContextId"] = context
    return info


class FakeChrome:
    def __init__(self):
        self.targets = [target("foreign", "other-context"), target("owned"), target("second"),
                        target("default", None), target("worker", kind="service_worker")]
        self.contexts = ["run-context", "other-context"]
        self.version = BROWSER_WS
        self.http_calls = []
        self.connections = []
        self.closed = []
        self.calls = []
        self.effects = []
        self.on_list = None
        self.on_navigate = None
        self.page_endpoints = {}
        self.create_error = None
        self.create_id = "created"
        self.create_context = "run-context"
        self.close_success = True
        self.navigation_error = False
        self.fail_method = None
        self.eval_exception = False

    def http(self, url):
        self.http_calls.append(url)
        if url == f"http://127.0.0.1:{PORT}/json/version":
            return {"webSocketDebuggerUrl": self.version}
        if url != f"http://127.0.0.1:{PORT}/json/list":
            raise AssertionError(f"unexpected HTTP request {url}")
        if self.on_list:
            self.on_list()
        return [
            {"id": item["targetId"], "type": item["type"], "url": item["url"],
             "webSocketDebuggerUrl": self.page_endpoints.get(
                 item["targetId"], f"ws://127.0.0.1:{PORT}/devtools/page/{item['targetId']}"
             )}
            for item in self.targets
        ]

    def connect(self, endpoint):
        self.connections.append(endpoint)
        chrome = self
        page_id = endpoint.rsplit("/", 1)[1]
        is_browser = "/devtools/browser/" in endpoint

        class Connection:
            def close(self):
                chrome.closed.append(endpoint)

            def call(self, method, params=None, timeout=30):
                params = params or {}
                chrome.calls.append((endpoint, method, copy.deepcopy(params)))
                if method == chrome.fail_method:
                    raise RuntimeError("injected CDP failure")
                if is_browser:
                    if method == "Target.getBrowserContexts":
                        return {"browserContextIds": list(chrome.contexts)}
                    if method == "Target.getTargets":
                        return {"targetInfos": copy.deepcopy(chrome.targets)}
                    if method == "Target.createTarget":
                        chrome.effects.append((method, copy.deepcopy(params)))
                        if chrome.create_error:
                            raise chrome.create_error
                        if chrome.create_id:
                            chrome.targets.append(target(chrome.create_id, chrome.create_context))
                        return {"targetId": chrome.create_id}
                    if method == "Target.closeTarget":
                        chrome.effects.append((method, copy.deepcopy(params)))
                        if chrome.close_success:
                            chrome.targets[:] = [item for item in chrome.targets if item["targetId"] != params["targetId"]]
                        return {"success": chrome.close_success}
                    raise AssertionError(f"unexpected browser CDP method {method}")
                chrome.effects.append((page_id, method, copy.deepcopy(params)))
                if method in ("Page.enable", "Runtime.enable", "Emulation.setDeviceMetricsOverride"):
                    return {}
                if method == "Page.navigate":
                    if chrome.on_navigate:
                        chrome.on_navigate(page_id)
                    if chrome.navigation_error:
                        return {"errorText": "injected navigation failure"}
                    for item in chrome.targets:
                        if item["targetId"] == page_id:
                            item["url"] = params["url"]
                    return {"frameId": "frame"}
                if method == "Runtime.evaluate":
                    if chrome.eval_exception:
                        return {"exceptionDetails": {"text": "injected JavaScript failure"}}
                    if "document.readyState" in params["expression"]:
                        page = next(item for item in chrome.targets if item["targetId"] == page_id)
                        return {"result": {"value": {"href": page["url"], "editor": True, "model": True}}}
                    return {"result": {"value": 42}}
                if method == "Page.captureScreenshot":
                    return {"data": base64.b64encode(b"deterministic-png").decode()}
                raise AssertionError(f"unexpected page CDP method {method}")

        return Connection()


class BrowserPagesTests(unittest.TestCase):
    def setUp(self):
        self.chrome = FakeChrome()
        self.state = {
            "debug_port": PORT, "browser_ws_url": BROWSER_WS,
            "browser_context_id": "run-context", "page_id": "owned", "page_url": URL,
            "chrome_pid": 1001, "identity": {"marker": "chrome"},
            "session": "owner-session", "session_signature": "signed-run",
            "mode": "chrome-headless", "width": 1140, "height": 780,
        }
        self.run = {"auth_pid": 1002, "auth_identity": {"marker": "auth"},
                    "session_signature": "signed-run", "url": URL}
        self.alive = {1001, 1002}
        self.identities = {1001: {"marker": "chrome"}, 1002: {"marker": "auth"}}
        self.saved = []
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(patch.dict(browser.os.environ, {"CHROME_DEVTOOLS_AXI_SESSION": "owner-session"}))
        self.stack.enter_context(patch.object(browser, "http_json", side_effect=self.chrome.http))
        self.stack.enter_context(patch.object(browser, "Cdp", side_effect=self.chrome.connect))
        self.stack.enter_context(patch.object(browser, "load_browser", side_effect=lambda run: self.state))
        self.stack.enter_context(patch.object(browser, "save_browser", side_effect=lambda run, state: self.saved.append(copy.deepcopy(state))))
        self.identity = self.stack.enter_context(patch.object(browser, "identity_matches", side_effect=lambda pid, identity: pid in self.alive and identity == self.identities.get(pid)))
        self.ensure = self.stack.enter_context(patch.object(browser, "ensure_headless", side_effect=AssertionError("Drive must not launch Chrome")))
        self.spawn = self.stack.enter_context(patch.object(browser.subprocess, "Popen", side_effect=AssertionError("Drive must not spawn")))
        self.subprocess_run = self.stack.enter_context(patch.object(browser.subprocess, "run", side_effect=AssertionError("Drive must not invoke a process")))
        self.mkdir = self.stack.enter_context(patch.object(Path, "mkdir", autospec=True))
        self.write = self.stack.enter_context(patch.object(Path, "write_bytes", autospec=True))

    def invoke(self, arguments):
        output = io.StringIO()
        with redirect_stdout(output):
            browser.run_headless(self.run, arguments)
        self.ensure.assert_not_called()
        self.spawn.assert_not_called()
        self.subprocess_run.assert_not_called()
        return output.getvalue()

    def assert_no_effects(self):
        self.assertEqual(self.chrome.effects, [])
        self.assertEqual(self.saved, [])
        self.mkdir.assert_not_called()
        self.write.assert_not_called()
        self.ensure.assert_not_called()
        self.spawn.assert_not_called()
        self.subprocess_run.assert_not_called()

    def assert_connections_closed(self):
        self.assertCountEqual(self.chrome.connections, self.chrome.closed)

    def test_closed_selection_never_falls_back_to_foreign_page(self):
        self.chrome.targets[:] = [target("foreign", "other-context")]
        before = copy.deepcopy(self.state)
        with self.assertRaises(SystemExit):
            browser.page_ws(self.state)
        self.assertEqual(self.state, before)
        self.assertEqual(self.chrome.connections, [BROWSER_WS])
        self.assert_no_effects()
        self.assert_connections_closed()

    def test_page_commands_refuse_closed_selection_before_effect(self):
        self.state["page_id"] = "closed"
        before = copy.deepcopy(self.state)
        for command in COMMANDS[:4]:
            with self.subTest(command=command), self.assertRaises(SystemExit):
                self.invoke(command)
        self.assertEqual(self.state, before)
        self.assert_no_effects()
        self.assertTrue(all(endpoint == BROWSER_WS for endpoint in self.chrome.connections))
        self.assert_connections_closed()

    def test_page_commands_refuse_foreign_selection_before_effect(self):
        self.state["page_id"] = "foreign"
        for command in COMMANDS[:4]:
            with self.subTest(command=command), self.assertRaises(SystemExit):
                self.invoke(command)
        self.assert_no_effects()
        self.assert_connections_closed()

    def test_open_navigates_only_selected_owned_page(self):
        self.invoke(["open"])
        navigation = [effect for effect in self.chrome.effects if effect[1] == "Page.navigate"]
        self.assertEqual(navigation, [("owned", "Page.navigate", {"url": URL})])
        self.assertEqual(self.saved[-1]["page_id"], "owned")
        self.assert_connections_closed()

    def test_eval_prints_result_from_selected_owned_page(self):
        self.assertEqual(self.invoke(["eval", "() => 42"]), "result: 42\n")
        evaluation = [effect for effect in self.chrome.effects if effect[1] == "Runtime.evaluate"]
        self.assertEqual(len(evaluation), 1)
        self.assertEqual(evaluation[0][0], "owned")
        self.assertTrue(evaluation[0][2]["awaitPromise"])
        self.assert_connections_closed()

    def test_resize_applies_and_persists_metrics_after_success(self):
        self.invoke(["resize", "820", "560"])
        metrics = [effect for effect in self.chrome.effects if effect[1] == "Emulation.setDeviceMetricsOverride"]
        self.assertEqual(metrics, [("owned", "Emulation.setDeviceMetricsOverride", {
            "width": 820, "height": 560, "deviceScaleFactor": 1, "mobile": False,
        })])
        self.assertEqual((self.saved[-1]["width"], self.saved[-1]["height"]), (820, 560))
        self.assert_connections_closed()

    def test_screenshot_writes_selected_page_bytes(self):
        self.invoke(["screenshot", "/unwritten/capture.png"])
        self.write.assert_called_once_with(Path("/unwritten/capture.png"), b"deterministic-png")
        captures = [effect for effect in self.chrome.effects if effect[1] == "Page.captureScreenshot"]
        self.assertEqual(captures, [("owned", "Page.captureScreenshot", {
            "format": "png", "fromSurface": True, "captureBeyondViewport": False,
            "clip": {"x": 0, "y": 0, "width": 1140, "height": 780, "scale": 1},
        })])
        self.assert_connections_closed()

    def test_pages_lists_only_current_context_without_page_effects(self):
        output = self.invoke(["pages"])
        self.assertEqual(output, f"owned,{URL}/owned,true\nsecond,{URL}/second,false\n")
        self.assertEqual(self.chrome.effects, [])
        self.assertEqual(self.chrome.connections, [BROWSER_WS])
        self.assert_connections_closed()

    def test_selection_recovers_closed_page_only_with_an_explicit_owned_id(self):
        self.state["page_id"] = "closed"
        self.invoke(["selectpage", "second"])
        self.assertEqual(self.saved[-1]["page_id"], "second")
        self.assertEqual(self.saved[-1]["page_url"], f"{URL}/second")
        self.assertEqual(self.chrome.effects, [])
        self.assert_connections_closed()

    def test_select_and_close_refuse_foreign_or_missing_target(self):
        before = copy.deepcopy(self.state)
        for command in ("selectpage", "closepage"):
            for page_id in ("foreign", "default", "worker", "closed"):
                with self.subTest(command=command, page_id=page_id), self.assertRaises(SystemExit):
                    self.invoke([command, page_id])
        self.assertEqual(self.state, before)
        self.assert_no_effects()
        self.assert_connections_closed()

    def test_newpage_creates_and_navigates_a_distinct_target_in_current_context(self):
        self.invoke(["newpage", URL])
        self.assertEqual(self.chrome.effects[0], ("Target.createTarget", {
            "url": "about:blank", "browserContextId": "run-context",
        }))
        navigation = [effect for effect in self.chrome.effects if len(effect) == 3 and effect[1] == "Page.navigate"]
        self.assertEqual(navigation, [("created", "Page.navigate", {"url": URL})])
        self.assertEqual(self.saved[-1]["page_id"], "created")
        self.assertEqual(next(item for item in self.chrome.targets if item["targetId"] == "owned")["url"], f"{URL}/owned")
        self.assert_connections_closed()

    def test_closepage_clears_closed_selection_without_fallback(self):
        self.invoke(["closepage", "owned"])
        self.assertEqual(self.chrome.effects, [("Target.closeTarget", {"targetId": "owned"})])
        self.assertNotIn("page_id", self.saved[-1])
        self.assertNotIn("page_url", self.saved[-1])
        with self.assertRaises(SystemExit):
            self.invoke(["eval", "() => 42"])
        self.assertNotIn("page_id", self.state)
        self.assert_connections_closed()

    def test_closing_another_owned_page_preserves_selection(self):
        self.invoke(["closepage", "second"])
        self.assertEqual(self.saved[-1]["page_id"], "owned")
        self.assertEqual(self.chrome.effects, [("Target.closeTarget", {"targetId": "second"})])

    def test_legacy_state_is_limited_to_default_context(self):
        self.state.pop("browser_context_id")
        self.state.pop("browser_ws_url")
        self.state["page_id"] = "default"
        self.assertEqual(self.invoke(["pages"]), f"default,{URL}/default,true\n")
        self.assertEqual(self.invoke(["eval", "() => 42"]), "result: 42\n")
        self.assertNotIn("browser_ws_url", self.state)
        self.assertNotIn("browser_context_id", self.state)
        with self.assertRaises(SystemExit):
            self.invoke(["selectpage", "owned"])
        self.assertEqual(self.state["page_id"], "default")
        self.assert_connections_closed()

    def test_legacy_newpage_omits_context_and_still_validates_default_membership(self):
        self.state.pop("browser_context_id")
        self.chrome.create_context = None
        self.invoke(["newpage", URL])
        self.assertEqual(self.chrome.effects[0], ("Target.createTarget", {"url": "about:blank"}))
        self.assertEqual(self.saved[-1]["page_id"], "created")
        self.assert_connections_closed()

    def test_invalid_explicit_context_refuses_before_transport(self):
        for context in (None, "", False, 4, []):
            self.state["browser_context_id"] = context
            with self.subTest(context=context), self.assertRaises(SystemExit):
                self.invoke(["pages"])
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_removed_context_refuses_every_route_before_effect(self):
        self.chrome.contexts.remove("run-context")
        self.chrome.targets[:] = [item for item in self.chrome.targets
                                  if item.get("browserContextId") != "run-context"]
        for command in COMMANDS:
            with self.subTest(command=command), self.assertRaises(SystemExit):
                self.invoke(command)
        self.assert_no_effects()
        self.assert_connections_closed()

    def test_initial_incognito_context_is_attributed_by_targets_even_when_not_enumerated(self):
        self.chrome.contexts.remove("run-context")
        self.assertEqual(self.invoke(["eval", "() => 42"]), "result: 42\n")
        self.assertEqual(self.invoke(["pages"]), f"owned,{URL}/owned,true\nsecond,{URL}/second,false\n")
        self.invoke(["newpage", URL])
        self.assertEqual(self.saved[-1]["page_id"], "created")
        self.assert_connections_closed()

    def test_empty_live_context_can_create_a_page(self):
        self.chrome.targets[:] = [target("foreign", "other-context")]
        self.assertEqual(self.invoke(["pages"]), "")
        self.invoke(["newpage", URL])
        self.assertEqual(self.saved[-1]["page_id"], "created")
        self.assert_connections_closed()

    def test_targets_attached_later_are_attributed_by_cdp_context(self):
        self.chrome.targets += [target("late-owned"), target("late-foreign", "other-context")]
        output = self.invoke(["pages"])
        self.assertIn(f"late-owned,{URL}/late-owned,false\n", output)
        self.assertNotIn("late-foreign", output)
        with self.assertRaises(SystemExit):
            self.invoke(["selectpage", "late-foreign"])
        self.invoke(["selectpage", "late-owned"])
        self.assertEqual(self.saved[-1]["page_id"], "late-owned")

    def test_context_is_checked_again_after_http_target_discovery(self):
        def replace_target_context():
            next(item for item in self.chrome.targets if item["targetId"] == "owned")["browserContextId"] = "other-context"
        self.chrome.on_list = replace_target_context
        with self.assertRaises(SystemExit):
            self.invoke(["eval", "() => 42"])
        self.assertEqual(self.chrome.connections, [BROWSER_WS])
        self.assert_no_effects()
        self.assert_connections_closed()

    def test_selected_target_closed_during_http_discovery_is_not_replaced(self):
        self.chrome.on_list = lambda: self.chrome.targets.remove(next(item for item in self.chrome.targets if item["targetId"] == "owned"))
        with self.assertRaises(SystemExit):
            self.invoke(["open", URL])
        self.assertEqual(self.state["page_id"], "owned")
        self.assert_no_effects()
        self.assert_connections_closed()

    def test_page_endpoint_must_match_owned_target_port_and_loopback(self):
        invalid = (
            f"ws://127.0.0.1:{PORT}/devtools/page/foreign",
            f"ws://127.0.0.1:{PORT + 1}/devtools/page/owned",
            f"ws://example.invalid:{PORT}/devtools/page/owned",
            f"http://127.0.0.1:{PORT}/devtools/page/owned",
            f"ws://127.0.0.1:{PORT}/devtools/page/owned?extra=1",
        )
        for endpoint in invalid:
            self.chrome.page_endpoints["owned"] = endpoint
            with self.subTest(endpoint=endpoint), self.assertRaises(SystemExit):
                self.invoke(["eval", "() => 42"])
        self.assertTrue(all(endpoint == BROWSER_WS for endpoint in self.chrome.connections))
        self.assert_no_effects()
        self.assert_connections_closed()

    def test_invalid_port_refuses_before_http_or_cdp(self):
        for port in (None, 0, -1, 65536, True, "43123", 43123.0):
            self.state["debug_port"] = port
            with self.subTest(port=port), self.assertRaises(SystemExit):
                self.invoke(["pages"])
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_changed_browser_generation_refuses_before_cdp(self):
        self.chrome.version = f"ws://127.0.0.1:{PORT}/devtools/browser/generation-two"
        with self.assertRaises(SystemExit):
            self.invoke(["pages"])
        self.assertEqual(self.chrome.connections, [])
        self.assertEqual(self.state["browser_ws_url"], BROWSER_WS)
        self.assert_no_effects()

    def test_browser_endpoint_must_be_local_and_use_recorded_port(self):
        for endpoint in (
            f"ws://127.0.0.1:{PORT + 1}/devtools/browser/generation-one",
            f"ws://example.invalid:{PORT}/devtools/browser/generation-one",
            f"ws://127.0.0.1:{PORT}/devtools/page/owned", None,
        ):
            self.chrome.version = endpoint
            with self.subTest(endpoint=endpoint), self.assertRaises(SystemExit):
                self.invoke(["pages"])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_invalid_recorded_generation_refuses_before_http(self):
        self.state["browser_ws_url"] = f"ws://127.0.0.1:{PORT + 1}/devtools/browser/generation-one"
        with self.assertRaises(SystemExit):
            self.invoke(["pages"])
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_creation_failure_preserves_selection_and_never_navigates(self):
        self.chrome.create_error = RuntimeError("injected creation failure")
        before = copy.deepcopy(self.state)
        with self.assertRaises(RuntimeError):
            self.invoke(["newpage", URL])
        self.assertEqual(self.state, before)
        self.assertEqual(self.saved, [])
        self.assertEqual(self.chrome.effects, [("Target.createTarget", {"url": "about:blank", "browserContextId": "run-context"})])
        self.assert_connections_closed()

    def test_missing_created_target_id_preserves_selection(self):
        self.chrome.create_id = None
        with self.assertRaises(SystemExit):
            self.invoke(["newpage", URL])
        self.assertEqual(self.state["page_id"], "owned")
        self.assertEqual(self.saved, [])
        self.assertEqual(len(self.chrome.effects), 1)
        self.assert_connections_closed()

    def test_foreign_created_target_is_never_driven_or_closed(self):
        self.chrome.create_context = "other-context"
        with self.assertRaises(SystemExit):
            self.invoke(["newpage", URL])
        self.assertEqual(self.state["page_id"], "owned")
        self.assertEqual(self.saved, [])
        self.assertEqual(self.chrome.effects, [("Target.createTarget", {"url": "about:blank", "browserContextId": "run-context"})])
        self.assertEqual(self.chrome.connections, [BROWSER_WS])
        self.assertTrue(any(item["targetId"] == "created" for item in self.chrome.targets))
        self.assert_connections_closed()

    def test_failed_newpage_navigation_closes_only_created_owned_page(self):
        self.chrome.navigation_error = True
        before = copy.deepcopy(self.state)
        with self.assertRaises(SystemExit):
            self.invoke(["newpage", URL])
        self.assertEqual(self.state, before)
        self.assertEqual(self.saved, [])
        self.assertEqual(self.chrome.effects[-1], ("Target.closeTarget", {"targetId": "created"}))
        self.assertFalse(any(item["targetId"] == "created" for item in self.chrome.targets))
        self.assertTrue(any(item["targetId"] == "owned" for item in self.chrome.targets))
        self.assert_connections_closed()

    def test_failed_newpage_never_closes_target_that_lost_attribution(self):
        self.chrome.navigation_error = True
        def replace_created_context(page_id):
            next(item for item in self.chrome.targets if item["targetId"] == page_id)["browserContextId"] = "other-context"
        self.chrome.on_navigate = replace_created_context
        with self.assertRaises(SystemExit):
            self.invoke(["newpage", URL])
        self.assertFalse(any(effect[0] == "Target.closeTarget" for effect in self.chrome.effects))
        self.assertEqual(self.saved, [])
        self.assertEqual(self.state["page_id"], "owned")
        self.assert_connections_closed()

    def test_failed_close_preserves_selection(self):
        self.chrome.close_success = False
        with self.assertRaises(SystemExit):
            self.invoke(["closepage", "owned"])
        self.assertEqual(self.state["page_id"], "owned")
        self.assertEqual(self.saved, [])
        self.assert_connections_closed()

    def test_failed_resize_does_not_publish_new_metrics(self):
        self.chrome.fail_method = "Emulation.setDeviceMetricsOverride"
        with self.assertRaises(RuntimeError):
            self.invoke(["resize", "820", "560"])
        self.assertEqual((self.state["width"], self.state["height"]), (1140, 780))
        self.assertEqual(self.saved, [])
        self.assert_connections_closed()

    def test_eval_exception_is_reported_without_state_publication(self):
        self.chrome.eval_exception = True
        with self.assertRaisesRegex(SystemExit, "injected JavaScript failure"):
            self.invoke(["eval", "() => 42"])
        self.assertEqual(self.saved, [])
        self.assert_connections_closed()

    def test_failed_screenshot_writes_no_file(self):
        self.chrome.fail_method = "Page.captureScreenshot"
        with self.assertRaises(RuntimeError):
            self.invoke(["screenshot", "/unwritten/capture.png"])
        self.write.assert_not_called()
        self.mkdir.assert_not_called()
        self.assertEqual(self.saved, [])
        self.assert_connections_closed()

    def test_failed_page_preparation_closes_connections_without_publication(self):
        self.chrome.fail_method = "Page.enable"
        with self.assertRaises(RuntimeError):
            self.invoke(["eval", "() => 42"])
        self.assertEqual(self.saved, [])
        self.assert_connections_closed()

    def test_missing_browser_never_contacts_cdp_or_spawns(self):
        self.state = None
        with self.assertRaises(SystemExit):
            self.invoke(["pages"])
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_dead_chrome_or_auth_refuses_every_route_before_transport(self):
        for dead in (1001, 1002):
            self.alive = {1001, 1002} - {dead}
            for command in COMMANDS:
                with self.subTest(dead=dead, command=command), self.assertRaises(SystemExit):
                    self.invoke(command)
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_both_recorded_identities_are_checked_before_cdp(self):
        self.invoke(["pages"])
        self.assertEqual(self.identity.call_args_list, [
            call(1001, {"marker": "chrome"}),
            call(1002, {"marker": "auth"}),
        ])

    def test_missing_process_identity_refuses_before_transport(self):
        self.state.pop("chrome_pid")
        with self.assertRaises(SystemExit):
            self.invoke(["pages"])
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_missing_or_changed_chrome_and_auth_identities_refuse_before_transport(self):
        for record, key in ((self.state, "identity"), (self.run, "auth_identity")):
            old = record[key]
            for identity in (None, {}, {"marker": "another-process"}):
                record[key] = identity
                with self.subTest(key=key, identity=identity), self.assertRaises(SystemExit):
                    self.invoke(["pages"])
            record[key] = old
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_changed_session_or_signature_refuses_before_transport(self):
        for key in ("session", "session_signature"):
            old = self.state[key]
            self.state[key] = "foreign-run"
            with self.subTest(key=key), self.assertRaises(SystemExit):
                self.invoke(["pages"])
            self.state[key] = old
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_invalid_commands_refuse_before_transport_or_creation(self):
        for command in ([], ["unknown"], ["eval"], ["newpage"], ["closepage"],
                        ["resize", "no", "560"], ["resize", "0", "560"],
                        ["open", "https://example.invalid"], ["newpage", "https://example.invalid"]):
            with self.subTest(command=command), self.assertRaises(SystemExit):
                self.invoke(command)
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_navigation_refuses_other_origin_credentials_and_malformed_url_before_effect(self):
        for url in (
            "http://127.0.0.1:43211/path", "http://127.0.0.1:43210@example.invalid/",
            "http://name:password@127.0.0.1:43210/", "http://@127.0.0.1:43210/",
            "https://127.0.0.1:43210/", "http://localhost:43210/",
            "http://127.0.0.1.example.invalid:43210/", "javascript:alert(1)",
            "data:text/html,hello", "//127.0.0.1:43210/", "http://127.0.0.1:bad/",
            "http://127.0.0.1:0/", "http://127.0.0.1:65536/", "http://127.0.0.1/",
            " http://127.0.0.1:43210/", "http://127.0.0.1:43210/\npath",
            "http://[invalid:43210/", "http:////127.0.0.1:43210/",
        ):
            for command in ("open", "newpage"):
                with self.subTest(command=command, url=url), self.assertRaises(SystemExit):
                    self.invoke([command, url])
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()

    def test_navigation_accepts_same_origin_path_and_query(self):
        url = URL + "/path?theme=dark#section"
        self.invoke(["open", url])
        navigation = [effect for effect in self.chrome.effects if effect[1] == "Page.navigate"]
        self.assertEqual(navigation, [("owned", "Page.navigate", {"url": url})])
        self.assertEqual(self.saved[-1]["page_url"], url)
        self.assert_connections_closed()

    def test_invalid_wrapper_origin_cannot_authorize_navigation(self):
        for origin in ("http://127.0.0.1:43210@example.invalid/", "https://127.0.0.1:43210/", "broken"):
            self.run["url"] = origin
            with self.subTest(origin=origin), self.assertRaises(SystemExit):
                self.invoke(["newpage", URL])
        self.assertEqual(self.chrome.http_calls, [])
        self.assertEqual(self.chrome.connections, [])
        self.assert_no_effects()


if __name__ == "__main__":
    unittest.main()
