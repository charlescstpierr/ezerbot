#!/usr/bin/env python3
"""Verify retained pure cron replay without evaluating any target JavaScript."""
from pathlib import Path
import hashlib
import json
import re

ROOT = Path(__file__).resolve().parents[1]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    capture = json.loads((ROOT / 'evidence/cron-candidate-capture.json').read_bytes())
    result = json.loads((ROOT / 'evidence/cron-candidate-results.json').read_bytes())
    assert capture['evidence_id'] == 'ev_ec367c9e76a03287bd7c8cc1e891d989cf2b9d47671f790d1136a8d95dd608ab'
    assert capture['operation'] == 'capture_process_scenario'
    n = capture['normalized_result']
    assert n['exit']['code'] == 0 and n['exit']['reason'] == 'exited'
    assert n['truncated'] is False and n['settlement']['state'] == 'quiesced'
    scenario = n['manifest']['scenario']
    assert scenario['working_directory'] == str(ROOT)
    assert scenario['executable'] == '/Users/cstpierrecharles/.local/opt/node-v24.18.0-darwin-x64/bin/node'
    assert scenario['arguments'] == ['tools/probe-cron-candidate-slices.mjs',
        '/Users/cstpierrecharles/.npm/_npx/dea9475797100b20/node_modules/rea-agents/package.json']
    raw = n['truncation_details']['raw_terminal']
    assert raw['observed_bytes'] == raw['retained_bytes'] == 9709
    assert raw['observed_frames'] == raw['retained_frames'] == len(n['frames']) == 10
    rendered = n['truncation_details']['rendered_terminal']
    assert rendered['observed_frames'] == rendered['retained_frames'] == len(n['rendered_frames']) == 10
    assert rendered['observed_bytes'] == rendered['retained_bytes']
    assert sum(len(f['data'].encode()) for f in n['frames']) == raw['retained_bytes']
    assert json.loads(''.join(f['data'] for f in n['frames'])) == result
    assert n['cleanup']['owned_process_group'] == 'verified' and n['cleanup']['temporary_root'] == 'removed'
    source = result['source']
    original = (ROOT / source['path']).read_bytes()
    assert len(original) == source['bytes'] and sha(original) == source['sha256']
    manifest_raw = (ROOT / source['manifest_path']).read_bytes()
    assert sha(manifest_raw) == source['manifest_sha256']
    manifest = json.loads(manifest_raw)
    assert manifest['source']['sha256'] == source['asar_sha256']
    assert next(e['sha256'] for e in manifest['entries'] if e['path'] == source['path'].removeprefix('artifacts/client/')) == source['sha256']
    assert sha((ROOT / 'tools/probe-cron-candidate-slices.mjs').read_bytes()) == result['harness_sha256']
    assert len(result['slices']) == 26
    assert len({s['name'] for s in result['slices']}) == 26
    utf16 = original.decode().encode('utf-16-le')
    program = []
    for section in result['slices']:
        selected = original[section['start_byte']:section['end_byte']]
        assert sha(selected) == section['sha256']
        assert utf16[2*section['start_utf16']:2*section['end_utf16']].decode('utf-16-le') == selected.decode()
        assert original[:section['start_byte']].count(b'\n') + 1 == section['raw_line']
        program.append(('var ' if section['type'] == 'VariableDeclarator' else '') + selected.decode() + ';')
    assert sha(('\n'.join(program) + '\nthis.selected = { mne, mi, mR };').encode()) == result['selected_program_sha256']
    expected = {'spring-gap': '2026-03-09T06:30:00.000Z', 'fall-first': '2026-11-01T05:30:00.000Z',
        'fall-second': '2026-11-01T06:30:00.000Z', 'leap-beyond-horizon': None,
        'leap-in-horizon': '2028-02-29T00:00:00.000Z', 'restricted-days-or-date': '2026-10-13T00:00:00.000Z',
        'restricted-days-or-weekday': '2026-10-19T00:00:00.000Z', 'sunday-seven': '2026-10-18T00:00:00.000Z',
        'strict-next-minute': '2026-10-17T10:01:00.000Z', 'next-minute-fraction': '2026-10-17T10:01:00.000Z',
        'interval-decimal': '2026-01-01T00:00:00.500Z', 'interval-phase': '2026-01-01T01:15:00.000Z',
        'interval-timezone-prefix-rejected': None, 'invalid-timezone-rejected': None,
        'alias-case-insensitive': '2026-01-02T00:00:00.000Z', 'seconds-field-rejected': None}
    assert len(result['cases']) == len(expected) == 16
    assert {c['id']: c['observed'] for c in result['cases']} == expected
    assert all(c['observed'] == c['expected'] for c in result['cases'])
    assert result['assertions_passed'] and result['selected_original_javascript_executed']
    assert not any(result[k] for k in ['original_renderer_bootstrapped', 'scheduler_executed', 'routine_created_or_run'])
    # Named direct call count is bounded evidence, never proof of no dynamic caller.
    assert len(re.findall(rb'\bmne\(', original)) == 2
    print(json.dumps({'status': 'passed', 'evidence_id': capture['evidence_id'], 'original_slices': 26,
        'cases': 16, 'raw_frames': 10, 'terminal_complete': True, 'scheduler_executed': False,
        'offline_verifier_evaluates_target': False}))


if __name__ == '__main__':
    main()
