import json, os, subprocess, sys
from pathlib import Path
wt=Path('/Users/cstpierrecharles/.treehouse/pm/ez-i295')
e=Path('/Users/cstpierrecharles/.treehouse/pm/_orch/pages/evidence/i295/final5')
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
    network=subprocess.run(['lsof','-nP','-a','-p',str(run['fake_pid']),'-i','-Fn'],capture_output=True,text=True,check=True)
    endpoints=[line[1:] for line in network.stdout.splitlines() if line.startswith('n')]
    assert endpoints and all(endpoint.startswith('127.0.0.1:') for endpoint in endpoints)
    record('faux-openai-boucle-locale',dict(pid=run['fake_pid'],endpoints=endpoints))
    (e/'owned-processes.json').write_text(json.dumps(dict(head=run['build_sha'],signed_root_valid=True,signed_home_valid=True,identities=identities),indent=2)+'\n')
    command('doctor-before','bash',str(s/'doctor.sh'),'--evidence',str(e))
    browser(e,'open',run['url'])
    wait_js(e,'() => document.querySelector("#composer-tools [data-picker=model]") ? {ready:true} : null','modèle absent')
    browser(e,'resize','1140','780')
    eval_js(e,theme_js('dark',True))
    pages_request(e)
    wait_idle()
    initial=eval_js(e,'''async () => {
      const id=document.querySelector('.page-link[aria-current="page"]').dataset.pageId;
      const page=(await GrokbotPanels.api('/api/pages/'+id)).page;
      window.proof={id,version:page.source_version,origin:page.conversation_id};
      return {id,source_version:page.source_version,anchors:page.anchors,has_previous:page.has_previous};
    }''')
    assert initial['anchors']==[] and not initial['has_previous']
    record('sans-commentaire',initial)
    shots('sans-commentaire')
    record('citation-confort',quote('Confort'))
    send_in_new_chat(e,'Garde ce commentaire ouvert.',False)
    wait_anchors(1);wait_idle()
    record('citation-famille',quote('Famille'))
    send_in_new_chat(e,'Autre commentaire à traiter.',False)
    anchors=wait_anchors(2);wait_idle()
    browser(e,'resize','1140','780')
    eval_js(e,'() => {document.querySelector(".page-canvas").scrollTop=0;return true;}')
    assert all(a['state']=='open' for a in anchors['anchors'])
    wait_js(e,'() => document.querySelectorAll(".page-anchor-number").length===2 ? {badges:2} : null','deux pastilles absentes')
    record('deux-ancres-ouvertes',anchors)
    shots('reperes-fermes')
    browser(e,'resize','1140','780')
    eval_js(e,'() => { [...document.querySelectorAll(".page-stage-bar button")].find(b=>b.textContent==="Commentaires (2)").click(); return true; }')
    shots('commentaires-ouverts')
    target=anchors['anchors'][0]
    eval_js(e,'() => { document.querySelector(".page-comment-item button").click();return true; }')
    navigation=wait_js(e,'''() => {
      const target=document.activeElement;
      return target?.dataset.messageId===MESSAGE ? {message_id:target.dataset.messageId,text:target.innerText} : null;
    }'''.replace('MESSAGE',json.dumps(target['message_id'])),'message cible non centré')
    record('voir-vrai-message',dict(expected=target,observed=navigation))
    quote('Confort');send_in_new_chat(e,'Change le prix de Confort',False)
    wait_anchors(3);wait_idle()
    edited=wait_js(e,'''async () => {
      const page=(await GrokbotPanels.api('/api/pages/'+proof.id)).page;
      return page.has_previous && page.anchors[2].state==='done' && document.querySelector('.page-canvas').innerText.includes('21 €') ? {source_version:page.source_version,has_previous:page.has_previous,anchors:page.anchors} : null;
    }''','modification absente',timeout=60)
    record('edit-page-traite',edited)
    shots('apres-modification')
    quote('21 €');send_in_new_chat(e,'Garde ce prix en commentaire.',False)
    wait_anchors(4);wait_idle()
    eval_js(e,'() => { [...document.querySelectorAll(".page-stage-bar button")].find(b=>b.textContent==="Revenir à avant").click();return true; }')
    restored=wait_js(e,'''async () => {
      const page=(await GrokbotPanels.api('/api/pages/'+proof.id)).page;
      const text=document.querySelector('.page-canvas').innerText;
      const rows=[...document.querySelectorAll('.page-comment-item')].map(el=>el.innerText);
      return page.source_version===proof.version && !page.has_previous && text.includes('19 €') && !text.includes('21 €') && rows.some(text=>text.includes('21 €')&&text.includes('Traité')) ? {page,rows,canvas:text,badges:document.querySelectorAll('.page-anchor-number').length} : null;
    }''','restauration non affichée')
    assert restored['page']['anchors'][3]['state']=='open'
    record('revenir-passage-disparu',restored)
    eval_js(e,'() => { const list=document.querySelector(".page-comments");list.scrollTop=list.scrollHeight;return true; }')
    shots('apres-restauration')
    orphan=eval_js(e,'''async () => {
      await GrokbotPanels.api('/api/conversations/delete',{id:proof.origin});
      await GrokbotPanels.refresh();document.getElementById('new-chat').click();
      document.querySelector('.page-link[data-page-id="'+proof.id+'"]').click();return {origin:proof.origin};
    }''')
    wait_js(e,'() => document.querySelector(".page-stage:not([hidden]) .page-canvas td") ? true : null','Page orpheline absente')
    quote('Famille');send_in_new_chat(e,'Reprends ce commentaire dans le nouveau fil.',False)
    new=wait_anchors(1);wait_idle()
    assert new['anchors'][0]['conversation_id']!=orphan['origin']
    current=page()
    assert current['conversation_id']==orphan['origin']
    eval_js(e,'() => { [...document.querySelectorAll(".page-stage-bar button")].find(b=>b.textContent==="Commentaires (1)").click();document.querySelector(".page-comment-item button").click();return true; }')
    actual=wait_js(e,'''() => document.activeElement?.dataset.messageId===MESSAGE ? {message_id:document.activeElement.dataset.messageId} : null'''.replace('MESSAGE',json.dumps(new['anchors'][0]['message_id'])),'message du nouveau fil non atteint')
    record('page-orpheline-vrai-fil',dict(origin=orphan['origin'],anchor=new['anchors'][0],observed=actual))
    shots('page-orpheline')
    command('doctor-after','bash',str(s/'doctor.sh'),'--evidence',str(e))
    command('compare','bash',str(s/'compare.sh'),'--evidence',str(e))
    command('owner-verdict','bash',str(s/'verdict.sh'),'--evidence',str(e),'--ref','HEAD','--status','pass')
finally:
    command('cleanup','bash',str(s/'cleanup.sh'),'--evidence',str(e))
