import json
import os
from pathlib import Path
import subprocess
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

for status in (400,):
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

record("preuve terminée", {"head":run["build_sha"], "captures":len(list(shots.glob("*.png")))})
