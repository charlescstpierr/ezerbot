#!/usr/bin/env python3
"""Save hashes and bounded source references for the two new parent reports."""
from pathlib import Path
import hashlib
import json
import re

ROOT=Path(__file__).resolve().parents[1]
REPORTS=['reports/compaction-runtimes.md','reports/native-helper-detail.md']
sources={}
anchors=[]
for name in REPORTS:
    report=ROOT/name
    for label,target in re.findall(r'\[([^\]]+)\]\(([^)]+)\)',report.read_text()):
        match=re.fullmatch(r'(.+):(\d+)',target.strip('<>'))
        if not match:
            continue
        path=(report.parent/match[1]).resolve()
        if not path.is_relative_to(ROOT/'artifacts/formatted'):
            continue
        rel=path.relative_to(ROOT/'artifacts/formatted')
        orig=ROOT/'artifacts/client'/rel
        ident=str(rel)
        if ident not in sources:
            sources[ident]={'id':ident,'original_path':str(orig),'original_sha256':hashlib.sha256(orig.read_bytes()).hexdigest(),
                           'formatted_path':str(path),'formatted_sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
        lines=path.read_text().split('\n')
        first=int(match[2]);last=min(first+29,len(lines))
        assert 1<=first<=last<=len(lines),(name,target)
        anchors.append({'id':'REF'+str(len(anchors)+1),'report':name,'label':label,'source':ident,
                        'formatted_start_line':first,'formatted_end_line':last,
                        'formatted_excerpt_sha256_utf8':hashlib.sha256('\n'.join(lines[first-1:last]).encode()).hexdigest(),
                        'limitation':'A bounded reference window, not the full callable body.'})
result={'schema':1,'reports':REPORTS,'target_execution_performed':False,'network_calls_to_target_performed':False,
        'rea_parent':'ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d',
        'sources':list(sources.values()),'anchors':anchors,
        'search':'evidence/uncovered-search.json',
        'native_evidence_files':['evidence/native-helper-macho.json','evidence/native-helper-plist.json','evidence/native-helper-symbols.json'],
        'checklist':[{'question':'Compaction effective','status':'partially_answered'},
                     {'question':'Runtimes locaux et routage','status':'answered-static-client-side'},
                     {'question':'Checkpoints et contexte délégué distants','status':'unresolved'},
                     {'question':'Métadonnées natives Messages','status':'answered-static'},
                     {'question':'Corps natifs et politique socket','status':'unresolved'}]}
dest=ROOT/'evidence/compaction-runtime-detail.json'
dest.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print(json.dumps({'sources':len(sources),'anchors':len(anchors),'output':str(dest.relative_to(ROOT))}))
