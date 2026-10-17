#!/usr/bin/env python3
"""Replay selected x64 MCP helper evidence; never launch or emulate target code.

capture invokes only nm/llvm-objdump on the saved file. verify reads saved bytes,
REA metadata, disassemblies, tables and ledger, without network or mounting.
"""
from pathlib import Path
from datetime import datetime,timezone
import argparse,hashlib,json,re,struct,subprocess
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'evidence/releases/0.71.2/native-mcp'
TARGET=ROOT/'inputs/releases/0.71.2/helper/Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService'
EXPECTED='c337d70faba610cbc78f437b9e19be40d8dbd68fb72c59643daf8263ad2a76b3'
TOOL=Path('/Library/Developer/CommandLineTools/usr/bin/llvm-objdump')
sha=lambda b:hashlib.sha256(b).hexdigest()
def save(p,d):p.write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n')
def state():
 binary=TARGET.read_bytes();assert sha(binary)==EXPECTED
 bundle=json.loads((OUT/'rea-macho-bundle.json').read_text());record=next(r for r in bundle['records'] if r.get('operation')=='inspect_macho');assert record['subject']['digest']['sha256']==EXPECTED
 meta=record['normalized_result'];arch=next(a for a in meta['architectures']['items'] if a['name']=='x86_64')
 def offset(vm):
  segment=next(s for s in meta['segments']['items'] if int(s['vm_address'],16)<=vm<int(s['vm_address'],16)+s['file_size'])
  return arch['file_offset']+segment['file_offset']+vm-int(segment['vm_address'],16)
 return binary,record,arch,offset
def assembly_rows(text):
 for number,line in enumerate(text.splitlines(),1):
  match=re.match(r'^([0-9a-f]+):\t((?:[0-9a-f]{2}(?: |\t))+)',line)
  if match:yield number,int(match[1],16),bytes.fromhex(match[2])
def capture():
 binary,_,_,_=state();records=json.loads((OUT/'selected-functions.json').read_text())
 for row in records:
  command=[str(TOOL),'--macho','--arch=x86_64','--disassemble','--dis-symname',row['symbol'],str(TARGET)]
  result=subprocess.run(command,capture_output=True,text=True,check=True);path=ROOT/row['path'];path.write_text(result.stdout);row.update(command=command,sha256=sha(path.read_bytes()),lines=len(result.stdout.splitlines()))
 assert sha(TARGET.read_bytes())==EXPECTED;save(OUT/'selected-functions.json',records)
 print(json.dumps({'functions':len(records),'target_executed':False,'target_emulated':False}))
def verify():
 result={'schema_version':1,'verified_at_utc':datetime.now(timezone.utc).isoformat(),'network_used':False,'mount_used':False,'target_executed':False,'target_emulated':False,'javascript_evaluated':False,'checks':{}}
 try:
  binary,record,arch,offset=state();result['checks']['universal_identity']=EXPECTED
  slice_bytes=binary[arch['file_offset']:arch['file_offset']+arch['size']];result['checks']['x64_slice_sha256']=sha(slice_bytes)
  ledger=json.loads((ROOT/'evidence/native-0712-mcp.json').read_text());assert ledger['target']['x64_slice']['sha256']==sha(slice_bytes)
  records=json.loads((OUT/'selected-functions.json').read_text());byte_rows=0
  for row in records:
   p=ROOT/row['path'];text=p.read_text();assert sha(p.read_bytes())==row['sha256']
   rows=list(assembly_rows(text));assert rows,(row['tag'],'No raw instruction rows')
   for _,vm,raw in rows:
    file_offset=offset(vm);assert binary[file_offset:file_offset+len(raw)]==raw,(row['tag'],hex(vm));byte_rows+=1
   saved=next(r for r in ledger['functions'] if r['tag']==row['tag']);start=saved['byte_extent']['universal_file_offset'];length=saved['byte_extent']['bytes'];assert sha(binary[start:start+length])==saved['byte_extent']['sha256']
  result['checks']['function_records']=len(records);result['checks']['assembly_raw_byte_rows']=byte_rows
  for table in ledger['jump_tables']:
   pos=offset(int(table['vm_address'],16));raw=binary[pos:pos+table['bytes']];assert sha(raw)==table['sha256']
   observed=[hex(int(table['vm_address'],16)+struct.unpack_from('<i',raw,i*4)[0]) for i in range(table['count'])];assert observed==table['targets']
  result['checks']['jump_tables']=len(ledger['jump_tables'])
  for literal in ledger['resolved_literals']:
   pos=offset(int(literal['vm_address'],16));raw=binary[pos:pos+literal['bytes']];assert raw.hex()==literal['raw_hex'] and raw.decode()==literal['value']
  result['checks']['resolved_literals']=len(ledger['resolved_literals'])
  by_tag={r['tag']:r for r in records}
  for anchor in ledger['anchors']:
   path=ROOT/by_tag[anchor['function']]['path'];lines=path.read_text().splitlines();fragment='\n'.join(lines[anchor['start_line']-1:anchor['end_line']]);assert anchor['literal_anchor'] in fragment and sha(fragment.encode())==anchor['excerpt_sha256']
  result['checks']['anchors']=len(ledger['anchors'])
  for file in ledger['files'].values():assert sha((ROOT/file['path']).read_bytes())==file['sha256']
  assert sha((ROOT/ledger['report']).read_bytes())==ledger['report_sha256'];result['checks']['artifact_and_report_hashes']=True
  copy=json.loads((OUT/'copy.json').read_text());assert copy['detach']['exit_code']==0 and copy['owned_mountpoint_removed'];result['checks']['recorded_mount_cleanup']=True
  assert ledger['controls']['rea_session_closed'] is True;result['checks']['rea_session_closed']=True;result['status']='passed'
 except Exception as error:
  result['status']='failed';result['error']=repr(error);save(OUT/'verification.json',result);raise
 save(OUT/'verification.json',result);print(json.dumps(result))
if __name__=='__main__':
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('action',choices=['capture','verify']);args=parser.parse_args()
 capture() if args.action=='capture' else verify()
