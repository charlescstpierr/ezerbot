import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

import schema

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def inventory(directory):
    result = []
    for path in sorted(directory.rglob('*')):
        assert not path.is_symlink()
        if path.is_dir():
            assert path.relative_to(directory).as_posix() == 'traces'
            continue
        name = path.relative_to(directory).as_posix()
        assert name in ['receipt.json', 'analysis.json', 'worst.json', 'flushes.json', 'scan.json', 'manifest.json'] or re.fullmatch(r'traces/thread-[1-9][0-9]*\.tsv', name)
        if name != 'manifest.json':
            result.append(dict(file=name, sha256=sha(path), bytes=path.stat().st_size))
    return result

def verify(directory):
    manifest = json.loads((directory / 'manifest.json').read_text())
    schema.fields(manifest, 'schema_version files')
    assert manifest['schema_version'] == 1 and manifest['files'] == inventory(directory)
    schema.receipt(json.loads((directory / 'receipt.json').read_text()))
    if (directory / 'analysis.json').exists():
        data = json.loads((directory / 'analysis.json').read_text())
        schema.fields(data, 'schema_version complete sensor rows measured_record_overhead_ns mean_record_overhead_ns fixtures')
        assert data['schema_version'] == 1 and data['complete'] is True and data['sensor'] == 'i557-tls-v1'
        schema.integer(data['rows'])
        schema.number(data['measured_record_overhead_ns'])
        schema.number(data['mean_record_overhead_ns'])
        assert len(data['fixtures']) == 1
        schema.analysis_fixture(data['fixtures'][0])
        worst = json.loads((directory / 'worst.json').read_text())
        assert type(worst) is list and len(worst) > 0
        for row in worst:
            schema.fields(row, 'ns fixture run request poll phase a b ok thread index')
            schema.numeric_tree(row)
        flushes = json.loads((directory / 'flushes.json').read_text())
        schema.fields(flushes, 'schema_version files rows flushes')
        assert flushes['schema_version'] == 1 and flushes['files'] == len(flushes['flushes'])
        assert sum(r['rows'] for r in flushes['flushes']) == data['rows'] == flushes['rows']
        for row in flushes['flushes']:
            schema.fields(row, 'thread rows ok')
            assert row['ok'] is True
            schema.integer(row['thread'])
            schema.integer(row['rows'])
            path = directory / 'traces' / f'thread-{row["thread"]}.tsv'
            assert schema.trace_file(path.read_bytes(), path.name) == row['rows']
        assert len(list((directory / 'traces').glob('thread-*.tsv'))) == flushes['files']
    scan = json.loads((directory / 'scan.json').read_text())
    schema.fields(scan, 'schema_version scanner_version scanner_sha256 initial_exit_code initial_payload_sha256 final_scan_required_exit_code')
    assert scan['schema_version'] == 1 and scan['scanner_version'] == '8.30.1'
    assert scan['initial_exit_code'] == scan['final_scan_required_exit_code'] == 0
    schema.digest(scan['scanner_sha256'])
    schema.digest(scan['initial_payload_sha256'])
    initial_files = [item for item in manifest['files'] if item['file'] != 'scan.json']
    assert hashlib.sha256(schema.encoded(initial_files)).hexdigest() == scan['initial_payload_sha256']

def build(raw, directory):
    assert not directory.exists()
    directory.mkdir()
    state = json.loads((raw / 'state.json').read_text())
    schema.receipt(state)
    (directory / 'receipt.json').write_bytes(schema.encoded(state))
    label = schema.LABELS[1]
    trace_directory = raw / (label + '-trace')
    if trace_directory.exists():
        assert not trace_directory.is_symlink()
        analysis = json.loads((raw / (label + '-analysis.json')).read_text())
        assert analysis['complete'] is True and analysis['label'] == label and analysis['sensor'] == 'i557-tls-v1'
        assert len(analysis['fixtures']) == 1
        fixture = {k: v for k, v in analysis['fixtures'][0].items() if k != 'worst_artifact'}
        schema.analysis_fixture(fixture)
        assert len(state['configurations']) >= 2
        instrumented = state['configurations'][1]
        assert instrumented['label'] == label
        assert instrumented['visible_count'] == fixture['visible']
        assert instrumented['words'] == [gap['from_words'] for gap in fixture['gaps']] + [40]
        assert instrumented['gaps_ms'] == [(gap['to_ns']-gap['from_ns'])//1_000_000 for gap in fixture['gaps']]
        text = (raw / (label + '.log')).read_text()
        text = re.sub(r'test [a-zA-Z0-9_]+ \.\.\. ok\n', '', text)
        text = text.replace('sensor=i557-tls-v1ok thread=', 'sensor=i557-tls-v1 thread=')
        receipts = re.findall(r'I557_FLUSH sensor=i557-tls-v1 thread=([0-9]+) rows=([0-9]+) ok=(true|false)', text)
        threads = {int(t): (int(n), ok == 'true') for t, n, ok in receipts}
        assert len(threads) == len(receipts)
        traces = list(sorted(trace_directory.iterdir()))
        assert len(traces) == len(analysis['files']) == len(threads)
        (directory / 'traces').mkdir()
        count = 0
        rows = []
        recorded = {Path(f['path']).name: f for f in analysis['files']}
        assert len(recorded) == len(traces)
        for path in traces:
            assert path.is_file() and not path.is_symlink()
            data = path.read_bytes()
            total = schema.trace_file(data, path.name)
            thread = int(path.name[7:-4])
            assert threads[thread] == (total, True)
            assert recorded[path.name]['sha256'] == hashlib.sha256(data).hexdigest()
            assert recorded[path.name]['rows'] == total
            assert Path(recorded[path.name]['path']).resolve() == path.resolve()
            (directory / 'traces' / path.name).write_bytes(data)
            count += total
            rows.append(dict(thread=thread, rows=total, ok=True))
        assert count == analysis['rows']
        exported = dict(schema_version=1, complete=True, sensor='i557-tls-v1', rows=count,
                        measured_record_overhead_ns=analysis['measured_record_overhead_ns'],
                        mean_record_overhead_ns=analysis['mean_record_overhead_ns'], fixtures=[fixture])
        (directory / 'analysis.json').write_bytes(schema.encoded(exported))
        (directory / 'flushes.json').write_bytes(schema.encoded(dict(schema_version=1, files=len(traces), rows=count, flushes=rows)))
        expected = raw / f'{label}-fixture-{fixture["fixture"]}-worst.json'
        assert Path(analysis['fixtures'][0]['worst_artifact']).resolve() == expected.resolve()
        worst = json.loads(expected.read_text())
        for row in worst:
            schema.fields(row, 'ns fixture run request poll phase a b ok thread index')
            schema.numeric_tree(row)
        (directory / 'worst.json').write_bytes(schema.encoded(worst))
    else:
        assert not state['measurement_valid']
    return state

def main():
    raw = Path(sys.argv[1]).resolve()
    scanner = Path(sys.argv[2]).resolve()
    directory = raw / 'export'
    build(raw, directory)
    env = {key: os.environ[key] for key in ['PATH', 'HOME']}
    version = subprocess.check_output([str(scanner), 'version'], env=env, text=True).strip()
    assert version == '8.30.1'
    (raw / 'empty-ignore').write_text('')
    def scan(name):
        command = [str(scanner), 'dir', str(directory), '--redact=100', '--no-banner', '--ignore-gitleaks-allow', '--gitleaks-ignore-path', str(raw / 'empty-ignore')]
        with (raw / (name + '.private')).open('wb') as log:
            result = subprocess.run(command, cwd=raw, env=env, stdout=log, stderr=subprocess.STDOUT)
        assert result.returncode == 0, 'Export scanner rejected bytes'
    initial = hashlib.sha256(schema.encoded(inventory(directory))).hexdigest()
    scan('gitleaks-initial')
    (directory / 'scan.json').write_bytes(schema.encoded(dict(schema_version=1, scanner_version=version,
        scanner_sha256=sha(scanner), initial_exit_code=0, initial_payload_sha256=initial, final_scan_required_exit_code=0)))
    (directory / 'manifest.json').write_bytes(schema.encoded(dict(schema_version=1, files=inventory(directory))))
    verify(directory)
    scan('gitleaks-final')
    verify(directory)
    (raw / 'export-ready.json').write_bytes(schema.encoded(dict(manifest_sha256=sha(directory / 'manifest.json'), final_scan_exit_code=0)))

if __name__ == '__main__':
    main()
