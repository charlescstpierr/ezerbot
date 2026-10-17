#!/usr/bin/env python3
"""Verify only the local explanatory page, separately from original-app evidence."""
from pathlib import Path
from html.parser import HTMLParser
import hashlib,importlib.util,json,os,urllib.parse
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('browser',ROOT/'tools/verify-goal-view-browser.py')
b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
b.ENV={**os.environ,'CHROME_DEVTOOLS_AXI_SESSION':'grok-remaining-review'}
PAGE=ROOT/'.lavish/remaining-grokbot.html'
class Links(HTMLParser):
    def __init__(self):super().__init__();self.links=[]
    def handle_starttag(self,tag,attrs):
        a=dict(attrs)
        for k in ['src','href']:
            if k in a:self.links.append(a[k])
def main():
    parsed=Links();parsed.feed(PAGE.read_text());local=[]
    for link in parsed.links:
        u=urllib.parse.urlsplit(link)
        if u.scheme or not u.path:continue
        p=(PAGE.parent/urllib.parse.unquote(u.path)).resolve();assert p.exists(),link;local.append(link)
    b.run('open',PAGE.as_uri());sizes=[]
    for width in [1280,500]:
        b.run('resize',str(width),'900');state=b.value(b.run('eval',b.SCRIPT))
        assert state['width']==width and state['keyboardWrap'] and not state['duplicateIds'] and state['svgCount']==2
        assert len(state['results'])==12
        for r in state['results']:assert r['matches'] and r['visible']==r['selected']==1 and not r['overflow'],r
        sizes.append(state);b.run('screenshot',str(ROOT/'.lavish'/f'remaining-{width}-light.png'))
        b.run('eval',"() => {document.getElementById('theme').click();document.getElementById('tab-phone').click();document.getElementById('panel-phone').scrollIntoView();return true}")
        b.run('screenshot',str(ROOT/'.lavish'/f'remaining-{width}-phone-dark.png'))
        extra=b.value(b.run('eval',"() => ({imageLoaded:document.querySelector('.runtime-image').naturalWidth>0,rows:document.querySelectorAll('tbody tr').length,overflow:document.documentElement.scrollWidth>innerWidth})"))
        assert extra['imageLoaded'] and extra['rows']==9 and not extra['overflow']
        b.run('eval',"() => {document.getElementById('tab-mac').click();document.getElementById('panel-mac').scrollIntoView();return true}")
        mac=b.value(b.run('eval',"() => ({visible:!document.getElementById('panel-mac').hidden,nodes:document.querySelectorAll('#mac-control-diagram g').length,revocation:document.getElementById('mac-revocation').textContent.includes('déjà admis'),overflow:document.documentElement.scrollWidth>innerWidth})"))
        assert mac['visible'] and mac['nodes']==5 and mac['revocation'] and not mac['overflow']
        b.run('screenshot',str(ROOT/'.lavish'/f'remaining-{width}-mac.png'))
        b.run('eval',"() => {document.getElementById('tab-bots').click();document.getElementById('panel-bots').scrollIntoView();return true}")
        bots=b.value(b.run('eval',"() => ({visible:!document.getElementById('panel-bots').hidden,routes:document.querySelectorAll('#panel-bots .cards article').length,overflow:document.documentElement.scrollWidth>innerWidth})"))
        assert bots['visible'] and bots['routes']==3 and not bots['overflow']
        b.run('screenshot',str(ROOT/'.lavish'/f'remaining-{width}-bots.png'))
    proof={'schema':1,'status':'passed','scope':'Own local explanation only; original runtime verified separately.',
      'page_sha256':hashlib.sha256(PAGE.read_bytes()).hexdigest(),'local_links_checked':len(local),
      'tab_theme_viewport_cases':24,'viewports':sizes,'original_runtime_verified_by_this_check':False,
      'ios_runtime_verified':False,'screenshots':[f'.lavish/remaining-{w}-{t}.png' for w in [1280,500] for t in ['light','phone-dark','mac','bots']]}
    (ROOT/'evidence/remaining-view-verification.json').write_text(json.dumps(proof,indent=2)+'\n')
    print(json.dumps({k:v for k,v in proof.items() if k not in ['viewports','screenshots']}))
if __name__=='__main__':main()
