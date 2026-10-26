import json
import sys
from pathlib import Path

repo = Path('/Users/cstpierrecharles/.treehouse/pm/ez-i561')
sys.path.insert(0, str(repo / '.cursor/skills/verify-ezerbot/scripts'))
from drive import browser, eval_js, wait_js
from capture import capture

e = Path(sys.argv[1])
eval_js(e, "() => { document.querySelector('[data-new-bot]').focus(); return true; }")
browser(e, 'press', 'Enter')
wait_js(e, "() => document.getElementById('bot-dialog').open", 'éditeur bot non ouvert')
eval_js(e, "() => { const i=document.getElementById('bot-name'); i.value='Assistant preuve'; i.dispatchEvent(new Event('input',{bubbles:true})); document.getElementById('bot-save').click(); return true; }")
created = wait_js(e, "() => { const bot=GrokbotPanels.state.bots.find(b=>b.name==='Assistant preuve'); return bot && !document.getElementById('bot-dialog').open ? {id:bot.id,name:bot.name} : false; }", 'bot non créé')
eval_js(e, "() => { [...document.querySelectorAll('.sidebar-bot')].find(b=>b.lastElementChild.textContent==='Assistant preuve').click(); return true; }")
browser(e, 'press', 'Meta+n')
wait_js(e, "() => document.getElementById('composer-bot-name').textContent==='Assistant preuve' && document.getElementById('conversation-title').textContent==='Nouvelle conversation'", 'nouveau fil sans bot choisi')
eval_js(e, "() => { document.querySelector('.gb-editor[aria-label=\"Votre message\"]').focus(); return true; }")
browser(e, 'type', 'Message de preuve du nouveau bot.')
wait_js(e, "() => !document.getElementById('send-button').disabled", 'envoi non disponible')
browser(e, 'press', 'Enter')
try:
    sent = wait_js(e, """() => {
      const bot=GrokbotPanels.state.bots.find(b=>b.name==='Assistant preuve');
      const c=GrokbotPanels.state.conversations.find(c=>c.bot_id===bot.id && c.messages.some(m=>m.role==='user' && m.body==='Message de preuve du nouveau bot.' && ['accepted','echoed'].includes(m.state)));
      return c && document.querySelector('#conversation-list [aria-current=page]')?.dataset.conversationId===c.id ? {conversation:c.id,bot:c.bot_id,states:c.messages.filter(m=>m.role==='user').map(m=>m.state)} : false;
    }""", 'nouvelle conversation non enregistrée', timeout=30)
except SystemExit:
    diagnostic=eval_js(e, """() => ({editorLength:document.querySelector('.gb-editor')?.innerText.length,selected:document.querySelector('#conversation-list [aria-current=page]')?.dataset.conversationId,conversations:GrokbotPanels.state.conversations.map(c=>({id:c.id,bot:c.bot_id,messages:c.messages.filter(m=>m.role==='user').map(m=>({state:m.state,bodyMatches:m.body==='Message de preuve du nouveau bot.',length:m.body?.length}))}))})""")
    (e / 'creation-diagnostic.json').write_text(json.dumps(diagnostic,indent=2)+'\n')
    raise
browser(e, 'resize', '820', '560')
capture(e, e / 'creation-820x560.png')
(e / 'creation-820x560.txt').write_text(eval_js(e, '() => document.body.innerText'))
(e / 'creation-checks.json').write_text(json.dumps({'created':created,'sent':sent,'keyboard':['Enter on plus','Meta+n','type message','Enter to send']},indent=2)+'\n')
print('création bot et conversation: ok')
