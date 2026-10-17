#!/usr/bin/env python3
"""Small reusable ledger for inert source excerpts from the saved Grok artifact."""
from pathlib import Path
import hashlib
import json

ROOT=Path(__file__).resolve().parents[1]

def sha(data):
    return hashlib.sha256(data).hexdigest()

class Ledger:
    def __init__(self,report,scope):
        self.report=report
        self.scope=scope
        self.sources={}
        self.anchors=[]
        self.findings=[]
        self.manifest={r['path']:r['sha256'] for r in json.loads((ROOT/'artifacts/manifest.json').read_text())['entries']}

    def excerpt(self,relative_path,start,end,literal):
        original=ROOT/'artifacts/client'/relative_path
        formatted=ROOT/'artifacts/formatted'/relative_path
        original_bytes=original.read_bytes()
        assert sha(original_bytes)==self.manifest[relative_path],relative_path
        formatted_bytes=formatted.read_bytes()
        source={'id':relative_path,'original_path':str(original),'original_sha256':sha(original_bytes),
                'formatted_path':str(formatted),'formatted_sha256':sha(formatted_bytes)}
        old=self.sources.get(relative_path)
        assert old is None or old==source,'Source changed during investigation'
        self.sources[relative_path]=source
        lines=formatted_bytes.decode().split('\n')
        assert 1<=start<=end<=len(lines),(relative_path,start,end)
        piece='\n'.join(lines[start-1:end])
        assert literal in piece,(relative_path,start,end,literal)
        ident='A'+str(len(self.anchors)+1)
        self.anchors.append({'id':ident,'source':relative_path,'formatted_start_line':start,'formatted_end_line':end,
                             'literal_anchor':literal,'formatted_excerpt_sha256_utf8':sha(piece.encode()),
                             'line_unit':'LF in derived formatted copy'})
        return ident

    def finding(self,ident,claim,anchors,limitation,classification='observation'):
        assert classification in {'observation','inference','contract','configuration','unknown'}
        assert limitation
        assert set(anchors)<=set(r['id'] for r in self.anchors)
        self.findings.append({'id':ident,'claim_fr':claim,'anchors':anchors,'classification':classification,
                              'limitation_fr':limitation,'runtime_observed':False})

    def artifact_excerpt(self,path,start,end,literal,derived_from,expected_origin_sha256):
        """Anchor an inert disassembly plus the saved binary it came from."""
        artifact=ROOT/path
        origin=ROOT/derived_from
        assert sha(origin.read_bytes())==expected_origin_sha256
        raw=artifact.read_bytes()
        self.sources[path]={'id':path,'kind':'derived_offline_disassembly',
            'original_path':str(artifact),'original_sha256':sha(raw),
            'formatted_path':str(artifact),'formatted_sha256':sha(raw),
            'derived_from_path':str(origin),'derived_from_sha256':expected_origin_sha256}
        lines=raw.decode().split('\n')
        assert 1<=start<=end<=len(lines)
        piece='\n'.join(lines[start-1:end])
        assert literal in piece,(path,start,end,literal)
        ident='A'+str(len(self.anchors)+1)
        self.anchors.append({'id':ident,'source':path,'formatted_start_line':start,'formatted_end_line':end,
            'literal_anchor':literal,'formatted_excerpt_sha256_utf8':sha(piece.encode()),'line_unit':'LF in offline LLVM disassembly'})
        return ident

    def write(self,path,checklist,searches=None):
        value={'schema':1,'report':self.report,'scope_fr':self.scope,'target':'Grok Bot 0.70.2 saved macOS artifact',
               'target_execution_performed':False,'network_calls_performed':False,
               'rea_parent':'ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d',
               'sources':list(self.sources.values()),'anchors':self.anchors,'findings':self.findings,
               'bounded_searches':searches or [],'checklist':checklist}
        for source in self.sources.values():
            assert sha(Path(source['original_path']).read_bytes())==source['original_sha256']
            assert sha(Path(source['formatted_path']).read_bytes())==source['formatted_sha256']
        (ROOT/path).write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')
        return {'findings':len(self.findings),'anchors':len(self.anchors),'sources':len(self.sources)}

def verify(path):
    ledger=json.loads(path.read_text())
    sources={r['id']:r for r in ledger['sources']}
    for source in sources.values():
        assert sha(Path(source['original_path']).read_bytes())==source['original_sha256']
        assert sha(Path(source['formatted_path']).read_bytes())==source['formatted_sha256']
        if 'derived_from_path' in source:
            assert sha(Path(source['derived_from_path']).read_bytes())==source['derived_from_sha256']
    for anchor in ledger['anchors']:
        p=Path(sources[anchor['source']]['formatted_path'])
        piece='\n'.join(p.read_text().split('\n')[anchor['formatted_start_line']-1:anchor['formatted_end_line']])
        assert sha(piece.encode())==anchor['formatted_excerpt_sha256_utf8'],anchor['id']
        assert anchor['literal_anchor'] in piece,anchor['id']
    assert (ROOT/ledger['report']).is_file()
    print(json.dumps({'ledger':str(path),'status':'passed','findings':len(ledger['findings']),'anchors':len(ledger['anchors'])}))

if __name__=='__main__':
    import sys
    assert len(sys.argv)>1,'Pass ledger JSON paths to verify'
    for arg in sys.argv[1:]:verify(Path(arg))
