import fcntl
from pathlib import Path
import subprocess
import sys

import schema
from observation import category, write

def execute(lock, seconds, receipt, command):
    entry = dict(schema_version=1, stage='entry', reason='pending', wrapper_return_code=None,
                 child_return_code=None, error_category=None, errno=None, started=False)
    write(receipt, entry, schema.gate_entry)
    proc = None
    try:
        entry['stage'] = 'lock'
        with lock.open('a') as handle:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            entry['stage'] = 'spawn'
            write(receipt, entry, schema.gate_entry)
            proc = subprocess.Popen(['perl', '-e', 'alarm shift; exec @ARGV; exit 127', seconds, *command])
            entry.update(stage='running', started=True)
            write(receipt, entry, schema.gate_entry)
            entry['child_return_code'] = proc.wait()
            entry.update(stage='finished', reason='completed', wrapper_return_code=proc.returncode)
    except Exception as error:
        if proc is not None:
            entry['child_return_code'] = proc.wait()
        entry['error_category'], entry['errno'] = category(error)
        entry['reason'] = 'lock_failed' if entry['stage'] == 'lock' else 'spawn_failed'
        entry['wrapper_return_code'] = 31 if entry['stage'] == 'lock' else 32
    write(receipt, entry, schema.gate_entry)
    return entry['wrapper_return_code']

if __name__ == '__main__':
    lock, seconds, receipt, *command = sys.argv[1:]
    raise SystemExit(execute(Path(lock), seconds, Path(receipt), command))
