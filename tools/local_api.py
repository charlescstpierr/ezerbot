"""Keep desktop wrapper launch authentication in process memory."""

from __future__ import annotations

import importlib.util
import http.client
import json
import os
from pathlib import Path
import secrets
import stat
import subprocess
import sys
import threading
import time
import urllib.request
from urllib.parse import urlsplit


class LocalApiError(RuntimeError):
    pass


def local_origin(url):
    parsed = urlsplit(url)
    if (parsed.scheme != "http" or parsed.hostname != "127.0.0.1"
            or parsed.username is not None or parsed.password is not None
            or parsed.port is None):
        raise LocalApiError("Expected a desktop wrapper loopback URL")
    return f"http://127.0.0.1:{parsed.port}"


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        raise LocalApiError("Desktop wrapper redirect refused")


class LaunchIdentity:
    def __init__(self):
        self.__secret = secrets.token_bytes(32)
        self.__origin = None
        self.__opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())

    @classmethod
    def from_pipe(cls):
        if not stat.S_ISFIFO(os.fstat(sys.stdin.fileno()).st_mode):
            raise LocalApiError("Launch authentication requires a pipe")
        secret = sys.stdin.buffer.read(33)
        if len(secret) != 32:
            raise LocalApiError("Invalid launch authentication channel")
        identity = cls()
        identity.__secret = secret
        return identity

    def spawn(self, argv, **kwargs):
        if "stdin" in kwargs:
            raise LocalApiError("Launch authentication owns stdin")
        process = subprocess.Popen(argv, stdin=subprocess.PIPE, **kwargs)
        try:
            stream = getattr(process.stdin, "buffer", process.stdin)
            stream.write(self.__secret)
            stream.flush()
            process.stdin.close()
        except BaseException:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            raise LocalApiError("Launch authentication could not be delivered") from None
        return process

    def headers(self, origin):
        origin = local_origin(origin)
        if self.__origin is not None and self.__origin != origin:
            raise LocalApiError("Desktop wrapper origin changed")
        self.__origin = origin
        return {"X-Ezerbot-Launch": self.__secret.hex(),
                "X-Grokbot-Request": "local", "Origin": origin}

    def open(self, url, data=None, headers=None, method=None, timeout=30):
        values = self.headers(local_origin(url))
        values.update(headers or {})
        request = urllib.request.Request(url, data=data, headers=values, method=method)
        return self.__opener.open(request, timeout=timeout)


class LocalEndpoint:
    def __init__(self, identity, url):
        self.__identity = identity
        self.origin = local_origin(url)
        self.port = urlsplit(self.origin).port

    def request(self, method, path, body=None, headers=None, timeout=30):
        if not path.startswith("/") or path.startswith("//"):
            raise LocalApiError("Expected a local wrapper path")
        values = self.__identity.headers(self.origin)
        values.update(headers or {})
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        try:
            connection.request(method, path, body=body, headers=values)
            response = connection.getresponse()
            content = response.read()
            if response.status in (301, 302, 303, 307, 308):
                raise LocalApiError("Desktop wrapper redirect refused")
            return response.status, content, {name.lower(): value for name, value in response.getheaders()}
        finally:
            connection.close()


def _browser_module():
    scripts = Path(__file__).resolve().parents[1] / ".cursor/skills/verify-ezerbot/scripts"
    previous = sys.path[:]
    try:
        sys.path.insert(0, str(scripts))
        spec = importlib.util.spec_from_file_location("_local_api_browser", scripts / "browser.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path[:] = previous


def launch_isolated_chrome(profile, log):
    profile = Path(profile)
    profile.mkdir(parents=True, exist_ok=True)
    driver = _browser_module()
    process = subprocess.Popen([
        driver.chrome_binary(), "--headless=new", "--disable-gpu", "--no-sandbox",
        "--disable-dev-shm-usage", "--no-first-run", "--no-default-browser-check",
        "--disable-extensions", "--incognito", "--use-mock-keychain", "--password-store=basic",
        "--disable-breakpad", "--disable-crash-reporter", f"--user-data-dir={profile}",
        "--remote-debugging-address=127.0.0.1", "--remote-debugging-port=0",
        "--window-size=1140,780",
    ], stdin=subprocess.DEVNULL, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 20
        port_file = profile / "DevToolsActivePort"
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise LocalApiError("Isolated Chrome stopped before startup")
            try:
                port = int(port_file.read_text().splitlines()[0])
                endpoint = f"http://127.0.0.1:{port}"
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())
                with opener.open(endpoint + "/json/version", timeout=2) as response:
                    ws_url = json.load(response)["webSocketDebuggerUrl"]
                return process, endpoint, ws_url
            except (OSError, ValueError, KeyError, IndexError):
                time.sleep(0.05)
        raise LocalApiError("Isolated Chrome did not open a debugging port")
    except BaseException:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        raise


class BrowserAuthentication:

    def __init__(self, identity, origin, ws_url):
        self.__identity = identity
        self.__origin = local_origin(origin)
        self.__cdp = _browser_module().Cdp(ws_url)
        self.__lock = threading.Lock()
        self.__pending = {}
        self.__sessions_pending = set()
        self.__sessions_ready = threading.Event()
        self.__sessions_ready.set()
        self.__next_id = 1
        self.__closed = False
        self.__failed = False
        self.__authenticated = set()
        self.__thread = threading.Thread(target=self.__read, daemon=True)
        self.__thread.start()
        try:
            if "/devtools/browser/" in ws_url:
                self.__call("Target.setAutoAttach", {"autoAttach": True,
                            "waitForDebuggerOnStart": True, "flatten": True})
                self.__call("Target.getTargets", {})
                if not self.__sessions_ready.wait(15):
                    raise LocalApiError("Browser targets did not finish authentication setup")
                self.check()
            else:
                self.__call("Network.setCacheDisabled", {"cacheDisabled": True})
                self.__call("Fetch.enable", self.__patterns())
        except BaseException:
            self.close()
            raise LocalApiError("Browser authentication could not start") from None

    @staticmethod
    def __patterns():
        return {"patterns": [{"urlPattern": "*", "requestStage": stage}
                             for stage in ("Request", "Response")]}

    def __send(self, method, params=None, session=None, waiter=None):
        with self.__lock:
            identifier = self.__next_id
            self.__next_id += 1
            message = {"id": identifier, "method": method, "params": params or {}}
            if session:
                message["sessionId"] = session
            if waiter:
                self.__pending[identifier] = waiter
            self.__cdp._send(0x1, json.dumps(message).encode())
        return identifier

    def __call(self, method, params):
        event = threading.Event()
        result = {"event": event, "failed": False}
        self.__send(method, params, waiter=result)
        if not event.wait(15) or result["failed"]:
            raise LocalApiError("Browser authentication command failed")

    def __paused(self, params, session):
        request_id = params["requestId"]
        key = (session, request_id)
        redirected = (session, params.get("redirectedRequestId"))
        network = (session, params.get("networkId"))
        if "responseStatusCode" in params or "responseErrorReason" in params:
            authenticated = key in self.__authenticated or network in self.__authenticated
            self.__authenticated.discard(key)
            self.__authenticated.discard(network)
            if authenticated and params.get("responseStatusCode") in (301, 302, 303, 307, 308):
                self.__send("Fetch.failRequest", {"requestId": request_id, "errorReason": "BlockedByClient"}, session)
            else:
                self.__send("Fetch.continueRequest", {"requestId": request_id}, session)
            return
        if redirected in self.__authenticated or (params.get("redirectedRequestId") and network in self.__authenticated):
            self.__send("Fetch.failRequest", {"requestId": request_id, "errorReason": "BlockedByClient"}, session)
            return
        request = params["request"]
        headers = [{"name": name, "value": str(value)} for name, value in request.get("headers", {}).items()
                   if name.lower() != "x-ezerbot-launch"]
        try:
            same_origin = local_origin(request["url"]) == self.__origin
        except (LocalApiError, ValueError):
            same_origin = False
        if same_origin:
            headers.extend({"name": name, "value": value}
                           for name, value in self.__identity.headers(self.__origin).items()
                           if name.lower() not in {header["name"].lower() for header in headers})
            self.__authenticated.add(key)
            if params.get("networkId"):
                self.__authenticated.add(network)
        self.__send("Fetch.continueRequest", {"requestId": request_id, "headers": headers}, session)

    def __read(self):
        try:
            self.__cdp.sock.settimeout(None)
            while not self.__closed:
                message = self.__cdp._message()
                if "id" in message:
                    with self.__lock:
                        waiter = self.__pending.pop(message["id"], None)
                    if waiter:
                        if "event" in waiter:
                            waiter["failed"] = "error" in message
                            waiter["event"].set()
                        else:
                            if "error" in message:
                                raise LocalApiError("Browser authentication command failed")
                            with self.__lock:
                                self.__sessions_pending.discard(waiter["session_ready"])
                                if not self.__sessions_pending:
                                    self.__sessions_ready.set()
                    elif "error" in message:
                        raise LocalApiError("Browser authentication command failed")
                elif message.get("method") == "Target.attachedToTarget":
                    session = message["params"]["sessionId"]
                    with self.__lock:
                        self.__sessions_pending.add(session)
                        self.__sessions_ready.clear()
                    self.__send("Network.setCacheDisabled", {"cacheDisabled": True}, session)
                    self.__send("Fetch.enable", self.__patterns(), session)
                    self.__send("Target.setAutoAttach", {"autoAttach": True, "waitForDebuggerOnStart": True,
                                                        "flatten": True}, session)
                    self.__send("Runtime.runIfWaitingForDebugger", {}, session,
                                waiter={"session_ready": session})
                elif message.get("method") == "Fetch.requestPaused":
                    self.__paused(message["params"], message.get("sessionId"))
        except BaseException:
            if not self.__closed:
                self.__failed = True
            with self.__lock:
                for waiter in self.__pending.values():
                    if "event" in waiter:
                        waiter["failed"] = True
                        waiter["event"].set()
                self.__pending.clear()
                self.__sessions_ready.set()

    def check(self):
        if self.__failed or self.__closed:
            raise LocalApiError("Browser authentication connection closed")

    def close(self):
        self.__closed = True
        self.__cdp.close()


def main():
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser-auth", nargs=2, metavar=("WS_URL", "ORIGIN"), required=True)
    args = parser.parse_args()
    authentication = None
    try:
        identity = LaunchIdentity.from_pipe()
        ws_url, origin = args.browser_auth
        authentication = BrowserAuthentication(identity, origin, ws_url)
        print("ready", flush=True)
        while True:
            authentication.check()
            time.sleep(0.2)
    except BaseException:
        print("Browser authentication stopped", file=sys.stderr)
        return 1
    finally:
        if authentication:
            authentication.close()


if __name__ == "__main__":
    raise SystemExit(main())
