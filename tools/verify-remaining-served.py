#!/usr/bin/env python3
"""Check the actual local review server, including artifact-relative assets."""
from pathlib import Path
from html.parser import HTMLParser
import base64,hashlib,importlib.util,json,os,urllib.parse,urllib.request
ROOT=Path(__file__).resolve().parents[1]
SESSION='http://127.0.0.1:4387/session/ea9b68021597d002'
spec=importlib.util.spec_from_file_location('browser',ROOT/'tools/verify-goal-view-browser.py')
b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
b.ENV={**os.environ,'CHROME_DEVTOOLS_AXI_SESSION':'grok-remaining-http'}
class Page(HTMLParser):
    def __init__(self):super().__init__();self.links=[];self.images=[]
    def handle_starttag(self,tag,attrs):
        a=dict(attrs)
        if tag=='a' and 'remaining-assets/' in a.get('href',''):self.links.append(a['href'])
        if tag=='img':self.images.append(a['src'])
def main():
    b.run('open',SESSION)
    url=b.value(b.run('eval',"() => ({url:document.querySelector('iframe').src})"))['url']
    assert url.startswith('http://127.0.0.1:4387/artifact/ea9b68021597d002/')
    with urllib.request.urlopen(url,timeout=15) as r:html=r.read().decode()
    parsed=Page();parsed.feed(html);records=[]
    for ref in sorted(set(parsed.links)):
        assert ref.startswith('remaining-assets/')
        dest=urllib.parse.urljoin(url,ref)
        with urllib.request.urlopen(dest,timeout=15) as r:data=r.read();status=r.status
        assert data==(ROOT/'.lavish'/ref).read_bytes(),ref
        records.append({'path':ref,'status':status,'sha256':hashlib.sha256(data).hexdigest()})
    expected={'remaining-assets/reports/'+name for name in ['local-cua-path-0712.md','local-cua-consent-0712.md','native-0712-mcp.md','native-arm64-residual.md','multibot-membership-paths.md','multibot-turn-paths.md','multibot-public-usage.md']}
    assert expected <= set(parsed.links), expected-set(parsed.links)
    assert len(parsed.images)==1 and parsed.images[0].startswith('data:image/png;base64,')
    png=base64.b64decode(parsed.images[0].split(',',1)[1])
    assert png==(ROOT/'evidence/original-startup-login.png').read_bytes()
    proof={'schema':1,'status':'passed','scope':'Own Lavish-served explanation and packaged local assets only',
      'session_url':SESSION,'page_sha256':hashlib.sha256((ROOT/'.lavish/remaining-grokbot.html').read_bytes()).hexdigest(),
      'source_image_embedded_and_equal':True,'image_sha256':hashlib.sha256(png).hexdigest(),'assets':records}
    (ROOT/'evidence/remaining-served-verification.json').write_text(json.dumps(proof,indent=2)+'\n')
    b.run('screenshot',str(ROOT/'.lavish/remaining-served.png'))
    print(json.dumps({'status':'passed','served_assets':len(records),'embedded_image_equal':True}))
if __name__=='__main__':main()
