#!/usr/bin/env python3
"""Run the hosted macOS Rust checks and publish only a bounded JSON receipt."""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import selectors
import shutil
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
PHASES = (
    ("fmt", "check", ("cargo", "fmt", "--all", "--check"), 300),
    ("clippy_debug", "debug", ("cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"), 2100),
    ("clippy_release", "release", ("cargo", "clippy", "--workspace", "--all-targets", "--release", "--locked", "--", "-D", "warnings"), 2100),
    ("test_workspace", "debug", ("cargo", "test", "--workspace", "--locked", "--no-fail-fast"), 2700),
)
MAX_REPORT = 1024 * 1024
MAX_OUTPUT = 16 * 1024 * 1024
MAX_SOURCE = 32 * 1024 * 1024
MAX_ITEMS = 2048
TOTAL_SECONDS = 6000
SHA = re.compile(r"[a-f0-9]{40}")
REPOSITORY = re.compile(r"[A-Za-z0-9_.-]{1,100}/[A-Za-z0-9_.-]{1,100}")
SAFE_PATH = re.compile(r"[A-Za-z0-9_./-]{1,240}")
SYMBOL = r"[A-Za-z_][A-Za-z0-9_]*"
SUMMARY = re.compile(r"test result: (ok|FAILED)\. (\d{1,9}) passed; (\d{1,9}) failed; (\d{1,9}) ignored; (\d{1,9}) measured; (\d{1,9}) filtered out;(?: finished in [0-9.]+s)?\s*")
COUNTS = ("passed", "failed", "ignored", "filtered")


class InvalidReceipt(Exception):
    pass


class Interrupted(Exception):
    pass


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z")


def initial_report():
    return {
        "schema_version": 1, "context": "rust-macos", "status": "rouge",
        "provenance": None, "started_at": now(), "finished_at": None,
        "reasons": [], "inventory_complete": False,
        "cache": {"key": None, "restore_outcome": None, "hit": None,
                  "saved": None, "bytes_before": None, "bytes_after": None},
        "phases": [
            {"id": name, "command": list(command), "profile": profile,
             "state": "not_run", "exit_code": None, "duration_ms": None,
             "counts": None, "summary_complete": None, "summaries": [],
             "failures": [], "unresolved_failures": 0, "reasons": []}
            for name, profile, command, _ in PHASES
        ],
    }


def save(output, report):
    data = (json.dumps(report, sort_keys=True, indent=2) + "\n").encode("utf-8")
    if len(data) > MAX_REPORT:
        report["status"] = "rouge"
        report["inventory_complete"] = False
        report["reasons"] = sorted(set(report["reasons"] + ["report_limit"]))
        for phase in report["phases"]:
            phase["failures"] = []
            phase["summaries"] = []
            if phase["summary_complete"] is not None:
                phase["summary_complete"] = False
        data = (json.dumps(report, sort_keys=True) + "\n").encode("utf-8")
    if len(data) > MAX_REPORT:
        raise InvalidReceipt()
    output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = output.with_name(output.name + ".new")
    with temporary.open("wb") as handle:
        os.chmod(temporary, 0o600)
        handle.write(data)
    os.replace(temporary, output)


def stop_process(process):
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        return process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        return process.wait(timeout=3)


def execute(command, root, env, seconds, output_limit=MAX_OUTPUT, disk_root=None):
    started = time.monotonic()
    try:
        process = subprocess.Popen(command, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   start_new_session=True)
    except OSError:
        return {"state": "not_run", "exit_code": None, "duration_ms": None,
                "raw": b"", "reasons": ["launch_error"]}
    raw, reasons = bytearray(), []
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                if disk_root is not None and shutil.disk_usage(disk_root).free < 1024**3:
                    reasons.append("disk_budget")
                    break
                if time.monotonic() - started >= seconds:
                    reasons.append("timeout")
                    break
                for key, _ in selector.select(timeout=min(0.2, max(0, seconds - (time.monotonic() - started)))):
                    block = os.read(key.fd, 65536)
                    if not block:
                        selector.unregister(key.fileobj)
                        continue
                    remaining = output_limit - len(raw)
                    raw.extend(block[:remaining])
                    if len(block) > remaining:
                        reasons.append("output_limit")
                        break
                if reasons:
                    break
            if not reasons:
                try:
                    process.wait(timeout=max(0.01, seconds - (time.monotonic() - started)))
                except subprocess.TimeoutExpired:
                    reasons.append("timeout")
    except (KeyboardInterrupt, Interrupted):
        reasons.append("interrupted")
    except Exception:
        reasons.append("runner_error")
    finally:
        if reasons or process.poll() is None:
            code = stop_process(process)
        else:
            code = process.returncode
        process.stdout.close()
    return {"state": "executed", "exit_code": code,
            "duration_ms": max(0, round((time.monotonic() - started) * 1000)),
            "raw": bytes(raw), "reasons": reasons}


def capture(command, root, env, seconds=60):
    result = execute(command, root, env, seconds, 1024 * 1024)
    if result["exit_code"] != 0 or result["reasons"]:
        raise InvalidReceipt()
    return result["raw"].decode("utf-8", errors="strict").strip()


def host_guard(env, root):
    if not (env.get("GITHUB_ACTIONS") == "true" and
            env.get("RUNNER_ENVIRONMENT") == "github-hosted" and
            env.get("RUNNER_OS") == "macOS" and platform.system() == "Darwin" and
            env.get("GITHUB_JOB") == "rust-macos" and
            Path(env.get("GITHUB_WORKSPACE", "")).resolve() == root.resolve()):
        raise InvalidReceipt()


def identities(env, root):
    host_guard(env, root)
    event_name = env.get("GITHUB_EVENT_NAME")
    if event_name not in ("pull_request", "push"):
        raise InvalidReceipt()
    event_path = Path(env["GITHUB_EVENT_PATH"])
    if event_path.stat().st_size > MAX_REPORT:
        raise InvalidReceipt()
    event = json.loads(event_path.read_text())
    repository = env.get("GITHUB_REPOSITORY", "")
    if not REPOSITORY.fullmatch(repository) or event["repository"]["full_name"] != repository:
        raise InvalidReceipt()
    event_sha = env.get("GITHUB_SHA", "")
    workflow_sha = env.get("GITHUB_WORKFLOW_SHA", "")
    workflow_ref = env.get("GITHUB_WORKFLOW_REF", "")
    if event_name == "pull_request":
        source_sha = event["pull_request"]["head"]["sha"]
        source_repository = event["pull_request"]["head"]["repo"]["full_name"]
        number = event["number"]
        if type(number) is not int or number <= 0 or env.get("GITHUB_REF") != "refs/pull/{}/merge".format(number):
            raise InvalidReceipt()
        expected_ref = repository + "/.github/workflows/rust-macos.yml@refs/pull/{}/merge".format(number)
    else:
        if event.get("ref") != "refs/heads/main" or env.get("GITHUB_REF") != "refs/heads/main":
            raise InvalidReceipt()
        source_sha = event["after"]
        source_repository = repository
        expected_ref = repository + "/.github/workflows/rust-macos.yml@refs/heads/main"
        if source_sha != event_sha:
            raise InvalidReceipt()
    if (not all(isinstance(value, str) and SHA.fullmatch(value) for value in (source_sha, event_sha, workflow_sha)) or
            not REPOSITORY.fullmatch(source_repository) or workflow_ref != expected_ref):
        raise InvalidReceipt()
    values = [env.get("GITHUB_RUN_ID", ""), env.get("GITHUB_RUN_ATTEMPT", "")]
    if not all(re.fullmatch(r"[1-9][0-9]{0,19}", value) for value in values):
        raise InvalidReceipt()
    return {"source_repository": source_repository, "source_sha": source_sha,
            "workflow_repository": repository, "workflow_sha": workflow_sha,
            "workflow_ref": workflow_ref, "event_sha": event_sha, "event": event_name,
            "run_id": int(values[0]), "run_attempt": int(values[1]), "job_name": "rust-macos"}


def fingerprint(root, env, expected):
    head = capture(["git", "rev-parse", "HEAD"], root, env)
    tree = capture(["git", "rev-parse", "HEAD^{tree}"], root, env)
    if head != expected or not SHA.fullmatch(tree):
        raise InvalidReceipt()
    capture(["git", "diff", "--quiet", "HEAD", "--"], root, env)
    capture(["git", "ls-files", "--error-unmatch", "Cargo.lock", "rust-toolchain.toml"], root, env)
    if any((root / name).is_symlink() for name in ("Cargo.lock", "rust-toolchain.toml")):
        raise InvalidReceipt()
    return {"checkout_sha": head, "tree_sha": tree,
            "lock_sha256": hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest()}


def cargo_environment(env, home, target):
    clean = dict(env)
    for key in list(clean):
        if key.startswith(("CARGO_", "RUST", "DYLD_", "LD_")):
            del clean[key]
    clean.update({"CARGO_HOME": str(home), "CARGO_TARGET_DIR": str(target),
                  "CARGO_BUILD_JOBS": "3", "CARGO_INCREMENTAL": "0",
                  "CARGO_TERM_COLOR": "never", "RUSTC_WRAPPER": "",
                  "RUST_BACKTRACE": "0"})
    return clean


def preflight(env, root, fresh):
    provenance = identities(env, root)
    runner_temp = Path(env["RUNNER_TEMP"]).resolve(strict=True)
    if not runner_temp.is_dir() or runner_temp == root or root in runner_temp.parents:
        raise InvalidReceipt()
    work = runner_temp / "rust-macos-{}-{}".format(provenance["run_id"], provenance["run_attempt"])
    home, target = work / "cargo-home", work / "target"
    if fresh:
        home.mkdir(parents=True, mode=0o700, exist_ok=False)
    elif not home.is_dir() or home.is_symlink():
        raise InvalidReceipt()
    child_env = cargo_environment(env, home, target)
    provenance.update(fingerprint(root, child_env, provenance["source_sha"]))
    config = (root / "rust-toolchain.toml").read_text()
    channel = re.search(r'^channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$', config, re.M)
    components = re.search(r'^components\s*=\s*\[\s*"rustfmt"\s*,\s*"clippy"\s*\]\s*$', config, re.M)
    if not channel or not components:
        raise InvalidReceipt()
    machine = platform.machine()
    arch = {"arm64": ("ARM64", "aarch64-apple-darwin"),
            "x86_64": ("X64", "x86_64-apple-darwin")}.get(machine)
    if not arch or env.get("RUNNER_ARCH") != arch[0] or capture(["uname", "-m"], root, child_env) != machine:
        raise InvalidReceipt()
    os_version = capture(["sw_vers", "-productVersion"], root, child_env)
    if not re.fullmatch(r"[0-9]{1,3}(?:\.[0-9]{1,3}){1,2}", os_version):
        raise InvalidReceipt()
    rust = capture(["rustc", "-Vv"], root, child_env, 480)
    release = re.search(r"^release: ([0-9]+\.[0-9]+\.[0-9]+)$", rust, re.M)
    commit = re.search(r"^commit-hash: ([a-f0-9]{40})$", rust, re.M)
    host = re.search(r"^host: ([a-z0-9_-]+)$", rust, re.M)
    cargo = re.fullmatch(r"cargo ([0-9]+\.[0-9]+\.[0-9]+) \(([a-f0-9]{7,40}) [0-9]{4}-[0-9]{2}-[0-9]{2}\)", capture(["cargo", "-V"], root, child_env))
    installed = capture(["rustup", "component", "list", "--installed"], root, child_env)
    if (not release or release[1] != channel[1] or not commit or not host or host[1] != arch[1] or not cargo or
            not all(name + "-" + arch[1] in installed.splitlines() for name in ("rustfmt", "clippy"))):
        raise InvalidReceipt()
    provenance["toolchain"] = {"channel": channel[1], "rustc_release": release[1],
                               "rustc_commit_hash": commit[1], "rustc_host": host[1],
                               "cargo_version": cargo[1], "cargo_commit": cargo[2],
                               "components": ["rustfmt", "clippy"]}
    provenance["os"] = {"name": "macOS", "version": os_version, "runner_label": "macos-latest"}
    for key in ("ImageOS", "ImageVersion"):
        value = env.get(key)
        if value is not None:
            if not re.fullmatch(r"[A-Za-z0-9_.-]{1,80}", value):
                raise InvalidReceipt()
            provenance["os"][key] = value
    provenance["architecture"] = {"machine": machine, "runner_arch": arch[0], "rust_host": host[1]}
    tool_hash = hashlib.sha256(json.dumps(provenance["toolchain"], sort_keys=True).encode()).hexdigest()
    cache_key = "rust-macos-v1-macOS-{}-{}-{}-{}-{}-exact".format(machine, tool_hash, provenance["lock_sha256"], provenance["source_sha"], provenance["run_id"])
    return provenance, child_env, home, cache_key


def source_inventory(root, env):
    tracked = capture(["git", "ls-files", "-z"], root, env).split("\0")
    symbols, targets, files = {}, set(), {}
    total = 0
    for name in tracked:
        path = Path(name)
        if not name or not SAFE_PATH.fullmatch(name) or ".." in path.parts:
            continue
        if path.suffix != ".rs" and path.name != "Cargo.toml":
            continue
        if (root / path).is_symlink():
            raise InvalidReceipt()
        size = (root / path).stat().st_size
        total += size
        if size > 4 * 1024 * 1024 or total > MAX_SOURCE:
            raise InvalidReceipt()
        source = (root / path).read_text()
        if path.suffix == ".rs":
            files[name] = len(source.splitlines())
            targets.add(path.stem.replace("-", "_"))
            for symbol in re.findall(r"\bfn\s+(" + SYMBOL + r")\s*\(", source):
                symbols.setdefault(symbol, set()).add(name + "::" + symbol)
        else:
            targets.update(value.replace("-", "_") for value in re.findall(r'^name\s*=\s*"([A-Za-z0-9_-]{1,100})"\s*$', source, re.M))
    if not files or not targets or len(files) > MAX_ITEMS:
        raise InvalidReceipt()
    return {"symbols": symbols, "targets": targets, "files": files}


def parse_test_stream(raw, inventory):
    summaries, failures, reasons = [], set(), set()
    current, unresolved = None, 0
    for line in raw.decode("utf-8", errors="replace").splitlines():
        if len(line) > 65536:
            reasons.add("output_limit")
            continue
        running = re.fullmatch(r"\s*Running .+ \((?:[^()]*[/\\])?([A-Za-z0-9_-]+)-[a-f0-9]{7,40}(?:\.exe)?\)\s*", line)
        doctest = re.fullmatch(r"\s*Doc-tests ([A-Za-z0-9_-]+)\s*", line)
        if running or doctest:
            name = (running or doctest)[1].replace("-", "_")
            current = None
            if name not in inventory["targets"]:
                reasons.add("unrecognized_target")
            elif len(summaries) >= MAX_ITEMS:
                reasons.add("inventory_limit")
            else:
                current = {"kind": "binary" if running else "doctest", "target": name,
                           "counts": None, "expected": None, "summary_complete": False}
                summaries.append(current)
            continue
        expected = re.fullmatch(r"running (\d{1,9}) tests?", line)
        if expected:
            if current is None:
                reasons.add("unbound_summary")
            elif current["counts"] is not None:
                if len(summaries) >= MAX_ITEMS:
                    reasons.add("inventory_limit")
                    current = None
                else:
                    current = {"kind": current["kind"], "target": current["target"],
                               "counts": None, "expected": int(expected[1]), "summary_complete": False}
                    summaries.append(current)
            elif current["expected"] is not None:
                reasons.add("missing_summary")
            else:
                current["expected"] = int(expected[1])
            continue
        summary = SUMMARY.fullmatch(line)
        if summary:
            if current is None or current["counts"] is not None:
                reasons.add("unbound_summary")
                continue
            passed, failed, ignored, measured, filtered = map(int, summary.groups()[1:])
            current["counts"] = dict(zip(COUNTS, (passed, failed, ignored, filtered)))
            current["summary_complete"] = (current["expected"] == passed + failed + ignored + measured and
                                            (summary[1] == "FAILED") == (failed > 0) and measured == 0)
            if measured:
                reasons.add("measured_tests")
            continue
        failed = re.fullmatch(r"test ([A-Za-z_][A-Za-z0-9_:]*) \.\.\. FAILED", line)
        doc_failed = re.fullmatch(r"test ([A-Za-z0-9_./-]+\.rs) - (?:[A-Za-z0-9_:]+ )?\(line (\d{1,9})\) \.\.\. FAILED", line)
        panic = re.fullmatch(r"thread '[^\n]{1,512}' panicked at ([A-Za-z0-9_./-]+\.rs):(\d{1,9}):\d{1,9}:", line)
        if failed:
            matches = inventory["symbols"].get(failed[1].split("::")[-1], set())
            if len(matches) == 1:
                failures.update(matches)
            else:
                unresolved += 1
        elif doc_failed:
            name, number = doc_failed[1], int(doc_failed[2])
            if name in inventory["files"] and 0 < number <= inventory["files"][name]:
                failures.add(name + ":" + str(number))
            else:
                unresolved += 1
        elif panic:
            name, number = panic[1], int(panic[2])
            if name in inventory["files"] and 0 < number <= inventory["files"][name]:
                failures.add(name + ":" + str(number))
        if len(failures) > MAX_ITEMS:
            failures = set(sorted(failures)[:MAX_ITEMS])
            reasons.add("inventory_limit")
    observed = [row["counts"] for row in summaries if row["counts"] is not None]
    counts = {key: sum(row[key] for row in observed) for key in COUNTS} if observed else None
    if failures and (counts is None or counts["failed"] == 0):
        reasons.add("failed_tests")
    complete = bool(summaries) and all(row["summary_complete"] for row in summaries) and not reasons
    if not complete:
        reasons.add("missing_summary")
    if not counts or sum(counts[key] for key in ("passed", "failed", "ignored")) == 0:
        reasons.add("empty_tests")
    if counts:
        for key, reason in (("failed", "failed_tests"), ("ignored", "ignored_tests"), ("filtered", "filtered_tests")):
            if counts[key]:
                reasons.add(reason)
        if counts["failed"] > len(failures) + unresolved:
            unresolved += counts["failed"] - len(failures) - unresolved
    if unresolved:
        reasons.add("unresolved_failure_identifier")
    return {"counts": counts, "summary_complete": complete, "summaries": summaries,
            "failures": sorted(failures), "unresolved_failures": unresolved, "reasons": sorted(reasons)}


def cache_bytes(home):
    cache = home / "registry" / "cache"
    total, count = 0, 0
    if (home / "registry").is_symlink() or cache.is_symlink():
        raise InvalidReceipt()
    if cache.exists():
        for directory, dirs, files in os.walk(cache, followlinks=False):
            for name in dirs + files:
                path = Path(directory) / name
                if path.is_symlink():
                    raise InvalidReceipt()
            for name in files:
                count += 1
                total += (Path(directory) / name).stat().st_size
                if count > 100000 or total > 10 * 1024**3:
                    raise InvalidReceipt()
    return total


def discard_archives(home):
    registry, cache = home / "registry", home / "registry" / "cache"
    if registry.is_symlink():
        registry.unlink()
    elif cache.is_symlink():
        cache.unlink()
    elif cache.exists():
        shutil.rmtree(cache)


def finish(report, output, provenance, root, env):
    if provenance is not None:
        try:
            after = fingerprint(root, env, provenance["source_sha"])
            if any(after[key] != provenance[key] for key in after):
                report["reasons"].append("checkout_changed")
        except Exception:
            report["reasons"].append("checkout_changed")
    for phase in report["phases"]:
        if phase["state"] == "not_run" and not phase["reasons"]:
            phase["reasons"] = ["preflight_failed" if provenance is None else
                                "interrupted" if "interrupted" in report["reasons"] else "runner_error"]
    if any(phase["state"] != "executed" or phase["exit_code"] != 0 or phase["reasons"] for phase in report["phases"]):
        report["reasons"].append("required_phase_failed")
    if not report["inventory_complete"]:
        report["reasons"].append("inventory_incomplete")
    report["reasons"] = sorted(set(report["reasons"]))
    report["finished_at"] = now()
    report["status"] = "rouge" if report["reasons"] else "vert"
    save(output, report)
    return 0 if report["status"] == "vert" else 1


def prepare(output, env, root=ROOT):
    report = initial_report()
    save(output, report)
    try:
        provenance, child_env, home, key = preflight(env, root, True)
        report["provenance"] = provenance
        report["cache"]["key"] = key
        if fingerprint(root, child_env, provenance["source_sha"]) != {key: provenance[key] for key in ("checkout_sha", "tree_sha", "lock_sha256")}:
            raise InvalidReceipt()
        save(output, report)
        with Path(env["GITHUB_OUTPUT"]).open("a") as handle:
            handle.write("cargo_home={}\ncache_key={}\n".format(home, key))
        print("rust-ci prepare rc=0", flush=True)
        return 0
    except (Exception, KeyboardInterrupt):
        report["provenance"] = None
        report["reasons"] = ["preflight_failed"]
        for phase in report["phases"]:
            phase["reasons"] = ["preflight_failed"]
        report["finished_at"] = now()
        save(output, report)
        print("rust-ci prepare rc=1", flush=True)
        return 1


def run(output, env, cache_outcome, cache_hit, root=ROOT, cache_matched_key=""):
    started = time.monotonic()
    report = initial_report()
    provenance, child_env = None, env
    try:
        if output.stat().st_size > MAX_REPORT:
            raise InvalidReceipt()
        previous = json.loads(output.read_text())
        save(output, report)
        provenance, child_env, home, key = preflight(env, root, False)
        if previous["provenance"] != provenance or previous["reasons"] or previous["cache"]["key"] != key:
            provenance = None
            raise InvalidReceipt()
        report["provenance"] = provenance
        report["started_at"] = previous["started_at"]
        report["cache"].update({"key": key, "restore_outcome": cache_outcome,
                                  "hit": {"true": True, "false": False, "": False}[cache_hit]})
        if cache_outcome != "success":
            report["cache"]["hit"] = None
            report["reasons"].append("cache_restore_failed")
            discard_archives(home)
        elif ((cache_hit == "true" and cache_matched_key != key) or
              (cache_matched_key and (cache_matched_key != key or cache_hit != "true"))):
            report["cache"]["hit"] = None
            report["reasons"].append("cache_key_mismatch")
            discard_archives(home)
        report["cache"]["bytes_before"] = cache_bytes(home)
        try:
            inventory = source_inventory(root, child_env)
            report["inventory_complete"] = True
        except Exception:
            inventory = {"symbols": {}, "targets": set(), "files": {}}
            report["reasons"].append("inventory_incomplete")
        save(output, report)
        interrupted = False
        for phase, (_, _, command, seconds) in zip(report["phases"], PHASES):
            remaining = TOTAL_SECONDS - (time.monotonic() - started)
            if interrupted or remaining <= 0:
                phase["reasons"] = ["interrupted" if interrupted else "budget_exhausted"]
            elif shutil.disk_usage(home).free < 2 * 1024**3:
                phase["reasons"] = ["disk_budget"]
            else:
                result = execute(command, root, child_env, min(seconds, remaining), disk_root=home)
                phase.update({key: result[key] for key in ("state", "exit_code", "duration_ms", "reasons")})
                if phase["state"] == "executed" and phase["id"] == "test_workspace":
                    parsed = parse_test_stream(result["raw"], inventory)
                    phase["reasons"] = sorted(set(phase["reasons"] + parsed.pop("reasons")))
                    phase.update(parsed)
                if phase["state"] == "executed" and phase["exit_code"] != 0:
                    phase["reasons"] = sorted(set(phase["reasons"] + ["command_failed"]))
                if "output_limit" in phase["reasons"] or "inventory_limit" in phase["reasons"]:
                    report["inventory_complete"] = False
                interrupted = "interrupted" in phase["reasons"]
            save(output, report)
            print("rust-ci phase={} state={} rc={}".format(phase["id"], phase["state"], phase["exit_code"]), flush=True)
        report["cache"]["bytes_after"] = cache_bytes(home)
    except (KeyboardInterrupt, Interrupted):
        report["reasons"].append("interrupted")
    except Exception:
        report["reasons"].append("preflight_failed" if report["provenance"] is None else "runner_error")
    finally:
        result = finish(report, output, provenance, root, child_env)
    print("rust-ci result rc={}".format(result), flush=True)
    return result


def main():
    class SafeParser(argparse.ArgumentParser):
        def error(self, _message):
            raise InvalidReceipt()

    parser = SafeParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prepare", action="store_true")
    parser.add_argument("--cache-outcome", choices=("success", "failure", "skipped", "cancelled"), default="skipped")
    parser.add_argument("--cache-hit", choices=("true", "false", ""), default="")
    parser.add_argument("--cache-matched-key", default="")
    args = parser.parse_args()
    output = args.output.resolve()
    if args.prepare:
        return prepare(output, dict(os.environ))
    return run(output, dict(os.environ), args.cache_outcome, args.cache_hit, cache_matched_key=args.cache_matched_key)


if __name__ == "__main__":
    def interrupt(_signum, _frame):
        raise Interrupted()

    signal.signal(signal.SIGTERM, interrupt)
    signal.signal(signal.SIGINT, interrupt)
    try:
        sys.exit(main())
    except (Exception, KeyboardInterrupt):
        print("rust-ci runner rc=1", flush=True)
        sys.exit(1)
