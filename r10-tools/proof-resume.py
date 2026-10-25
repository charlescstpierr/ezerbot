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
events = json.loads((evidence / "cases.json").read_text())

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
assert js("() => !document.querySelector('.gb-user[data-message-id=\"" + unknown["message_id"] + "\"] .gb-status-alert')")
record("T20 renvoi serveur réel après perte du reçu", js("() => window.__requests"))
reset_fetch()
wait("() => document.getElementById('send-button').dataset.mode !== 'stop'", "réponse serveur terminée")

with sqlite3.connect(run["db"]) as db:
    db.execute("DELETE FROM bot_pages WHERE id=?", (original["page_anchor"]["page_id"],))
missing = js("async () => {const s=await (await fetch('/api/state')).json();const m=s.conversations.flatMap(c=>c.messages).find(m=>m.id===" + json.dumps(message_id) + ");return {anchor:m.payload?.page_anchor||null,present:!!m};}")
assert missing == {"anchor": None, "present": True}
request = {k:v for k,v in original.items() if k != "page_anchor"}
absent = js("async () => {const r=await fetch('/api/send',{method:'POST',headers:{'Content-Type':'application/json','X-Grokbot-Request':'local'},body:JSON.stringify(" + json.dumps(request) + ")});return {status:r.status,body:await r.json()};}")
assert absent["status"] == 409
record("T23 Page supprimée uniquement par fixture SQL", {"dto": missing, "direct": absent, "production_delete_found": False})

record("preuve terminée", {"head": run["build_sha"], "captures": len(list(shots.glob('*.png')))})
