#!/usr/bin/env python3
"""Bounded offline verification; never import or evaluate target JavaScript."""
import json
import re
import sys
sys.dont_write_bytecode = True
from versioned_evidence_ledger import ROOT, sha, verify

PROOF = 'evidence/messages-worker-recovery.json'
D = 'dist/local-exec-daemon/main.cjs'
W = 'dist/local-exec-daemon/messages-worker.cjs'


def main():
    result = verify(PROOF)
    proof = json.loads((ROOT / PROOF).read_text())
    assert sha((ROOT / proof['replay_verifier']).read_bytes()) == proof['replay_verifier_sha256']
    rea = proof['saved_rea']
    bundle_raw = (ROOT / rea['bundle_path']).read_bytes()
    assert sha(bundle_raw) == rea['bundle_sha256']
    bundle = json.loads(bundle_raw)
    record = next(r for r in bundle['records'] if r['evidence_id'] == rea['inventory_evidence_id'])
    assert record['operation'] == 'inventory_artifact'
    graph = record['normalized_result']
    assert graph['manifest']['manifest_id'] == rea['manifest_id']
    nodes = {n['artifact_id']: n for n in graph['nodes']}
    occurrences = {o['occurrence_id']: o for o in graph['occurrences']}
    for binding in rea['source_bindings']:
        occurrence = occurrences[binding['occurrence_id']]
        assert occurrence['logical_path'] == binding['logical_path']
        assert occurrence['hash_status'] == 'verified'
        assert occurrence['logical_path'].endswith('/' + binding['source'])
        node = nodes[occurrence['artifact_id']]
        source = next(s for s in proof['sources'] if s['id'] == binding['source'])
        assert node['sha256'] == binding['original_sha256'] == source['original_sha256']

    bodies = {s['id']: (ROOT / s['formatted_path']).read_text() for s in proof['sources']}

    def excerpt(path, first, last):
        return '\n'.join(bodies[path].split('\n')[first-1:last])

    wrapper = excerpt(D, 128041, 128078)
    assert 'r ??= i()' in wrapper and 'a.pending.clear()' in wrapper
    assert 'messages_worker_failed' in wrapper and 'messages_worker_exited' in wrapper
    assert 'r.worker.postMessage(d)' in wrapper
    assert not any(token in wrapper for token in ['setTimeout', '.terminate(', 'sendKey', 'requestId'])
    deadline = excerpt(D, 128080, 128082)
    assert 'Hoe(t) ? void 0 : Date.now() + _Pe(t)' in deadline
    assert len(list(re.finditer(r'\b_Pe\(', bodies[D]))) == 2
    worker_loop = excerpt(W, 2576, 2602)
    assert 'Date.now() > r' in worker_loop and 'ns(e).then(rs)' in worker_loop
    assert not any(token in worker_loop for token in ['setTimeout', 'sendKey', 'requestId'])
    helper_args = excerpt(W, 2448, 2452)
    assert 'textFile' in helper_args
    assert not any(token in helper_args for token in ['sendKey', 'requestId', 'requestID'])
    helper_call = excerpt(W, 2374, 2393)
    assert helper_call.index('randomUUID') < helper_call.index('for (;;)')
    assert 'b !== "ENOENT" && b !== "ECONNREFUSED"' in helper_call
    assert 'e === "send-message" || "prompt" in t' in excerpt(W, 2423, 2426)
    assert 'i.setTimeout(n ?? 0)' in excerpt(W, 2336, 2369)
    report = (ROOT / proof['report']).read_text()
    for finding in proof['findings']:
        assert finding['id'] in report
        for anchor in finding['anchors']:
            assert anchor in {a['id'] for a in proof['anchors']}
    assert {s['id'] for s in proof['sources']} == {D, W}
    result.update(saved_rea_bindings=2, bounded_static_assertions='passed',
                  target_javascript_evaluated=False, target_executed=False,
                  network_used=False, files_written=False)
    print(json.dumps(result, ensure_ascii=False))


if __name__ == '__main__':
    main()
