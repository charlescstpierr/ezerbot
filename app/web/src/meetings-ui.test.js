import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.className = ""; this.dataset = {}; this.attributes = {}; this.checked = false; }
  append(...children) { for (const child of children) { this.children.push(child); child.parent = this; } }
  after(child) { this.parent.append(child); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  setAttribute(key, value) { this.attributes[key] = value; }
  querySelector(selector) { return descendants(this).find(el => selector.startsWith(".") ? el.className.split(" ").includes(selector.slice(1)) : el.tagName === selector); }
  showModal() { this.open = true; }
  close() { this.open = false; }
}
const descendants = el => [el, ...el.children.flatMap(descendants)];
function fixture() {
  const root = new Element("div"), top = new Element("header"), tools = new Element("div"), panels = new Element("div"), settings = new Element("div"); root.append(top, tools, panels, settings);
  const context = { conversation: { id: "chat" }, bot: { id: "grokbot" }, state: { meetings: { debug_fake: true } } };
  let registration, poll, token, status = "recording", approved = true, delayed;
  const calls = [], requests = [], notices = [];
  const bridge = async body => {
    calls.push(body); token = body.token;
    if (body.command === "status" && delayed) return new Promise(resolve => { delayed.resolve = resolve; });
    return { token, status: ({start:status,stop:status === "failed" ? "failed" : "ready",cancel:"idle",pause:"paused",resume:"recording",retry:"ready"})[body.command] || status,
      transcript: "Décision : jeudi. Alice prépare la démo.", data: status === "no_audio" ? "" : "UklGRg==", elapsed_ms: 4200, too_long: status === "too_long" };
  };
  const host = { register(panel) { registration = panel; }, composerChanged() {}, notify(message) { notices.push(message); }, draft: () => "", draftDocument: () => ({}),
    async refresh() {}, openConversation() { return true; },
    async api(path, body) { requests.push({path,body}); if (path === "/api/meetings/start") return { id: "meeting_one", conversation_id: "chat" }; return {}; },
  };
  const document = { createElement: tag => new Element(tag), createTextNode: text => Object.assign(new Element("text"),{textContent:text}), getElementById: id => id === "composer-tools" ? tools : panels, querySelector: () => top };
  runInNewContext(readFileSync(new URL("../../ui/panels/meetings.js",import.meta.url),"utf8"),{
    document, window: {GrokbotPanels:host,webkit:{messageHandlers:{ezerbotVoice:{postMessage:bridge}}},addEventListener(){}},
    setTimeout(fn) {poll=fn;},clearTimeout(){poll=null;},confirm:()=>approved,
  });
  registration.settingsSection(settings); registration.composer.mount(root,context); registration.onState(context.state,context);
  const id = name => descendants(root).find(el=>el.id === name);
  const text = name => descendants(root).find(el=>el.textContent === name);
  return { root,context,registration,calls,requests,notices,id,text,set status(value){status=value;},set approved(value){approved=value;},
    async start() { id("meeting-start").onclick(); id("meeting-consent").checked=true; id("meeting-consent").onchange(); await id("meeting-begin").onclick(); },
    hold() { delayed={}; }, release() {delayed.resolve({status:"recording",token});}, async poll() {await poll?.();},
  };
}
test("meeting consent gate, persistent indicator across navigation, pause/resume and explicit default-delete save", async () => {
  const f=fixture(); f.id("meeting-start").onclick();
  assert.equal(f.id("meeting-begin").disabled,true); await f.id("meeting-begin").onclick(); assert.equal(f.calls.length,0);
  await f.start(); assert.equal(f.registration.composer.blocked(),true); assert.equal(f.id("meeting-recording").hidden,false);
  assert.equal(f.calls[0].consent,true); assert.equal(f.calls[0].purpose,"meeting");
  f.registration.onState(f.context.state,{...f.context,conversation:{id:"other"}}); assert.equal(f.calls.length,1,"navigation never cancels capture");
  await f.text("Pause").onclick(); assert.equal(f.id("meeting-recording").dataset.phase,"paused");
  await f.text("Reprendre").onclick(); await f.text("Arrêter").onclick();
  assert.equal(f.id("meeting-recording").dataset.phase,"ready"); assert.equal(f.id("meeting-keep-audio").checked,false);
  await f.text("Conserver et produire le résumé").onclick();
  const saved=f.requests.find(r=>r.path==="/api/meetings/finalize"); assert.equal(saved.body.keep_audio,false); assert.match(saved.body.transcript,/Alice/);
  assert.ok(f.requests.some(r=>r.path==="/api/meetings/summarize")); assert.ok(f.calls.some(r=>r.command==="cancel")); assert.equal(f.id("meeting-recording").hidden,true);
});
test("a late poll cannot erase stop receipt; raw audio is kept only with the user's checkbox", async () => {
  const f=fixture(); await f.start(); f.hold(); const waiting=f.poll(); await Promise.resolve();
  await f.text("Arrêter").onclick(); f.release(); await waiting; assert.equal(f.id("meeting-recording").dataset.phase,"ready");
  f.id("meeting-keep-audio").checked=true; await f.text("Conserver et produire le résumé").onclick();
  assert.equal(f.requests.find(r=>r.path==="/api/meetings/finalize").body.keep_audio,true);
});
test("denial, no audio, cap and transcription retry have French feedback and never capture through browser APIs",async()=>{
  const f=fixture(); f.status="mic_denied"; await f.start(); assert.ok(descendants(f.root).some(el=>el.textContent?.includes("Accès au microphone refusé")));
  assert.ok(!f.requests.some(r=>r.path==="/api/meetings/finalize")); await f.text("Supprimer la capture").onclick();
  f.status="recording"; await f.start(); f.status="no_audio"; await f.text("Arrêter").onclick(); assert.ok(descendants(f.root).some(el=>el.textContent?.includes("Aucun audio enregistré"))); await f.text("Supprimer la capture").onclick();
  f.status="recording"; await f.start(); f.status="failed"; await f.text("Arrêter").onclick(); assert.equal(f.id("meeting-recording").dataset.phase,"failed");
  f.status="too_long"; await f.text("Réessayer la transcription").onclick(); assert.ok(descendants(f.root).some(el=>el.textContent?.includes("Limite de 10 minutes atteinte")));
  const source=readFileSync(new URL("../../ui/panels/meetings.js",import.meta.url),"utf8"); assert.doesNotMatch(source,/navigator\.mediaDevices|new (?:webkit)?SpeechRecognition/);
});
test("deleting kept audio or an unfinished capture requires the user's decision",async()=>{
  const f=fixture(); await f.start(); f.approved=false; await f.text("Supprimer la capture").onclick(); assert.ok(!f.requests.some(r=>r.path==="/api/meetings/discard"));
  await f.registration.onCardAction({kind:"meeting",payload:{id:"meeting_one"}},"meeting-delete-audio"); assert.ok(!f.requests.some(r=>r.path==="/api/meetings/delete-audio"));
  f.approved=true; await f.registration.onCardAction({kind:"meeting",payload:{id:"meeting_one"}},"meeting-delete-audio"); assert.ok(f.requests.some(r=>r.path==="/api/meetings/delete-audio"));
});
