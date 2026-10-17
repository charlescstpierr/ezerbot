#!/usr/bin/env python3
"""Verify saved static local-control evidence; never execute target code."""
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import datetime
import hashlib
import json
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[1]
COMMANDS=[
    ['tools/build-local-cua-path-0712.py','--verify'],
    ['tools/evidence_ledger.py','evidence/local-cua-consent-0712.json'],
    ['evidence/native-arm64-residual-verify.py'],
    ['tools/inspect-native-0712-mcp.py','verify'],
]
REPORTS=['local-cua-path-0712.md','local-cua-consent-0712.md',
         'native-arm64-residual.md','native-0712-mcp.md']
PROOFS=['evidence/local-cua-path-0712.json','evidence/local-cua-consent-0712.json',
        'evidence/native-arm64-residual.json','evidence/native-0712-mcp.json']

def run(args):
    result=subprocess.run([sys.executable,*args],cwd=ROOT,capture_output=True,text=True)
    assert result.returncode==0,(args,result.stdout,result.stderr)
    return {'argv':[sys.executable,*args],'exit_code':result.returncode,
            'result':json.loads(result.stdout)}

def main():
    with ThreadPoolExecutor(max_workers=4) as pool:
        checks=list(pool.map(run,COMMANDS))
    consent=json.loads((ROOT/'evidence/local-cua-consent-0712.json').read_text())
    for item in consent['pinning_manifests']:
        assert hashlib.sha256(Path(item['path']).read_bytes()).hexdigest()==item['sha256_at_creation']
    manifests={version:{e['path']:e['sha256'] for e in json.loads((ROOT/path).read_text())['entries']}
               for version,path in [('0.70.2','artifacts/manifest.json'),
                                     ('0.71.2','artifacts/releases/0.71.2/asar-inventory.json')]}
    for source in consent['sources']:
        version,path=source['id'].split(':',1)
        assert manifests[version][path]==source['original_sha256']
    records=[]
    for path in [*('reports/'+x for x in REPORTS),*PROOFS]:
        raw=(ROOT/path).read_bytes()
        records.append({'path':path,'sha256':hashlib.sha256(raw).hexdigest()})
    value={'schema':1,'status':'passed','checked_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
           'scope':'Static saved Mac artifacts only: original .70 arm64 subset and .71 JavaScript/native MCP/consent',
           'target_executed':False,'network_used':False,'checks':checks,'artifacts':records,
           'consent_sources_manifest_bound':len(consent['sources']),
           'goal_complete':False,'ios_binary_acquired':False,'server_bodies_acquired':False}
    (ROOT/'evidence/local-control-followup-verification.json').write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({'status':'passed','independent_checks':len(checks),'artifacts_hashed':len(records),'target_executed':False}))

if __name__=='__main__':main()
