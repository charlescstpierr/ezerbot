import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.attributes = {}; this.className = ""; this.hidden = false; }
  set textContent(value) { this.text = String(value); this.children = []; }
  get textContent() { return (this.text || "") + this.children.map(c => c.textContent).join(""); }
  append(...children) { this.children.push(...children); }
  prepend(...children) { this.children.unshift(...children); }
  replaceChildren(...children) { this.children = children; this.text = ""; }
  setAttribute(k,v) { this.attributes[k] = String(v); }
  removeAttribute(k) { delete this.attributes[k]; }
  after(e) { this.children.push(e); }
  showModal() { this.open = true; }
  close() { this.open = false; }
  showPopover() { this.popover = true; }
  hidePopover() { this.popover = false; }
  matches() { return Boolean(this.popover); }
  get classList() { return { toggle: (name,enabled) => { const classes=new Set(this.className.split(" ").filter(Boolean)); if(enabled)classes.add(name);else classes.delete(name);this.className=[...classes].join(" "); } }; }
}
const all = e => [e,...e.children.flatMap(all)];
const byText = (e,text) => all(e).find(n => n.tagName === "button" && n.textContent === text);
function fixture() {
  const top = new Element("div"), panels = new Element("div"), settings = new Element("div"), body = new Element("body");
  let registration; const listeners = {}, calls = [], notices = [], timers = []; let nativeCalls = 0;
  const state = { bots:[{id:"grokbot",name:"Ezerbot"}], conversations:[{id:"chat",title:"Démonstration"}], screen:{bots:[],fake:true,session:null} };
  const context = { conversation:{id:"chat",title:"Démonstration",kind:"chat"},bot:state.bots[0] };
  const host = { state,register(r) { registration=r;r.settingsSection(settings);r.onState(state,context); }, notify(n) {notices.push(n);},
    async api(path, payload) { calls.push([path,payload]);if(path.endsWith("/bot"))state.screen.bots=[payload];if(path.endsWith("/grant"))state.screen.session={...payload,id:7,expires_at:Date.now()+60000,paused:false};if(path.endsWith("/revoke"))state.screen.session=null;if(path.endsWith("/fake-input"))state.screen.session.paused=true;if(path.endsWith("/resume"))state.screen.session.paused=false; },
    async refresh(){registration.onState(state,context);},
  };
  const window = { GrokbotPanels:host,addEventListener(k,fn){(listeners[k]||=[]).push(fn);},webkit:{messageHandlers:{ezerbotNotifications:{postMessage(){nativeCalls++;}}}} };
  const document = { body,createElement:t=>new Element(t),querySelector:s=>s === "dialog[open]" ? null : top,getElementById:()=>panels,addEventListener(){} };
  runInNewContext(readFileSync(new URL("../../ui/panels/screen.js",import.meta.url),"utf8"),{window,document,HTMLDialogElement:Element,setTimeout:fn=>{timers.push(fn);return timers.length;},clearTimeout(){}});
  return {top,panels,settings,body:top,state,registration,host,listeners,calls,notices,timers,window,nativeCalls:()=>nativeCalls};
}
test("per-bot screen setting is off, test permission buttons never call a native bridge", async () => {
  const f=fixture();const input=all(f.settings).find(e=>e.tagName==="input");assert.equal(input.checked,false);assert.equal(input.attributes.role,"switch");
  await byText(f.settings,"Ouvrir Accessibilité").onclick();await byText(f.settings,"Ouvrir Enregistrement de l’écran").onclick();assert.equal(f.nativeCalls(),0);assert.equal(f.notices.length,2);
  input.checked=true;await input.onchange();assert.equal(f.calls[0][0],"/api/screen/bot");assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0][1])),{bot_id:"grokbot",enabled:true});
});
test("emergency revoke bypasses a pending resume request", async () => {
  const f = fixture();
  f.state.screen = { ...f.state.screen, session: { id: 7, paused: true, expires_at: Date.now() + 60000 } };
  await f.host.refresh();
  const api = f.host.api;
  let finishResume;
  f.host.api = async (path, payload) => {
    if (path.endsWith("/resume")) await new Promise(resolve => { finishResume = resolve; });
    else await api(path, payload);
  };
  const pending = byText(f.body, "Reprendre").onclick();
  assert.equal(typeof finishResume, "function");
  await byText(f.body, "Arrêter").onclick();
  assert.equal(f.calls.at(-1)[0], "/api/screen/revoke");
  assert.equal(f.state.screen.session, null);
  finishResume();
  await pending;
  assert.equal(f.state.screen.session, null);
});
test("expiry clears the banner even without a polling revision", async () => {
  const f=fixture();
  f.state.screen={...f.state.screen,session:{id:7,expires_at:Date.now()+1000,paused:false}};
  await f.host.refresh();
  assert.equal(f.registration.wantsPolling(),true);
  f.host.refresh=async()=>{}; // an offline refresh must not leave a false active banner
  f.timers.at(-1)();
  assert.equal(f.registration.wantsPolling(),false);
  assert.equal(all(f.body).find(e=>e.className==="screen-overlay").hidden,true);
});
test("scoped consent keeps shell input active; outside input pauses; resume and Escape work", async () => {
  const f=fixture();f.state.screen.bots=[{bot_id:"grokbot",enabled:true}];await f.host.refresh();
  byText(f.top,"Écran du Mac").onclick();const dialog=all(f.panels).find(e=>e.className.includes("screen-dialog"));
  const consent=all(dialog).find(e=>e.name==="screen-consent");assert.equal(consent.required,true);assert.match(dialog.textContent,/Fermez vos fenêtres privées/);
  const allow = byText(dialog,"Accorder cette session"); assert.equal(allow.disabled,true); consent.checked=true;consent.onchange();assert.equal(allow.disabled,false);
  const duration=all(dialog).find(e=>e.name==="screen-duration");duration.value="1";all(dialog).find(e=>e.tagName==="form").onsubmit({preventDefault(){}});await new Promise(r=>setImmediate(r));
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls.at(-1))),["/api/screen/grant",{bot_id:"grokbot",conversation_id:"chat",minutes:1}]);
  assert.match(f.body.textContent,/Ezerbot contrôle l’écran — Échap pour arrêter/);assert.equal(f.registration.wantsPolling(),true);
  const preview=all(f.panels).find(e=>e.className.includes("screen-preview"));assert.match(preview.className,/screen-preview-active/);
  // Pointer/keyboard events in Ezerbot (approval card, composer, Stop) must not
  // produce a fake external-input request or interrupt an active session.
  for (const name of ["pointerdown","pointermove","keydown","wheel"]) {
    for (const fn of f.listeners[name] || []) await fn({isTrusted:true,key:"Enter",target:allow});
  }
  assert.equal(f.state.screen.session.paused,false);
  assert.equal(f.calls.filter(([path])=>path==="/api/screen/fake-input").length,0);
  await byText(dialog,"Simuler une entrée hors d’Ezerbot").onclick();assert.match(dialog.textContent,/En pause — Vous avez la main/);assert.match(f.body.textContent,/Vous avez repris la main/);
  await byText(dialog,"Reprendre le bot").onclick();assert.equal(f.calls.at(-1)[0],"/api/screen/resume");
  for(const fn of f.listeners.keydown)fn({key:"Escape",preventDefault(){}});await new Promise(r=>setImmediate(r));assert.equal(f.calls.at(-1)[0],"/api/screen/revoke");assert.equal(f.registration.wantsPolling(),false);
  assert.doesNotMatch(preview.className,/screen-preview-active/);
});
test("le Mode YOLO ouvre l’écran au bot sans réglage, la session et l’entrée restent à accorder", async () => {
  const f = fixture();
  const entry = all(f.top).find(e => e.className.includes("screen-button"));
  f.state.yolo = { mode: "yolo", bots: [{ bot_id: "grokbot", mode: "inherit", effective: "yolo" }] };
  await f.host.refresh();
  const input = all(f.settings).find(e => e.tagName === "input");
  assert.equal(input.checked, true);
  assert.equal(input.disabled, true);
  assert.equal(input.title, "Ouvert par le Mode YOLO");
  assert.match(f.settings.textContent, /Le Mode YOLO ouvre l’écran/);
  assert.equal(f.state.screen.bots.length, 0, "aucun réglage n’a été écrit");
  // Retour à « Demander » : le réglage d’avant (éteint) reprend effet.
  f.state.yolo = { mode: "ask", bots: [{ bot_id: "grokbot", mode: "inherit", effective: "ask" }] };
  await f.host.refresh();
  const asked = all(f.settings).find(e => e.tagName === "input");
  assert.equal(asked.checked, false);
  assert.equal(asked.disabled, false);
  assert.match(f.settings.textContent, /Désactivé par défaut/);
});
