import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
class Element {
  constructor(tag) { this.tagName = tag; this.className = ""; this.children = []; this.dataset = {}; this.attributes = {}; this.style = { setProperty: (key, value) => { this.style[key] = value; } }; }
  append(...children) { for (const child of children) { child.parent = this; this.children.push(child); } }
  before(child) { this.parent.append(child); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
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
  const calls = [], bodies = [], uploads = [], created = [], notices = [];
  let purpose = "dictation", startExtras = {}, tooLong = false, microphones = [{id:"built-in",name:"Microphone intégré"},{id:"usb",name:"Microphone USB"}];
  const storage = new Map();
  const richDocument = {type:"doc",content:[{type:"paragraph",content:[{type:"botMention",attrs:{id:"grokbot",label:"Ezerbot"}},{type:"text",text:draft}]}]};
  let holdStatus = false, delayedStatus, conversationVoice = false;
  const bridge = async body => {
    calls.push(body.command); bodies.push(body); if (body.command === "start") { token = body.token; purpose = body.purpose; }
    if (body.command === "microphones") return {microphones};
    if (body.command === "status" && holdStatus) return new Promise(resolve => { delayedStatus = resolve; });
    return {status: body.command === "cancel" ? "idle" : body.command === "stop" || body.command === "retry" ? result === "failed" ? "failed" : "ready" : result,
      token, transcript: "Bonjour le monde.", data: "UklGRg==", elapsed_ms: 4200, level: 0.5, language: "fr", too_long: tooLong, max_seconds: purpose === "memo" ? 60 : 300, ...startExtras};
  };
  const host = {register(panel) { registration = panel; panel.composer.mount(container,ctx); }, composerChanged() {}, appendDraft(text) { draft += `\n${text}`; },
    async api(path, body) { if (path === "/api/conversations") { created.push(body); return {conversation:{id:"created-chat"}}; } uploads.push({path,body}); if (path === "/api/voice/microphone") return {microphone:body.id}; return {id:"voice_one",duration_ms:1000}; },
    async refresh() {}, openConversation(id) {
      // selectConversation saves the departing welcome draft, including its rich document.
      storage.set("grokbot.draft.new", draft);
      storage.set("grokbot.draft.new.document", JSON.stringify(richDocument));
      assert.equal(storage.get(`grokbot.draft.${id}.document`), JSON.stringify(richDocument), "rich text must be handed off before opening");
      ctx.conversation={id}; registration.onState(ctx.state, ctx); return true;
    },
    async ensureDraftConversation() {
      const { conversation } = await this.api("/api/conversations", { bot_id: ctx.bot.id });
      storage.set(`grokbot.draft.${conversation.id}`, draft);
      storage.set(`grokbot.draft.${conversation.id}.document`, JSON.stringify(richDocument));
      registration.composer.transfer({ from: `new:${ctx.bot.id}`, to: conversation.id }, { ...ctx, conversation });
      this.openConversation(conversation.id);
      storage.delete("grokbot.draft.new"); storage.delete("grokbot.draft.new.document");
      return conversation;
    }, notify(text) { notices.push(text); }, notifyDraft(text) { notices.push(text); }, draft() {return draft;}, draftDocument() {return richDocument;} };
  const document = {documentElement:{lang:"fr"},createElement:tag=>new Element(tag), createTextNode:text=>Object.assign(new Element("text"),{textContent:text}), getElementById:()=>send, addEventListener(name, fn) { if (name === "keydown") keydown = fn; }};
  const window = {GrokbotPanels:host,EzerbotVoiceMode:{active:()=>conversationVoice},webkit:{messageHandlers:{ezerbotVoice:{postMessage:bridge}}},addEventListener() {}};
  runInNewContext(readFileSync(new URL("../../ui/panels/voice.js", import.meta.url),"utf8"), {
    localStorage:{setItem(key,value){storage.set(key,String(value));},getItem(key){return storage.get(key) ?? null;},removeItem(key){storage.delete(key);}}, window, document, crypto:{randomUUID:()=>"voice-token"},setTimeout(fn){poll=fn;},clearTimeout(){poll=null;},
  });
  return { root, ctx, registration, calls, bodies, notices, uploads, set startExtras(value){startExtras=value;}, set tooLong(value){tooLong=value;}, set microphones(value){microphones=value;}, settings(){const container=new Element("div");registration.settingsSection(container,ctx);return container;}, created, storage, setVoiceActive(value){conversationVoice=value;window.EzerbotDictation.refresh();}, get draft(){return draft;}, set meeting(value){host.meetingActive=()=>value; registration.onState(ctx.state,ctx);}, set result(value){result=value;}, holdPoll() { holdStatus = true; }, releasePoll() { delayedStatus({status:"recording",token}); }, keydown: event=>keydown(event), poll:async()=>{await poll?.();await flush();} };
}
test("conversation voice immediately disables dictation and memo, and restores them on exit", async () => {
  const f=fixture();const mic=cls(f.root,"voice-mic"),memo=cls(f.root,"voice-mode").children[0];
  f.setVoiceActive(true);
  assert.equal(mic.disabled,true);assert.equal(memo.disabled,true);
  assert.equal(mic.title,"Indisponible pendant le mode voix");
  assert.match(mic.attributes["aria-label"],/indisponible/);
  await mic.onclick();assert.equal(f.calls.length,0,"there is no second capture session");
  f.setVoiceActive(false);
  assert.equal(mic.disabled,false);assert.equal(memo.disabled,false);
  assert.equal(mic.title,"Dicter un message");
});

test("dictation level, Escape stop and review never send the message", async () => {
  const f = fixture();
  await cls(f.root,"voice-mic").onclick();
  assert.equal(cls(f.root,"voice-level").style["--level"],"0.5");
  assert.equal(cls(f.root,"voice-level").attributes["aria-valuenow"],"0.50");
  assert.equal(cls(f.root,"voice-status").textContent,"Écoute en cours · Échap pour arrêter");
  assert.equal(cls(f.root,"voice-time").textContent,"0:04 / 5:00");
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
test("a memo hands the word starts of its transcript to the Mac with the audio", async () => {
  const words = [{text:"Bonjour",start_ms:0},{text:"le",start_ms:500},{text:"monde.",start_ms:null}];
  const f = fixture(); cls(f.root,"voice-mode").children[0].checked = true; f.startExtras = {words};
  await cls(f.root,"voice-mic").onclick(); await cls(f.root,"voice-mic").onclick();
  assert.equal(f.uploads[0].path,"/api/voice/memos");
  assert.equal(JSON.stringify(f.uploads[0].body.words),JSON.stringify(words));
});
test("a memo recorded without word starts uploads an empty list", async () => {
  const f = fixture(); cls(f.root,"voice-mode").children[0].checked = true;
  await cls(f.root,"voice-mic").onclick(); await cls(f.root,"voice-mic").onclick();
  assert.equal(JSON.stringify(f.uploads[0].body.words),"[]");
});
test("a dictation never uploads words", async () => {
  const f = fixture(); f.startExtras = {words:[{text:"Bonjour",start_ms:0}]};
  await cls(f.root,"voice-mic").onclick(); await cls(f.root,"voice-mic").onclick();
  assert.equal(f.uploads.length,0);
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

test("meeting capture disables dictation and memo with an explicit French explanation", async () => {
  const f=fixture(); f.meeting=true;
  assert.equal(cls(f.root,"voice-mic").disabled,true);
  assert.equal(cls(f.root,"voice-mode").children[0].disabled,true);
  assert.match(cls(f.root,"voice-mic").title,/Indisponible pendant/);
  await cls(f.root,"voice-mic").onclick(); assert.equal(f.calls.length,0);
  f.meeting=false; assert.equal(cls(f.root,"voice-mic").disabled,false);
});

test("denial explains System Settings and never accesses browser media", async () => {
  const f = fixture(); f.result="mic_denied"; await cls(f.root,"voice-mic").onclick();
  assert.match(cls(f.root,"voice-status").textContent,/Accès au microphone refusé/);
  const settings=descendants(f.root).find(el=>el.textContent==="Ouvrir Réglages Système");
  assert.equal(settings.hidden,false); await settings.onclick(); assert.ok(f.calls.includes("settings"));
  const source=readFileSync(new URL("../../ui/panels/voice.js",import.meta.url),"utf8");
  assert.doesNotMatch(source,/navigator\.mediaDevices|new (?:webkit)?SpeechRecognition/);
});

const press = (f, timeStamp) => cls(f.root,"voice-mic").onpointerdown({button:0,timeStamp});
const release = (f, timeStamp) => cls(f.root,"voice-mic").onpointerup({button:0,timeStamp});
const click = f => cls(f.root,"voice-mic").onclick({detail:1});
const saved = f => f.uploads.filter(upload => upload.path === "/api/voice/microphone").map(upload => ({...upload.body}));
const status = f => cls(f.root,"voice-status").textContent;
const cmd = (key, extra = {}) => ({key, metaKey:true, preventDefault(){}, stopImmediatePropagation(){}, ...extra});

test("dictation records with the chosen microphone, announces it and sends nothing", async () => {
  const f = fixture(); f.ctx.state.voice.microphone = "usb"; f.startExtras = {microphone_id:"usb",microphone_name:"Microphone USB",microphone_fallback:false};
  await cls(f.root,"voice-mic").onclick();
  const start = f.bodies.find(body => body.command === "start");
  assert.equal(start.microphone,"usb"); assert.equal(start.purpose,"dictation");
  assert.equal(status(f),"Écoute en cours · Microphone USB · Échap pour arrêter");
  await cls(f.root,"voice-mic").onclick();
  assert.equal(f.draft,"Texte existant.\nBonjour le monde.");
  assert.equal(f.uploads.length,0,"nothing is sent and no memo is kept");
  assert.equal(f.notices.length,0);
});
test("a memo starts as a memo and keeps the one minute scale", async () => {
  const f = fixture(); cls(f.root,"voice-mode").children[0].checked = true;
  await cls(f.root,"voice-mic").onclick();
  assert.equal(f.bodies.find(body => body.command === "start").purpose,"memo");
  assert.equal(cls(f.root,"voice-time").textContent,"0:04 / 1:00");
});
test("a removed microphone is announced, forgotten and the default one is used", async () => {
  const f = fixture(); f.ctx.state.voice.microphone = "usb";
  f.startExtras = {microphone_id:"built-in",microphone_name:"Microphone intégré",microphone_fallback:true};
  await cls(f.root,"voice-mic").onclick();
  const phrase = "Le microphone choisi n’est plus disponible. Ezerbot utilise le microphone par défaut.";
  assert.deepEqual(f.notices,[phrase]);
  assert.deepEqual(saved(f),[{id:""}]);
  assert.equal(cls(f.root,"voice-mic").attributes["aria-pressed"],"true","dictation still runs");
  assert.match(status(f),/Microphone intégré/);
});
test("a microphone picked while a dictation runs after a fallback is not erased by the next poll", async () => {
  const f = fixture(); f.ctx.state.voice.microphone = "usb";
  f.startExtras = {microphone_id:"built-in",microphone_name:"Microphone intégré",microphone_fallback:true};
  await cls(f.root,"voice-mic").onclick();
  const section = f.settings(); await flush();
  const radio = descendants(descendants(section).find(el => el.dataset.id === "usb")).find(el => el.tagName === "input");
  radio.checked = true; await radio.onchange(); await flush();
  await f.poll(); await f.poll();
  assert.deepEqual(saved(f),[{id:""},{id:"usb"}], "the status polls do not forget the new choice");
  assert.equal(f.notices.length,1);
});
test("a short click starts and a second click stops, as before", async () => {
  const f = fixture();
  press(f,100); release(f,180); click(f); await flush();
  assert.equal(cls(f.root,"voice-mic").attributes["aria-pressed"],"true","the short press leaves the dictation running");
  assert.equal(f.draft,"Texte existant.");
  press(f,5000); release(f,5090); await click(f); await flush();
  assert.equal(f.draft,"Texte existant.\nBonjour le monde.");
});
test("holding for half a second talks until release, which transcribes", async () => {
  const f = fixture();
  press(f,1000); await flush();
  assert.equal(cls(f.root,"voice-mic").attributes["aria-pressed"],"true");
  release(f,1500); await click(f); await flush(); await flush();
  assert.equal(f.draft,"Texte existant.\nBonjour le monde.","release transcribes into the draft");
  assert.equal(f.calls.filter(call => call === "start").length,1,"the click after release does not restart");
  assert.ok(f.calls.includes("stop"));
  assert.equal(f.uploads.length,0);
});
test("a press released before the microphone is ready still stops once it is", async () => {
  const f = fixture(); f.result = "authorizing";
  press(f,0); await flush(); release(f,900); f.result = "recording";
  await f.poll(); await flush();
  assert.ok(f.calls.includes("stop"));
});
test("⌘D starts and stops the dictation, a bare D does not", async () => {
  const f = fixture();
  f.keydown({key:"d",preventDefault(){},stopImmediatePropagation(){}}); await flush();
  assert.equal(f.calls.length,0);
  f.keydown(cmd("d")); await flush();
  assert.equal(cls(f.root,"voice-mic").attributes["aria-pressed"],"true");
  f.keydown(cmd("D")); await flush(); await flush();
  assert.equal(f.draft,"Texte existant.\nBonjour le monde.");
  f.setVoiceActive(true); f.keydown(cmd("d")); await flush();
  assert.equal(f.calls.filter(call => call === "start").length,1,"voice mode keeps the microphone");
});
test("dictation shows a five minute scale", async () => {
  const f = fixture();
  await cls(f.root,"voice-mic").onclick();
  assert.equal(cls(f.root,"voice-time").textContent,"0:04 / 5:00");
});
test("the five minute limit stops the dictation, hands the text to the draft to review and sends nothing", async () => {
  const f = fixture(); f.tooLong = true;
  await cls(f.root,"voice-mic").onclick(); f.result = "ready"; await f.poll();
  assert.equal(f.draft,"Texte existant.\nBonjour le monde.");
  assert.equal(status(f),"Limite de 5 minutes atteinte. L’enregistrement est arrêté ; relisez la transcription avant d’envoyer.");
  assert.equal(f.registration.composer.blocked(),false);
  assert.equal(f.uploads.length,0);
});
test("a long dictation with a passage that could not be transcribed says so and still reaches the draft", async () => {
  const f = fixture(); f.startExtras = {partial:true};
  await cls(f.root,"voice-mic").onclick(); f.result = "ready"; await f.poll();
  assert.equal(f.draft,"Texte existant.\nBonjour le monde.");
  assert.equal(status(f),"Transcription ajoutée au brouillon, avec un passage non transcrit. Relisez-la avant d’envoyer.");
});
test("the memo limit stays at one minute and keeps the memo", async () => {
  const f = fixture(); f.tooLong = true; cls(f.root,"voice-mode").children[0].checked = true;
  await cls(f.root,"voice-mic").onclick(); f.result = "ready"; await f.poll();
  assert.equal(status(f),"Limite de 60 secondes atteinte. L’enregistrement est arrêté ; relisez la transcription avant d’envoyer.");
  assert.equal(f.uploads[0].path,"/api/voice/memos");
});
test("the microphone setting lists the choices, marks the chosen one and saves a change", async () => {
  const f = fixture(); f.ctx.state.voice.microphone = "usb";
  const section = f.settings(); await flush();
  assert.match(descendants(section).map(el => el.textContent).join(" "),/Choisissez le microphone qu’Ezerbot écoute quand vous lui parlez\./);
  const options = descendants(section).filter(el => el.className.split(" ").includes("voice-microphone-option"));
  assert.deepEqual(options.map(el => el.dataset.id),["","built-in","usb"]);
  assert.deepEqual(options.map(el => el.dataset.selected),["false","false","true"]);
  assert.deepEqual(options.map(el => descendants(el).some(child => child.textContent === "Choisi")),[false,false,true]);
  const radio = descendants(options[1]).find(el => el.tagName === "input");
  radio.checked = true; await radio.onchange(); await flush();
  assert.deepEqual(saved(f),[{id:"built-in"}]);
});
test("opening the setting with a microphone that is gone announces the fallback once", async () => {
  const f = fixture(); f.ctx.state.voice.microphone = "usb"; f.microphones = [{id:"built-in",name:"Microphone intégré"}];
  const section = f.settings(); await flush(); await flush();
  const phrase = "Le microphone choisi n’est plus disponible. Ezerbot utilise le microphone par défaut.";
  assert.ok(descendants(section).some(el => el.textContent === phrase));
  assert.deepEqual(saved(f),[{id:""}]);
});

test("dictation passes the saved locale to Speech while the interface stays French", async () => {
  const f = fixture();
  for (const language of ["en-US", "fr-CA"]) {
    f.ctx.state.voice.language = language;
    f.registration.onState(f.ctx.state, f.ctx);
    await cls(f.root,"voice-mic").onclick();
    assert.equal(f.bodies.filter(body => body.command === "start").at(-1).language, language);
    await cls(f.root,"voice-mic").onclick();
  }
});
