import json, os, subprocess, sys
from pathlib import Path
wt=Path('/Users/cstpierrecharles/.treehouse/pm/ez-i295')
e=Path('/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i295/r2/treated')
s=wt/'.cursor/skills/verify-ezerbot/scripts'
sys.path.insert(0,str(s))
from drive import browser, eval_js, wait_js, theme_js, metrics_js, pages_request, send_in_new_chat as base_send
from verify_lib import load_run, assert_created_run_root, assert_created_home, identity_matches
os.environ['CHROME_DEVTOOLS_AXI_SESSION']='pm-i295'
run=load_run(e)
cases=[]

def record(name,value):
    cases.append(dict(name=name,head=run['build_sha'],result=value))
    (e/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2)+'\n')
    print(name, 'ok', flush=True)

def command(name,*args):
    with (e/(name+'.log')).open('w') as log:
        result=subprocess.run(args,cwd=wt,stdout=log,stderr=subprocess.STDOUT)
    if result.returncode: raise RuntimeError(name+' exit '+str(result.returncode))

def wait_idle():
    wait_js(e,'() => !GrokbotPanels.busy && document.getElementById("send-button").dataset.mode !== "stop" ? {ok:true} : null','tour non terminé',timeout=60)

def quote(passage):
    return eval_js(e,'''() => {
      const passage=PASSAGE;
      const cell=[...document.querySelectorAll('.page-canvas td')].find(el=>el.textContent===passage);
      if(!cell)throw new Error('cellule absente');
      cell.scrollIntoView({block:'center'});
      const range=document.createRange();range.selectNodeContents(cell);
      getSelection().removeAllRanges();getSelection().addRange(range);
      document.dispatchEvent(new Event('selectionchange'));
      const comment=document.querySelector('.page-comment');
      if(comment.hidden)throw new Error('Commenter absent');
      comment.click();
      const chip=document.querySelector('.composer-area > .gb-quote-chip');
      if(chip.hidden||!chip.textContent.includes(passage))throw new Error('citation absente');
      return {passage,chip:chip.textContent};
    }'''.replace('PASSAGE',json.dumps(passage)))

def send_in_new_chat(evidence,text,new_chat):
    eval_js(evidence,'''() => {
      const editor=document.querySelector('.gb-editor[aria-label="Votre message"]');
      editor.focus();const range=document.createRange();range.selectNodeContents(editor);range.collapse(false);
      getSelection().removeAllRanges();getSelection().addRange(range);return true;
    }''')
    base_send(evidence,text,new_chat)

def page():
    return eval_js(e,'async () => (await GrokbotPanels.api("/api/pages/"+window.proof.id)).page')

def wait_anchors(count):
    return wait_js(e,'''async () => {
      const page=(await GrokbotPanels.api('/api/pages/'+proof.id)).page;
      return page.anchors.length===COUNT ? {anchors:page.anchors,has_previous:page.has_previous,source_version:page.source_version} : null;
    }'''.replace('COUNT',str(count)),'ancres absentes')

def shots(label):
    for theme in ('dark','light'):
      for width,height in ((1140,780),(820,560)):
        browser(e,'resize',str(width),str(height))
        first=eval_js(e,theme_js(theme,True)); second=eval_js(e,theme_js(theme,True))
        metrics=eval_js(e,metrics_js())
        if label=='commentaires-ouverts':
            eval_js(e,'() => {const canvas=document.querySelector(".page-canvas");canvas.scrollTop=canvas.scrollHeight-canvas.clientHeight;return true;}')
        assert first['pressed'] and second['pressed'] and metrics['canvas']=={'dark':'#0e0f12','light':'#ffffff'}[theme]
        path=e/'spheres/pages'/f'{label}-{theme}-{width}x{height}.png'
        path.parent.mkdir(parents=True,exist_ok=True)
        action=eval_js(e,'''() => ({text:document.body.innerText,comments:document.querySelector('.page-comments')?.innerText,badges:document.querySelectorAll('.page-anchor-number').length,lines:document.querySelectorAll('.page-anchor-line').length,canvas:document.querySelector('.page-canvas')?.innerText,overflow:document.documentElement.scrollWidth>innerWidth})''')
        browser(e,'screenshot',str(path))
        path.with_suffix('.txt').write_text(action['text'])
        path.with_suffix('.json').write_text(json.dumps(dict(action=label,metrics=metrics,observed=action),ensure_ascii=False,indent=2)+'\n')
        assert not action['overflow'],'débordement horizontal'
    record(label,{'captures':4})


try:
    assert_created_home(Path(run['home']),assert_created_run_root(Path(run['run_root']),run))
    identities={name:dict(pid=run[name+'_pid'],matches=identity_matches(run[name+'_pid'],run[name+'_identity'])) for name in ('wrapper','fake','auth')}
    assert all(item['matches'] for item in identities.values())
    (e/'owned-processes.json').write_text(json.dumps(dict(head=run['build_sha'],signed_root_valid=True,signed_home_valid=True,identities=identities),indent=2)+'\n')
    command('doctor-before','bash',str(s/'doctor.sh'),'--evidence',str(e))
    browser(e,'open',run['url'])
    wait_js(e,'() => document.querySelector("#composer-tools [data-picker=model]") ? true : null','modèle absent')
    browser(e,'resize','1140','780')
    pages_request(e);wait_idle()
    eval_js(e,"""async () => {
      const id=document.querySelector('.page-link[aria-current="page"]').dataset.pageId;
      const page=(await GrokbotPanels.api('/api/pages/'+id)).page;
      window.proof={id,version:page.source_version,origin:page.conversation_id};return true;
    }""")
    quote('Confort');send_in_new_chat(e,'Change le prix de Confort',False)
    wait_anchors(1);wait_idle()
    wait_js(e,'() => document.querySelector(".page-canvas").innerText.includes("21 €") ? true : null','prix modifié absent')
    quote('21 €');send_in_new_chat(e,'Garde ce prix en commentaire.',False)
    wait_anchors(2);wait_idle()
    eval_js(e,'() => { [...document.querySelectorAll(".page-stage-bar button")].find(b=>b.textContent==="Revenir à avant").click();return true; }')
    wait_js(e,'() => document.querySelector(".page-canvas").innerText.includes("19 €") && !document.querySelector(".page-canvas").innerText.includes("21 €") ? true : null','source non restaurée')
    result=eval_js(e,"""async () => {
      const list=document.querySelector('.page-comments');
      if(list.hidden)[...document.querySelectorAll('.page-stage-bar button')].find(b=>b.textContent==='Commentaires (2)').click();
      const rows=[...list.children].map(row=>row.innerText);
      if(!rows.some(row=>row.includes('21 €')&&row.includes('Traité')))throw new Error('passage disparu non traité dans la liste');
      list.scrollTop=list.scrollHeight;
      const page=(await GrokbotPanels.api('/api/pages/'+proof.id)).page;
      return {rows,canvas:document.querySelector('.page-canvas').innerText,source_version:page.source_version,has_previous:page.has_previous,anchors:page.anchors};
    }""")
    assert result['source_version'] and not result['has_previous']
    record('passage-disparu-traite',result)
    shots('passage-disparu-traite')
    command('doctor-after','bash',str(s/'doctor.sh'),'--evidence',str(e))
    command('compare','bash',str(s/'compare.sh'),'--evidence',str(e))
    command('owner-verdict','bash',str(s/'verdict.sh'),'--evidence',str(e),'--ref','HEAD','--status','pass')
finally:
    command('cleanup','bash',str(s/'cleanup.sh'),'--evidence',str(e))
