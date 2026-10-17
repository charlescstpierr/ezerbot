#!/usr/bin/env python3
"""Check the own communications explainer; never the original application."""
from pathlib import Path
import importlib.util,json,hashlib,os
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('browser',ROOT/'tools/verify-goal-view-browser.py')
b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
b.ENV={**os.environ,'CHROME_DEVTOOLS_AXI_SESSION':'grok-communications'}
PAGE=ROOT/'.lavish/communications-grokbot.html'
SCRIPT="""() => {
 const tabs=[...document.querySelectorAll('[data-topic]')],results=[];
 for(const theme of ['light','dark']){
  if(document.documentElement.dataset.theme!==theme)document.getElementById('theme').click();
  tabs[0].click();
  for(const button of document.querySelectorAll('[data-packet]')){
   button.click();const fields=document.querySelectorAll('#packet-fields dt');
   results.push({kind:'packet',id:button.dataset.packet,theme,fields:fields.length,text:document.getElementById('packet-text').textContent,pressed:document.querySelectorAll('[data-packet][aria-pressed=true]').length,overflow:document.documentElement.scrollWidth>innerWidth});
  }
  for(const tab of tabs){tab.click();const active=[...document.querySelectorAll('[role=tabpanel]')].filter(p=>!p.hidden);results.push({kind:'tab',id:tab.dataset.topic,theme,active:active.length,matches:active[0]?.id==='panel-'+tab.dataset.topic,selected:document.querySelectorAll('[role=tab][aria-selected=true]').length,overflow:document.documentElement.scrollWidth>innerWidth})}
  for(const report of document.querySelectorAll('details.report')){report.open=true;results.push({kind:'report',id:report.id,theme,open:report.open,overflow:document.documentElement.scrollWidth>innerWidth});report.open=false}
 }
 const ids=[...document.querySelectorAll('[id]')].map(e=>e.id),duplicateIds=ids.filter((id,i)=>ids.indexOf(id)!==i);
 tabs.at(-1).focus();tabs.at(-1).dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}));
 const keyboardWrap=tabs[0]===document.activeElement&&tabs[0].getAttribute('aria-selected')==='true';
 if(document.documentElement.dataset.theme!=='light')document.getElementById('theme').click();tabs[0].click();document.querySelector('[data-packet=direct]').click();scrollTo(0,0);
 return {width:innerWidth,height:innerHeight,results,duplicateIds,keyboardWrap,svgCount:document.querySelectorAll('svg').length};
}"""
def main():
 b.run('open',PAGE.as_uri());sizes=[];counts={'tab':0,'packet':0,'report':0}
 for width in [1280,500]:
  b.run('resize',str(width),'900');state=b.value(b.run('eval',SCRIPT))
  assert state['width']==width and state['keyboardWrap'] and not state['duplicateIds'] and state['svgCount']==1
  for r in state['results']:
   counts[r['kind']]+=1;assert r['overflow'] is False,r
   if r['kind']=='tab':assert r['active']==r['selected']==1 and r['matches'],r
   elif r['kind']=='packet':assert r['fields']>=5 and r['pressed']==1 and r['text'],r
   else:assert r['open'],r
  sizes.append(state);b.run('screenshot',str(ROOT/'.lavish'/f'communications-{width}-light.png'))
  b.run('eval',"() => {document.getElementById('theme').click();document.querySelector('[data-topic=iphone]').click();document.getElementById('panel-iphone').scrollIntoView();return true}")
  b.run('screenshot',str(ROOT/'.lavish'/f'communications-{width}-iphone-dark.png'))
  b.run('eval',"() => {document.getElementById('theme').click();return true}")
 result={'status':'passed','scope':'Own offline explanatory page only','original_runtime_verified':False,'ios_runtime_verified':False,'page_sha256':hashlib.sha256(PAGE.read_bytes()).hexdigest(),'cases':counts,'viewports':sizes}
 (ROOT/'evidence/communications-browser-verification.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps({'status':'passed','actual_widths':[s['width'] for s in sizes],'cases':counts}))
if __name__=='__main__':main()
