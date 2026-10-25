"""Observe the expected child command before recording its process identity."""

from __future__ import annotations

import subprocess
import time
from contextlib import contextmanager
from pathlib import Path

import verify_lib


def wait_for_command(process: subprocess.Popen, command: list[str], timeout: float = 5) -> dict:
    expected = [Path(command[0]).name, *command[1:]]
    deadline = time.monotonic() + timeout
    while process.poll() is None:
        identity = verify_lib.proc_identity(process.pid)
        words = (identity or {}).get("command", "").split()
        if len(words) >= len(expected):
            observed = words[-len(expected):]
            observed[0] = Path(observed[0]).name
            if observed == expected:
                return identity
        if time.monotonic() >= deadline:
            raise AssertionError("la commande enfant attendue n'est pas observable")
        time.sleep(0.01)
    raise AssertionError("l'enfant est terminé avant l'observation de sa commande")


@contextmanager
def recorded_child(command: list[str], *, start_new_session: bool = False):
    process = subprocess.Popen(command, start_new_session=start_new_session)
    try:
        yield process, wait_for_command(process, command)
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=5)
