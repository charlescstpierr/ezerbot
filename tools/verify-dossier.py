#!/usr/bin/env python3
"""Verify target identity, extracted bytes, evidence links and design coverage."""
from pathlib import Path
import hashlib
import json
import re

root=Path(__file__).resolve().parents[1]
def read(name):return json.loads((root/name).read_text())
manifest=read('artifacts/manifest.json')
summary=read('evidence/rea-summary.json')
contracts=read('evidence/protocol-contracts.json')
parity=read('evidence/parity.json')
saved_source=root/'inputs/Grok Bot.app/Contents/Resources/app.asar'
source=saved_source if saved_source.exists() else Path(manifest['source']['path'])
assert hashlib.sha256(source.read_bytes()).hexdigest()==manifest['source']['sha256']
assert summary['artifact']['sha256']==manifest['source']['sha256']
assert summary['summary']['statistics']['parsed_javascript_files']==600
assert summary['summary']['statistics']['parse_failures']==0
assert summary['summary']['coverage']['application']['status']=='partial'
assert summary['summary']['coverage']['semantic']['status']=='partial'
assert len(manifest['entries'])==673
contradictions=[]
for item in manifest['entries']:
    assert item['kind']=='file'
    p=root/'artifacts/client'/item['path']
    assert p.stat().st_size==item['size'],item['path']
    assert hashlib.sha256(p.read_bytes()).hexdigest()==item['sha256'],item['path']
    if item['integrity']=='observed-untrusted':contradictions.append(item['path'])
assert len(contradictions)==6
assert hashlib.sha256((root/'artifacts/client'/contracts['source']['path']).read_bytes()).hexdigest()==contracts['source']['sha256']
assert contracts['descriptorRecovery']['executesTargetJavaScript'] is True
assert contracts['descriptorRecovery']['applicationLaunched'] is False
assert contracts['descriptorRecovery']['securitySandboxGuarantee'] is False
assert contracts['counts']=={'fullServices':7,'fullServiceMethods':484,'uniqueRpcTypePairs':429,'messages':2313,'enums':287,'filesScanned':599}
assert len(parity['features'])==45
ids={f['id'] for f in parity['features']}
assert len(ids)==len(parity['features'])
assert {'teaching','spending','hosting','routines','closed-backend','mobile','native-internals'}.issubset(ids)
assert all(f['status'] in parity['status_legend'] for f in parity['features'])
architecture=(root/'reports/architecture-rust.md').read_text()
for required in ('InlinePrompt','SkillInvocation','SpendingService','HostingService','DemonstrationSession','OutcomeUnknown','ReplicaEpoch','UTF-16','24 kHz','AccountRuntime'):
    assert required in architecture,required
assert 'Une routine référence une version de skill' not in architecture
assert 'MeetingsService' not in (root/'evidence/parity.json').read_text()
assert read('evidence/runtime-status.json')['runtime_evidence_id'] is None
assert read('evidence/computer-use-helper.json')['outside_asar'] is True
assert read('evidence/computer-use-helper.json')['asar_integrity_contradiction'] is False
assert read('evidence/electron-fuses.json')['wires'][0]['fuses']['EnableNodeCliInspectArguments']=='disabled'
checked=0
for report in sorted((root/'reports').glob('*.md')):
    text=report.read_text()
    assert not re.search(r'\b(TODO|TBD)\b',text),report.name
    for target in re.findall(r'\]\(([^)]+)\)',text):
        target=target.strip('<>')
        if target.startswith(('https://','http://','#')):continue
        match=re.fullmatch(r'(.+):(\d+)',target)
        path,line=(match.group(1),int(match.group(2))) if match else (target,None)
        p=Path(path)
        if not p.is_absolute():p=report.parent/p
        assert p.exists(),(report.name,target)
        if line:assert 1<=line<=len(p.read_text().split('\n')),(report.name,target)
        checked+=1
result={'status':'passed','scope':'Static artifact identity, inventory, links, coverage and design consistency',
    'application_runtime_verified':False,'rust_implementation_verified':False,
    'source_sha256':manifest['source']['sha256'],'extracted_files_verified':673,
    'native_integrity_contradictions_preserved':len(contradictions),
    'report_file_links_verified':checked,'feature_families':len(parity['features']),
    'checks':['Original ASAR hash matches retained REA evidence.','Every extracted file hash/size matches the manifest.','Controlled descriptor evaluation is labelled explicitly.','Declared RPC/message/enum counts match the inventory.','Spending, hosting, teaching and prompt-only routines are included.','All linked local report sources exist with bounded line anchors.','Runtime, backend and native unknowns remain explicit.']}
(root/'evidence/dossier-verification.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
