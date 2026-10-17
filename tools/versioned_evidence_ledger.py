#!/usr/bin/env python3
"""Hash-bound static excerpts from a separately retained release.

This helper never imports or evaluates target JavaScript. Native disassembly and
runtime observations require their own provenance; this schema is for text only.
"""
from pathlib import Path
import hashlib,json
ROOT=Path(__file__).resolve().parents[1]
def sha(b):return hashlib.sha256(b).hexdigest()
class Ledger:
    def __init__(self,version,report,scope):
        self.version=version;self.report=report;self.scope=scope
        self.base=Path('artifacts/releases')/version
        self.manifest_path=self.base/'asar-inventory.json'
        self.inventory=json.loads((ROOT/self.manifest_path).read_text())
        self.manifest={x['path']:x for x in self.inventory['entries']}
        self.sources={};self.anchors=[];self.findings=[]
    def excerpt(self,path,start,end,literal):
        original=self.base/'client'/path;formatted=self.base/'formatted'/path
        b=(ROOT/original).read_bytes();f=(ROOT/formatted).read_bytes()
        assert sha(b)==self.manifest[path]['sha256'],path
        lines=f.decode().split('\n');assert 1<=start<=end<=len(lines)
        excerpt='\n'.join(lines[start-1:end]);assert literal in excerpt,(path,start,end,literal)
        self.sources[path]={'id':path,'original_path':original.as_posix(),'original_sha256':sha(b),
            'formatted_path':formatted.as_posix(),'formatted_sha256':sha(f)}
        ident='A'+str(len(self.anchors)+1)
        self.anchors.append({'id':ident,'source':path,'start_line':start,'end_line':end,'literal':literal,
            'excerpt_sha256':sha(excerpt.encode()),'line_unit':'LF in separately formatted release copy'})
        return ident
    def finding(self,ident,claim,anchors,limitation,classification='observation'):
        assert limitation and anchors and classification in {'observation','contract','inference','unknown'}
        assert set(anchors)<=set(a['id'] for a in self.anchors)
        self.findings.append({'id':ident,'claim_fr':claim,'classification':classification,
            'anchors':anchors,'limitation_fr':limitation,'runtime_observed':False})
    def write(self,path,checklist,searches=None):
        value={'schema':'grokbot.versioned-static-ledger.v1','scope_fr':self.scope,'report':self.report,
          'report_sha256':sha((ROOT/self.report).read_bytes()),'version':self.version,
          'asar_sha256':self.inventory['asar_sha256'],'manifest_path':self.manifest_path.as_posix(),
          'manifest_sha256':sha((ROOT/self.manifest_path).read_bytes()),'target_executed':False,'network_used':False,
          'sources':list(self.sources.values()),'anchors':self.anchors,'findings':self.findings,
          'checklist':checklist,'bounded_searches':searches or []}
        (ROOT/path).write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')
        return value
def verify(path):
    d=json.loads((ROOT/path).read_text());assert d['schema']=='grokbot.versioned-static-ledger.v1'
    manifest=(ROOT/d['manifest_path']).read_bytes();assert sha(manifest)==d['manifest_sha256']
    m=json.loads(manifest);assert m['asar_sha256']==d['asar_sha256']
    entries={e['path']:e for e in m['entries']};sources={s['id']:s for s in d['sources']}
    for s in sources.values():
        assert sha((ROOT/s['original_path']).read_bytes())==s['original_sha256']==entries[s['id']]['sha256']
        assert sha((ROOT/s['formatted_path']).read_bytes())==s['formatted_sha256']
    for a in d['anchors']:
        lines=(ROOT/sources[a['source']]['formatted_path']).read_text().split('\n')
        excerpt='\n'.join(lines[a['start_line']-1:a['end_line']])
        assert a['literal'] in excerpt and sha(excerpt.encode())==a['excerpt_sha256'],a['id']
    assert sha((ROOT/d['report']).read_bytes())==d['report_sha256']
    return {'status':'passed','version':d['version'],'sources':len(sources),'anchors':len(d['anchors']),'findings':len(d['findings'])}
if __name__=='__main__':
    import sys
    for path in sys.argv[1:]:print(json.dumps(verify(path)))
