#!/usr/bin/env python3
"""Verify the Rust core, actual CLI restart, and immutable original evidence."""
from datetime import datetime, timezone
from pathlib import Path
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / 'evidence/rust-verification.json'
entry = shutil.which('cargo') or str(Path.home() / '.cargo/bin/cargo')
environment = os.environ.copy()
environment['PATH'] = str(Path(entry).parent) + os.pathsep + environment.get('PATH', '')


def fingerprint():
    paths = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', Path(__file__).resolve()]
    paths += sorted((ROOT / 'crates').rglob('*.rs'))
    paths += sorted((ROOT / 'crates').rglob('Cargo.toml'))
    paths += sorted(path for directory in ['app/ui', 'app/native']
                    for path in (ROOT / directory).rglob('*')
                    if path.is_file() and path.suffix in ['.html', '.css', '.js', '.m'])
    if (ROOT / 'tools/build-app.sh').exists():
        paths.append(ROOT / 'tools/build-app.sh')
    return {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in paths}


results = []


def check(name, command, expected_exit=0):
    print(f'{name}…', flush=True)
    completed = subprocess.run(command, cwd=ROOT, env=environment,
                               capture_output=True, text=True)
    result = {'name': name, 'command': command, 'exit_code': completed.returncode,
              'expected_exit': expected_exit, 'passed': completed.returncode == expected_exit,
              'stdout': completed.stdout, 'stderr': completed.stderr}
    results.append(result)
    if not result['passed']:
        print((completed.stdout + completed.stderr)[-6000:], file=sys.stderr)
    return result


sources = fingerprint()
check('cargo version', [entry, '--version'])
check('rustc version', [str(Path(entry).parent / 'rustc'), '--version'])
check('format', [entry, 'fmt', '--all', '--', '--check'])
for script in sorted((ROOT / 'app/ui').glob('*.js')):
    check(f'wrapper JavaScript syntax: {script.name}', ['node', '--check', str(script.relative_to(ROOT))])
tests = check('workspace tests', [entry, 'test', '--workspace', '--locked'])
check('clippy', [entry, 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings'])
check('saved evidence', [sys.executable, 'tools/verify-handoff.py', '--evidence-only'])
check('static dossier', [sys.executable, 'tools/verify-dossier.py'])

with tempfile.TemporaryDirectory(prefix='grokbot-verification-') as directory:
    database = Path(directory) / 'session.sqlite'
    command = [entry, 'run', '--locked', '-p', 'grokbot-try', '--']
    demo = check('SQLite demo', command + ['demo', '--db', str(database)])
    status = check('separate process reload', command + ['status', '--db', str(database)])
    expected_rows = ['msg-repetition', 'msg-acceptation', 'couverture 2']
    results.append({'name': 'persisted state visible',
                    'passed': demo['passed'] and status['passed'] and
                    all(row in status['stdout'] for row in expected_rows)})
    before = hashlib.sha256(database.read_bytes()).hexdigest() if database.exists() else None
    second = check('existing database refusal', command + ['demo', '--db', str(database)], 1)
    after = hashlib.sha256(database.read_bytes()).hexdigest() if database.exists() else None
    results.append({'name': 'existing database preserved', 'passed': second['passed'] and
                    before is not None and before == after, 'before': before, 'after': after})

    flow_database = Path(directory) / 'flow.sqlite'
    flow = check('orchestrated flow', command + ['flow', '--db', str(flow_database)])
    flow_status = check('orchestrated flow reload', command + ['status', '--db', str(flow_database)])
    results.append({'name': 'reconciled messages and snapshot visible', 'passed':
                    flow['passed'] and flow_status['passed'] and
                    '2 envois, 2 recherches' in flow['stdout'] and
                    all(value in flow_status['stdout'] for value in
                        ['msg-orchestration-loss', 'msg-orchestration-not-found', 'couverture 2']) and
                    flow_status['stdout'].count('écho reçu') == 2})
    before_flow = hashlib.sha256(flow_database.read_bytes()).hexdigest() if flow_database.exists() else None
    repeated = check('orchestrated database refusal', command + ['flow', '--db', str(flow_database)], 1)
    after_flow = hashlib.sha256(flow_database.read_bytes()).hexdigest() if flow_database.exists() else None
    results.append({'name': 'orchestrated database preserved', 'passed': repeated['passed'] and
                    before_flow is not None and before_flow == after_flow,
                    'before': before_flow, 'after': after_flow})

stable = fingerprint() == sources
results.append({'name': 'source unchanged during verification', 'passed': stable})
report = {
    'status': 'passed' if all(item['passed'] for item in results) else 'failed',
    'verified_at_utc': datetime.now(timezone.utc).isoformat(),
    'scope': 'Independent Rust core and local SQLite scenarios; no original authenticated runtime',
    'authenticated_original_runtime_verified': False,
    'source_files_sha256': sources,
    'passing_tests_including_doctests': sum(int(value) for value in
        re.findall(r'test result: ok\. (\d+) passed;', tests['stdout'])),
    'results': results,
}
REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps({key: report[key] for key in ['status', 'passing_tests_including_doctests']},
                 ensure_ascii=False))
sys.exit(0 if report['status'] == 'passed' else 1)
