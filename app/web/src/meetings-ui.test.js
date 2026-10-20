import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.className = ""; this.dataset = {}; this.attributes = {}; this.checked = false; this.listeners = {}; }
  append(...children) { for (const child of children) { this.children.push(child); child.parent = this; } }
  after(child) { this.parent.append(child); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  setAttribute(key, value) { this.attributes[key] = value; }
  addEventListener(name, fn) { this.listeners[name] = fn; }
  querySelector(selector) { return descendants(this).find(el => selector.startsWith(".") ? el.className.split(" ").includes(selector.slice(1)) : el.tagName === selector); }
  showModal() { this.open = true; }
  close() { this.open = false; this.listeners.close?.(); }
}
const descendants = el => [el, ...el.children.flatMap(descendants)];
function fixture() {
  const root = new Element("div"), top = new Element("header"), panels = new Element("div"), settings = new Element("div"); root.append(top, panels, settings);
  const context = { conversation: { id: "chat" }, bot: { id: "grokbot" }, state: { meetings: { debug_fake: true } } };
  let registration, poll, token, status = "recording", delayed;
  const calls = [], requests = [], notices = [], events = [], storage = new Map();
  const bridge = async body => {
    calls.push(body); token = body.token;
    if (body.command === "status" && delayed) return new Promise(resolve => { delayed.resolve = resolve; });
    return { token, status: ({start:status,stop:status === "failed" ? "failed" : "ready",cancel:"idle",pause:"paused",resume:"recording",retry:"ready"})[body.command] || status,
      transcript: "Décision : jeudi. Alice prépare la démo.", data: status === "no_audio" ? "" : "UklGRg==", elapsed_ms: 4200, too_long: status === "too_long" };
  };
  const host = { register(panel) { registration = panel; }, composerChanged() {}, notify(message) { notices.push(message); }, draft: () => "", draftDocument: () => ({}),
    async refresh() {}, openConversation(id) { context.conversation = {id}; registration.onState(context.state, context); return true; },
    async api(path, body) { requests.push({path,body}); if (path === "/api/meetings/start") return { id: "meeting_one", conversation_id: body.conversation_id || "created", title: body.title }; return {}; },
  };
  const document = { createElement: tag => new Element(tag), createTextNode: text => Object.assign(new Element("text"),{textContent:text}), getElementById: () => panels, querySelector: () => top };
  runInNewContext(readFileSync(new URL("../../ui/panels/meetings.js",import.meta.url),"utf8"),{
    document, window: {GrokbotPanels:host,webkit:{messageHandlers:{ezerbotVoice:{postMessage:bridge}}},addEventListener(){},dispatchEvent(event){events.push(event.type);}},
    Event:class {constructor(type){this.type=type;}},localStorage:{getItem:k=>storage.get(k),setItem:(k,v)=>storage.set(k,v),removeItem:k=>storage.delete(k)},
    setTimeout(fn) {poll=fn;},clearTimeout(){poll=null;},confirm(){throw new Error("Native confirm is unavailable in WKWebView");},
  });
  registration.settingsSection(settings); registration.composer.mount(root,context); registration.onState(context.state,context);
  const id = name => descendants(root).find(el=>el.id === name);
  const text = name => descendants(root).find(el=>el.textContent === name);
  return { root,context,registration,calls,requests,notices,events,host,id,text,set status(value){status=value;},
    async decision(action, accepted) { const pending=action(); await Promise.resolve(); assert.equal(id("meeting-confirmation").open,true); id(accepted ? "meeting-confirmation-accept" : "meeting-confirmation-cancel").onclick(); await pending; },
    async start() { id("meeting-start").onclick(); id("meeting-consent").checked=true; id("meeting-consent").onchange(); await id("meeting-begin").onclick(); },
    hold() { delayed={}; }, release() {delayed.resolve({status:"recording",token});}, async poll() {await poll?.();},
  };
}
test("meeting consent, destination/title, dictation signal, pause/resume, default-delete save and blocked reason", async () => {
  const f=fixture(); f.id("meeting-start").onclick();
  assert.equal(f.id("meeting-begin").disabled,true); await f.id("meeting-begin").onclick(); assert.equal(f.calls.length,0);
  await f.start(); assert.equal(f.registration.composer.blocked(),true); assert.equal(f.id("meeting-recording").hidden,false);
  assert.equal(f.host.meetingActive(),true); assert.ok(f.events.includes("grokbot:meeting-change")); assert.match(f.registration.composer.blockedReason(),/Envoi suspendu/);
  assert.ok(descendants(f.root).some(el=>el.textContent?.includes("Réunion en cours · Réunion")));
  f.registration.onState(f.context.state,{...f.context,conversation:{id:"other"}}); assert.equal(f.calls.length,1,"navigation never cancels capture");
  assert.equal(f.id("meeting-open-conversation").hidden,false); f.id("meeting-open-conversation").onclick(); assert.equal(f.context.conversation.id,"chat");
  await f.text("Pause").onclick(); await f.text("Reprendre").onclick(); await f.text("Arrêter").onclick();
  assert.equal(f.host.meetingActive(),false); assert.equal(f.id("meeting-recording").dataset.phase,"ready"); assert.equal(f.id("meeting-keep-audio").checked,false);
  assert.equal(f.root.querySelector(".meeting-reminder").hidden,true);
  assert.equal(f.root.querySelector(".meeting-review").children[1].className,"meeting-review-actions","actions precede the expandable transcript");
  await f.id("meeting-save").onclick();
  const saved=f.requests.find(r=>r.path==="/api/meetings/finalize"); assert.equal(saved.body.keep_audio,false); assert.match(saved.body.transcript,/Alice/);
  assert.ok(f.requests.some(r=>r.path==="/api/meetings/summarize")); assert.ok(f.calls.some(r=>r.command==="cancel")); assert.equal(f.id("meeting-recording").hidden,true);
});
test("late poll cannot erase stop receipt; audio kept only with explicit checkbox", async () => {
  const f=fixture(); await f.start(); f.hold(); const waiting=f.poll(); await Promise.resolve();
  await f.text("Arrêter").onclick(); f.release(); await waiting; assert.equal(f.id("meeting-recording").dataset.phase,"ready");
  f.id("meeting-keep-audio").checked=true; await f.id("meeting-save").onclick();
  assert.equal(f.requests.find(r=>r.path==="/api/meetings/finalize").body.keep_audio,true);
});
test("denial, no audio, cap and retry have alert tones, correct exits and system settings",async()=>{
  const f=fixture(); f.status="mic_denied"; await f.start(); assert.equal(f.id("meeting-recording").dataset.tone,"danger");
  assert.equal(f.id("meeting-system-settings").hidden,false); await f.id("meeting-system-settings").onclick(); assert.ok(f.calls.some(c=>c.command==="settings"));
  assert.equal(f.id("meeting-discard").textContent,"Fermer"); await f.id("meeting-discard").onclick();
  f.status="recording"; await f.start(); f.status="no_audio"; await f.text("Arrêter").onclick(); assert.equal(f.id("meeting-recording").dataset.tone,"danger"); await f.id("meeting-discard").onclick();
  f.status="recording"; await f.start(); f.status="failed"; await f.text("Arrêter").onclick(); assert.equal(f.id("meeting-recording").dataset.tone,"danger");
  f.status="too_long"; await f.text("Réessayer la transcription").onclick(); assert.equal(f.id("meeting-recording").dataset.tone,"warning");
  assert.ok(descendants(f.root).some(el=>el.textContent==="Limite de 10 minutes atteinte · Capture arrêtée"));
});
test("deleting raw audio or a capture uses styled asynchronous decisions, never window.confirm",async()=>{
  const f=fixture(); await f.start();
  await f.decision(()=>f.id("meeting-discard").onclick(),false); assert.ok(!f.requests.some(r=>r.path==="/api/meetings/discard"));
  await f.decision(()=>f.registration.onCardAction({kind:"meeting",payload:{id:"meeting_one"}},"meeting-delete-audio"),false); assert.ok(!f.requests.some(r=>r.path==="/api/meetings/delete-audio"));
  await f.decision(()=>f.registration.onCardAction({kind:"meeting",payload:{id:"meeting_one"}},"meeting-delete-audio"),true); assert.ok(f.requests.some(r=>r.path==="/api/meetings/delete-audio"));
  await f.decision(()=>f.id("meeting-discard").onclick(),true); assert.ok(f.requests.some(r=>r.path==="/api/meetings/discard")); assert.equal(f.host.meetingActive(),false);
});
test("a late close event from a cancelled dialog cannot cancel a new decision",async()=>{
  const f=fixture(); const message={kind:"meeting",payload:{id:"meeting_one"}};
  await f.decision(()=>f.registration.onCardAction(message,"meeting-delete-audio"),false);
  const pending=f.registration.onCardAction(message,"meeting-delete-audio"); await Promise.resolve();
  f.id("meeting-confirmation").listeners.close(); assert.equal(f.id("meeting-confirmation").open,true);
  f.id("meeting-confirmation-accept").onclick(); await pending;
  assert.ok(f.requests.some(r=>r.path==="/api/meetings/delete-audio"));
});

test("welcome capture is created atomically with its title; notes prevent removing its conversation",async()=>{
  const f=fixture(); f.context.conversation=null; f.registration.onState(f.context.state,f.context); await f.start();
  const request=f.requests.find(r=>r.path==="/api/meetings/start"); assert.equal(request.body.conversation_id,null); assert.equal(request.body.title,"Réunion");
  assert.ok(!f.requests.some(r=>r.path==="/api/conversations")); f.host.draft=()=>"Mes notes";
  await f.decision(()=>f.id("meeting-discard").onclick(),true); assert.equal(f.requests.find(r=>r.path==="/api/meetings/discard").body.preserve_conversation,true);
});
test("meeting cannot interrupt an active dictation, and closing a busy response never cancels its microphone",async()=>{
  const f=fixture(); f.host.voiceActive=()=>true; f.registration.onState(f.context.state,f.context);
  assert.equal(f.id("meeting-start").disabled,true); await f.start(); assert.equal(f.requests.length,0);
  f.host.voiceActive=()=>false; f.status="busy"; await f.start(); await f.id("meeting-discard").onclick();
  assert.ok(!f.calls.some(c=>c.command==="cancel")); assert.ok(f.requests.some(r=>r.path==="/api/meetings/discard"));
});

test("dismissing a restarted capture preserves its conversation when notes are present",async()=>{
  const f=fixture(); f.host.draft=()=>"Notes à conserver";
  f.registration.onState({meetings:{interrupted:[{id:"meeting_old",title:"Réunion interrompue",conversation_id:"chat"}]}},f.context);
  await f.text("Fermer").onclick(); assert.equal(f.requests.find(r=>r.path==="/api/meetings/discard").body.preserve_conversation,true);
});

test("meeting surfaces never invoke native confirm or browser media; unavailable summary copy is conditional",()=>{
  const source=readFileSync(new URL("../../ui/panels/meetings.js",import.meta.url),"utf8"); assert.doesNotMatch(source,/\bconfirm\s*\(|navigator\.mediaDevices|new (?:webkit)?SpeechRecognition/);
  const card=readFileSync(new URL("./cards/meeting.jsx",import.meta.url),"utf8"); assert.match(card,/message\.body && <p[^>]+>Résumé généré/); assert.match(card,/month: "long"/);
  const app=readFileSync(new URL("../../ui/app.js",import.meta.url),"utf8"); assert.match(app,/composer\?\.blockedReason/); assert.match(app,/typeof blockedReason === "string" \? blockedReason/);
});
