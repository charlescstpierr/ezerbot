#!/usr/bin/env python3
"""Record or verify saved files and symlinks without launching the target."""
from pathlib import Path
import argparse
import hashlib
import json
import os

root = Path(__file__).resolve().parents[1]
manifest_path = root/'evidence/handoff-manifest.json'
result_path = root/'evidence/handoff-verification.json'
excluded = {manifest_path, result_path}

def inventory():
    items = []
    for path in sorted(root.rglob('*')):
        if path in excluded or '__pycache__' in path.parts:
            continue
        relative = str(path.relative_to(root))
        if path.is_symlink():
            if not path.resolve().is_relative_to(root):
                raise ValueError(f'Symlink escapes handoff: {relative}')
            items.append({'path':relative, 'kind':'symlink', 'target':os.readlink(path)})
        elif path.is_file():
            digest = hashlib.sha256()
            with path.open('rb') as source:
                for chunk in iter(lambda:source.read(1024*1024), b''):
                    digest.update(chunk)
            items.append({'path':relative, 'kind':'file', 'bytes':path.stat().st_size,
                'sha256':digest.hexdigest()})
    return items

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--record', action='store_true', help='Create baseline; refuse an existing baseline')
parser.add_argument('--evidence-only', action='store_true',
    help='Verify immutable saved artifacts and evidence against the historical baseline')
args = parser.parse_args()
if args.evidence_only:
    if args.record:
        parser.error('--evidence-only cannot replace the historical baseline')
    baseline = json.loads(manifest_path.read_text())['entries']
    generated = {'evidence/dossier-verification.json', 'evidence/handoff-verification.json'}
    saved = [item for item in baseline if item['path'].startswith(('inputs/', 'artifacts/', 'evidence/'))
        and item['path'] not in generated]
    if not saved:
        raise SystemExit('No saved evidence in historical baseline')
    changes = []
    for item in saved:
        path = root/item['path']
        if item['kind'] == 'symlink':
            valid = path.is_symlink() and os.readlink(path) == item['target']
            valid = valid and path.resolve().is_relative_to(root)
        elif item['kind'] == 'file' and path.is_file() and not path.is_symlink():
            digest = hashlib.sha256()
            with path.open('rb') as source:
                for chunk in iter(lambda: source.read(1024*1024), b''):
                    digest.update(chunk)
            valid = path.stat().st_size == item['bytes'] and digest.hexdigest() == item['sha256']
        else:
            valid = False
        if not valid:
            changes.append(item['path'])
    if changes:
        raise SystemExit('Saved evidence changed: '+json.dumps(changes, ensure_ascii=False))
    print(json.dumps({'status': 'passed', 'saved_entries_verified': len(saved),
        'scope': 'Historical saved artifact and evidence bytes; evolving source/docs excluded'},
        ensure_ascii=False))
    raise SystemExit(0)
current = inventory()
if args.record:
    if manifest_path.exists():
        raise SystemExit('Baseline exists. Verify it instead of replacing it.')
    manifest_path.write_text(json.dumps({'format':1, 'root_at_save':str(root),
        'exclusions':['evidence/handoff-manifest.json','evidence/handoff-verification.json','__pycache__'],
        'entries':current},ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({'status':'baseline-recorded','entries':len(current)}))
else:
    baseline = json.loads(manifest_path.read_text())['entries']
    if baseline != current:
        before = {item['path']:item for item in baseline}
        after = {item['path']:item for item in current}
        changes = [key for key in sorted(before.keys()|after.keys()) if before.get(key)!=after.get(key)]
        raise SystemExit('Saved files changed: '+json.dumps(changes,ensure_ascii=False))
    features = json.loads((root/'evidence/parity.json').read_text())['features']
    state = json.loads((root/'SESSION_STATE.json').read_text())
    assert len(features)==45
    assert len(state['pending_feature_implementation'])==45
    assert {f['id'] for f in features}=={f['id'] for f in state['pending_feature_implementation']}
    assert state['rust_product_implemented'] is False
    assert state['authenticated_runtime_verified'] is False
    assert (root/'MANQUES_COMPLETS.md').read_text().count('| Non implémenté |')==45
    result = {'status':'passed', 'saved_entries_verified':len(current),
        'regular_files':sum(item['kind']=='file' for item in current),
        'symlinks':sum(item['kind']=='symlink' for item in current),
        'regular_file_bytes':sum(item.get('bytes',0) for item in current),
        'pending_feature_families':45,
        'scope':'Saved files, links and handoff scope; no product runtime or Rust compilation'}
    result_path.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps(result,ensure_ascii=False,indent=2))
