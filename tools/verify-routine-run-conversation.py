#!/usr/bin/env python3
"""Verify immutable sources, LF excerpts and bounded lexical searches; no JS evaluation."""
from pathlib import Path
from datetime import datetime,timezone
import hashlib,json,re
ROOT=Path(__file__).resolve().parents[1];LEDGER=ROOT/'evidence/routine-run-conversation.json';OUT=ROOT/'evidence/routine-run-conversation-verification.json'
sha=lambda b:hashlib.sha256(b).hexdigest()
def main():
 result={'verified_at_utc':datetime.now(timezone.utc).isoformat(),'network_used':False,'target_executed':False,'javascript_evaluated':False,'mount_used':False,'rea_session_mutated':False,'checks':{}}
 try:
  d=json.loads(LEDGER.read_text());n=0
  for v in d['sources'].values():
   for kind in ('original','formatted'):
    raw=(ROOT/v[kind+'_path']).read_bytes();assert sha(raw)==v[kind+'_sha256'];n+=1
   assert b'\r\n' not in (ROOT/v['formatted_path']).read_bytes()
  result['checks']['source_hashes']=n
  for a in d['anchors']:
   s=d['sources'][a['source']];lines=(ROOT/s['formatted_path']).read_text().split('\n');fragment='\n'.join(lines[a['start_line']-1:a['end_line']]);assert a['literal_anchor'] in fragment and sha(fragment.encode())==a['excerpt_sha256'],a['id']
  result['checks']['anchors']=len(d['anchors'])
  for search in d['bounded_searches']:
   observed=[]
   for s in search['scanned_sources']:
    raw=(ROOT/s['path']).read_bytes();assert sha(raw)==s['sha256'];text=raw.decode();matches=list(re.finditer(search['pattern'],text,search['flags']))
    if matches:observed.append({'path':s['path'],'count':len(matches),'matches':[{'character_offset':m.start(),'text':m[0],'context':text[max(0,m.start()-85):m.end()+125]} for m in matches]})
   assert observed==search['matches'],search['id']
  result['checks']['bounded_searches']=len(d['bounded_searches']);result['checks']['searched_sources']=len(d['bounded_searches'][0]['scanned_sources'])
  for v in d['files'].values():assert sha((ROOT/v['path']).read_bytes())==v['sha256']
  assert sha((ROOT/d['report']).read_bytes())==d['report_sha256'];result['checks']['report_and_proof_hashes']=True;result['status']='passed'
 except Exception as e:
  result['status']='failed';result['error']=repr(e);OUT.write_text(json.dumps(result,indent=2)+'\n');raise
 OUT.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n');print(json.dumps(result))
if __name__=='__main__':main()
