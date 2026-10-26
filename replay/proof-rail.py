import argparse
import json
import sys
from pathlib import Path

repo = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i561')
sys.path.insert(0, str(repo / '.cursor/skills/verify-ezerbot/scripts'))
from drive import browser, eval_js, wait_js

parser = argparse.ArgumentParser()
parser.add_argument('evidence', type=Path)
args = parser.parse_args()
e = args.evidence
checks = []

def check(name, source):
    result = wait_js(e, source, name)
    checks.append({'check': name, 'result': result})

def click(selector):
    eval_js(e, f'() => {{ document.querySelector({json.dumps(selector)}).click(); return true; }}')

def shot(name):
    target = e / 'navigation'
    target.mkdir(exist_ok=True)
    browser(e, 'screenshot', str(target / f'{name}.png'))
    (target / f'{name}.txt').write_text(eval_js(e, '() => document.body.innerText'))

browser(e, 'resize', '820', '560')
check('cinq destinations nommées', '''() => {
  const names=[...document.querySelectorAll('[data-sidebar-destination]')].map(b=>b.getAttribute('aria-label'));
  if (JSON.stringify(names)!==JSON.stringify(['Conversations','Sujets','Routines','Notifications','Réglages'])) return false;
  return names;
}''')
check('Pages masqué si vide', "() => document.querySelector('.sidebar-pages').hidden")
eval_js(e, "() => { [...document.querySelectorAll('.sidebar-bot')].find(b=>b.lastElementChild.textContent==='Atelier').click(); return true; }")
check('bot courant Atelier', "() => document.getElementById('composer-bot-name').textContent==='Atelier'")
click('[data-sidebar-destination=routines]')
check('Routines du bot courant et actif', "() => document.querySelector('.routines-dialog')?.open && document.querySelector('.routines-dialog select option:checked')?.textContent==='Atelier' && document.querySelector('[data-sidebar-destination=routines]').getAttribute('aria-pressed')==='true'")
shot('routines-820x560')
browser(e, 'press', 'Escape')
check('Échap ferme Routines', "() => !document.querySelector('.routines-dialog').open && document.querySelector('[data-sidebar-destination=conversations]').getAttribute('aria-pressed')==='true'")
click('[data-sidebar-destination=notifications]')
check('Notifications ouvertes et actif', "() => document.querySelector('.notifications-dialog').open && document.querySelector('[data-sidebar-destination=notifications]').getAttribute('aria-pressed')==='true'")
shot('notifications-820x560')
browser(e, 'press', 'Escape')
click('#settings-button')
check('Réglages ouverts et actif', "() => document.getElementById('settings-dialog').open && document.getElementById('settings-button').getAttribute('aria-pressed')==='true'")
shot('reglages-820x560')
browser(e, 'press', 'Escape')
click('.memory-entry')
check('Mémoire accessible', "() => !document.querySelector('.memory-space').hidden")
shot('memoire-820x560')
eval_js(e, "() => { document.querySelector('[data-sidebar-destination=topics]').focus(); return true; }")
browser(e, 'press', 'Enter')
check('Sujets au clavier', "() => document.activeElement.id==='new-topic' && document.querySelector('.memory-space').hidden && document.querySelector('[data-sidebar-destination=topics]').getAttribute('aria-pressed')==='true'")
shot('sujets-820x560')
click('#new-topic')
check('vrai formulaire de sujet', "() => Boolean(document.querySelector('dialog[open] #topic-name-input'))")
browser(e, 'press', 'Escape')
eval_js(e, "() => { document.getElementById('home-link').focus(); return true; }")
browser(e, 'press', 'Tab')
check('Tab atteint Conversations', "() => document.activeElement.getAttribute('aria-label')==='Conversations'")
browser(e, 'press', 'Enter')
check('Conversations au clavier', "() => document.activeElement.id==='conversation-list' && document.querySelector('[data-sidebar-destination=conversations]').getAttribute('aria-pressed')==='true'")
browser(e, 'press', 'Meta+k')
check('raccourci recherche', "() => document.activeElement.id==='search'")
eval_js(e, "() => { document.getElementById('search').value='Notes du projet 32'; document.getElementById('search').dispatchEvent(new Event('input',{bubbles:true})); return true; }")
check('recherche filtre réellement', "() => document.querySelectorAll('#conversation-list .conversation-item').length===1 && document.querySelector('#conversation-list .conversation-name').textContent==='Notes du projet 32'")
eval_js(e, "() => { document.getElementById('search').value=''; document.getElementById('search').dispatchEvent(new Event('input',{bubbles:true})); return true; }")
click('#conversation-list .conversation-item')
check('sélection du fil', "() => Boolean(document.querySelector('#conversation-list [aria-current=page]'))")
browser(e, 'press', 'Meta+n')
check('nouveau fil garde son bot', "() => document.getElementById('conversation-title').textContent==='Nouvelle conversation' && document.getElementById('composer-bot-name').textContent==='Atelier' && !document.querySelector('#conversation-list [aria-current=page]')")
eval_js(e, "() => { document.querySelector('.sidebar-content').scrollTop=0; return true; }")
shot('conversations-820x560')
check('liste longue sans recouvrement', '''() => {
  const content=document.querySelector('.sidebar-content'); content.scrollTop=content.scrollHeight;
  const last=document.querySelector('#conversation-list .conversation-item:last-child');
  const r=last.getBoundingClientRect(), s=content.getBoundingClientRect(), rail=document.querySelector('.sidebar-rail').getBoundingClientRect();
  if (!(content.scrollHeight>content.clientHeight && r.top>=s.top && r.bottom<=s.bottom && rail.right<=s.left)) return false;
  return {items:document.querySelectorAll('#conversation-list .conversation-item').length, last:r.toJSON(), content:s.toJSON(), rail:rail.toJSON()};
}''')
shot('defilement-820x560')
click('#conversation-list .conversation-item:last-child')
check('dernier fil sélectionnable', "() => document.querySelector('#conversation-list .conversation-item:last-child').getAttribute('aria-current')==='page'")
check('plancher typographique et pas de Local', '''() => {
  const small=[...document.querySelectorAll('#sidebar *')].filter(e=>e.getClientRects().length && [...e.childNodes].some(n=>n.nodeType===3 && n.textContent.trim()) && parseFloat(getComputedStyle(e).fontSize)<11.5);
  return small.length===0 && !/\\bLocal\\b/i.test(document.getElementById('sidebar').textContent);
}''')
eval_js(e, "() => { document.querySelector('.sidebar-content').scrollTop=0; return true; }")
click('[data-new-bot]')
check('plus ouvre le vrai éditeur bot', "() => document.getElementById('bot-dialog').open && Boolean(document.getElementById('bot-name'))")
browser(e, 'press', 'Escape')
(e / 'rail-checks.json').write_text(json.dumps(checks, indent=2, ensure_ascii=False)+'\n')
print(f'rail: {len(checks)} contrôles réussis')
