#!/usr/bin/env python3
"""Capture/verify bounded Messages x64 evidence without running the target."""
from pathlib import Path
from datetime import datetime,timezone
import argparse,hashlib,json,re,subprocess
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'evidence/releases/0.71.2/native-messages'
TARGET=ROOT/'inputs/releases/0.71.2/helper/Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService'
EXPECTED='c337d70faba610cbc78f437b9e19be40d8dbd68fb72c59643daf8263ad2a76b3'
LLVM=Path('/Library/Developer/CommandLineTools/usr/bin/llvm-objdump');sha=lambda b:hashlib.sha256(b).hexdigest()
def save(p,d):p.write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n')
def layout():
 b=TARGET.read_bytes();assert sha(b)==EXPECTED
 r=next(r for r in json.loads((ROOT/'evidence/releases/0.71.2/native-mcp/rea-macho-bundle.json').read_text())['records'] if r.get('operation')=='inspect_macho');assert r['subject']['digest']['sha256']==EXPECTED
 m=r['normalized_result'];a=next(a for a in m['architectures']['items'] if a['name']=='x86_64')
 def off(vm):
  s=next(s for s in m['segments']['items'] if int(s['vm_address'],16)<=vm<int(s['vm_address'],16)+s['file_size']);return a['file_offset']+s['file_offset']+vm-int(s['vm_address'],16)
 return b,a,off
def rows(text):
 for i,line in enumerate(text.splitlines(),1):
  m=re.match(r'^([0-9a-f]+):\t((?:[0-9a-f]{2}(?: |\t))+)',line)
  if m:yield i,int(m[1],16),bytes.fromhex(m[2])
def capture():
 layout();records=json.loads((OUT/'selected-functions.json').read_text())
 for r in records:
  cmd=[str(LLVM),'--macho','--arch=x86_64','--disassemble','--dis-symname',r['symbol'],str(TARGET)];result=subprocess.run(cmd,capture_output=True,text=True,check=True);p=ROOT/r['path'];p.write_text(result.stdout);r.update(command=cmd,sha256=sha(p.read_bytes()),lines=len(result.stdout.splitlines()))
 save(OUT/'selected-functions.json',records);print(json.dumps({'functions':len(records),'target_executed':False}))
def verify():
 result={'verified_at_utc':datetime.now(timezone.utc).isoformat(),'network_used':False,'target_executed':False,'target_emulated':False,'mount_used':False,'checks':{}}
 try:
  b,a,off=layout();ledger=json.loads((ROOT/'evidence/native-messages-recovery.json').read_text());assert ledger['target']['sha256']==EXPECTED
  assert sha(b[a['file_offset']:a['file_offset']+a['size']])==ledger['target']['x64_slice']['sha256'];result['checks']['target_identity']=True
  count=0
  for f in ledger['functions']:
   p=ROOT/f['path'];assert sha(p.read_bytes())==f['sha256'];rr=list(rows(p.read_text()));assert rr
   for _,vm,raw in rr:assert b[off(vm):off(vm)+len(raw)]==raw,(f['tag'],hex(vm));count+=1
   e=f['byte_extent'];assert sha(b[e['file_offset']:e['file_offset']+e['bytes']])==e['sha256']
  result['checks'].update(functions=len(ledger['functions']),instruction_byte_rows=count)
  for v in ledger['resolved_literals']:
   raw=b[off(int(v['vm_address'],16)):off(int(v['vm_address'],16))+v['bytes']];assert raw.hex()==v['raw_hex'] and raw.decode()==v['value']
  result['checks']['resolved_literals']=len(ledger['resolved_literals'])
  for t in ledger['data_tables']:
   raw=b[off(int(t['vm_address'],16)):off(int(t['vm_address'],16))+t['bytes']];assert raw.hex()==t['raw_hex'] and sha(raw)==t['sha256']
   if t.get('kind')=='four Swift small-string argument pairs':
    observed=[]
    for i in range(4):
     pair=[]
     for j in range(2):
      chunk=raw[i*32+j*16:i*32+j*16+16];assert chunk[15]&0xe0==0xe0;pair.append(chunk[:chunk[15]&15].decode())
     observed.append(pair)
    assert observed==t['pairs']
  result['checks']['data_tables']=len(ledger['data_tables'])
  diagnostic=json.loads((OUT/'lexical-diagnostics.json').read_text());slice_bytes=b[a['file_offset']:a['file_offset']+a['size']];symbol_text=(ROOT/'evidence/releases/0.71.2/native-mcp/symbols-x64.txt').read_text();asm_text='\n'.join((ROOT/f['path']).read_text() for f in json.loads((OUT/'selected-functions.json').read_text()))
  for q in diagnostic['patterns_case_insensitive']:
   assert len(re.findall(q.encode(),slice_bytes,re.I))==diagnostic['slice_ascii_matches'][q]
   assert len(re.findall(q,symbol_text,re.I))==diagnostic['symbol_matches'][q]
   assert len(re.findall(q,asm_text,re.I))==diagnostic['selected_asm_matches'][q]
  result['checks']['lexical_diagnostics_only']=True
  for v in ledger['anchors']:
   lines=(ROOT/v['path']).read_text().split('\n');s='\n'.join(lines[v['start_line']-1:v['end_line']]);assert v['literal_anchor'] in s and sha(s.encode())==v['excerpt_sha256']
  result['checks']['anchors']=len(ledger['anchors'])
  for v in ledger['files'].values():assert sha((ROOT/v['path']).read_bytes())==v['sha256']
  assert sha((ROOT/ledger['report']).read_bytes())==ledger['report_sha256'];result['checks']['artifact_hashes']=True;result['status']='passed'
 except Exception as e:
  result['status']='failed';result['error']=repr(e);save(OUT/'verification.json',result);raise
 save(OUT/'verification.json',result);print(json.dumps(result))
if __name__=='__main__':
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['capture','verify']);a=p.parse_args();capture() if a.action=='capture' else verify()
