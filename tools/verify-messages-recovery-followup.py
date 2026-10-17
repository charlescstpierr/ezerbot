#!/usr/bin/env python3
"""Verify the static Messages chain and retained controlled replay offline."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import datetime
import hashlib
import json
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def check(arguments):
    completed = subprocess.run([sys.executable, *arguments], cwd=ROOT, text=True, capture_output=True)
    assert completed.returncode == 0, (arguments, completed.stdout, completed.stderr)
    return {'argv': [sys.executable, *arguments], 'result': json.loads(completed.stdout)}


def main():
    commands = [['tools/verify-messages-worker-recovery.py'],
                ['tools/inspect-native-messages.py', 'verify'],
                ['tools/verify-messages-worker-slices.py']]
    with ThreadPoolExecutor(max_workers=3) as pool:
        results = list(pool.map(check, commands))
    paths = ['reports/messages-worker-recovery.md', 'reports/native-messages-recovery.md',
             'reports/messages-worker-slice-replay.md', 'evidence/messages-worker-recovery.json',
             'evidence/native-messages-recovery.json', 'evidence/messages-worker-slice-capture.json',
             'evidence/messages-worker-slice-results.json', 'tools/probe-messages-worker-slices.mjs',
             'tools/verify-messages-worker-slices.py']
    result = {'schema': 1, 'status': 'passed',
              'checked_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'checks': results, 'version': '0.71.2', 'worker_findings': 12, 'worker_anchors': 39,
              'native_functions': 20, 'native_instruction_byte_rows': 4337, 'native_anchors': 33,
              'controlled_replay_slices': 16, 'controlled_replay_cases': 4,
              'rea_evidence_id': 'ev_e0e22e161c96d6a1db6661fbe68e99e006c3a35204228b02374fce15a8585934',
              'verification_evaluates_target': False, 'capture_evaluated_original_slices': True,
              'original_application_executed_by_capture': False, 'native_helper_executed': False,
              'messages_accessed_or_sent': False, 'native_exactly_once_proven': False,
              'product_feature_implemented': False,
              'artifacts': [{'path': p, 'sha256': hashlib.sha256((ROOT / p).read_bytes()).hexdigest()} for p in paths]}
    (ROOT / 'evidence/messages-recovery-followup-verification.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'status': 'passed', 'checks': len(results), 'worker_findings': 12,
                      'native_functions': 20, 'controlled_replay_cases': 4,
                      'verification_evaluates_target': False, 'product_feature_implemented': False}))


if __name__ == '__main__':
    main()
