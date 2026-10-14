#!/usr/bin/env python3
"""Rebuild focused, static source references and validate descriptor inventory."""
import hashlib
import json
import re
from pathlib import Path
root = Path(__file__).resolve().parent.parent
aliases = {
 'P': 'dist/electron-main/proto.cjs',
 'M': 'dist/electron-main/main-app.cjs',
 'A': 'dist/renderer/assets/index.eager-app-BfVhIMYf.js',
 'F': 'dist/renderer/assets/index.eager-platform-DUN5VkaE.js',
 'C': 'dist/node-agent-coordinator/main.cjs',
}
# Claims are manual interpretations of the inspected source. Anchors are regenerated.
traces = [
 ('PROTO-FIELDS','P','D6',r'function\s+D6\(', 'Decodes compact field descriptors, optional/repeated/oneof/map.'),
 ('TRANSPORT','M','te',r'function\s+te\(', 'Constructs Connect client from explicit backend and service descriptor.'),
 ('DOMAIN-ROUTER','C','IJ',r'function\s+IJ\(', 'Selects server/gateway based on harness and operation, validates and classifies unimplemented.'),
 ('DOMAIN-CONTROL','C','ur',r'async\s+function\s+ur\(', 'Sends a validated server-agent request across coordinator-control boundary.'),
 ('SEND-RPC','M','z8e',r'async\s+function\s+z8e\(', 'Maps domain commands to GrokBot RPCs; success uses delivery enum.'),
 ('SEND-RETRY','M','W8e',r'function\s+W8e\(', 'Queries durable send status before repeating a failed send.'),
 ('SEND-STATUS','M','q8e',r'function\s+q8e\(', 'Translates missing, accepted, pending, rejected and uncertain durability.'),
 ('SEND-JOURNAL','A','fIe',r'function\s+fIe\(', 'Owns durable local send phases, echo retirement, resend aliases and reconnection reconciliation.'),
 ('TRANSCRIPT-DOMAIN','A','OTe',r'function\s+OTe\(', 'Owns independent per-agent/session replica, history, optimistic overlay and hydration.'),
 ('TRANSCRIPT-REPLICA','F','$C',r'class\s+\$C\s*\{', 'Orders deltas by epoch/sequence, rejects stale snapshots and resynchronizes on gaps.'),
 ('TRANSCRIPT-WATCH','C','ND',r'function\s+ND\(', 'Calls backend watch/list, presigns blobs, uses resolved access headers.'),
 ('TRANSCRIPT-TRANSLATE','C','lM',r'function\s+lM\(', 'Converts protobuf generations/updatedSeq into domain epochs/sequences, retries absent bodies.'),
 ('TRANSCRIPT-MUTATION-SORT','C','o9',r'function\s+o9\(', 'Merges entries and deletes sorted by updatedSeq.'),
 ('TRANSCRIPT-STORE-READ','M','Tft',r'function\s+Tft\(', 'Lists rows, fetches bodies and parses transcript JSON.'),
 ('ROUTINE-DOMAIN','A','d2e',r'function\s+d2e\(', 'Maintains routine snapshots, manual-run guards and webhook polling.'),
 ('ROUTINE-RECORD','M','Upn',r'function\s+Upn\(', 'Checks known JSON routine record fields before sorting lists.'),
 ('ROUTINE-RPC','M','o7e',r'function\s+o7e\(', 'Maps routine read/enable/delete and webhook credentials to backend RPCs.'),
 ('MEMORY-PLUGIN-RPC','M','x9e',r'function\s+x9e\(', 'Maps plugins, skills and personal-memory promotion to GrokBot RPCs.'),
 ('MEMORY-PLUGIN-DOMAIN','A','VU',r'function\s+VU\(', 'Replaces per-bot plugin snapshot after mutation and revalidates on reconnect.'),
 ('BOT-FILES','M','I9e',r'function\s+I9e\(', 'Hashes file, begins upload, PUTs content and finalizes file metadata.'),
 ('BOT-MAP','M','Ew',r'function\s+Ew\(', 'Maps server/agent identity and viewer permissions to UI row.'),
 ('ACCESS-CONFLICT','M','Msn',r'function\s+Msn\(', 'Classifies retryable Aborted ErrorDetails as access state changed.'),
 ('MARKETPLACE','M','L$e',r'function\s+L\$e\(', 'Loads versioned recipe and paginated public marketplace listings.'),
 ('TEMPLATE-IMPORT','M','net',r'function\s+net\(', 'Validates expected identity/harness and flags after server import.'),
 ('VOICE-CREDENTIAL','M','zYt',r'async\s+function\s+zYt\(', 'Mints ephemeral voice credential or maps refusal fault.'),
 ('VOICE-HOST','A','xu',r'class\s+xu\s*\{', 'Restricts dialable voice URL prefix to wss://api.x.ai/.'),
 ('VOICE-FRAMES','A','Kn.realtimeProtocols',r'static\s+realtimeProtocols\(', 'Uses secret subprotocol and constructs resumable PCM24k session.update.'),
]
source_cache = {}
def source(alias):
 if alias not in source_cache:
  rel=aliases[alias]
  raw=(root/'artifacts/client'/rel).read_text()
  pretty=(root/'artifacts/formatted'/rel).read_text()
  source_cache[alias]=(rel,raw,pretty)
 return source_cache[alias]
ledger=[]
for ident,alias,symbol,pattern,claim in traces:
 rel,raw,pretty=source(alias)
 mr=re.search(pattern,raw)
 mp=re.search(pattern,pretty)
 if mr is None or mp is None:
  raise AssertionError(f'Missing trace {ident}: {symbol}')
 ledger.append({'id':ident,'claim':claim,'evidenceType':'static observation with manually traced callers/state','runtimeObserved':False,'symbol':symbol,'source':{'path':rel,'sha256':hashlib.sha256(raw.encode()).hexdigest(),'originalCharacterOffset':len(raw[:mr.start()].encode('utf-16-le'))//2,'originalOffsetUnit':'UTF-16 code units in decoded JavaScript','formattedLine':pretty.count('\n',0,mp.start())+1},'excerpt':pretty[mp.start():mp.start()+1600],'limits':['No authenticated execution or backend implementation observed.','Excerpt anchors the symbol; report describes the traced larger function.']})
(root/'evidence/protocol-flows.json').write_text(json.dumps({'target':'Grok Bot 0.70.2','traces':ledger},indent=2,ensure_ascii=False)+'\n')
a=json.loads((root/'evidence/protocol-contracts.json').read_text())
assert a['counts']['fullServices']==7
assert a['counts']['fullServiceMethods']==484
assert a['counts']['uniqueRpcTypePairs']==429
assert a['counts']['messages']==2313
assert a['counts']['enums']==287
messages={m['typeName']:m for m in a['messages']}
enums={e['typeName']:e for e in a['enums']}
for m in a['messages']:
 assert m['source']['formattedLine'] is not None and m['source']['originalCharacterOffset'] is not None,m['typeName']
 assert len({f['number'] for f in m['fields']})==len(m['fields']),m['typeName']
 for f in m['fields']:
  typ=f.get('type') or f.get('valueType')
  if '.' in typ:
   assert typ in messages or typ in enums,(m['typeName'],typ)
for s in a['services']:
 for m in s['methods']:
  assert m['input'] in messages and m['output'] in messages
  assert m['kind'] in ['Unary','ServerStreaming','ClientStreaming','BiDiStreaming']
rel,raw,pretty=source('P')
assert hashlib.sha256(raw.encode()).hexdigest()==a['source']['sha256']
assert a['counts']['filesScanned']==599
verification={'status':'passed','scope':'offline generator output and source anchor consistency only','runtimeVerified':False,'descriptorRecovery':a['descriptorRecovery'],'inventoryCounts':a['counts'],'focusedTraces':len(ledger),'checks':['Source SHA-256 matches generated inventory.','All message fields have unique wire numbers.','All referenced message and enum types resolve.','All RPC inputs and outputs resolve.','Every manually selected source symbol is present in original and formatted copy.'],'limitations':['This does not validate backend semantics or runtime behavior.','Call candidates are not confirmed invocations.']}
(root/'evidence/protocol-verification.json').write_text(json.dumps(verification,indent=2)+'\n')
print(json.dumps(verification,indent=2))
