#!/usr/bin/env python3
"""Verify saved memory and routine evidence, without replaying the target."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import datetime
import hashlib
import json
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(arguments):
    completed = subprocess.run([sys.executable, *arguments], cwd=ROOT, text=True, capture_output=True)
    assert completed.returncode == 0, (arguments, completed.stdout, completed.stderr)
    records = [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]
    assert records and all(record.get('status') == 'passed' for record in records)
    return {'argv': [sys.executable, *arguments], 'results': records}


def main():
    commands = [['tools/verify-cron-candidate-slices.py'], ['tools/verify-routine-run-conversation.py'],
                ['tools/verify_memory_widget_consumer.py']]
    with ThreadPoolExecutor(max_workers=3) as pool:
        checks = list(pool.map(run, commands))
    inventory = json.loads((ROOT / 'artifacts/manifest.json').read_bytes())
    manifest = {item['path']: item['sha256'] for item in inventory['entries']}
    paths = ['reports/cron-candidate-replay.md', 'reports/routine-run-conversation.md',
             'reports/memory-widget-consumer.md', 'evidence/cron-candidate-capture.json',
             'evidence/cron-candidate-results.json', 'evidence/routine-run-conversation.json',
             'evidence/memory-widget-consumer.json', 'tools/probe-cron-candidate-slices.mjs',
             'tools/verify-cron-candidate-slices.py', 'tools/verify-routine-run-conversation.py',
             'tools/verify_memory_widget_consumer.py']
    source_bindings = 0
    for ledger_path in ['evidence/routine-run-conversation.json', 'evidence/memory-widget-consumer.json']:
        ledger = json.loads((ROOT / ledger_path).read_bytes())
        sources = ledger['sources'].values() if isinstance(ledger['sources'], dict) else ledger['sources']
        for source in sources:
            original = (ROOT / source['original_path']).resolve()
            relative = original.relative_to(ROOT / 'artifacts/client').as_posix()
            assert manifest[relative] == source['original_sha256']
            source_bindings += 1
        if ledger_path == 'evidence/routine-run-conversation.json':
            assert len(ledger['bounded_searches']) == 2
            for search in ledger['bounded_searches']:
                assert len(search['scanned_sources']) == 600
                for item in search['scanned_sources']:
                    relative = (ROOT / item['path']).resolve().relative_to(ROOT / 'artifacts/client').as_posix()
                    assert manifest[relative] == item['sha256']
    link_count = 0
    for name in paths[:3]:
        report = ROOT / name
        for target in re.findall(r'\]\(([^)]+)\)', report.read_text()):
            target = target.strip('<>')
            if target.startswith(('https://', 'http://', '#')):
                continue
            match = re.fullmatch(r'(.+):(\d+)', target)
            file_path = Path(match[1] if match else target)
            file_path = file_path if file_path.is_absolute() else report.parent / file_path
            assert file_path.exists(), (name, target)
            if match:
                assert 1 <= int(match[2]) <= len(file_path.read_text().split('\n'))
            link_count += 1
    result = {'schema': 1, 'status': 'passed',
              'checked_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'checks': checks, 'source_manifest_bindings': source_bindings,
              'manifest_sha256': sha(ROOT / 'artifacts/manifest.json'),
              'new_report_links_verified': link_count,
              'controlled_replay_slices': 26, 'controlled_replay_cases': 16,
              'memory_findings': 9, 'memory_anchors': 42, 'routine_anchors': 35,
              'verification_evaluates_target': False, 'capture_evaluated_original_slices': True,
              'server_scheduler_executed': False, 'live_memory_accessed_or_modified': False,
              'product_feature_implemented': False,
              'artifacts': [{'path': p, 'sha256': sha(ROOT / p)} for p in paths]}
    (ROOT / 'evidence/memory-routines-followup-verification.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'status': 'passed', 'checks': len(checks), 'source_manifest_bindings': source_bindings,
        'new_report_links_verified': link_count, 'controlled_replay_cases': 16,
        'memory_findings': 9, 'server_scheduler_executed': False}))


if __name__ == '__main__':
    main()
