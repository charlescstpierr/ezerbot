#!/usr/bin/env python3
"""Check the investigator's HTML page in a separate Chrome session."""
from pathlib import Path
import subprocess,json,re,os,hashlib
ROOT=Path(__file__).resolve().parents[1]
CLI=Path.home()/'.local/bin/chrome-devtools-axi'
PAGE=ROOT/'.lavish/goal-neuf-sujets.html'
ENV={**os.environ,'CHROME_DEVTOOLS_AXI_SESSION':'grok-goal'}
def run(*args):
 return subprocess.run([str(CLI),*args],env=ENV,capture_output=True,text=True,check=True).stdout
def value(output):
 found=re.search(r'^result:\s*(.+)$',output,re.M)
 assert found,output
 v=json.loads(found[1])
 while isinstance(v,str):v=json.loads(v)
 return v
SCRIPT="""() => {
 const results=[];
 const tabs=[...document.querySelectorAll('[data-topic]')];
 const reports=[...document.querySelectorAll('details.report')];
 for(const theme of ['light','dark']) {
  if(document.documentElement.dataset.theme!==theme)document.getElementById('theme').click();
  for(const tab of tabs){
   tab.click();
   const visible=[...document.querySelectorAll('[role=tabpanel]')].filter(e=>!e.hidden);
   results.push({theme,topic:tab.dataset.topic,visible:visible.length,
    matches:visible[0]?.id==='panel-'+tab.dataset.topic,
    selected:document.querySelectorAll('[role=tab][aria-selected=true]').length,
    overflow:document.documentElement.scrollWidth>innerWidth});
  }
  for(const report of reports){report.open=true;results.push({theme,report:report.id,open:report.open,overflow:document.documentElement.scrollWidth>innerWidth});report.open=false}
 }
 tabs[tabs.length-1].focus();tabs[tabs.length-1].dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}));
 const keyboardWrap=tabs[0].getAttribute('aria-selected')==='true'&&document.activeElement===tabs[0];
 const ids=[...document.querySelectorAll('[id]')].map(e=>e.id);
 const duplicateIds=ids.filter((id,i)=>ids.indexOf(id)!==i);
 if(document.documentElement.dataset.theme!=='light')document.getElementById('theme').click();
 tabs[0].click();window.scrollTo(0,0);
 return {width:innerWidth,height:innerHeight,results,keyboardWrap,duplicateIds,svgCount:document.querySelectorAll('svg').length};
}"""
def main():
 run('open',PAGE.as_uri())
 sizes=[]
 for width in [1280,500]:
  run('resize',str(width),'900')
  state=value(run('eval',SCRIPT))
  assert state['width']==width and state['svgCount']==1 and not state['duplicateIds']
  assert state['keyboardWrap']
  for r in state['results']:
   assert r['overflow'] is False,r
   if 'topic' in r:assert r['matches'] and r['visible']==1 and r['selected']==1,r
   else:assert r['open'],r
  sizes.append(state)
  run('screenshot',str(ROOT/'.lavish'/f'goal-{width}-light.png'))
  run('eval',"() => {document.getElementById('theme').click();window.scrollTo(0,0);return document.documentElement.dataset.theme}")
  run('screenshot',str(ROOT/'.lavish'/f'goal-{width}-dark.png'))
  run('eval',"() => {document.getElementById('theme').click();return true}")
 result={'status':'passed','scope':'Own offline report page, not original app runtime','target_runtime_verified':False,
  'page_sha256':hashlib.sha256(PAGE.read_bytes()).hexdigest(),'viewports':sizes,
  'tab_theme_viewport_cases':36,'report_theme_viewport_cases':16,
  'screenshots':[f'.lavish/goal-{w}-{t}.png' for w in [1280,500] for t in ['light','dark']]}
 (ROOT/'evidence/goal-browser-verification.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps({'status':'passed','actual_widths':[s['width'] for s in sizes],'tab_cases':36,'report_cases':16}))
if __name__=='__main__':main()
