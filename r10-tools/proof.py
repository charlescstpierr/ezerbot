import json
import os
from pathlib import Path
import subprocess
import sqlite3
import sys
import time

evidence = Path(sys.argv[1]).resolve()
run = json.loads((evidence / "run.json").read_text())
assert run["session"] == "pm-i294"
assert os.environ.get("CHROME_DEVTOOLS_AXI_SESSION") == "pm-i294"
axi = "/Users/cstpierrecharles/.local/bin/chrome-devtools-axi"
shots = evidence / "spheres" / "pages"
shots.mkdir(parents=True, exist_ok=True)
events = []

def command(*args):
    result = subprocess.run([axi, *map(str, args)], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(" ".join(map(str, args[:1])) + " " + (result.stdout + result.stderr)[:2000])
    return result.stdout

def js(source):
    raw = command("eval", source)
    value = next((line[8:] for line in raw.splitlines() if line.startswith("result: ")), None)
    if value is None:
        raise AssertionError(raw[:1200])
    for _ in range(4):
        try:
            value = json.loads(value)
        except (ValueError, TypeError):
            break
    return value

def wait(source, name):
    for _ in range(80):
        value = js(source)
        if value:
            return value
        time.sleep(.35)
    raise AssertionError("Attente échouée " + name)

def record(name, data):
    events.append({"case": name, "result": data})
    (evidence / "cases.json").write_text(json.dumps(events, ensure_ascii=False, indent=2))
    print(name, json.dumps(data, ensure_ascii=False), flush=True)

def capture(name):
    command("screenshot", shots / (name + ".png"))
    text = js("() => document.body.innerText")
    (shots / (name + ".txt")).write_text(text)

def type_text(text):
    assert js("() => { const e=document.querySelector('.gb-editor[aria-label=\"Votre message\"]'); e.focus(); return document.execCommand('insertText',false," + json.dumps(text) + "); }")

def send():
    wait("() => { const b=document.getElementById('send-button'); if(b.disabled) return false; b.click(); return true; }", "envoi disponible")

def quote():
    if not js("() => [...document.querySelectorAll('.page-canvas td')].some(c=>c.textContent==='Confort')"):
        assert js("() => {const p=document.querySelector('.page-link');if(!p)return false;p.click();return true;}")
        wait("() => [...document.querySelectorAll('.page-canvas td')].some(c=>c.textContent==='Confort')", "Page ouverte pour citer")
    assert js("() => { document.activeElement.blur(); const c=[...document.querySelectorAll('.page-canvas td')].find(x=>x.textContent==='Confort'); if(!c) return false; c.scrollIntoView({block:'center'}); const r=document.createRange();r.selectNodeContents(c);const s=getSelection();s.removeAllRanges();s.addRange(r);document.dispatchEvent(new Event('selectionchange'));return true; }")
    wait("() => !document.querySelector('.page-comment').hidden", "Commenter")
    assert js("() => { document.querySelector('.page-comment').click();return !document.querySelector('.composer-area > .gb-quote-chip').hidden; }")

def clean_editor():
    js("() => { const e=document.querySelector('.gb-editor[aria-label=\"Votre message\"]'); e.focus();document.execCommand('selectAll');document.execCommand('delete');return true; }")

def idle():
    wait("() => !window.GrokbotPanels.busy && !document.querySelector('.gb-stop')", "fin de soumission")

def reset_fetch():
    js("() => { if(window.__originalFetch) window.fetch=window.__originalFetch; window.__originalFetch=null;return true; }")

def intercept(mode):
    return js("() => { window.__originalFetch ||= window.fetch; window.__requests=[]; window.__mode=" + json.dumps(mode) + "; window.fetch=async (url,opts)=>{const u=String(url);if(u==='/api/send'||u==='/api/messages/resend'){const body=JSON.parse(opts.body);window.__requests.push({route:u,body});if(window.__mode==='network'&&u==='/api/send')throw new Error('Interruption de preuve');if(typeof window.__mode==='number'&&u==='/api/send')return new Response(JSON.stringify({error:'Refus contrôlé '+window.__mode}),{status:window.__mode,headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'}});if(window.__mode==='fallback'&&u==='/api/messages/resend')return new Response(JSON.stringify({error:'Message introuvable'}),{status:404,headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'}});}return window.__originalFetch(url,opts);}; return true; }")

def ready():
    command("open", run["url"])
    wait("() => !!document.querySelector('.gb-editor') && !!document.querySelector('#composer-tools [data-picker=model]')", "éditeur et modèle")

ready()
command("resize", 1140, 780)
type_text("Fais-moi une page des forfaits")
send()
wait("() => document.querySelectorAll('.page-canvas td').length >= 3", "Page Trois forfaits")
wait("() => !window.GrokbotPanels.busy && document.getElementById('send-button').dataset.mode !== 'stop'", "réponse Page terminée")
record("Page réelle du faux fournisseur", js("() => ({title:document.querySelector('.page-stage').innerText,model:document.querySelector('[data-picker=model]').innerText})"))

assert js("() => {document.activeElement.blur();const cells=[...document.querySelectorAll('.page-canvas td')];const a=cells.find(c=>c.textContent==='Confort'),b=cells.find(c=>c.textContent==='Famille');const r=document.createRange();r.setStart(a.firstChild,0);r.setEnd(b.firstChild,b.firstChild.length);const s=getSelection();s.removeAllRanges();s.addRange(r);document.dispatchEvent(new Event('selectionchange'));return true;}")
wait("() => !document.querySelector('.page-comment').hidden", "Commenter plusieurs cellules")
js("() => {document.querySelector('.page-comment').click();return true;}")
intercept(None)
type_text("Commentaire sur plusieurs cellules")
send()
wait("() => !window.GrokbotPanels.busy && document.querySelector('.composer-area > .gb-quote-chip').hidden", "citation plusieurs cellules envoyée")
multi = js("() => window.__requests[0].body")
assert "Confort" in multi["page_anchor"]["passage"] and "Famille" in multi["page_anchor"]["passage"]
assert js("() => [...document.querySelectorAll('.gb-user .gb-quote-chip')].some(c=>c.innerText.includes('Confort')&&c.innerText.includes('Famille'))")
record("Non-régression sélection multi-cellules", multi)
capture("citation-multicellules-envoyee")
reset_fetch()
wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "réponse multi-cellules terminée")
invalid = {**multi, "message_id": "preuve-espaces-invalides", "page_anchor": {**multi["page_anchor"], "passage": "Con fort"}}
result = js("async () => {const r=await fetch('/api/send',{method:'POST',headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'},body:JSON.stringify(" + json.dumps(invalid) + ")});return {status:r.status,body:await r.json()};}")
record("Non-régression espaces jamais supprimés", result)
assert result["status"] == 400

for order in ("file-first", "send-first"):
    js("async () => {const s=await (await fetch('/api/state')).json(),p=s.pages.by_bot.grokbot[0];await fetch('/api/conversations/delete',{method:'POST',headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'},body:JSON.stringify({id:p.conversation_id})});document.getElementById('new-chat').click();await window.GrokbotPanels.refresh();document.querySelector('.page-link').click();return true;}")
    wait("() => document.querySelectorAll('.page-canvas td').length>=3", "Page orpheline avant fichier")
    quote()
    intercept(400)
    js("() => {const prior=window.fetch;window.__creates=0;let release;const gate=new Promise(r=>release=r);window.__createRelease=release;window.__holdCreate=" + ("true" if order == "send-first" else "false") + ";window.fetch=async(u,o)=>{if(String(u)==='/api/conversations'){window.__creates++;if(window.__holdCreate)await gate;}return prior(u,o);};return true;}")
    if order == "send-first":
        type_text("Brouillon avec ajout pendant création")
        send()
        wait("() => window.__creates===1", "création retenue")
    js("() => {const transfer=new DataTransfer();transfer.items.add(new File(['Document synthétique de preuve'], 'preuve.txt', {type:'text/plain'}));const input=document.querySelector('.attachment-picker');input.files=transfer.files;input.dispatchEvent(new Event('change'));return true;}")
    if order == "send-first":
        js("() => {window.__createRelease();return true;}")
    else:
        wait("() => document.querySelector('.attachment-list')?.innerText.includes('preuve.txt') && document.querySelector('.attachment-chip')?.dataset.status==='ready'", "fichier prêt")
        type_text("Brouillon créé par fichier")
        send()
    wait("() => !window.GrokbotPanels.busy && document.getElementById('toast').innerText.includes('Refus contrôlé')", "refus avec fichier")
    assert js("() => window.__creates===1 && !document.querySelector('.composer-area > .gb-quote-chip').hidden && document.querySelector('.attachment-list').innerText.includes('preuve.txt')")
    record("N01/N04 T24 " + order, js("() => ({creations:window.__creates,quote:document.querySelector('.composer-area > .gb-quote-chip').innerText,file:document.querySelector('.attachment-list').innerText,requests:window.__requests})"))
    wait("() => document.querySelector('.attachment-chip')?.dataset.status==='ready'", "import terminé avant réessai")
    js("() => {window.__mode=null;return true;}")
    send()
    wait("() => !window.GrokbotPanels.busy && document.querySelector('.composer-area > .gb-quote-chip').hidden", "réessai citation et fichier")
    request = js("() => window.__requests.at(-1).body")
    assert request["page_anchor"]["passage"] == "Confort" and len(request["attachments"]) == 1
    record("T24 réessai complet " + order, request)
    reset_fetch()
    wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "fin réponse fichier")

for status in (400, 409):
    info = js("async () => { const s=await (await fetch('/api/state')).json();const p=s.pages.by_bot.grokbot[0];await fetch('/api/conversations/delete',{method:'POST',headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'},body:JSON.stringify({id:p.conversation_id})});document.getElementById('new-chat').click();await window.GrokbotPanels.refresh();return {page:p.id,origin:p.conversation_id}; }")
    js("() => { document.querySelector('.page-link').click();return true; }")
    wait("() => document.querySelectorAll('.page-canvas td').length >= 3", "Page orpheline")
    quote()
    intercept(status)
    type_text("Retouche refusée " + str(status))
    send()
    wait("() => !window.GrokbotPanels.busy && document.getElementById('toast').innerText.includes('Refus contrôlé')", "refus")
    data = js("() => ({requests:window.__requests,chip:!document.querySelector('.composer-area > .gb-quote-chip').hidden,text:document.querySelector('.gb-editor').innerText,selected:localStorage.getItem('grokbot.selected')})")
    assert data["chip"] and data["requests"][0]["body"]["page_anchor"]["passage"] == "Confort"
    assert data["selected"] != info["origin"]
    record("T01/T02 refus " + str(status), data)
    for theme in ("dark", "light"):
        js("() => { document.getElementById('settings-button').click();document.querySelector('[data-theme-choice=\"" + theme + "\"]').click();document.getElementById('settings-dialog').close();return true; }")
        for width, height in ((1140, 780), (820, 560)):
            command("resize", width, height)
            time.sleep(.3)
            layout = js("() => {const c=document.querySelector('.composer-area > .gb-quote-chip').getBoundingClientRect();const e=document.querySelector('.gb-editor').getBoundingClientRect();return {theme:document.documentElement.dataset.theme,canvas:getComputedStyle(document.documentElement).getPropertyValue('--canvas').trim(),above:c.bottom<=e.top+1,width:innerWidth,height:innerHeight};}")
            assert layout["above"] and layout["theme"] == theme
            capture(f"orphan-{status}-{theme}-{width}")
            record(f"capture {status} {theme} {width}", layout)
    command("resize", 1140, 780)
    js("() => {window.__mode=null;return true;}")
    send()
    wait("() => !window.GrokbotPanels.busy && document.querySelector('.composer-area > .gb-quote-chip').hidden", "nouvel essai conservé")
    data = js("() => window.__requests")
    assert data[0]["body"]["page_anchor"] == data[1]["body"]["page_anchor"]
    record("nouvel essai " + str(status), data)
    reset_fetch()
    wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "réponse terminée")

quote()
intercept(None)
js("() => { const prior=window.fetch;let release;const gate=new Promise(r=>release=r);window.__release=release;let first=true;window.fetch=async(u,o)=>{if(String(u)==='/api/send'&&first){first=false;await gate;}return prior(u,o);};return true;}")
type_text("Accusé tardif de la première citation")
send()
wait("() => window.GrokbotPanels.busy", "envoi retenu")
js("() => {document.querySelector('.gb-quote-remove').click();return true;}")
quote()
type_text("Nouvelle citation du même passage")
js("() => {window.__release();return true;}")
wait("() => !window.GrokbotPanels.busy", "ancien accusé")
assert js("() => !document.querySelector('.composer-area > .gb-quote-chip').hidden && document.querySelector('.gb-editor').innerText.includes('Nouvelle citation')")
capture("citation-recente-apres-accuse")
record("T12 nouvelle citation après ancien accusé", js("() => ({chip:document.querySelector('.composer-area > .gb-quote-chip').innerText,draft:document.querySelector('.gb-editor').innerText})"))
wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "première réponse terminée")
send()
wait("() => !window.GrokbotPanels.busy && document.querySelector('.composer-area > .gb-quote-chip').hidden", "seconde citation envoyée")
requests = js("() => window.__requests")
assert len(requests) == 2 and requests[0]["body"]["page_anchor"] == requests[1]["body"]["page_anchor"]
reset_fetch()
wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "seconde réponse terminée")
quote()
js("() => {const i=document.querySelector('.page-rename input');i.value='Forfaits vérifiés';i.dispatchEvent(new Event('change'));return true;}")
wait("() => document.querySelector('.composer-area > .gb-quote-chip').innerText.includes('Forfaits vérifiés') && [...document.querySelectorAll('.page-link')].some(p=>p.textContent==='Forfaits vérifiés')", "titre courant")
record("T10 renommage", js("() => document.querySelector('.composer-area > .gb-quote-chip').innerText"))
intercept("network")
type_text("Tentative récupérée après rechargement")
send()
wait("() => !window.GrokbotPanels.busy && !!document.querySelector('.gb-user .gb-status-alert')", "tentative récupérable")
original = js("() => window.__requests[0].body")
record("T22 tentative avant journal", original)
assert str(Path(run["db"]).resolve()).startswith(str(evidence.parent / "r10-tmp"))
with sqlite3.connect(run["db"]) as db:
    account, bot = db.execute("SELECT account_id,bot_id FROM wrapper_conversations WHERE id=?", (original["conversation_id"],)).fetchone()
    db.execute("INSERT INTO wrapper_group_inputs VALUES (?,?,?,?,?,?,?)", (account, original["conversation_id"], original["message_id"], "[]", json.dumps([bot]), 0, 0))
    db.execute("INSERT INTO wrapper_messages(account_id,conversation_id,message_id,kind,created_at) VALUES (?,?,?,'submission',?)", (account, original["conversation_id"], original["message_id"], int(time.time()*1000)))
    anchor = original["page_anchor"]
    db.execute("INSERT INTO wrapper_page_anchors VALUES (?,?,?,?,?,'open',?)", (original["conversation_id"], original["message_id"], anchor["page_id"], anchor["passage"], anchor["source_version"], int(time.time()*1000)))
raw = js("async () => (await (await window.__originalFetch('/api/state')).json()).conversations.flatMap(c=>c.messages).some(m=>m.id===" + json.dumps(original["message_id"]) + ")")
assert raw is False
record("T22 métadonnées commitées sans entrée journal", {"message_id": original["message_id"], "dto_present": raw, "fixture": "même transaction synthétique que crates/wrapper/src/tests.rs"})
reset_fetch()
ready()
wait("() => !!document.querySelector('.gb-user .gb-status-alert')", "tentative après rechargement")
intercept(None)
message_id = original["message_id"]

def resend_button(mid):
    assert js("() => {const m=document.querySelector('.gb-user[data-message-id=\"" + mid + "\"] .gb-message-menu button[aria-label=\"Actions du message\"]');if(!m)return false;m.click();return true;}")
    wait("() => [...document.querySelectorAll('[role=menuitem]')].some(b=>b.textContent.trim()==='Renvoyer')", "menu Renvoyer")
    js("() => { [...document.querySelectorAll('[role=menuitem]')].find(b=>b.textContent.trim()==='Renvoyer').click();return true;}")

record("T19 libellé réel", js("() => ({repriseDirecte:[...document.querySelectorAll('.gb-retry')].some(b=>b.textContent.includes('Reprendre')),status:document.querySelector('.gb-user .gb-status-alert').textContent})"))
resend_button(message_id)
wait("() => window.__requests.some(r=>r.route==='/api/send') && !window.GrokbotPanels.busy", "reprise effective par repli")
retried = js("() => window.__requests.find(r=>r.route==='/api/send').body")
assert retried == original
record("T19/T22 menu React Renvoyer après rechargement", js("() => window.__requests"))
reset_fetch()
wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "reprise terminée")
capture("reprise-react")

for mode in ("fallback", 400, 409, "network", "server400", "server409", "servernetwork"):
    intercept(mode)
    js("() => {const prior=window.fetch;window.fetch=async(u,o)=>{if(String(u)==='/api/state'){const r=await prior(u,o),s=await r.json();for(const c of s.conversations){const i=c.messages.findIndex(m=>m.id===" + json.dumps(message_id) + ");if(i>=0){c.messages=c.messages.slice(0,i+1);c.messages[i].state='queued';}}return new Response(JSON.stringify(s),{status:200,headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'}});}if(String(u)==='/api/messages/resend')return new Response(JSON.stringify({error:'Message introuvable'}),{status:404,headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'}});return prior(u,o);};window.GrokbotPanels.refresh();return true;}")
    wait("() => !!document.querySelector('.gb-user[data-message-id=\"" + message_id + "\"] .gb-status-alert')", "vue en attente synthétique")
    if isinstance(mode, str) and mode.startswith("server"):
        status = mode.removeprefix("server")
        js("() => {const prior=window.fetch;window.fetch=async(u,o)=>{if(String(u)==='/api/messages/resend'){window.__requests.push({route:String(u),body:JSON.parse(o.body)});" + ("throw new Error('Interruption de renvoi');" if status == "network" else "return new Response(JSON.stringify({error:'Refus serveur contrôlé'}),{status:" + status + ",headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'}});") + "}return prior(u,o);};return true;}")
    resend_button(message_id)
    wait("() => !window.GrokbotPanels.busy && window.__requests.length > 0", "renvoi traité")
    data = js("() => window.__requests")
    for request in data:
        if request["route"] == "/api/send":
            assert request["body"]["page_anchor"] == original["page_anchor"]
        assert request["body"]["message_id"] == message_id
    assert js("() => !!document.querySelector('.gb-user[data-message-id=\"" + message_id + "\"] .gb-quote-chip')")
    record("T20 React repli " + str(mode), {"requests": data, "state_fixture": "queued uniquement dans DTO navigateur, message serveur intact"})
    reset_fetch()
    js("() => {window.GrokbotPanels.refresh();return true;}")
    wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "renvoi terminé")

clean_editor()
quote()
intercept(None)
js("() => {const c=document.getElementById('simulate-loss');c.checked=true;c.dispatchEvent(new Event('change'));return true;}")
type_text("Envoi inconnu avec ancre durable")
send()
wait("() => !window.GrokbotPanels.busy && window.__requests.length>0", "perte de reçu simulée")
unknown = js("() => window.__requests[0].body")
wait("() => !!document.querySelector('.gb-user[data-message-id=\"" + unknown["message_id"] + "\"] .gb-status-alert')", "message réellement incertain")
resend_button(unknown["message_id"])
wait("() => !window.GrokbotPanels.busy && window.__requests.some(r=>r.route==='/api/messages/resend')", "renvoi serveur")
assert js("() => {const m=document.querySelector('.gb-user[data-message-id=\"" + unknown["message_id"] + "\"]');return !!m && !m.querySelector('.gb-status-alert');}")
record("T20 renvoi serveur réel après perte du reçu", js("() => window.__requests"))
reset_fetch()
wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "réponse serveur terminée")

clean_editor()
quote()
intercept(None)
js("() => {const c=document.getElementById('simulate-loss');c.checked=true;c.dispatchEvent(new Event('change'));return true;}")
type_text("Reprise avec Page absente par fixture")
send()
wait("() => !window.GrokbotPanels.busy && window.__requests.length>0", "soumission avant Page absente")
absent_original = js("() => window.__requests[0].body")
absent_id = absent_original["message_id"]
wait("() => !!document.querySelector('.gb-user[data-message-id=\"" + absent_id + "\"] .gb-status-alert')", "soumission non livrée avant suppression synthétique")
reset_fetch()
with sqlite3.connect(run["db"]) as db:
    db.execute("DELETE FROM bot_pages WHERE id=?", (absent_original["page_anchor"]["page_id"],))
missing = js("async () => {const s=await (await fetch('/api/state')).json();const m=s.conversations.flatMap(c=>c.messages).find(m=>m.id===" + json.dumps(absent_id) + ");return {anchor:m.payload?.page_anchor||null,present:!!m};}")
assert missing == {"anchor": None, "present": True}
request = {k:v for k,v in absent_original.items() if k != "page_anchor"}
absent = js("async () => {const r=await fetch('/api/send',{method:'POST',headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'},body:JSON.stringify(" + json.dumps(request) + ")});return {status:r.status,body:await r.json()};}")
assert absent["status"] == 409
record("T23 Page supprimée uniquement par fixture SQL", {"dto": missing, "direct": absent, "production_delete_found": False})
ready()
intercept(None)
resend_button(absent_id)
wait("() => !window.GrokbotPanels.busy && window.__requests.some(r=>r.route==='/api/messages/resend')", "renvoi serveur avec Page absente")
assert js("() => {const m=document.querySelector('.gb-user[data-message-id=\"" + absent_id + "\"]');return !!m && !m.querySelector('.gb-status-alert');}")
assert js("() => window.__requests.every(r=>r.route==='/api/messages/resend')")
record("T23 renvoi serveur réel après reload sans Page", js("() => window.__requests"))
reset_fetch()

record("preuve terminée", {"head": run["build_sha"], "captures": len(list(shots.glob('*.png')))})
