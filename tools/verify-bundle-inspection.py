#!/usr/bin/env python3
"""Prove private state inspection against two owned, signed native bundles."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
from unittest.mock import patch

from bundle_inspection import BundleInspection, BundleInspectionError


ROOT = Path(__file__).resolve().parents[1]


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def absent(pid):
    try:
        os.kill(pid, 0)
        return False
    except ProcessLookupError:
        return True


def prove(app, evidence):
    checks = []
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    checks.append("Both runs use the same verified signed bundle executable")
    owned = []
    homes = []
    with BundleInspection(app, evidence, timeout=3) as target:
        with BundleInspection(app, evidence, timeout=3) as other:
            bindings = [item.ready for item in (target, other)]
            owned = [pid for binding in bindings for pid in (binding["host_pid"], binding["engine_pid"])]
            homes = [target.home, other.home]
            (evidence / "owned.json").write_text(json.dumps({"bindings": bindings,
                                                          "homes": [str(home) for home in homes]}, indent=2) + "\n")
            for item in (target, other):
                state = item.state(item.ready["engine_pid"], item.ready["url"])
                require(state["status"] == 200 and state["mode"] == "simulated", "Owned bundle state failed")
            checks.append("Each healthy bundle authenticates its own engine through its shipped WebView")
            engine = target.ready["engine_pid"]
            os.kill(engine, signal.SIGSTOP)
            try:
                healthy = other.state(other.ready["engine_pid"], other.ready["url"])
                require(healthy["status"] == 200, "Independent healthy engine stopped responding")
                try:
                    target.state(engine, target.ready["url"])
                except BundleInspectionError:
                    checks.append("Suspended inspected engine fails while the independent engine remains healthy")
                else:
                    raise AssertionError("Another healthy engine masked the inspected engine failure")
            finally:
                os.kill(engine, signal.SIGCONT)
    require(all(absent(pid) for pid in owned), "An owned native process survived EOF cleanup")
    require(all(not home.exists() for home in homes), "An isolated native HOME survived cleanup")
    checks.append("EOF stops only the owned hosts and engines and removes both private HOMEs")

    early = BundleInspection(app, evidence, timeout=3, shutdown_timeout=0.5)
    original_read = early._read_message
    early_pids = []

    def die_before_registration(seconds):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and early.process.poll() is None:
            children = subprocess.run(["/usr/bin/pgrep", "-P", str(early.pid)], text=True,
                                      stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=2)
            for text in children.stdout.split():
                child = int(text)
                command = subprocess.run(["/bin/ps", "-p", str(child), "-o", "command="], text=True,
                                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=2)
                if command.stdout.strip().startswith(str(app / "Contents/MacOS/grokbot-wrapper") + " "):
                    sockets = subprocess.run(["/usr/sbin/lsof", "-nP", "-a", "-p", str(child),
                                              "-iTCP", "-sTCP:LISTEN", "-Fn"], text=True,
                                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=2)
                    if sockets.returncode or "n127.0.0.1:" not in sockets.stdout:
                        continue
                    require(os.getsid(child) == early.pid, "Early engine does not belong to the owned session")
                    early_pids.extend([early.pid, child])
                    (evidence / "early-owned.json").write_text(json.dumps({"pids": early_pids,
                                                                         "session": early.pid}, indent=2) + "\n")
                    early.process.kill()
                    early.process.wait(timeout=5)
                    return original_read(seconds)
            time.sleep(0.01)
        raise AssertionError("No owned native engine appeared before readiness")

    with patch.object(early, "_read_message", side_effect=die_before_registration):
        try:
            early.__enter__()
        except BundleInspectionError:
            checks.append("Host death before readiness registration cleans its engine through the owned POSIX session")
        else:
            early.close()
            raise AssertionError("Dead host before readiness was accepted")
    require(len(early_pids) == 2 and all(absent(pid) for pid in early_pids) and not early.home.exists(),
            "Early native host death left an owned process or HOME")
    owned.extend(early_pids)

    for fault in ("stopped-host", "dead-host"):
        broken = BundleInspection(app, evidence, timeout=3, shutdown_timeout=0.5).__enter__()
        broken_pids = [broken.pid, broken.ready["engine_pid"]]
        broken_home = broken.home
        if fault == "stopped-host":
            os.kill(broken.pid, signal.SIGSTOP)
        else:
            broken.process.kill()
            broken.process.wait(timeout=5)
        try:
            broken.close()
        except BundleInspectionError:
            checks.append("Abnormal host closure stays a failure and cleans its recorded engine, " + fault)
        else:
            raise AssertionError("An abnormal native host closure was reported as successful")
        require(all(absent(pid) for pid in broken_pids) and not broken_home.exists(), "Abnormal host run was not cleaned")
        owned.extend(broken_pids)

    with BundleInspection(app, evidence, timeout=3) as invalid:
        invalid_pids = [invalid.pid, invalid.ready["engine_pid"]]
        invalid_home = invalid.home
        invalid.process.stdin.write(b"unexpected\n")
        invalid.process.stdin.flush()
        try:
            invalid.state(invalid.ready["engine_pid"], invalid.ready["url"])
        except BundleInspectionError:
            checks.append("Native private channel refuses commands outside the fixed state contract")
        else:
            raise AssertionError("Native private channel accepted an unknown command")
    require(all(absent(pid) for pid in invalid_pids) and not invalid_home.exists(), "Invalid command run was not cleaned")
    owned.extend(invalid_pids)

    for fault in ("current-stderr", "replaced-stderr", "replaced-engine-log"):
        journal_case = BundleInspection(app, evidence, timeout=3).__enter__()
        journal_pids = [journal_case.pid, journal_case.ready["engine_pid"]]
        journal_home = journal_case.home
        try:
            path = journal_home / "shell.err" if fault != "replaced-engine-log" else (
                journal_home / "Library/Application Support/Grokbot/logs/engine.log")
            if fault != "current-stderr":
                renamed = path.with_name(path.name + ".original")
                path.rename(renamed)
                path.touch(mode=0o600)
                path = renamed
            with path.open("a") as journal:
                journal.write("access_token\n")
            try:
                journal_case.state(journal_case.ready["engine_pid"], journal_case.ready["url"])
            except BundleInspectionError:
                checks.append("Native state refuses journal fault without returning content, " + fault)
            else:
                raise AssertionError("Native state accepted an unsafe journal")
        finally:
            try:
                journal_case.close()
            except BundleInspectionError:
                require(journal_case.process.returncode == 1, "Unexpected native journal cleanup failure")
                checks.append("Native shutdown rechecks journals after stopping its engine, " + fault)
            else:
                raise AssertionError("Native shutdown accepted an unsafe journal")
        require(all(absent(pid) for pid in journal_pids) and not journal_home.exists(), "Journal negative case was not cleaned")
        owned.extend(journal_pids)

    host = app / "Contents/MacOS/Ezerbot"
    for fault in ("input", "output", "home", "extra-argument", "named-pipe"):
        with tempfile.TemporaryDirectory(prefix="bundle-refusal-", dir=evidence) as directory:
            home = Path(directory).resolve()
            home.chmod(0o700)
            environment = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(home),
                           "CFFIXED_USER_HOME": str(home), "EZERBOT_TEST_HOME": str(home)}
            if fault == "home":
                environment.pop("EZERBOT_TEST_HOME")
            arguments = [str(host), "--inspect-private"]
            if fault == "extra-argument":
                arguments.append("unexpected")
            descriptor = None
            if fault == "named-pipe":
                fifo = home / "named.fifo"
                os.mkfifo(fifo, 0o600)
                descriptor = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
            try:
                with (home / "shell.err").open("wb") as log:
                    result = subprocess.run(arguments, env=environment,
                                            stdin=(descriptor if descriptor is not None else
                                                   subprocess.DEVNULL if fault == "input" else subprocess.PIPE),
                                            stdout=subprocess.DEVNULL if fault == "output" else subprocess.PIPE,
                                            stderr=log, timeout=10)
                require(result.returncode == 2 and not result.stdout and
                        not (home / "Library").exists(), "Invalid private launch was not refused before workspace creation")
                checks.append("Private launch refuses " + fault + " before creating any workspace")
            finally:
                if descriptor is not None:
                    os.close(descriptor)

    report = {"head": subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip(),
              "app": str(app), "sha256": hashlib.sha256(host.read_bytes()).hexdigest(),
              "checks": checks, "owned_pids_absent": owned, "homes_removed": True, "status": "passed"}
    (evidence / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS real signed bundle, exact engine state, independent healthy engine counterexample and cleanup")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, default=ROOT / "app/build/Ezerbot.app")
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    try:
        prove(args.app.resolve(), args.evidence.resolve())
    except Exception:
        raise SystemExit("Bundle inspection proof failed. No response or secret value is printed.") from None


if __name__ == "__main__":
    main()
