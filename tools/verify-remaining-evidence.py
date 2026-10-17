#!/usr/bin/env python3
"""Verify new native bytes, static anchors and retained public response bodies."""
from pathlib import Path
from datetime import datetime
from zoneinfo import ZoneInfo
import argparse,concurrent.futures,hashlib,html,json,re,urllib.request
ROOT=Path(__file__).resolve().parents[1]
def digest(b):return hashlib.sha256(b).hexdigest()
def raw(p):return (ROOT/p).read_bytes() if not Path(p).is_absolute() else Path(p).read_bytes()
def check_file(r):assert digest(raw(r['path']))==r['sha256'],r['path']
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--refresh-public',action='store_true');args=ap.parse_args()
    native=json.loads(raw('evidence/native-input-residual.json'))
    target=raw(native['target']['path']);assert digest(target)==native['target']['sha256']
    count=total=0;slice_offset=int(native['target']['slice_offset'],16)
    def offset(va):
        seg=next(s for s in native['target']['segments'] if s['vmaddr']<=va<s['vmaddr']+s['filesize'])
        return slice_offset+seg['fileoff']+va-seg['vmaddr']
    for fn in native['functions']:
        check_file(fn);start=int(fn['start_va'],16);end=int(fn['end_va_exclusive'],16)
        off=offset(start);assert off==int(fn['universal_file_offset'],16)
        assert digest(target[off:off+end-start])==fn['body_sha256']
        instructions=byte_count=0
        for line in raw(fn['path']).decode().split('\n'):
            m=re.match(r'^([0-9a-f]+):\t((?:[0-9a-f]{2} ?)+)\t',line)
            if not m:continue
            va=int(m[1],16);b=bytes.fromhex(m[2]);at=offset(va)
            assert target[at:at+len(b)]==b,(fn['path'],m[1])
            instructions+=1;byte_count+=len(b)
        assert instructions==fn['instruction_count'] and byte_count==fn['instruction_bytes_verified'],fn['path']
        count+=instructions;total+=byte_count
    assert count==native['coverage']['displayed_instructions_verified_against_original']
    assert total==native['coverage']['displayed_bytes_verified_against_original']
    anchors=0
    for c in native['claims']:
        for a in c['anchors']:
            assert raw(a['source']).decode().split('\n')[a['line']-1]==a['excerpt']
            off=int(a['original_universal_offset'],16);b=bytes.fromhex(a['instruction_bytes'])
            assert offset(int(a['va'],16))==off and target[off:off+len(b)]==b
            anchors+=1
    for c in native['constants']:
        off=int(c['universal_file_offset'],16);b=bytes.fromhex(c['bytes'])
        assert offset(int(c['va'],16))==off and target[off:off+len(b)]==b
        assert digest(b)==c['sha256']
    for r in native['prior_provenance']:check_file(r)
    check_file(native['report'])
    dist=json.loads(raw('evidence/distribution-mobile-discovery.json'))
    for r in dist['sources'].values():
        assert digest(raw(r['original_path']))==r['original_sha256']
        assert digest(raw(r['formatted_path']))==r['formatted_sha256']
    for r in dist['resources'].values():check_file(r)
    static_anchors=0
    for f in dist['findings']:
        for a in f['sources']:
            lines=raw(a['formatted_path']).decode().split('\n')
            piece='\n'.join(lines[a['lines']['start']-1:a['lines']['end']])
            assert piece==a['excerpt'] and a['anchor'] in piece
            assert digest(piece.encode())==a['excerpt_sha256']
            static_anchors+=1
    http=0
    for r in dist['public_observations']:
        if 'body_sha256' not in r:continue
        assert r['body_encoding']=='utf-8'
        b=r['body_utf8'].encode()
        assert len(b)==r['body_bytes'] and digest(b)==r['body_sha256']
        http+=1
    assert static_anchors==dist['verification']['source_anchor_count']
    assert digest(raw(dist['report']))==dist['verification']['report_sha256']
    public=json.loads(raw('evidence/public-behavior-residual.json'))
    assert digest(raw(public['report']))==public['verification']['report_sha256_utf8']
    assert datetime.fromisoformat(public['consulted_at_utc']).astimezone(ZoneInfo('America/Montreal')).date().isoformat()==public['consulted_date_local_America_Montreal']
    assert len(public['public_sources'])==7 and len(public['public_anchors'])==17
    if args.refresh_public:
        def retrieve(source):
            req=urllib.request.Request(source['retrieval_url'],headers={'User-Agent':'GrokBot-local-evidence-review/1.0'})
            with urllib.request.urlopen(req,timeout=25) as response:body=response.read();status=response.status
            text=body.decode('utf-8');data=json.loads(text) if source['id']=='P7' else None
            if data:
                post=next(p for p in data['post_stream']['posts'] if p['post_number']==33)
                text=post['cooked']
            text=' '.join(html.unescape(re.sub(r'<[^>]*>',' ',text)).split())
            anchors=[a for a in public['public_anchors'] if a['source']==source['id']]
            matched=[a['id'] for a in anchors if a['literal_anchor'] in text]
            assert len(matched)==len(anchors),(source['id'],matched)
            return {'source':source['id'],'url':source['retrieval_url'],'status':status,'observed_at_utc':datetime.now(ZoneInfo('UTC')).isoformat(),
              'response_sha256':digest(body),'original_fetch_hash_matches':digest(body)==source['raw_response_sha256'],
              'anchors_verified':matched,'body_retained':False}
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:reviews=list(pool.map(retrieve,public['public_sources']))
        (ROOT/'evidence/public-behavior-independent-review.json').write_text(json.dumps({'schema':1,'status':'passed','method':'Independent public unauthenticated GET and visible-text anchor check; response bodies not retained, so anchor checks are observations at fetch time.',
          'records':reviews,'anchors_checked':sum(len(x['anchors_verified']) for x in reviews)},indent=2)+'\n')
    result={'schema':1,'status':'passed','native_symbol_bodies':len(native['functions']),
      'native_instructions_checked':count,'native_bytes_checked':total,'native_claim_anchors_checked':anchors,
      'native_constants_checked':len(native['constants']),'distribution_static_anchors_checked':static_anchors,
      'retained_public_http_bodies_checked':http,'authenticated_runtime_observed':False,
      'public_documentary_sources':len(public['public_sources']),'public_documentary_anchors':len(public['public_anchors']),
      'documentary_anchor_limit':'Public response bodies are not retained; an optional refresh checks anchors again at the new consultation time.',
      'ledger_sha256':{p:digest(raw(p)) for p in ['evidence/native-input-residual.json','evidence/distribution-mobile-discovery.json','evidence/public-behavior-residual.json']}}
    (ROOT/'evidence/remaining-evidence-verification.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))
if __name__=='__main__':main()
