import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import schema
import host
from observation import category, write

def execute(root, out, label, command, environment, entry_path):
    return _execute(root, out, label, command, environment, entry_path, ci=False)

def execute_ci(root, out, label, command, environment, entry_path):
    return _execute(root, out, label, command, environment, entry_path, ci=True)

def _execute(root, out, label, command, environment, entry_path, ci):
    entry = dict(schema_version=2, execution_context='unverified-ci' if ci else 'mac-local', label=label, stage='entry', reason='pending', load_before=None,
                 wrapper_return_code=None, child_return_code=None, initial_return_code=None,
                 error_category=None, errno=None, head=None, patch_sha256=None, started=False)
    write(entry_path, entry, schema.run_entry)
    proc = None
    def finish(code, reason, error=None):
        entry.update(wrapper_return_code=code, reason=reason)
        if error is not None:
            entry['error_category'], entry['errno'] = category(error)
        write(entry_path, entry, schema.run_entry)
        return code
    try:
        entry['stage'] = 'precondition'
        load = os.getloadavg()
        entry['load_before'] = list(load)
        write(entry_path, entry, schema.run_entry)
        if ci:
            entry['stage'] = 'host'
            entry['execution_context'] = host.github_actions(root, out, environment, Path(__file__).resolve().parent)
            write(entry_path, entry, schema.run_entry)
        if any((out / (label + suffix)).exists() for suffix in ['.log', '.json', '.done']):
            entry['initial_return_code'] = 21
            return finish(21, 'duplicate_label')
        if not ci and load[1] >= 20:
            entry['initial_return_code'] = 20
            return finish(20, 'load_gate_closed')
        entry['stage'] = 'provenance'
        head = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
        schema.digest(head, 40)
        patch = subprocess.check_output(['git', '-C', str(root), 'diff', '--binary'])
        for name in subprocess.check_output(['git', '-C', str(root), 'ls-files', '--others', '--exclude-standard'], text=True).splitlines():
            if name.startswith('crates/'):
                patch += subprocess.run(['git', '-C', str(root), 'diff', '--no-index', '--binary', '--', '/dev/null', name], stdout=subprocess.PIPE).stdout
        entry.update(head=head, patch_sha256=hashlib.sha256(patch).hexdigest())
        entry['stage'] = 'environment'
        env = {key: environment[key] for key in ['PATH', 'HOME', 'CARGO_HOME', 'RUSTUP_HOME', 'CARGO_BUILD_JOBS', 'CARGO_TERM_COLOR']}
        env.update(CARGO_TARGET_DIR=str(root / 'target'), EZERBOT_GIT_COMMIT=head, TMPDIR=str(out / 'isolated'))
        Path(env['TMPDIR']).mkdir(exist_ok=True)
        if label.startswith('trace'):
            env['I557_TRACE_DIR'] = str(out / (label + '-trace'))
        meta = dict(label=label, execution_context=entry['execution_context'], command=command, cwd=str(root), head=head, patch_sha256=entry['patch_sha256'],
                    load_before=list(load), profile='cargo test dev/debug', started_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()))
        (out / (label + '.json')).write_text(json.dumps(meta, indent=2))
        start = time.monotonic()
        samples = []
        entry['stage'] = 'spawn'
        write(entry_path, entry, schema.run_entry)
        with (out / (label + '.log')).open('wb') as output:
            proc = subprocess.Popen(command, cwd=root, env=env, stdout=output, stderr=subprocess.STDOUT)
            entry.update(stage='running', started=True)
            write(entry_path, entry, schema.run_entry)
            meta['pid'] = proc.pid
            while proc.poll() is None:
                samples.append(dict(seconds=time.monotonic()-start, load=os.getloadavg()))
                time.sleep(1)
            code = proc.returncode
        entry.update(stage='collection', child_return_code=code, initial_return_code=code)
        write(entry_path, entry, schema.run_entry)
        meta.update(exit_code=code, seconds=time.monotonic()-start, load_after=list(os.getloadavg()), load_samples=samples)
        artifacts = []
        for line in (out / (label + '.log')).read_text(errors='replace').splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get('reason') == 'compiler-artifact' and row.get('executable') and row['target']['name'] == 'chat_reliability':
                executable = Path(row['executable'])
                artifacts.append(dict(path=str(executable), sha256=hashlib.sha256(executable.read_bytes()).hexdigest(), profile=row.get('profile')))
        for arg in command:
            if arg in [str(out / 'baseline-executable'), str(out / 'instrumented-executable')]:
                executable = Path(arg)
                artifacts.append(dict(path=arg, sha256=hashlib.sha256(executable.read_bytes()).hexdigest(), profile='archived cargo test dev/debug'))
        meta['executables'] = artifacts
        (out / (label + '.json')).write_text(json.dumps(meta, indent=2))
        (out / (label + '.done')).write_text(str(code) + '\n')
        entry['stage'] = 'finished'
        return finish(code, 'completed')
    except Exception as error:
        if proc is not None:
            entry['child_return_code'] = entry['initial_return_code'] = proc.wait()
        elif isinstance(error, subprocess.CalledProcessError):
            entry['initial_return_code'] = error.returncode
        reason = {'host': 'host_context_invalid', 'precondition': 'precondition_failed', 'provenance': 'provenance_failed',
                  'environment': 'environment_failed', 'spawn': 'spawn_failed',
                  'running': 'collection_failed', 'collection': 'collection_failed'}.get(entry['stage'], 'unexpected_preparation')
        return finish(22 if entry['stage'] == 'spawn' else 24, reason, error)

if __name__ == '__main__':
    out = Path(os.environ['I557_EVIDENCE'])
    label = sys.argv[1]
    assert label in schema.LABELS
    entry = dict(schema_version=2, execution_context='mac-local', label=label, stage='entry', reason='local_runtime_forbidden',
                 load_before=list(os.getloadavg()), wrapper_return_code=25, child_return_code=None, initial_return_code=25,
                 error_category=None, errno=None, head=None, patch_sha256=None, started=False)
    write(out / (label + '-entry.json'), entry, schema.run_entry)
    raise SystemExit(25)
