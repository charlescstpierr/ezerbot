"""Exercise the operator proof's native transport without a ChatGPT account."""

import argparse
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile

from proof_chatgpt_mac import NativeHost, ProofError, ROOT, compile_host, require


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--wrapper", type=Path, default=ROOT / "target/debug/grokbot-wrapper")
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    checks = []

    def check(label, condition):
        require(condition, label)
        checks.append(label)

    with tempfile.TemporaryDirectory(prefix="native-proof-", dir=args.evidence) as directory:
        work = Path(directory).resolve()
        host_path = compile_host(work, args.wrapper)
        host = NativeHost(host_path, work)
        engine_pid = None
        try:
            children = subprocess.run(["/usr/bin/pgrep", "-P", str(host.process.pid), "grokbot-wrapper"],
                                      capture_output=True, text=True, timeout=5)
            identifiers = children.stdout.split()
            check("Native proof owns exactly one wrapper child", children.returncode == 0 and
                  len(identifiers) == 1 and identifiers[0].isdigit())
            engine_pid = int(identifiers[0])
            status, state = host.call("GET", "/api/state")
            check("Unsigned native workspace becomes ready without waiting for account login", status == 200 and
                  state.get("provider", {}).get("signed_in") is False and state.get("mode") == "simulated")
            for method, path, body in (
                ("GET", "/api/usage", None),
                ("POST", "/api/preferences", {"model": "gpt-6.1-sol", "effort": "high"}),
                ("POST", "/api/conversations", {}),
                ("POST", "/api/provider/test", {"refresh": True}),
                ("POST", "/api/usage/refresh", {}),
                ("POST", "/api/provider/signout", {}),
            ):
                status, data = host.call(method, path, body)
                check("Native authenticated " + method + " " + path, status == 200)
                if path == "/api/conversations":
                    conversation = data["conversation"]["id"]
            status, _ = host.call("POST", "/api/send", {"conversation_id": conversation,
                                 "message_id": "native-transport-proof", "body": "Transport uniquement, aucun compte réel."})
            check("Native authenticated POST /api/send", status == 200)
            for command in ({"method": "GET", "path": "/api/secrets"},
                            {"method": "DELETE", "path": "/api/state"},
                            {"method": "GET", "path": "/api/state?anything=1"},
                            {"command": "identity"}):
                try:
                    host.command(command)
                except ProofError:
                    checks.append("Native proof command outside fixed contract refused")
                else:
                    raise ProofError("Native proof accepted an unrestricted command")
            host.check_journals()
            checks.append("Native launch identity absent from engine and shell journals")
            generated = "eyJ" + secrets.token_urlsafe(24)
            refused_sensitive = False
            try:
                host.call("POST", "/api/conversations", {"title": generated})
            except ProofError as error:
                refused_sensitive = str(error) == "Le pilote natif a refusé la commande."
            check("Sensitive response is refused before serialization", refused_sensitive)
        finally:
            host.close()
        check("Owned native host stopped cleanly", host.process.returncode == 0)
        try:
            os.kill(engine_pid, 0)
            engine_stopped = False
        except ProcessLookupError:
            engine_stopped = True
        check("Owned wrapper PID is absent after native host shutdown", engine_stopped)
        with (work / "shell.err").open("w") as log:
            refused = subprocess.run([str(host_path)], stdin=subprocess.DEVNULL,
                                     stdout=subprocess.PIPE, stderr=log, env={"PATH": os.environ.get("PATH", ""),
                                     "HOME": str(work), "EZERBOT_TEST_HOME": str(work), "CFFIXED_USER_HOME": str(work)},
                                     timeout=10)
        check("Native proof refuses non-pipe input before launching Cocoa", refused.returncode == 2)
    report = {"scope": "Native transport only. Real ChatGPT journey not executed.", "checks": checks, "passed": True}
    (args.evidence / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print("Native proof transport PASS. Real ChatGPT journey not executed.")


if __name__ == "__main__":
    try:
        main()
    except Exception:
        raise SystemExit("Native proof transport failed. No secret value is printed.") from None
