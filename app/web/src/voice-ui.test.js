import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
class Element {
  constructor(tag) { this.tagName = tag; this.className = ""; this.children = []; this.dataset = {}; this.attributes = {}; this.style = { setProperty: (key, value) => { this.style[key] = value; } }; }
  append(...children) { for (const child of children) { child.parent = this; this.children.push(child); } }
  before(child) { this.parent.append(child); }
  remove() { this.parent.children = this.parent.children.filter(child => child !== this); }
  setAttribute(key, value) { this.attributes[key] = String(value); }
}
const descendants = root => [root, ...root.children.flatMap(descendants)];
const cls = (root, name) => descendants(root).find(el => el.className.split(" ").includes(name));
const flush = () => new Promise(resolve => setImmediate(resolve));
function fixture() {
  const root = new Element("div"), send = new Element("button"); root.append(send);
  const container = new Element("div"); root.append(container);
  const ctx = { state: { voice: { drafts: [] } }, conversation: { id: "chat" }, bot: { id: "grokbot" } };
  let registration, keydown, poll, token, draft = "Texte existant.", result = "recording";
  const calls = [], uploads = [], created = [];
  const storage = new Map();
  const richDocument = {type:"doc",content:[{type:"paragraph",content:[{type:"botMention",attrs:{id:"grokbot",label:"Ezerbot"}},{type:"text",text:draft}]}]};
  let holdStatus = false, delayedStatus;
  const bridge = async body => {
    calls.push(body.command); if (body.command === "start") token = body.token;
    if (body.command === "status" && holdStatus) return new Promise(resolve => { delayedStatus = resolve; });
    return {status: body.command === "cancel" ? "idle" : body.command === "stop" || body.command === "retry" ? result === "failed" ? "failed" : "ready" : result,
      token, transcript: "Bonjour le monde.", data: "UklGRg==", elapsed_ms: 4200, level: 0.5, language: "fr"};
  };
  const host = {register(panel) { registration = panel; panel.composer.mount(container,ctx); }, composerChanged() {}, appendDraft(text) { draft += `\n${text}`; },
    async api(path, body) { if (path === "/api/conversations") { created.push(body); return {conversation:{id:"created-chat"}}; } uploads.push({path,body}); return {id:"voice_one",duration_ms:1000}; },
    async refresh() {}, openConversation(id) {
      // selectConversation saves the departing welcome draft, including its rich document.
      storage.set("grokbot.draft.new", draft);
      storage.set("grokbot.draft.new.document", JSON.stringify(richDocument));
      assert.equal(storage.get(`grokbot.draft.${id}.document`), JSON.stringify(richDocument), "rich text must be handed off before opening");
      ctx.conversation={id}; registration.onState(ctx.state, ctx); return true;
    }, notify() {}, draft() {return draft;}, draftDocument() {return richDocument;} };
  const document = {documentElement:{lang:"fr"},createElement:tag=>new Element(tag), createTextNode:text=>Object.assign(new Element("text"),{textContent:text}), getElementById:()=>send, addEventListener(name, fn) { if (name === "keydown") keydown = fn; }};
  runInNewContext(readFileSync(new URL("../../ui/panels/voice.js", import.meta.url),"utf8"), {
    localStorage:{setItem(key,value){storage.set(key,String(value));},getItem(key){return storage.get(key) ?? null;},removeItem(key){storage.delete(key);}}, window:{GrokbotPanels:host,webkit:{messageHandlers:{ezerbotVoice:{postMessage:bridge}}},addEventListener() {}}, document, crypto:{randomUUID:()=>"voice-token"},setTimeout(fn){poll=fn;},clearTimeout(){poll=null;},
  });
  return { root, ctx, registration, calls, uploads, created, storage, get draft(){return draft;}, set result(value){result=value;}, holdPoll() { holdStatus = true; }, releasePoll() { delayedStatus({status:"recording",token}); }, keydown: event=>keydown(event), poll:async()=>{await poll?.();await flush();} };
}
test("dictation level, Escape stop and review never send the message", async () => {
  const f = fixture();
  await cls(f.root,"voice-mic").onclick();
  assert.equal(cls(f.root,"voice-level").style["--level"],"0.5");
  assert.equal(cls(f.root,"voice-level").attributes["aria-valuenow"],"0.50");
  assert.equal(cls(f.root,"voice-status").textContent,"Écoute en cours · Échap pour arrêter");
  assert.equal(cls(f.root,"voice-time").textContent,"0:04 / 1:00");
  assert.equal(f.registration.composer.blocked(),true);
  f.keydown({key:"Escape",preventDefault(){},stopImmediatePropagation(){}}); await flush();
  assert.equal(f.draft,"Texte existant.\nBonjour le monde.");
  assert.equal(f.registration.composer.blocked(),false);
  assert.equal(f.uploads.length,0); assert.ok(f.calls.includes("cancel"));
});
test("cancel and switching conversations discard the recording, not another draft", async () => {
  const f = fixture(); await cls(f.root,"voice-mic").onclick();
  const cancel = descendants(f.root).find(el=>el.textContent==="Annuler"); cancel.onclick(); await flush();
  assert.equal(f.draft,"Texte existant.");
  await cls(f.root,"voice-mic").onclick();
  f.registration.onState({}, {...f.ctx,conversation:{id:"other"}});
  await f.poll(); assert.equal(f.draft,"Texte existant.");
  assert.ok(f.calls.filter(c=>c==="cancel").length>=2);
});
test("memo attaches WAV plus transcript and error retry does not rerecord", async () => {
  const f = fixture(); cls(f.root,"voice-mode").children[0].checked=true;
  f.result="failed"; await cls(f.root,"voice-mic").onclick();
  // Restart fixture with a recording response, then fail only the transcription.
  f.result="recording"; await cls(f.root,"voice-mic").onclick();
  f.result="failed"; await cls(f.root,"voice-mic").onclick();
  assert.match(cls(f.root,"voice-status").textContent,/transcription a échoué/);
  assert.equal(cls(f.root,"voice-region").dataset.tone,"danger");
  assert.ok(descendants(f.root).some(el=>el.textContent==="Fermer" && !el.hidden), "nothing is running to cancel");
  assert.equal(f.registration.composer.blocked(),false, "the existing draft stays sendable");
  f.result="ready"; await descendants(f.root).find(el=>el.textContent==="Réessayer").onclick();
  assert.equal(f.uploads[0].path,"/api/voice/memos");
  assert.equal(f.uploads[0].body.transcript,"Bonjour le monde.");
  assert.equal(f.uploads[0].body.upload_id,"voice-token");
  assert.deepEqual(Array.from(f.registration.sendExtras().attachments),["voice_one"]);
  f.registration.onState({voice:{drafts:[]}}, f.ctx);
  assert.deepEqual(Array.from(f.registration.sendExtras().attachments),["voice_one"], "stale state cannot drop a completed upload");
  assert.equal(f.calls.filter(c=>c==="start").length,2);
  assert.equal(cls(f.root,"voice-status-row").hidden,true, "the kept memo chip replaces the review note");
});
test("a memo after dictation on the welcome screen survives creation of its conversation", async () => {
  const f = fixture(); f.ctx.conversation=null; f.registration.onState(f.ctx.state,f.ctx);
  await cls(f.root,"voice-mic").onclick(); await cls(f.root,"voice-mic").onclick();
  assert.match(cls(f.root,"voice-status").textContent,/Relisez/);
  cls(f.root,"voice-mode").children[0].checked=true;
  await cls(f.root,"voice-mic").onclick();
  assert.equal(f.ctx.conversation, null, "recording alone creates no thread");
  assert.equal(cls(f.root,"voice-mic").attributes["aria-pressed"],"true");
  await cls(f.root,"voice-mic").onclick();
  assert.equal(f.ctx.conversation.id,"created-chat");
  assert.equal(f.created.length,1);
  assert.equal(f.uploads[0].body.conversation_id,"created-chat");
  assert.deepEqual(Array.from(f.registration.sendExtras().attachments),["voice_one"]);
  assert.equal([...f.storage.keys()].some(key=>key.startsWith("grokbot.draft.new")),false,"returning home cannot restore a ghost document");
  assert.ok(f.storage.has("grokbot.draft.created-chat.document"));
});
test("a memo refused, cancelled or failed on the welcome screen creates no thread", async () => {
  const f = fixture(); f.ctx.conversation=null; f.registration.onState(f.ctx.state,f.ctx);
  cls(f.root,"voice-mode").children[0].checked=true;
  f.result="mic_denied"; await cls(f.root,"voice-mic").onclick();
  assert.equal(cls(f.root,"voice-region").dataset.tone,"danger");
  descendants(f.root).find(el=>el.textContent==="Fermer").onclick();
  f.result="offline_unavailable"; await cls(f.root,"voice-mic").onclick();
  assert.equal(cls(f.root,"voice-region").dataset.tone,"warning");
  descendants(f.root).find(el=>el.textContent==="Fermer").onclick();
  f.result="recording"; await cls(f.root,"voice-mic").onclick();
  descendants(f.root).find(el=>el.textContent==="Annuler").onclick();
  await cls(f.root,"voice-mic").onclick();
  f.result="failed"; await cls(f.root,"voice-mic").onclick();
  assert.match(cls(f.root,"voice-status").textContent,/transcription a échoué/);
  assert.equal(f.created.length,0);
  assert.equal(f.ctx.conversation,null);
});
test("late recording poll cannot erase a stopped transcript receipt", async () => {
  const f = fixture(); await cls(f.root,"voice-mic").onclick();
  f.holdPoll(); const pending = f.poll(); await flush();
  await cls(f.root,"voice-mic").onclick();
  assert.match(cls(f.root,"voice-status").textContent,/Relisez/);
  f.releasePoll(); await pending;
  assert.match(cls(f.root,"voice-status").textContent,/Relisez/);
  assert.equal(f.registration.composer.blocked(),false);
  assert.equal(f.draft,"Texte existant.\nBonjour le monde.");
});
test("a staged memo on an empty thread prevents bot switching just like other attachments", () => {
  const source = readFileSync(new URL("../../ui/app.js",import.meta.url),"utf8");
  const fn = source.match(/function stagedOnEmptyThread\(\) \{[\s\S]*?\n  \}/)[0];
  for (const kind of ["attachment-chip", "voice-memo-chip"]) {
    let messages = [];
    const staged = () => runInNewContext(`(${fn})()`, {currentConversation:()=>({messages}),$:()=>({querySelector:selector=>selector.includes(`.${kind}`) ? {} : null})});
    assert.equal(staged(),true,kind);
    messages = [{}]; assert.equal(staged(),false,"nonempty thread is not reassigned by bot selection");
  }
  assert.match(source,/Retirez les pièces jointes pour changer de bot/);
});

test("denial explains System Settings and never accesses browser media", async () => {
  const f = fixture(); f.result="mic_denied"; await cls(f.root,"voice-mic").onclick();
  assert.match(cls(f.root,"voice-status").textContent,/Accès au microphone refusé/);
  const settings=descendants(f.root).find(el=>el.textContent==="Ouvrir Réglages Système");
  assert.equal(settings.hidden,false); await settings.onclick(); assert.ok(f.calls.includes("settings"));
  const source=readFileSync(new URL("../../ui/panels/voice.js",import.meta.url),"utf8");
  assert.doesNotMatch(source,/navigator\.mediaDevices|new (?:webkit)?SpeechRecognition/);
});
