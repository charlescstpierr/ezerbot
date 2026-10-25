import fcntl
from pathlib import Path
import subprocess
import sys

lock, seconds, *command = sys.argv[1:]
with Path(lock).open('a') as handle:
    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    proc = subprocess.Popen(['perl', '-e', 'alarm shift; exec @ARGV', seconds, *command])
    raise SystemExit(proc.wait())
