#!/usr/bin/env python3
"""Disassemble selected saved x86_64 helper functions without running the app."""
from pathlib import Path
import hashlib
import json
import re
import subprocess

ROOT=Path(__file__).resolve().parents[1]
TARGET=ROOT/'inputs/Grok Bot.app/Contents/Helpers/Grok Bot Computer Use.app/Contents/MacOS/CUGrokBotService'
TOOL=Path('/Library/Developer/CommandLineTools/usr/bin/llvm-objdump')
PATTERNS={
    'accept_loop':r'CULocalRPCListenerC10acceptLoop.*Int32V_tF$',
    'peer_policy':r'CULocalRPCPeerPolicyV6allows',
    'bundled_policy':r'CULocalRPCBundledPeerRequirement.*V6allows',
    'socket_peer':r'CULocalRPCPeerIdentityV14fileDescriptor.*Tf4nd_n$',
    'signing_identity':r'CULocalRPCCodeSigningIdentity.*V4code.*Tf4nd_n$',
    'signing_guest':r'CULocalRPCCodeSigningIdentity.*V15guestAttributes.*Tf4nd_n$',
    'listener_start':r'CULocalRPCListenerC5start10Foundation3URLVyKF$',
    'application_rpc_setup':r'CUServiceApplication.*startAppControlRPCService.*Tf4nnnd_n$',
}
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def run(cmd):return subprocess.run(cmd,capture_output=True,text=True,check=True)

def main():
    before=sha(TARGET)
    table=run(['/usr/bin/nm','-arch','x86_64','-n',str(TARGET)]).stdout
    tablepath=ROOT/'evidence/native-helper-x64-symbol-table.txt'
    tablepath.write_text(table)
    symbols=[]
    for line in table.splitlines():
        match=re.match(r'([0-9a-f]{16}) ([tT]) (.+)',line)
        if match:symbols.append({'address':match[1],'symbol':match[3]})
    records=[]
    for tag,pattern in PATTERNS.items():
        matches=[r for r in symbols if re.search(pattern,r['symbol'])]
        assert len(matches)==1,(tag,len(matches))
        seed=matches[0]
        cmd=[str(TOOL),'--macho','--arch=x86_64','--disassemble','--dis-symname',seed['symbol'],str(TARGET)]
        result=run(cmd)
        output=ROOT/'evidence'/('native-deep-'+tag+'.asm.txt')
        output.write_text(result.stdout)
        assert seed['symbol']+':' in result.stdout
        records.append({'id':tag,**seed,'path':str(output.relative_to(ROOT)),
                        'sha256':sha(output),'command':cmd,'lines':len(result.stdout.splitlines()),
                        'exit_code':result.returncode})
    assert sha(TARGET)==before
    metadata=json.loads((ROOT/'evidence/native-helper-macho.json').read_text())
    assert metadata['subject']['digest']['sha256']==before
    normalized=metadata['normalized_result']
    arch=next(r for r in normalized['architectures']['items'] if r['name']=='x86_64')
    vm=0x1001504f0
    segment=next(r for r in normalized['segments']['items'] if int(r['vm_address'],16)<=vm<int(r['vm_address'],16)+r['vm_size'])
    offset=arch['file_offset']+segment['file_offset']+vm-int(segment['vm_address'],16)
    literal=TARGET.read_bytes()[offset:offset+20]
    assert literal==b'anchor apple generic'
    value={'method':'Shell LLVM Mach-O disassembly, not REA pseudocode or runtime','target':str(TARGET),
           'target_sha256':before,'architecture':'x86_64','target_executed':False,
           'tool':{'path':str(TOOL),'sha256':sha(TOOL),'version':run([str(TOOL),'--version']).stdout},
           'symbol_table':{'path':str(tablepath.relative_to(ROOT)),'sha256':sha(tablepath)},'records':records,
           'resolved_literals':[{'vm_address':hex(vm),'architecture':'x86_64','universal_file_offset':offset,
                                'value':literal.decode(),'read_bytes_hex':literal.hex(),
                                'interpretation':'Swift requirement string; LLVM literal annotation is biased.'}],
           'limits':['Private Swift ABI and optimized branches require interpretation.',
                     'No arm64 control-flow equivalence established.',
                     'Symbol extents can include alignment and jump-table bytes.',
                     'Literal-pool labels can be biased by Swift string representation.']}
    (ROOT/'evidence/native-deep-disassembly.json').write_text(json.dumps(value,indent=2)+'\n')
    print(json.dumps({'functions':len(records),'target_sha256':before,'target_executed':False}))

if __name__=='__main__':main()
