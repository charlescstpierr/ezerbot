#!/usr/bin/env python3
"""Offline verification of a retained REA capture, without evaluating target JS."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    capture_path = ROOT / 'evidence/messages-worker-slice-capture.json'
    results_path = ROOT / 'evidence/messages-worker-slice-results.json'
    capture = json.loads(capture_path.read_bytes())
    result = json.loads(results_path.read_bytes())
    observed = capture['normalized_result']
    assert capture['evidence_id'] == 'ev_e0e22e161c96d6a1db6661fbe68e99e006c3a35204228b02374fce15a8585934'
    assert capture['operation'] == 'capture_process_scenario'
    assert observed['exit']['code'] == 0 and observed['exit']['reason'] == 'exited'
    scenario = observed['manifest']['scenario']
    assert scenario['working_directory'] == str(ROOT)
    assert scenario['arguments'] == ['tools/probe-messages-worker-slices.mjs',
        '/Users/cstpierrecharles/.npm/_npx/dea9475797100b20/node_modules/rea-agents/package.json']
    assert scenario['executable'] == '/Users/cstpierrecharles/.local/opt/node-v24.18.0-darwin-x64/bin/node'
    assert scenario['filesystem_observation_paths'] == [] and scenario['events'] == []
    raw = observed['truncation_details']['raw_terminal']
    assert raw['observed_bytes'] == raw['retained_bytes'] == 9075
    assert sum(len(frame['data'].encode('utf-8')) for frame in observed['frames']) == raw['retained_bytes']
    assert raw['observed_frames'] == raw['retained_frames'] == len(observed['frames'])
    assert json.loads(''.join(f['data'] for f in observed['frames'])) == result
    assert observed['cleanup']['owned_process_group'] == 'verified'
    assert observed['cleanup']['temporary_root'] == 'removed'
    # Retain the distinction: rendered-terminal budget loss, complete raw JSON.
    assert observed['truncated'] is True
    assert observed['truncation_details']['rendered_terminal']['retained_frames'] == 7
    assert observed['cleanup']['unverified_processes']
    source = result['source']
    original = (ROOT / source['path']).read_bytes()
    assert len(original) == source['bytes'] and sha(original) == source['sha256']
    inventory_raw = (ROOT / source['inventory_path']).read_bytes()
    assert sha(inventory_raw) == source['inventory_sha256']
    inventory = json.loads(inventory_raw)
    entry = next(e for e in inventory['entries'] if e['path'] == 'dist/local-exec-daemon/messages-worker.cjs')
    assert entry['sha256'] == source['sha256']
    assert inventory['asar_sha256'] == source['asar_sha256']
    assert sha((ROOT / 'tools/probe-messages-worker-slices.mjs').read_bytes()) == result['harness_sha256']
    program = []
    assert len(result['slices']) == 16
    text = original.decode('utf-8')
    for section in result['slices']:
        selected = original[section['start_byte']:section['end_byte']]
        assert sha(selected) == section['sha256']
        # Babel offsets count UTF-16 code units; byte ranges are separate.
        utf16 = text.encode('utf-16-le')
        assert utf16[section['start_utf16'] * 2:section['end_utf16'] * 2].decode('utf-16-le') == selected.decode('utf-8')
        assert original[:section['start_byte']].count(b'\n') + 1 == section['raw_line']
        program.append(('var ' if section['type'] == 'VariableDeclarator' else '') + selected.decode('utf-8') + ';')
    selected_program = '\n'.join(program) + '\nthis.selected = { ze, Vr, Hr };'
    assert sha(selected_program.encode()) == result['selected_program_sha256']
    assert result['assertions_passed'] is True
    assert result['selected_original_js_executed'] is True
    for key in ['original_application_executed', 'native_helper_executed', 'messages_accessed_or_sent']:
        assert result[key] is False
    cases = {case['id']: case for case in result['cases']}
    independent = cases['independent-calls']
    assert independent['fixture_effects'] == 2
    assert len({a['envelope']['requestID'] for a in independent['socket_attempts']}) == 2
    for attempt in independent['socket_attempts']:
        envelope = attempt['envelope']
        assert set(envelope) == {'arguments', 'id', 'method', 'requestID'}
        assert set(envelope['arguments']) == {'to', 'service', 'textFile'}
        assert envelope['id'] == 1 and envelope['method'] == 'messages/send'
        assert attempt['timeout_ms'] is None
    retry = cases['reconnect-before-effect']
    assert retry['fixture_effects'] == 1
    assert len(retry['socket_attempts']) == 2
    assert retry['socket_attempts'][0]['serialized'] == retry['socket_attempts'][1]['serialized']
    assert len(retry['launch_calls']) == 1 and retry['delays_ms'] == [200]
    lost = cases['fixture-effect-then-lost-reply']
    assert lost['first_error'] == 'sidecar_closed' and lost['fixture_effects'] == 2
    assert len(lost['socket_attempts']) == 2
    assert lost['socket_attempts'][0]['envelope']['requestID'] != lost['socket_attempts'][1]['envelope']['requestID']
    assert not lost['launch_calls'] and not lost['delays_ms']
    assert cases['deadline-selector']['deadlines'] == {'send': 'undefined', 'snapshot': 10000,
        'resolve': 4000, 'permissions_with_prompt_true': 'undefined', 'permissions_with_prompt_false': 'undefined'}
    print(json.dumps({'status': 'passed', 'evidence_id': capture['evidence_id'], 'slices': 16,
        'cases': 4, 'raw_terminal_complete': True, 'rendered_terminal_complete': False,
        'original_application_executed': False, 'offline_verifier_evaluates_target': False}))


if __name__ == '__main__':
    main()
