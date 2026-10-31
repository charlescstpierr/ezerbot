#!/usr/bin/env python3
"""Run the checked-out suite and report only controlled public failure details."""

import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
EXCLUDED = "a_command_that_reaches_the_data_without_naming_it_reads_nothing"
LINUX_SKIPS = {
    "the Cocoa shell compiles on macOS",
    "build signing diagnostic keeps the explicit absent-identity path ad hoc without a real keychain or a build",
}
SUMMARY = re.compile(r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out;", re.M)
SYMBOL = r"[A-Za-z_][A-Za-z0-9_:]*"
INVENTORY_FAILURES = {
    "unterminated block comment", "unterminated raw string", "unrecognized Rust token",
    "unterminated string", "cfg not requires one argument", "unbalanced Rust delimiters",
    "unsupported platform or ignore cfg_attr", "platform cfg has no following item",
    "unsupported #[path] literal", "unsupported #[path] attribute", "unsupported test attribute",
    "unsupported test item", "unreachable platform module graph",
}


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z")


def save(output, report):
    with tempfile.NamedTemporaryFile(mode="w", dir=output.parent, delete=False) as handle:
        json.dump(report, handle, indent=2, sort_keys=True)
        handle.write("\n")
    os.replace(handle.name, output)


def git(*args):
    result = subprocess.run(["git", *args], cwd=ROOT, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, text=True)
    if result.returncode:
        raise RuntimeError("git")
    return result.stdout.strip()


def preflight_failure_detail(error):
    public_types = (
        RuntimeError, ValueError, KeyboardInterrupt, OSError, FileNotFoundError,
        PermissionError, IsADirectoryError, NotADirectoryError, subprocess.SubprocessError,
        subprocess.CalledProcessError, subprocess.TimeoutExpired, json.JSONDecodeError,
    )
    error_type = type(error)
    detail = {"type": error_type.__name__ if error_type in public_types else "Exception",
              "message": "preflight check failed"}
    if error_type not in (RuntimeError, ValueError) or len(error.args) != 1 or type(error.args[0]) is not str:
        return detail
    message = error.args[0]
    if error_type is RuntimeError:
        if message in {"git", "sha", "expected-sha", "platform"}:
            detail["message"] = message
        return detail
    location = re.fullmatch(r"(crates/(?:[A-Za-z0-9_-]+/)+[A-Za-z0-9_-]+\.rs): (.*)", message)
    reason = location[2] if location else message
    prefix = location[1] + ": " if location else ""
    if reason in INVENTORY_FAILURES or re.fullmatch(
        r"cannot resolve gated module [A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*", reason
    ):
        detail["message"] = prefix + reason
    elif reason.startswith("unsupported platform cfg: "):
        detail["message"] = prefix + "unsupported platform cfg"
    return detail


def rust_results(raw):
    summaries = [tuple(map(int, row)) for row in SUMMARY.findall(raw)]
    counts = dict(zip(("passed", "failed", "ignored", "filtered"),
                      (sum(row[index] for row in summaries) for index in range(4))))
    failures, target = [], "suite"
    for line in raw.splitlines():
        running = re.search(r"\bRunning .*\((?:[^()]*[/\\])?([A-Za-z0-9_-]+)-[a-f0-9]+\)", line)
        doctest = re.fullmatch(r"\s*Doc-tests ([A-Za-z0-9_-]+)\s*", line)
        if running or doctest:
            target = (running or doctest).group(1)
        failed = re.fullmatch(r"test (" + SYMBOL + r") \.\.\. FAILED", line)
        if failed:
            failures.append("rust:" + target + ":" + failed.group(1))
    return counts, failures, bool(summaries)


def node_results(raw, phase, files, macos):
    totals = {key: re.findall(r"^# " + key + r" (\d+)\s*$", raw, re.M)
              for key in ("pass", "fail", "skipped", "tests", "todo", "cancelled")}
    complete = all(len(totals[key]) == 1 for key in ("pass", "fail", "skipped", "tests"))
    values = {key: int(matches[-1]) if matches else 0 for key, matches in totals.items()}
    counts = {"passed": values["pass"], "failed": values["fail"], "ignored": values["skipped"]}
    skipped = re.findall(r"^\s*ok \d+ - (.*?) # SKIP\s*$", raw, re.M)
    allowed = not macos and phase == "web" and len(skipped) == len(set(skipped)) and set(skipped) <= LINUX_SKIPS
    unexpected_skip = values["skipped"] != len(skipped) or bool(skipped) and not allowed
    failures = []
    for block in re.findall(r"^\s*not ok \d+[^\n]*\n(.*?)(?=^\s*(?:not )?ok \d+|\Z)", raw, re.M | re.S):
        identifier = phase + ":suite"
        for location in re.finditer(r"^\s*location:\s*['\"]?(?:file://)?([^\n]*?):(\d+):\d+['\"]?\s*$", block, re.M):
            path, line = location.group(1, 2)
            for file in files:
                if path in (file, str(ROOT / file)) and 0 < int(line) <= len((ROOT / file).read_text().splitlines()):
                    identifier = phase + ":" + file + ":" + line
        failures.append(identifier)
    if unexpected_skip or values["todo"] or values["cancelled"]:
        failures.append(phase + ":unexpected-skip")
    if values["tests"] == 0 or values["tests"] != sum(values[key] for key in ("pass", "fail", "skipped", "todo", "cancelled")):
        complete = False
    return counts, failures, complete


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--macos", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    metadata = {key: os.environ.get(env, "") for key, env in (("run_id", "GITHUB_RUN_ID"), ("run_attempt", "GITHUB_RUN_ATTEMPT"))}
    invalid_metadata = any(not re.fullmatch(r"[0-9]*", value) for value in metadata.values())
    report = {
        "schema_version": 1, "context": "suite-macos-hebdo" if args.macos else "suite-complete-linux",
        "sha": "", "started_at": now(), "finished_at": None, "status": "rouge",
        **{key: value if re.fullmatch(r"[0-9]*", value) else "" for key, value in metadata.items()},
        "counts": dict.fromkeys(("passed", "failed", "ignored", "absent_linux", "partial_linux", "runtime_branches_linux", "excluded"), 0),
        "failures": ["runner:invalid-run-metadata"] if invalid_metadata else [], "failure_detail": None,
        "platform_inventory": {}, "inventory_delta": {}, "phases": {},
        "known_exclusions": [] if args.macos else [{"issue": 528, "platform": "linux", "test": EXCLUDED}],
    }
    report["counts"]["excluded"] = len(report["known_exclusions"])
    save(output, report)
    try:
        logs_parent = output.parent / "logs"
        logs_parent.mkdir(mode=0o700, exist_ok=True)
        logs = Path(tempfile.mkdtemp(prefix="suite-", dir=logs_parent))
        sha = git("rev-parse", "HEAD")
        if not re.fullmatch(r"[a-f0-9]{40,64}", sha):
            raise RuntimeError("sha")
        report["sha"] = sha
        if (os.environ.get("GITHUB_SHA") or sha) != sha:
            raise RuntimeError("expected-sha")
        if args.macos and sys.platform != "darwin" or not args.macos and sys.platform != "linux":
            raise RuntimeError("platform")
        git("diff", "--quiet", "HEAD", "--")
        spec = importlib.util.spec_from_file_location("suite_platform_inventory", ROOT / "tools/suite-platform-inventory.py")
        inventory = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(inventory)
        current = inventory.inventory(ref=report["sha"])
        if current.get("ignored", []):
            report["failures"].append("inventory:ignored-tests")
        baseline = json.loads((ROOT / "docs/suite-platform-baseline.json").read_text())
        report["platform_inventory"] = current
        for key, count in (("absent", "absent_linux"), ("partial", "partial_linux"), ("runtime_branches", "runtime_branches_linux")):
            report["counts"][count] = len(current[key])
            before, after = set(baseline[key]), set(current[key])
            report["inventory_delta"][key] = {"added": sorted(after - before), "removed": sorted(before - after)}
        if any(delta[direction] for delta in report["inventory_delta"].values() for direction in ("added", "removed")):
            report["failures"].append("inventory:drift")
        web = sorted(str(path.relative_to(ROOT)) for path in ROOT.glob("app/web/src/*.test.js"))
        ui = sorted(str(path.relative_to(ROOT)) for pattern in ("app/ui/*.test.cjs", "app/ui/panels/*.test.cjs") for path in ROOT.glob(pattern))
        cargo = ["cargo", "test", "--workspace", "--locked", "--no-fail-fast"]
        if not args.macos:
            cargo += ["--", "--skip", EXCLUDED, "--exact"]
        phases = [("npm", ["npm", "ci", "--prefix", "app/web", "--no-audit", "--no-fund"], []),
                  ("rust", cargo, []),
                  ("web", ["node", "--test", "--test-reporter=tap", *web], web),
                  ("ui", ["node", "--test", "--test-reporter=tap", *ui], ui)]
        env = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target"), "CARGO_TERM_COLOR": "never", "RUSTC_WRAPPER": ""}
        for phase, command, files in phases:
            print("suite phase " + phase, flush=True)
            outcome = {"exit_code": None, "passed": 0, "failed": 0, "ignored": 0}
            report["phases"][phase] = outcome
            if phase == "rust" and shutil.disk_usage(ROOT).free < 20 * 1024**3:
                report["failures"].append("rust:insufficient-disk")
                continue
            if phase in ("web", "ui") and not files:
                report["failures"].append(phase + ":no-test-files")
                continue
            try:
                with (logs / (phase + ".log")).open("wb") as handle:
                    os.chmod(handle.name, 0o600)
                    outcome["exit_code"] = subprocess.run(command, cwd=ROOT, env=env, stdout=handle, stderr=subprocess.STDOUT).returncode
                raw = (logs / (phase + ".log")).read_text(errors="replace")
                if phase != "npm":
                    counts, failures, complete = rust_results(raw) if phase == "rust" else node_results(raw, phase, files, args.macos)
                    for key in ("passed", "failed", "ignored"):
                        outcome[key] = counts[key]
                        report["counts"][key] += counts[key]
                    report["failures"].extend(failures)
                    if not complete or counts["passed"] + counts["failed"] + counts["ignored"] == 0:
                        report["failures"].append(phase + ":missing-summary")
                    if counts["failed"]:
                        report["failures"].append(phase + ":failed-tests")
                    if phase == "rust" and counts["ignored"]:
                        report["failures"].append("rust:unexpected-ignored")
                    if phase == "rust" and counts["filtered"] != (0 if args.macos else 1):
                        report["failures"].append("rust:exclusion-mismatch")
                if outcome["exit_code"] != 0:
                    report["failures"].append(phase + ":command-failed")
            except Exception:
                report["failures"].append(phase + ":runner-error")
            save(output, report)
    except (Exception, KeyboardInterrupt) as error:
        report["failures"].append("runner:preflight-failed")
        if not report["phases"]:
            detail = preflight_failure_detail(error)
            report["failure_detail"] = detail
            print("suite runner:preflight-failed " + detail["type"] + ": " + detail["message"], file=sys.stderr)
    finally:
        if set(report["phases"]) != {"npm", "rust", "web", "ui"} or any(phase["exit_code"] != 0 for phase in report["phases"].values()):
            report["failures"].append("runner:incomplete")
        try:
            if git("rev-parse", "HEAD") != report["sha"]:
                report["failures"].append("runner:sha-changed")
            git("diff", "--quiet", "HEAD", "--")
        except Exception:
            report["failures"].append("runner:tracked-tree-changed")
        report["failures"] = sorted(set(report["failures"]))
        report["finished_at"] = now()
        report["status"] = "rouge" if report["failures"] else "vert"
        save(output, report)
        print("suite " + report["status"] + " " + json.dumps(report["counts"], sort_keys=True), flush=True)
    return 0 if report["status"] == "vert" else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        print("suite runner:error", file=sys.stderr)
        sys.exit(1)
