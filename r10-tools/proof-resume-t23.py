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

def resend_button(mid):
    assert js("() => {const m=document.querySelector('.gb-user[data-message-id=\"" + mid + "\"] .gb-message-menu button[aria-label=\"Actions du message\"]');if(!m)return false;m.click();return true;}")
    wait("() => [...document.querySelectorAll('[role=menuitem]')].some(b=>b.textContent.trim()==='Renvoyer')", "menu Renvoyer")
    js("() => { [...document.querySelectorAll('[role=menuitem]')].find(b=>b.textContent.trim()==='Renvoyer').click();return true;}")

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
