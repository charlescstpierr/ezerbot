#!/usr/bin/env python3
"""Browser harness: chrome-devtools-axi, or a Chrome headless CDP stand-in.

Commands match the axi CLI used in this repo: open, resize, screenshot, eval,
pages, selectpage, newpage, closepage. eval prints `result: <json>`.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from urllib.parse import urlparse

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import (
    MAC_AXI,
    MAC_CHROME,
    LOGIN_HOME,
    assert_created_home,
    assert_created_run_root,
    chrome_environment,
    identity_matches,
    load_browser,
    load_run,
    owns_axi_session,
    proc_identity,
    require_absolute,
    run_session,
    run_lock,
    save_json,
    session_id,
)

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def axi_binary() -> str:
    override = os.environ.get("EZERBOT_VERIFY_AXI")
    if override:
        return str(Path(override).expanduser())
    default = Path(LOGIN_HOME) / MAC_AXI
    if default.is_file() and os.access(default, os.X_OK):
        return str(default)
    found = shutil.which("chrome-devtools-axi")
    if found:
        return found
    raise SystemExit(
        "chrome-devtools-axi introuvable. Sur Mac : ~/.local/bin/chrome-devtools-axi. "
        "Le substitut Linux se demande avec EZERBOT_VERIFY_BROWSER=chrome-headless."
    )


def chrome_binary() -> str:
    override = os.environ.get("EZERBOT_VERIFY_CHROME")
    if override:
        return str(Path(override).expanduser())
    if sys.platform == "darwin":
        return MAC_CHROME
    # Prefer the stable binary. A PATH wrapper (for example /usr/local/bin/google-chrome)
    # can pin --user-data-dir to the real home and join an existing Chrome.
    for candidate in (
        "/opt/google/chrome/chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ):
        if os.access(candidate, os.X_OK):
            return candidate
    for name in ("google-chrome-stable", "chromium", "chromium-browser", "google-chrome"):
        found = shutil.which(name)
        if found:
            return found
    raise SystemExit("Chrome introuvable")


def browser_mode() -> str:
    mode = os.environ.get("EZERBOT_VERIFY_BROWSER", "").strip()
    if mode:
        if mode not in ("chrome-devtools-axi", "chrome-headless"):
            raise SystemExit(f"EZERBOT_VERIFY_BROWSER inconnu: {mode}")
        return mode
    return "chrome-devtools-axi"


class SocketBuffer:
    def __init__(self, sock: socket.socket):
        self.sock = sock
        self.buffer = b""

    def read_exact(self, size: int) -> bytes:
        while len(self.buffer) < size:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("websocket coupé")
            self.buffer += chunk
        data, self.buffer = self.buffer[:size], self.buffer[size:]
        return data

    def read_until(self, marker: bytes) -> bytes:
        while marker not in self.buffer:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("websocket coupé pendant la poignée de main")
            self.buffer += chunk
        index = self.buffer.index(marker) + len(marker)
        data, self.buffer = self.buffer[:index], self.buffer[index:]
        return data


class Cdp:
    def __init__(self, ws_url: str):
        parsed = urlparse(ws_url)
        if parsed.hostname not in ("127.0.0.1", "localhost"):
            raise SystemExit("le débogueur Chrome doit rester sur la boucle locale")
        port = parsed.port or 80
        path = parsed.path or "/"
        if parsed.query:
            path += "?" + parsed.query
        self.sock = socket.create_connection((parsed.hostname, port), timeout=10)
        self.reader = SocketBuffer(self.sock)
        key = base64.b64encode(os.urandom(16)).decode()
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {parsed.hostname}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(request.encode())
        header = self.reader.read_until(b"\r\n\r\n").decode(errors="replace")
        accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
        if f"Sec-WebSocket-Accept: {accept}" not in header:
            raise ConnectionError("poignée de main websocket refusée")
        self.next_id = 1

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass

    def _send(self, opcode: int, payload: bytes) -> None:
        mask = os.urandom(4)
        header = bytearray([0x80 | opcode])
        length = len(payload)
        if length < 126:
            header.append(0x80 | length)
        elif length < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack(">H", length))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack(">Q", length))
        header.extend(mask)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.sock.sendall(bytes(header) + masked)

    def _frame(self) -> tuple[bool, int, bytes]:
        first = self.reader.read_exact(2)
        fin = bool(first[0] & 0x80)
        opcode = first[0] & 0x0F
        length = first[1] & 0x7F
        if length == 126:
            length = struct.unpack(">H", self.reader.read_exact(2))[0]
        elif length == 127:
            length = struct.unpack(">Q", self.reader.read_exact(8))[0]
        if first[1] & 0x80:
            mask = self.reader.read_exact(4)
            payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(self.reader.read_exact(length)))
        else:
            payload = self.reader.read_exact(length)
        return fin, opcode, payload

    def _message(self) -> dict:
        chunks: list[bytes] = []
        while True:
            fin, opcode, payload = self._frame()
            if opcode == 0x9:
                self._send(0xA, payload)
                continue
            if opcode == 0x8:
                raise ConnectionError("websocket fermé par Chrome")
            if opcode in (0x1, 0x0):
                chunks.append(payload)
                if fin:
                    return json.loads(b"".join(chunks))

    def call(self, method: str, params: dict | None = None, timeout: float = 30) -> dict:
        message_id = self.next_id
        self.next_id += 1
        self._send(0x1, json.dumps({"id": message_id, "method": method, "params": params or {}}).encode())
        deadline = time.time() + timeout
        self.sock.settimeout(timeout)
        while time.time() < deadline:
            message = self._message()
            if message.get("id") == message_id:
                if "error" in message:
                    raise RuntimeError(json.dumps(message["error"]))
                return message.get("result") or {}
        raise TimeoutError(method)


def http_json(url: str):
    with urllib.request.urlopen(url, timeout=5) as response:
        return json.load(response)


def debugger_url(value: str, port: int, path: str) -> str:
    try:
        parsed = urlparse(value)
        valid = (
            parsed.scheme == "ws"
            and parsed.hostname in ("127.0.0.1", "localhost")
            and parsed.port == port
            and parsed.username is None
            and parsed.password is None
            and not (parsed.query or parsed.fragment)
            and parsed.path.startswith(path)
            and bool(parsed.path[len(path):])
            and "/" not in parsed.path[len(path):]
        )
    except (TypeError, ValueError, AttributeError):
        valid = False
    if not valid:
        raise SystemExit("refus: endpoint CDP invalide pour ce navigateur")
    return value


def browser_websocket(state: dict) -> str:
    port = state.get("debug_port")
    if type(port) is not int or not 0 < port < 65536:
        raise SystemExit("refus: port CDP invalide")
    expected = state.get("browser_ws_url")
    if "browser_ws_url" in state:
        debugger_url(expected, port, "/devtools/browser/")
    version = http_json(f"http://127.0.0.1:{port}/json/version")
    endpoint = debugger_url(version.get("webSocketDebuggerUrl"), port, "/devtools/browser/")
    if expected and urlparse(expected).path != urlparse(endpoint).path:
        raise SystemExit("refus: la génération du navigateur CDP a changé")
    return endpoint


def initial_page(endpoint: str) -> dict:
    targets = http_json(endpoint + "/json/list")
    page = next((item for item in targets if item.get("type") == "page"), None)
    if not page:
        raise SystemExit("Chrome n'a aucune page")
    if page.get("url", "").startswith("chrome://newtab"):
        # AXI refuses ambiguous page dumps, including the localized private-tab title's newline.
        cdp = Cdp(page["webSocketDebuggerUrl"])
        try:
            cdp.call("Page.navigate", {"url": "about:blank"})
        finally:
            cdp.close()
        page["url"] = "about:blank"
    return page


def browser_state_path(run: dict) -> Path:
    return Path(run["run_root"]) / "browser.json"


def save_browser(run: dict, state: dict) -> None:
    save_json(browser_state_path(run), state)


def ensure_headless(run: dict, *, headed: bool = False) -> dict:
    session = session_id()
    if not os.environ.get("CHROME_DEVTOOLS_AXI_SESSION"):
        raise SystemExit("CHROME_DEVTOOLS_AXI_SESSION est obligatoire, même pour le substitut headless")
    state = load_browser(run)
    if state:
        if state.get("session") != session:
            raise SystemExit("une autre session Chrome occupe déjà ce run")
        if state.get("chrome_pid") and identity_matches(int(state["chrome_pid"]), state.get("identity")):
            return state
    home = Path(run["home"])
    profile = home / "chrome-profile"
    if profile.is_symlink() or (profile.exists() and not profile.is_dir()):
        raise SystemExit("refus: le profil Chrome doit être un répertoire privé")
    environment = chrome_environment(home)
    profile.mkdir(mode=0o700, exist_ok=True)
    profile.chmod(0o700)
    log_path = Path(run["run_root"]) / "chrome.log"
    binary = chrome_binary()
    # One token for the profile: a split --user-data-dir plus a URL is two
    # headless targets on Chrome 156 and the process exits. No initial URL;
    # navigation happens afterwards through CDP.
    command = [
        binary,
        *([] if headed else ["--headless=new"]),
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--force-device-scale-factor=1",
        "--incognito",
        "--use-mock-keychain",
        "--password-store=basic",
        f"--user-data-dir={profile}",
        f"--crash-dumps-dir={Path(run['home']) / 'crashpad'}",
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=0",
        "--window-size=1140,780",
    ]
    with log_path.open("ab") as log:
        process = subprocess.Popen(
            command,
            stdout=log,
            stderr=log,
            stdin=subprocess.DEVNULL,
            start_new_session=True,
            env=environment,
        )
    port_file = profile / "DevToolsActivePort"
    deadline = time.time() + 20
    while time.time() < deadline:
        if port_file.is_file() and port_file.stat().st_size > 0:
            break
        if process.poll() is not None:
            tail = log_path.read_text(errors="replace").splitlines()[-20:] if log_path.is_file() else []
            raise SystemExit("Chrome headless s'est arrêté.\n" + "\n".join(tail))
        time.sleep(0.1)
    else:
        tail = log_path.read_text(errors="replace").splitlines()[-20:] if log_path.is_file() else []
        raise SystemExit("Chrome n'a pas ouvert de port de débogage.\n" + "\n".join(tail))
    port = int(port_file.read_text().splitlines()[0])
    identity = proc_identity(process.pid)
    state = {
        "mode": "chrome-devtools-axi" if headed else "chrome-headless",
        "session": session,
        "session_signature": run.get("session_signature"),
        "chrome_pid": process.pid,
        "pgid": process.pid,
        "identity": identity,
        "debug_port": port,
        "profile": str(profile),
        "started_by": "verify-ezerbot",
        "stand_in_flags": [] if headed else ["--headless=new", "--no-sandbox", "--disable-dev-shm-usage"],
        "width": 1140,
        "height": 780,
    }
    save_browser(run, state)
    state["browser_ws_url"] = browser_websocket(state)
    save_browser(run, state)
    page = initial_page(f"http://127.0.0.1:{port}")
    browser = Cdp(state["browser_ws_url"])
    try:
        targets = browser.call("Target.getTargets").get("targetInfos", [])
        target = next((item for item in targets if item.get("targetId") == page["id"] and item.get("type") == "page"), None)
        if target is None:
            raise SystemExit("la page initiale Chrome n'est plus disponible")
        if target.get("browserContextId"):
            state["browser_context_id"] = target["browserContextId"]
    finally:
        browser.close()
    state["page_id"] = page["id"]
    state["page_url"] = page.get("url", "")
    save_browser(run, state)
    return state


class BrowserPages:
    def __init__(self, state: dict):
        self.state = state
        self.context = state.get("browser_context_id")
        if "browser_context_id" in state and (not isinstance(self.context, str) or not self.context):
            raise SystemExit("refus: contexte Chrome invalide")
        self.cdp = Cdp(browser_websocket(state))

    def close(self) -> None:
        self.cdp.close()

    def pages(self) -> list[dict]:
        targets = self.cdp.call("Target.getTargets").get("targetInfos", [])
        pages = [
            item for item in targets
            if item.get("type") == "page"
            and item.get("browserContextId") in ((None, "") if self.context is None else (self.context,))
        ]
        if self.context is not None:
            contexts = self.cdp.call("Target.getBrowserContexts").get("browserContextIds", [])
            if self.context not in contexts and not pages:
                raise SystemExit("le contexte Chrome de ce run n'est plus disponible")
        return pages

    def require_page(self, page_id: str | None = None) -> dict:
        selected = self.state.get("page_id") if page_id is None else page_id
        for page in self.pages():
            if selected and page.get("targetId") == selected:
                return page
        raise SystemExit("page Chrome absente du contexte de ce run")

    def page_ws(self, page_id: str | None = None) -> str:
        page = self.require_page(page_id)
        targets = http_json(f"http://127.0.0.1:{self.state['debug_port']}/json/list")
        for item in targets:
            if item.get("type") == "page" and item.get("id") == page["targetId"]:
                endpoint = debugger_url(item.get("webSocketDebuggerUrl"), self.state["debug_port"], "/devtools/page/")
                if urlparse(endpoint).path != f"/devtools/page/{page['targetId']}":
                    raise SystemExit("refus: endpoint CDP d'une autre page")
                self.require_page(page["targetId"])
                return endpoint
        raise SystemExit("page Chrome sans endpoint CDP")

    def connect(self, page_id: str | None = None) -> Cdp:
        return Cdp(self.page_ws(page_id))

    def create_page(self) -> str:
        self.pages()
        params = {"url": "about:blank"}
        if self.context is not None:
            params["browserContextId"] = self.context
        page_id = self.cdp.call("Target.createTarget", params).get("targetId")
        if not isinstance(page_id, str) or not page_id:
            raise SystemExit("Chrome n'a pas créé de page")
        self.require_page(page_id)
        return page_id

    def close_page(self, page_id: str) -> None:
        self.require_page(page_id)
        if not self.cdp.call("Target.closeTarget", {"targetId": page_id}).get("success"):
            raise SystemExit("Chrome n'a pas fermé la page")
        if self.state.get("page_id") == page_id:
            self.state.pop("page_id", None)
            self.state.pop("page_url", None)


def page_ws(state: dict) -> str:
    pages = BrowserPages(state)
    try:
        return pages.page_ws()
    finally:
        pages.close()


def prepare(cdp: Cdp) -> None:
    cdp.call("Page.enable")
    cdp.call("Runtime.enable")


def eval_expression(source: str) -> str:
    stripped = source.strip()
    if stripped.startswith("(") or stripped.startswith("async") or stripped.startswith("function"):
        return f"({stripped})()"
    return stripped


def command_eval(cdp: Cdp, source: str) -> None:
    result = cdp.call(
        "Runtime.evaluate",
        {
            "expression": eval_expression(source),
            "awaitPromise": True,
            "returnByValue": True,
            "userGesture": True,
        },
        timeout=30,
    )
    if result.get("exceptionDetails"):
        details = result["exceptionDetails"]
        text = details.get("text") or details.get("exception", {}).get("description") or json.dumps(details)
        raise SystemExit(f"error: {text}")
    print("result: " + json.dumps(result.get("result", {}).get("value"), ensure_ascii=False))


def apply_metrics(cdp: Cdp, state: dict) -> None:
    """Re-apply the viewport. Headless emulation does not survive a new CDP session."""
    width = int(state.get("width") or 1140)
    height = int(state.get("height") or 780)
    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        {"width": width, "height": height, "deviceScaleFactor": 1, "mobile": False},
    )


def command_resize(cdp: Cdp, state: dict, width: int, height: int) -> None:
    apply_metrics(cdp, {**state, "width": width, "height": height})
    state.update({"width": width, "height": height})


def require_wrapper_url(url: str, wrapper_url: str) -> None:
    origins = []
    try:
        for value in (url, wrapper_url):
            if not isinstance(value, str) or any(character.isspace() or ord(character) < 32 or ord(character) == 127 for character in value):
                raise ValueError("URL invalide")
            parsed = urlparse(value)
            if (parsed.scheme != "http" or parsed.hostname != "127.0.0.1"
                    or parsed.username is not None or parsed.password is not None
                    or parsed.port is None or not 0 < parsed.port < 65536):
                raise ValueError("origine invalide")
            origins.append((parsed.scheme, parsed.hostname, parsed.port))
    except (TypeError, ValueError, AttributeError) as error:
        raise SystemExit("refus: navigation hors de l'origine exacte du wrapper") from error
    if origins[0] != origins[1]:
        raise SystemExit("refus: navigation hors de l'origine exacte du wrapper")


def command_open(cdp: Cdp, url: str, wrapper_url: str) -> None:
    require_wrapper_url(url, wrapper_url)
    if cdp.call("Page.navigate", {"url": url}).get("errorText"):
        raise SystemExit("la navigation Chrome a échoué")
    expression = """() => ({
      state: document.readyState,
      href: location.href,
      ready: !!document.getElementById("home-link"),
      editor: !!document.querySelector(".gb-editor"),
      model: !!document.querySelector('#composer-tools [data-picker="model"]'),
      pending: performance.getEntriesByType("resource").filter((entry) => entry.responseEnd === 0).map((entry) => entry.name).slice(0, 8)
    })"""
    deadline = time.time() + 25
    last = None
    while time.time() < deadline:
        result = cdp.call(
            "Runtime.evaluate",
            {"expression": eval_expression(expression), "returnByValue": True},
            timeout=5,
        )
        last = result.get("result", {}).get("value")
        href = str(last.get("href", "")) if isinstance(last, dict) else ""
        if href.startswith(url) and isinstance(last, dict) and last.get("editor") and last.get("model"):
            return
        time.sleep(0.15)
    raise SystemExit(f"la page n'a pas fini de charger: {last}")


def command_screenshot(cdp: Cdp, state: dict, path: Path) -> None:
    width = int(state.get("width") or 1140)
    height = int(state.get("height") or 780)
    result = cdp.call(
        "Page.captureScreenshot",
        {
            "format": "png",
            "fromSurface": True,
            "captureBeyondViewport": False,
            "clip": {"x": 0, "y": 0, "width": width, "height": height, "scale": 1},
        },
        timeout=30,
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(base64.b64decode(result["data"]))


def run_axi(run: dict, arguments: list[str]) -> None:
    root = assert_created_run_root(require_absolute(run.get("run_root"), "run_root"), run)
    assert_created_home(require_absolute(run.get("home"), "HOME"), root)
    session = run_session(run)
    if not session or os.environ.get("CHROME_DEVTOOLS_AXI_SESSION") != session:
        raise SystemExit("refus: la session axi ne correspond pas à la session signée du run")
    if str(Path.home()) != run["axi_home"]:
        raise SystemExit("refus: HOME axi ne correspond pas au run")
    state = load_browser(run)
    if state is not None and not owns_axi_session(run, state):
        raise SystemExit("refus: le navigateur n'appartient pas à cette session du run")
    binary = axi_binary()
    attributed = dict(state or {})
    attributed.update({"mode": "chrome-devtools-axi", "session": session,
                       "session_signature": run["session_signature"], "started_by": "verify-ezerbot"})
    if attributed.get("debug_port"):
        if not identity_matches(int(attributed.get("chrome_pid") or 0), attributed.get("identity")):
            raise SystemExit("Chrome de ce run n'est plus disponible")
        if not identity_matches(int(run.get("auth_pid") or 0), run.get("auth_identity")):
            raise SystemExit("le transport authentifié de ce run n'est plus disponible")
    save_browser(run, attributed)
    env = os.environ.copy()
    env["CHROME_DEVTOOLS_AXI_SESSION"] = session
    env["HOME"] = run["axi_home"]
    env.pop("CHROME_DEVTOOLS_AXI_AUTO_CONNECT", None)
    if attributed.get("debug_port"):
        env["CHROME_DEVTOOLS_AXI_BROWSER_URL"] = f"http://127.0.0.1:{attributed['debug_port']}"
    result = subprocess.run([binary, *arguments], env=env)
    raise SystemExit(result.returncode)


def run_headless(run: dict, arguments: list[str]) -> None:
    if not arguments:
        raise SystemExit("commande navigateur manquante")
    name = arguments[0]
    arity = {"open": (1, 2), "eval": (2, 2), "resize": (3, 3), "screenshot": (2, 2),
             "pages": (1, 1), "selectpage": (2, 2), "newpage": (2, 2), "closepage": (2, 2)}
    if name not in arity:
        raise SystemExit(f"commande inconnue: {name}")
    minimum, maximum = arity[name]
    if not minimum <= len(arguments) <= maximum:
        raise SystemExit("arguments de commande navigateur invalides")
    if name in ("open", "newpage"):
        url = arguments[1] if len(arguments) > 1 else run["url"]
        require_wrapper_url(url, run["url"])
    if name == "resize":
        try:
            width, height = int(arguments[1]), int(arguments[2])
        except ValueError as error:
            raise SystemExit("dimensions Chrome invalides") from error
        if width <= 0 or height <= 0:
            raise SystemExit("dimensions Chrome invalides")
    state = load_browser(run)
    if not state:
        raise SystemExit("Chrome de ce run n'est plus disponible")
    if not os.environ.get("CHROME_DEVTOOLS_AXI_SESSION") or state.get("session") != session_id():
        raise SystemExit("refus: une autre session Chrome occupe ce run")
    if state.get("session_signature") != run.get("session_signature"):
        raise SystemExit("refus: le navigateur n'appartient pas à ce run")
    for pid, identity, message in (
        (state.get("chrome_pid"), state.get("identity"), "Chrome de ce run n'est plus disponible"),
        (run.get("auth_pid"), run.get("auth_identity"), "le transport authentifié de ce run n'est plus disponible"),
    ):
        if type(pid) is not int or pid <= 0 or not identity_matches(pid, identity):
            raise SystemExit(message)
    pages = BrowserPages(state)
    try:
        if name == "pages":
            for item in pages.pages():
                selected = "true" if item.get("targetId") == state.get("page_id") else "false"
                print(f"{item['targetId']},{item.get('url','')},{selected}")
        elif name == "selectpage":
            page = pages.require_page(arguments[1])
            state.update({"page_id": page["targetId"], "page_url": page.get("url", "")})
        elif name == "closepage":
            pages.close_page(arguments[1])
        else:
            created = pages.create_page() if name == "newpage" else None
            cdp = None
            try:
                cdp = pages.connect(created)
                prepare(cdp)
                if name != "resize":
                    apply_metrics(cdp, state)
                if name in ("open", "newpage"):
                    command_open(cdp, url, run["url"])
                    state.update({"page_id": created or state["page_id"], "page_url": url})
                elif name == "resize":
                    command_resize(cdp, state, width, height)
                elif name == "screenshot":
                    command_screenshot(cdp, state, Path(arguments[1]))
                elif name == "eval":
                    command_eval(cdp, arguments[1])
            except BaseException:
                if created:
                    try:
                        pages.close_page(created)
                    except (Exception, SystemExit):
                        pass
                raise
            finally:
                if cdp is not None:
                    cdp.close()
        save_browser(run, state)
    finally:
        pages.close()


def main() -> None:
    parser = argparse.ArgumentParser(description="Pilote Chrome pour verify-ezerbot")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    evidence = args.evidence or (Path(os.environ["EZERBOT_VERIFY_EVIDENCE_DIR"]) if os.environ.get("EZERBOT_VERIFY_EVIDENCE_DIR") else None)
    if evidence is None:
        raise SystemExit("EZERBOT_VERIFY_EVIDENCE_DIR ou --evidence est requis")
    with run_lock(evidence):
        run = load_run(evidence)
        if run.get("cleaned") or run.get("cleaning"):
            raise SystemExit("refus: le nettoyage de ce run a commencé")
        root = assert_created_run_root(require_absolute(run.get("run_root"), "run_root"), run)
        assert_created_home(require_absolute(run.get("home"), "HOME"), root)
        mode = browser_mode()
        state = load_browser(run)
        if state is not None and state.get("mode") != mode:
            raise SystemExit("refus: ce run utilise déjà un autre mode de navigateur")
        if mode == "chrome-headless":
            run_headless(run, command)
        else:
            run_axi(run, command)


if __name__ == "__main__":
    main()
