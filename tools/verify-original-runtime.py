#!/usr/bin/env python3
"""Recompute source identity from the retained original-app CDP observation."""
from pathlib import Path
import hashlib,json
ROOT=Path(__file__).resolve().parents[1]
CAPTURE='ev_c53b22e5265cd881f109292c4a6a6d4ad7a227f9fdeade0565c0105795301e08'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    bundle=ROOT/'evidence/remaining-rea-runtime-bundle.json'
    d=json.loads(bundle.read_text())
    r=next(x for x in d['records'] if x['evidence_id']==CAPTURE)
    assert r['operation']=='inspect_electron_page'
    n=r['normalized_result']
    manifest=json.loads((ROOT/'artifacts/manifest.json').read_text())
    m={x['path']:x for x in manifest['entries']}
    prefix=str(ROOT/'inputs/Grok Bot.app/Contents/Resources/app.asar')+'/'
    assert n['target']['file_path']==prefix+'dist/renderer/index.html'
    rows=[]
    for x in n['scripts']['items']:
        assert x['file_path'].startswith(prefix),x['file_path']
        relative=x['file_path'][len(prefix):];src=x['source']
        assert src['included'],relative
        b=src['artifact']['text'].encode()
        actual=hashlib.sha256(b).hexdigest()
        assert len(b)==src['artifact']['bytes']
        assert actual==src['artifact']['sha256']==m[relative]['sha256']==sha(ROOT/'artifacts/client'/relative),relative
        rows.append({'runtime_file_path':x['file_path'],'relative_path':relative,
            'included':True,'captured_sha256':actual,'recomputed_sha256':actual,
            'manifest_sha256':m[relative]['sha256'],'exact_byte_match':True})
    assert len(rows)==n['scripts']['total']==45
    assert sha(ROOT/'inputs/Grok Bot.app/Contents/Resources/app.asar')==manifest['source']['sha256']
    access=json.loads((ROOT/'evidence/original-access-inventory.json').read_text())
    saved=next(x for x in access['apps'] if x['path']==str(ROOT/'inputs/Grok Bot.app'))
    assert sha(ROOT/'inputs/Grok Bot.app/Contents/MacOS/Grok Bot')==saved['executable_sha256']
    assert access['startup_probe']['credentials_entered'] is False
    assert access['startup_probe']['process_present_after_cleanup'] is False
    value={'schema':1,'method':'Independent byte-digest verification of captured script sources against the immutable extracted manifest; this is not a REA graph reconciliation.',
      'capture_evidence_id':CAPTURE,'bundle':{'path':str(bundle.relative_to(ROOT)),'sha256':sha(bundle)},
      'capture_window':n['capture_window'],'target':n['target'],'runtime_observed':True,'authenticated':False,
      'scripts':rows,'counts':{'captured':len(rows),'exact_byte_match':len(rows)},
      'completeness':n['completeness'],
      'limitations':['Only a fresh-profile login screen was observed.',
        'A loaded script does not prove every contained feature executed.',
        'Source matching does not reveal server code, request model context or native iOS implementation.',
        'REA filtered two scripts outside the selected target scope and three without attributable origin.']}
    out=ROOT/'evidence/original-runtime-source-matches.json'
    out.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')
    assert 'Se connecter' in (ROOT/'evidence/original-startup-accessibility.txt').read_text()
    assert (ROOT/'evidence/original-startup-login.png').read_bytes().startswith(b'\x89PNG\r\n\x1a\n')
    proof={'schema':1,'status':'passed','capture_evidence_id':CAPTURE,'captured_scripts_verified':len(rows),
      'authenticated_scenario_observed':False,'source_asar_sha256':manifest['source']['sha256'],
      'files':{str(p.relative_to(ROOT)):sha(p) for p in [bundle,out,ROOT/'evidence/original-access-inventory.json',
          ROOT/'evidence/original-startup-accessibility.txt',ROOT/'evidence/original-startup-login.png']}}
    (ROOT/'evidence/original-runtime-verification.json').write_text(json.dumps(proof,indent=2)+'\n')
    print(json.dumps({k:v for k,v in proof.items() if k!='files'}))
if __name__=='__main__':main()
