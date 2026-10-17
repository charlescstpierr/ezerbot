#!/usr/bin/env python3
"""Verify all nine topics against retained artifacts, not a live Grok account."""
from pathlib import Path
import hashlib,json,re
from evidence_ledger import verify
ROOT=Path(__file__).resolve().parents[1]
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
 aggregate=json.loads((ROOT/'evidence/goal-nine-topics.json').read_text())
 assert aggregate['target_executed'] is False
 assert aggregate['rust_changes_performed'] is False
 assert sorted(t['number'] for t in aggregate['topics'])==list(range(1,10))
 assert len({t['id'] for t in aggregate['topics']})==9
 findings=anchors=0
 for record in aggregate['ledgers']:
  path=ROOT/record['ledger'];assert sha(path)==record['sha256']
  verify(path)
  value=json.loads(path.read_text())
  assert value['target_execution_performed'] is False
  assert all(f['runtime_observed'] is False and f['limitation_fr'] for f in value['findings'])
  assert value['report']==record['report']
  findings+=len(value['findings']);anchors+=len(value['anchors'])
 assert len(aggregate['ledgers'])==4
 assert aggregate['counts']['findings']==findings
 assert aggregate['counts']['anchors']==anchors
 for topic in aggregate['topics']:
  assert topic['status']=='partially_answered'
  assert topic['unresolved_fr'] and topic['established_fr']
  assert (ROOT/topic['report']).is_file() and (ROOT/topic['ledger']).is_file()
 native=json.loads((ROOT/aggregate['native_evidence']).read_text())
 target=Path(native['target']);assert sha(target)==native['target_sha256']
 assert native['target_executed'] is False and native['architecture']=='x86_64'
 assert len(native['records'])==8
 for r in native['records']:
  p=ROOT/r['path'];assert sha(p)==r['sha256']
  assert r['symbol']+':' in p.read_text() and r['exit_code']==0
 fields=native['resolved_field_offsets'];assert len(fields)==6
 binary=target.read_bytes()
 for field in fields:
  pos=field['universal_file_offset'];raw=binary[pos:pos+8]
  assert raw.hex()==field['raw_hex']
  assert int.from_bytes(raw,'little')==field['field_offset']
 assert sha(ROOT/native['field_offset_evidence']['path'])==native['field_offset_evidence']['sha256']
 page=ROOT/'.lavish/goal-neuf-sujets.html';markup=page.read_text()
 assert len(re.findall(r"role='tab' ",markup))==9
 assert len(re.findall(r"role='tabpanel' ",markup))==9
 assert len(re.findall(r"id='report-",markup))==4
 links=0
 for url in re.findall(r'''href=["']([^"']+)["']''',markup):
  if url.startswith(('https://','http://','#','data:')):continue
  assert (page.parent/url).is_file(),url
  links+=1
 result={'status':'passed','scope':'Static evidence, original hashes, anchored excerpts, native bodies and review page links',
  'target_runtime_verified':False,'topics':9,'reports':4,'findings':findings,'anchors':anchors,
  'new_native_function_bodies':8,'native_field_offsets':len(fields),'review_page_links':links,'page_sha256':sha(page)}
 browserpath=ROOT/'evidence/goal-browser-verification.json'
 if browserpath.is_file():
  browser=json.loads(browserpath.read_text())
  if browser.get('status')=='passed' and browser.get('page_sha256')==sha(page):
   result['review_page_browser_verification']='passed'
   result['browser_evidence']='evidence/goal-browser-verification.json'
 (ROOT/'evidence/goal-verification.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps(result,indent=2))
if __name__=='__main__':main()
