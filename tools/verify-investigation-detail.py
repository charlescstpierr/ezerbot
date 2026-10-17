#!/usr/bin/env python3
"""Verify continuation sources, reference windows, native bytes and review links."""
from pathlib import Path
from html.parser import HTMLParser
import hashlib
import json
import re
from urllib.parse import unquote

ROOT=Path(__file__).resolve().parents[1]
LEDGERS=['cron-detail','computer-detail','other-capabilities-detail','compaction-runtime-detail','delegation-lifecycle-deep','bot-personality-detail']
REPORTS=['investigation-continuation','cron-detail','computer-detail','other-capabilities-detail','compaction-runtimes','native-helper-detail','native-peer-deep','delegation-lifecycle-deep','bot-personality-detail']

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def read(name):
    return json.loads((ROOT/name).read_text())

manifest=read('artifacts/manifest.json')
originals={r['path']:r['sha256'] for r in manifest['entries']}
checks={'ledgers':0,'sources':0,'reference_windows':0,'report_links':0,'native_strings':0,'view_links':0}
for name in LEDGERS:
    ledger=read('evidence/'+name+'.json')
    checks['ledgers']+=1
    sources=ledger.get('sources',[])
    if isinstance(sources,dict):
        sources=[dict(v,id=k) for k,v in sources.items()]
    indexed={s['id']:s for s in sources}
    for source in sources:
        if 'original_path' not in source:
            continue
        original=Path(source['original_path'])
        if not original.is_absolute():
            original=ROOT/original
        assert digest(original)==source['original_sha256'],original
        rel=original.relative_to(ROOT/'artifacts/client').as_posix()
        assert digest(original)==originals[rel],rel
        formatted=Path(source['formatted_path'])
        if not formatted.is_absolute():
            formatted=ROOT/formatted
        assert digest(formatted)==source['formatted_sha256'],formatted
        checks['sources']+=1
    for anchor in ledger.get('anchors',[]):
        if 'formatted_start_line' not in anchor:
            continue
        source=indexed[anchor['source']]
        path=Path(source['formatted_path'])
        if not path.is_absolute():
            path=ROOT/path
        start,end=anchor['formatted_start_line'],anchor['formatted_end_line']
        lines=path.read_text().split('\n')
        assert 1<=start<=end<=len(lines),(name,anchor['id'])
        text='\n'.join(lines[start-1:end])
        hashes={hashlib.sha256(text.encode()).hexdigest(),hashlib.sha256((text+'\n').encode()).hexdigest()}
        assert anchor['formatted_excerpt_sha256_utf8'] in hashes,(name,anchor['id'])
        checks['reference_windows']+=1
    for finding in ledger.get('findings',[]):
        for source in finding.get('sources',[]):
            if not isinstance(source,dict) or 'excerpt_sha256' not in source:
                continue
            path=ROOT/source['formatted_path']
            text='\n'.join(path.read_text().split('\n')[source['lines']['start']-1:source['lines']['end']])
            assert hashlib.sha256(text.encode()).hexdigest()==source['excerpt_sha256'],(name,finding['id'])
            assert source['anchor'] in text,(name,finding['id'])
            checks['reference_windows']+=1
    for source in ledger.get('sourceEvidence',[]):
        original=ROOT/'artifacts/client'/source['path']
        assert digest(original)==source['originalSha256']==originals[source['path']],source['path']
        anchor=source['anchor']
        raw_original=original.read_bytes()
        pos=anchor['originalByteOffset']
        assert len(raw_original[:pos].decode('utf-8').encode('utf-16-le'))//2==anchor['originalCharacterOffset'],source['id']
        assert original.read_bytes()[pos:].startswith(anchor['literal'].encode()),source['id']
        if 'formattedLines' in source:
            formatted=ROOT/source['formattedPath']
            assert digest(formatted)==source['formattedSha256'],formatted
            start,end=source['formattedLines']['start'],source['formattedLines']['end']
            piece='\n'.join(formatted.read_text().split('\n')[start-1:end])+'\n'
            assert hashlib.sha256(piece.encode()).hexdigest()==source['excerptSha256Utf8'],source['id']
            checks['reference_windows']+=1
        checks['sources']+=1

for name in REPORTS:
    report=ROOT/'reports'/(name+'.md')
    for target in re.findall(r'\]\(([^)]+)\)',report.read_text()):
        target=target.strip('<>')
        if target.startswith(('https://','http://','#')):
            continue
        match=re.fullmatch(r'(.+):(\d+)',target)
        path,line=(match[1],int(match[2])) if match else (target,None)
        p=Path(path)
        if not p.is_absolute():
            p=report.parent/p
        assert p.is_file(),(name,target)
        if line:
            assert 1<=line<=len(p.read_text().split('\n')),(name,target)
        checks['report_links']+=1

search=read('evidence/uncovered-search.json')
js=list(p for p in (ROOT/'artifacts/client').rglob('*') if p.is_file() and p.suffix in {'.js','.cjs','.mjs'})
assert len(js)==search['javascript_files_scanned']==600
for row in search['matching_files']:
    path=ROOT/row['path']
    assert digest(path)==row['sha256']
    text=path.read_text()
    for hit in row['hits']:
        assert len(list(re.finditer(re.escape(hit['token']),text,re.I)))==hit['count']
helper=ROOT/search['helper']['path']
raw=helper.read_bytes()
assert hashlib.sha256(raw).hexdigest()==search['helper']['sha256']
for row in search['helper']['strings']:
    value=row['value'].encode('ascii')
    assert raw[row['file_offset']:row['file_offset']+len(value)]==value
    checks['native_strings']+=1
native=[read('evidence/'+n+'.json') for n in ['native-helper-macho','native-helper-plist','native-helper-symbols']]
assert all(r['subject']['digest']['sha256']==digest(helper) for r in native)
exports={r['name'].lstrip('_') for r in native[0]['normalized_result']['exports']['items']}
assert all(r['input'] in exports and r['status']=='demangled' for r in native[2]['normalized_result']['symbols'])
assert native[1]['normalized_result']['bundle']['identifier']=='co.anysphere.grok-bot-computer-use'
disasm=read('evidence/native-deep-disassembly.json')
assert disasm['target_sha256']==digest(helper) and disasm['target_executed'] is False
assert digest(ROOT/disasm['symbol_table']['path'])==disasm['symbol_table']['sha256']
for record in disasm['records']:
    path=ROOT/record['path']
    assert digest(path)==record['sha256'] and record['symbol']+':' in path.read_text(),record['id']
for row in disasm['resolved_literals']:
    value=bytes.fromhex(row['read_bytes_hex']);pos=row['universal_file_offset']
    assert raw[pos:pos+len(value)]==value
checks['native_function_bodies']=len(disasm['records'])

class Page(HTMLParser):
    def __init__(self):
        super().__init__();self.ids=[];self.links=[];self.tabs=0
    def handle_starttag(self,tag,attrs):
        a=dict(attrs)
        if 'id' in a:self.ids.append(a['id'])
        if a.get('role')=='tab':self.tabs+=1
        if tag=='a' and 'href' in a:self.links.append(a['href'])
pagepath=ROOT/'.lavish/enquete-grokbot.html'
page=Page();page.feed(pagepath.read_text())
assert len(set(page.ids))==len(page.ids)
assert page.tabs==7
for href in page.links:
    if href.startswith(('http://','https://')):continue
    if href.startswith('#'):
        assert href[1:] in page.ids,href
    else:
        assert (pagepath.parent/unquote(href.split('#')[0])).is_file(),href
    checks['view_links']+=1
result={'status':'passed','checks':checks,'javascript_files':len(js),
        'reports':{str((ROOT/'reports'/(n+'.md')).relative_to(ROOT)):digest(ROOT/'reports'/(n+'.md')) for n in REPORTS},
        'view_sha256':digest(pagepath),'application_runtime_verified':False,'backend_reconstructed':False,
        'scope':'Saved artifact bytes, ledger reference windows and local review links; not authenticated product behavior.'}
(ROOT/'evidence/investigation-detail-verification.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(result,ensure_ascii=False,indent=2))
