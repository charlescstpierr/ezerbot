#!/usr/bin/env python3
"""Inspect saved helper lease functions without executing the target."""
from pathlib import Path
import hashlib,json,re,subprocess,struct
ROOT=Path(__file__).resolve().parents[1]
TARGET=ROOT/'inputs/Grok Bot.app/Contents/Helpers/Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService'
TOOL=Path('/Library/Developer/CommandLineTools/usr/bin/llvm-objdump')
PATTERNS={
 'start_entry':r'CURemoteControlLeaseC5start.*YaKFTY0_$',
 'start_ready':r'CURemoteControlLeaseC5start.*stoppedtyXEfU0_$',
 'start_locked':r'CURemoteControlLeaseC5start.*activationTasktSgyXEfU_$',
 'watch_holder':r'CURemoteControlLeaseC15watchHolderExit.*tF$',
 'revoke_locked':r'CURemoteControlLeaseC12revokeLocked.*_tF$',
 'admit_locked':r'CURemoteControlLeaseC5admit.*yXEfU_$',
 'release_locked':r'CURemoteControlLeaseC7release.*yXEfU_$',
 'holder_exit':r'CURemoteControlLeaseC15watchHolderExit.*tFyycfU_$',
}
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 before=sha(TARGET)
 symbols=[]
 table=ROOT/'evidence/native-helper-x64-symbol-table.txt'
 for line in table.read_text().split('\n'):
  m=re.match(r'([0-9a-f]{16}) ([tT]) (.+)',line)
  if m:symbols.append({'address':m[1],'symbol':m[3]})
 records=[]
 for tag,pattern in PATTERNS.items():
  matches=[r for r in symbols if re.search(pattern,r['symbol'])]
  assert len(matches)==1,(tag,len(matches),matches)
  seed=matches[0]
  cmd=[str(TOOL),'--macho','--arch=x86_64','--disassemble','--dis-symname',seed['symbol'],str(TARGET)]
  result=subprocess.run(cmd,capture_output=True,text=True,check=True)
  output=ROOT/'evidence'/('native-lease-'+tag+'.asm.txt')
  output.write_text(result.stdout)
  assert seed['symbol']+':' in result.stdout
  records.append({'id':tag,**seed,'path':str(output.relative_to(ROOT)),'sha256':sha(output),'command':cmd,'exit_code':result.returncode})
 assert sha(TARGET)==before
 metadata=json.loads((ROOT/'evidence/native-helper-macho.json').read_text())['normalized_result']
 arch=next(a for a in metadata['architectures']['items'] if a['name']=='x86_64')
 fields=[]
 binary=TARGET.read_bytes()
 for line in table.read_text().split('\n'):
  if 'CURemoteControlLeaseC' not in line or not line.endswith('vpWvd'):continue
  addr=int(line.split()[0],16)
  seg=next(s for s in metadata['segments']['items'] if int(s['vm_address'],16)<=addr<int(s['vm_address'],16)+s['vm_size'])
  offset=arch['file_offset']+seg['file_offset']+addr-int(seg['vm_address'],16)
  raw=binary[offset:offset+8]
  fields.append({'symbol':line.split()[-1],'vm_address':hex(addr),'universal_file_offset':offset,
   'field_offset':struct.unpack('<Q',raw)[0],'raw_hex':raw.hex()})
 fieldpath=ROOT/'evidence/native-lease-field-offsets.txt'
 fieldpath.write_text('x86_64 field offsets read from saved Mach-O (little endian uint64)\n'+
  '\n'.join(f"{f['symbol']} field_offset={hex(f['field_offset'])} universal_file_offset={f['universal_file_offset']} raw={f['raw_hex']}" for f in fields)+'\n')
 value={'schema':1,'target':str(TARGET),'target_sha256':before,'target_executed':False,
  'architecture':'x86_64','method':'offline LLVM selected-function disassembly',
  'tool':{'path':str(TOOL),'sha256':sha(TOOL)},'symbol_table':{'path':str(table.relative_to(ROOT)),'sha256':sha(table)},
  'records':records,'resolved_field_offsets':fields,'field_offset_evidence':{'path':str(fieldpath.relative_to(ROOT)),'sha256':sha(fieldpath)},
  'limits':['No arm64 equivalence checked.','Swift optimized ABI interpretation is limited to visible branches and calls.','Symbol extents may include noninstruction alignment or jump tables.','Native helper capability does not establish active agent invocation.']}
 (ROOT/'evidence/native-lease-disassembly.json').write_text(json.dumps(value,indent=2)+'\n')
 print(json.dumps({'functions':len(records),'target_executed':False,'target_sha256':before}))
if __name__=='__main__':main()
