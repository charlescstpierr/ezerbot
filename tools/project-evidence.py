#!/usr/bin/env python3
"""Project retained REA evidence without repeating the original analysis."""
from pathlib import Path
import json

root = Path(__file__).resolve().parents[1]
bundle = json.loads((root / 'evidence/rea-bundle.json').read_text())
parent = next(r for r in bundle['records'] if r['operation'] == 'analyze_javascript_application')
selected_by_identity = {}
for node in parent['normalized_result']['graph']['nodes']:
    if node['kind'] != 'javascript-asset':
        continue
    for obs in node['observations']:
        name = obs.get('label', '')
        if not any(name.startswith(prefix) for prefix in ('dist/electron-main/', 'dist/electron-preload/', 'dist/local-exec-daemon/', 'dist/node-agent-coordinator/')):
            continue
        if '/locales/' in name:
            continue
        props = obs.get('properties', {})
        static = props.get('static_analysis') or {}
        semantic = props.get('semantic_module_analysis') or {}
        key = (node['node_id'], name)
        projected = {'path':name, 'node_id':node['node_id'], 'sha256':node['identity'].get('sha256'),
            'static_keys':list(static),
            'static_counts':{k:len(v) for k,v in static.items() if isinstance(v,(list,dict))},
            'semantic_keys':list(semantic),
            'evidence':obs['evidence'],
            'projection_failure':props.get('application_projection_failure')}
        previous = selected_by_identity.get(key)
        if previous is None or len(projected['static_keys']) > len(previous['static_keys']):
            selected_by_identity[key] = projected
selected = list(selected_by_identity.values())
result = {'parent_evidence_id':parent['evidence_id'],
    'source_sha256':parent['subject']['digest']['sha256'],
    'coverage':parent['normalized_result']['graph']['coverage'],
    'core_files':sorted(selected,key=lambda x:x['path'])}
(root / 'evidence/rea-core-nodes.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'parent_evidence_id':parent['evidence_id'],'core_files':len(selected),
    'nodes':[{'path':x['path'],'node_id':x['node_id'],'static_counts':x['static_counts']} for x in result['core_files']]},indent=2))
