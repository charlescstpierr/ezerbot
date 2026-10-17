#!/usr/bin/env python3
"""Verify static communications evidence and the own explanatory page."""
from pathlib import Path
import hashlib,json,re
from evidence_ledger import verify
ROOT=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 agg=json.loads((ROOT/'evidence/communications-investigation.json').read_text())
 assert agg['phone_selected_by_user']=='iPhone'
 assert agg['authenticated_runtime_verified'] is False and agg['native_ios_artifact_available'] is False
 assert agg['rust_changes_performed'] is False
 assert len(agg['reports'])==4 and len(agg['coverage'])==7
 totals={'findings':0,'anchors':0}
 for record in agg['reports']:
  p=ROOT/record['ledger'];assert sha(p)==record['ledger_sha256'];verify(p)
  ledger=json.loads(p.read_text())
  assert ledger['target_execution_performed'] is False
  assert ledger['report']==record['report']
  assert all(f['runtime_observed'] is False and f['limitation_fr'] for f in ledger['findings'])
  for name in totals:totals[name]+=len(ledger[name])
 for name,count in totals.items():assert agg['counts'][name]==count
 catalog_path=ROOT/'evidence/protocol-contracts.json'
 contracts=json.loads((ROOT/agg['contracts']).read_text())
 assert sha(catalog_path)==contracts['catalog_sha256']
 assert contracts['target_executed'] is False
 assert contracts['inherited_inventory_method']['securitySandboxGuarantee'] is False
 catalog=json.loads(catalog_path.read_text())
 messages={m['typeName']:m for m in catalog['messages']}
 for msg in contracts['messages']:
  assert msg==messages[msg['typeName']]
  path=ROOT/'artifacts/formatted'/msg['source']['path']
  assert msg['compactDescriptor'] in path.read_text()
 direct=next(m for m in contracts['messages'] if m['typeName'].endswith('.SendGrokBotAgentMessageRequest'))
 assert {f['jsonName'] for f in direct['fields']}=={'fromAgentId','toAgentId','messageId','text','sentAtMs','replyToId'}
 assert contracts['examples']['reply']['replyToId']==contracts['examples']['direct']['messageId']
 search=json.loads((ROOT/agg['searches']).read_text())
 manifest={r['path']:r['sha256'] for r in json.loads((ROOT/'artifacts/manifest.json').read_text())['entries']}
 assert len(search['files'])==search['file_count']==600
 assert len({r['path'] for r in search['files']})==600
 counts={name:0 for name in search['searches']}
 for record in search['files']:
  path=ROOT/'artifacts/client'/record['path']
  assert sha(path)==record['sha256']==manifest[record['path']]
  text=path.read_text()
  for name,s in search['searches'].items():counts[name]+=sum(1 for _ in re.finditer(s['pattern'],text))
 for name,s in search['searches'].items():assert counts[name]==s['count']==len(s['hits']),name
 assert counts['direct_send_call']==counts['room_turn_call']==counts['apns_specific']==0
 trace=json.loads((ROOT/agg['rea_trace']).read_text())
 if 'result' in trace:trace=trace['result']
 assert trace['normalized_result']['coverage']['status']=='no-match'
 assert trace['normalized_result']['coverage']['source_graph_status']=='partial'
 page=ROOT/'.lavish/communications-grokbot.html';markup=page.read_text()
 assert len(re.findall("role='tab' ",markup))==4
 assert len(re.findall("role='tabpanel'",markup))==4
 assert len(re.findall("id='report-",markup))==4
 assert len(re.findall("data-packet='",markup))==3
 checked=0
 for link in re.findall(r'''href=["']([^"']+)["']''',markup):
  if link.startswith(('https://','http://','#','data:')):continue
  assert (page.parent/link).is_file(),link;checked+=1
 result={'status':'passed','scope':'Saved static artifacts, ledgers, contract projection, bounded search corpus and own HTML links','target_runtime_verified':False,'native_ios_runtime_verified':False,'reports':4,**totals,'messages':len(contracts['messages']),'search_files':600,'review_links':checked,'page_sha256':sha(page)}
 bpath=ROOT/'evidence/communications-browser-verification.json'
 if bpath.exists():
  browser=json.loads(bpath.read_text())
  if browser.get('page_sha256')==sha(page) and browser.get('status')=='passed':result['review_browser_verification']='passed'
 (ROOT/'evidence/communications-verification.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps(result,indent=2))
if __name__=='__main__':main()
