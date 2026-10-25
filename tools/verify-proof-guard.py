#!/usr/bin/env python3

import argparse
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import subprocess
import sys
import tempfile
import urllib.error

from local_api import LaunchIdentity

ROOT = Path(__file__).resolve().parents[1]


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def contains_sensitive_pattern(text):
    return bool(re.search(r"eyJ[A-Za-z0-9_-]{8,}|Bearer\s+\S+|refresh_token|access_token|id_token", text, re.I))


def main():
    parser = argparse.ArgumentParser(description="Proof of authenticated local calls, without a real account")
    parser.add_argument("--guard-calls", action="store_true", required=True)
    parser.parse_args()
    binary = Path(os.environ.get("EZERBOT_WRAPPER", ROOT / "target/debug/grokbot-wrapper"))
    if not binary.is_file():
        binary = Path(os.environ.get("EZERBOT_APP", ROOT / "app/build/Ezerbot.app")) / "Contents/MacOS/grokbot-wrapper"
    require(binary.is_file(), "Build the wrapper through cargo-gate before running the guard proof")
    identity = LaunchIdentity()
    url = None
    with tempfile.TemporaryDirectory(prefix="ezerbot-proof-guard-") as directory:
        process = identity.spawn([str(binary), "--port", "0", "--db", str(Path(directory) / "workspace.sqlite")],
            env={"PATH": os.environ.get("PATH", ""), "HOME": directory, "CFFIXED_USER_HOME": directory},
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                require(bool(selector.select(20)), "The isolated wrapper did not announce its URL")
            url = json.loads(process.stdout.readline())["url"]
            launch_header = identity.headers(url)["X-Ezerbot-Launch"].encode()

            def call(path, body=None, headers=None):
                data = json.dumps(body).encode() if body is not None else None
                values = {"Content-Type": "application/json", **(headers or {})}
                try:
                    with identity.open(url + path, data=data, headers=values, timeout=30) as response:
                        status, content = response.status, response.read()
                except urllib.error.HTTPError as error:
                    status, content = error.code, error.read()
                require(launch_header not in content, "The wrapper disclosed its launch authentication")
                require(not contains_sensitive_pattern(content.decode(errors="replace")), "The local response contains a credential pattern")
                return status, content

            require(call("/api/state")[0] == 200, "State request failed")
            require(call("/api/preferences", {"model": "gpt-6.1-sol", "effort": "high"})[0] == 200,
                    "Preferences request failed")
            status, content = call("/api/conversations", {})
            require(status == 200, "Conversation request failed")
            conversation = json.loads(content)["conversation"]["id"]
            require(call("/api/send", {"conversation_id": conversation, "message_id": "proof-1", "body": "ping"})[0] != 403,
                    "The guard rejected a legitimate send")
            require(call("/api/provider/test", {"refresh": True})[0] == 200, "Provider test request failed")
            require(call("/api/usage")[0] == 200, "Usage request failed")
            require(call("/api/usage/refresh", {})[0] == 200, "Usage refresh request failed")
            require(call("/api/provider/signout", {})[0] == 200, "Signout request failed")
            status, content = call("/api/preferences", {}, {"X-Grokbot-Request": ""})
            require(status == 403, "The guard accepted a missing local marker")
            require("ne provient pas de l'application locale" in content.decode(), "The local marker refusal changed")
            require(call("/api/preferences", {}, {"Origin": "https://example.invalid"})[0] == 403,
                    "The guard accepted a foreign origin")
            require(call("/api/state", headers={"X-Ezerbot-Launch": ""})[0] == 403,
                    "The guard accepted missing launch authentication")
            require(call("/api/state", headers={"X-Ezerbot-Launch": secrets.token_bytes(32).hex()})[0] == 403,
                    "The guard accepted unrelated launch authentication")
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            process.stdout.close()
            diagnostics = process.stderr.read()
            process.stderr.close()
            require((url is None or identity.headers(url)["X-Ezerbot-Launch"] not in diagnostics) and
                    not contains_sensitive_pattern(diagnostics), "The isolated guard journal disclosed a credential")
    print("guard-calls ok, real ChatGPT journey not executed")


if __name__ == "__main__":
    try:
        main()
    except BaseException:
        print("Isolated guard proof failed", file=sys.stderr)
        raise SystemExit(1)
