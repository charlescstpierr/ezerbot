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
    identity_matches,
    load_run,
    proc_identity,
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


def browser_state_path(run: dict) -> Path:
    return Path(run["run_root"]) / "browser.json"


def load_browser(run: dict) -> dict | None:
    path = browser_state_path(run)
    if not path.is_file():
        return None
    return json.loads(path.read_text())


def save_browser(run: dict, state: dict) -> None:
    path = browser_state_path(run)
    path.write_text(json.dumps(state, indent=2) + "\n")


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
    profile = Path(run["home"]) / "chrome-profile"
    profile.mkdir(parents=True, exist_ok=True)
    log_path = Path(run["run_root"]) / "chrome.log"
    log = log_path.open("ab")
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
        "--incognito",
        "--use-mock-keychain",
        "--password-store=basic",
        f"--user-data-dir={profile}",
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=0",
        "--window-size=1140,780",
    ]
    process = subprocess.Popen(
        command,
        stdout=log,
        stderr=log,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
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
    targets = http_json(f"http://127.0.0.1:{port}/json/list")
    page = next((item for item in targets if item.get("type") == "page"), None)
    if not page:
        raise SystemExit("Chrome n'a aucune page")
    state["page_id"] = page["id"]
    state["page_url"] = page.get("url", "")
    save_browser(run, state)
    return state


def page_ws(state: dict) -> str:
    targets = http_json(f"http://127.0.0.1:{state['debug_port']}/json/list")
    page_id = state.get("page_id")
    for item in targets:
        if item.get("type") == "page" and item.get("id") == page_id and item.get("webSocketDebuggerUrl"):
            return item["webSocketDebuggerUrl"]
    for item in targets:
        if item.get("type") == "page" and item.get("webSocketDebuggerUrl"):
            state["page_id"] = item["id"]
            return item["webSocketDebuggerUrl"]
    raise SystemExit("page Chrome introuvable")


def connect(state: dict) -> Cdp:
    return Cdp(page_ws(state))


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
    state["width"] = width
    state["height"] = height
    apply_metrics(cdp, state)


def command_open(cdp: Cdp, url: str) -> None:
    if not url.startswith("http://127.0.0.1:"):
        raise SystemExit("refus: navigation hors du wrapper de bouclage")
    cdp.call("Page.navigate", {"url": url})
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


def command_pages(state: dict) -> None:
    targets = http_json(f"http://127.0.0.1:{state['debug_port']}/json/list")
    current = state.get("page_id")
    for item in targets:
        if item.get("type") != "page":
            continue
        selected = "true" if item.get("id") == current else "false"
        print(f"{item.get('id')},{item.get('url','')},{selected}")


def run_axi(arguments: list[str], run: dict) -> None:
    if not os.environ.get("CHROME_DEVTOOLS_AXI_SESSION"):
        raise SystemExit("CHROME_DEVTOOLS_AXI_SESSION est obligatoire")
    env = os.environ.copy()
    env["CHROME_DEVTOOLS_AXI_SESSION"] = session_id()
    state = load_browser(run)
    if not state or not identity_matches(int(state.get("chrome_pid") or 0), state.get("identity")):
        raise SystemExit("Chrome de ce run n'est plus disponible")
    if not identity_matches(int(run.get("auth_pid") or 0), run.get("auth_identity")):
        raise SystemExit("le transport authentifié de ce run n'est plus disponible")
    env["CHROME_DEVTOOLS_AXI_BROWSER_URL"] = f"http://127.0.0.1:{state['debug_port']}"
    env.pop("CHROME_DEVTOOLS_AXI_AUTO_CONNECT", None)
    result = subprocess.run([axi_binary(), *arguments], env=env)
    raise SystemExit(result.returncode)


def run_headless(run: dict, arguments: list[str]) -> None:
    if not arguments:
        raise SystemExit("commande navigateur manquante")
    state = ensure_headless(run)
    name = arguments[0]
    cdp = connect(state)
    try:
        prepare(cdp)
        if name != "resize":
            apply_metrics(cdp, state)
        if name == "open":
            command_open(cdp, arguments[1] if len(arguments) > 1 else run["url"])
        elif name == "newpage":
            command_open(cdp, arguments[1])
        elif name == "resize":
            command_resize(cdp, state, int(arguments[1]), int(arguments[2]))
            save_browser(run, state)
        elif name == "screenshot":
            command_screenshot(cdp, state, Path(arguments[1]))
        elif name == "eval":
            command_eval(cdp, arguments[1])
        elif name == "pages":
            command_pages(state)
        elif name == "selectpage":
            state["page_id"] = arguments[1]
            save_browser(run, state)
        elif name == "closepage":
            cdp.call("Target.closeTarget", {"targetId": arguments[1]})
        else:
            raise SystemExit(f"commande inconnue: {name}")
    finally:
        cdp.close()
        save_browser(run, state)


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
    run = load_run(evidence)
    if browser_mode() == "chrome-headless":
        run_headless(run, command)
    else:
        run_axi(command, run)


if __name__ == "__main__":
    main()
