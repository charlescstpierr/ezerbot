import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import { createVoiceReader, voiceNeedsAction } from "./voice-conversation.js";
class Element {
  constructor(tag) { this.tagName=tag; this.children=[]; this.dataset={}; this.attributes={}; this.value=""; this.scrollHeight=300; this.scrollTop=0; }
  append(...children) { for (const child of children) {if(child.parent)child.parent.children=child.parent.children.filter(c=>c!==child);child.parent=this;this.children.push(child);} }
  replaceChildren(...children) {this.children=[];this.append(...children);}
  prepend(child) {if(child.parent)child.parent.children=child.parent.children.filter(c=>c!==child);child.parent=this;this.children.unshift(child);}
  before(child) {this.parent.append(child);}
  setAttribute(k,v) {this.attributes[k]=String(v);}
}
const all = n => [n,...n.children.flatMap(all)];
const cls = (n,c) => all(n).find(x => x.className === c);
function fixture() {
  let startExtras={}, notices=[], panel, timer, keydown, source={status:"idle",transcript:"",utterance_id:0,barged:0}, scenario="", nextSend, refusal="";
  const calls=[], sends=[], follows=[];
  let dictationRefreshes=0, clock=0, bargeOnSpeak=false, refuseSpeak=false;
  const root=new Element("div"), send=new Element("button"), settings=new Element("div"), composer=new Element("div"), controls=new Element("div");controls.className="voice-controls";root.append(controls,send,settings,composer);
  const voices=[{id:"v-a",name:"Voix A",language:"fr-CA"},{id:"v-b",name:"Voix B",language:"fr-FR"},{id:"v-en",name:"Voice C",language:"en-US"}];
  const bots={ezerbot:{voice:"v-a",speed:1,language:"",chosen:false},chef:{voice:"v-b",speed:1.8,language:"fr",chosen:true}};
  let context={connected:true,bot:{id:"ezerbot",name:"Ezerbot"},conversation:{id:"chat",messages:[]},state:{voice:{bots,voices,single:false}}};
  const bridge=async body => {
    calls.push(body);
    if(body.command === "options")return {voices};
    if(body.command === "start")source={status:scenario || "listening",token:body.token,transcript:"",utterance_id:0,barged:0,...startExtras};
    if(body.command === "speak" && refuseSpeak){refuseSpeak=false;return {status:"tts_unavailable"};}
    if(body.command === "speak"){source.speaking=true;if(bargeOnSpeak){bargeOnSpeak=false;source.speaking=false;source.barged++;}}
    if(body.command === "interrupt") { source.speaking=false;source.transcript=""; }
    if(body.command === "cancel")source.status="idle";
    if(body.command === "pause")source.paused=true;
    if(body.command === "resume")source.paused=false;
    return {...source};
  };
  const panels={
    register(p){ panel=p;p.composer.mount(composer,context);p.onState(context.state,context);},
    composerChanged(){}, notify(text){notices.push(text);}, async refresh(){}, async api(path,body){calls.push({path,...body});if(path==="/api/voice/bot"&&refusal)throw new Error(refusal);if(path==="/api/conversations")return {conversation:{id:"created"}};},
    openConversation(id){context={...context,conversation:{id,messages:[]}};panel.onState(context.state,context);return true;},
    async sendVoice(text,id){sends.push({text,id});nextSend?.();},
    followVoiceReply(id){follows.push(id);},
  };
  runInNewContext(readFileSync(new URL("../../ui/panels/voice-conversation.js",import.meta.url),"utf8"),{
    window:{GrokbotPanels:panels,EzerbotMicrophone:{current:()=>context.state.voice.microphone||"",fellBack(id){notices.push("repli:"+id);}},EzerbotDictation:{refresh(){dictationRefreshes++;},cancel(){} },GrokbotChat:{createVoiceReader,voiceNeedsAction},webkit:{messageHandlers:{ezerbotVoiceSession:{postMessage:bridge}}},addEventListener(){}},
    document:{createElement:tag=>new Element(tag),getElementById:()=>send,querySelector:()=>controls,addEventListener(name,fn){if(name==="keydown")keydown=fn;}},
    Date:{now:()=>clock},crypto:{randomUUID:()=>"token"},setTimeout(fn){timer=fn;return 1;},clearTimeout(){timer=null;},
  });
  const section=bot=>{const container=new Element("div");const cleanup=panel.botSection(bot,container,context);return {container,cleanup};};
  return {section,set clock(ms){clock=ms;},set bargeOnSpeak(value){bargeOnSpeak=value;},set refuseSpeak(value){refuseSpeak=value;},set refusal(text){refusal=text;},root,calls,sends,follows,notices,panel,set startExtras(value){startExtras=value;},setMeetingActive(value){panels.meetingActive=()=>value;panel.onState(context.state,context);},get dictationRefreshes(){return dictationRefreshes;},get source(){return source;},set scenario(s){scenario=s;},set nextSend(fn){nextSend=fn;},get context(){return context;},
    setContext(c){context=c;panel.onState(c.state,c);},tick:async()=>timer?.(),keydown:e=>keydown(e)};
}
test("end-to-end voice sends only the recognized utterance once, reads streamed sentences, pauses on approval",async()=>{
  const f=fixture(); await cls(f.root,"voice-session-toggle").onclick();
  f.source.transcript="Bonjour";f.source.barged=1;await f.tick();
  assert.equal(cls(f.root,"voice-live-transcript").textContent,"Bonjour");
  f.source.final_text="Bonjour Ezerbot.";f.source.utterance_id=1;
  f.nextSend=()=>f.context.conversation.messages.push({id:"r",role:"assistant",kind:"message",state:"streaming",body:"Bonjour. Je vous écoute"});
  await f.tick();await f.tick();
  assert.deepEqual(f.sends,[{text:"Bonjour Ezerbot.",id:"chat"}]);
  assert.equal(f.calls.filter(c=>c.command==="speak").length,1);
  assert.equal(f.calls.find(c=>c.command==="speak").text,"Bonjour.");
  f.context.conversation.messages.push({id:"approval",kind:"approval",payload:{status:{status:"pending"}}});
  await f.tick();assert.equal(f.source.paused,true);assert.match(cls(f.root,"voice-session-state").textContent,/carte/);
  assert.equal(all(f.root).find(e=>e.textContent==="Interrompre").hidden,true,"no interrupt action while a card waits");
  f.source.barged=2;f.source.utterance_id=2;f.source.transcript="Oui";f.source.final_text="Oui";await f.tick();
  await f.tick();await f.tick();
  assert.equal(cls(f.root,"voice-live-transcript").textContent,"Oui");
  assert.equal(cls(f.root,"voice-held-notice").hidden,false,"held-input warning survives repeated polling");
  f.source.transcript="Oui vas-y";await f.tick();
  assert.equal(cls(f.root,"voice-held-notice").hidden,false,"new partial speech cannot remove the warning");
  assert.equal(f.sends.length,1,"speech must not approve an action or leak a card answer into the model");
  assert.equal(f.calls.filter(c=>c.path==="/api/stop").length,0,"speaking does not cancel the pending approval");
  f.context.conversation.messages[1].payload.status.status="denied";
  f.context.conversation.messages.push({id:"after",role:"assistant",kind:"message",state:"streaming",body:"Refus respecté."});
  await f.tick();assert.equal(f.source.paused,false);
  assert.equal(f.calls.filter(c=>c.command==="speak").at(-1).text,"Refus respecté.");
  assert.equal(cls(f.root,"voice-held-notice").hidden,true);
  assert.deepEqual(f.follows,["chat"],"only the resumed spoken reply requests scroll-follow");
  await f.tick();assert.deepEqual(f.follows,["chat"],"polling cannot repeatedly pull the reader to the bottom");
});
test("voice controls share one toolbar group; listening has no false interrupt and live text follows its tail",async()=>{
  const f=fixture();const button=cls(f.root,"voice-session-toggle");
  assert.equal(button.parent.className,"voice-controls");
  assert.equal(button.attributes["aria-label"],"Mode voix");
  assert.equal(cls(f.root,"voice-session-icon").attributes["aria-hidden"],"true");
  const before=f.dictationRefreshes;await button.onclick();
  assert.ok(f.dictationRefreshes>before,"dictation refreshes immediately without waiting for /api/state");
  assert.equal(all(f.root).find(e=>e.textContent==="Interrompre").hidden,true);
  f.source.transcript="Une très longue phrase. ".repeat(80)+"Voici les derniers mots.";await f.tick();
  const transcript=cls(f.root,"voice-live-transcript");
  assert.ok(transcript.textContent.endsWith("Voici les derniers mots."));
  assert.equal(transcript.scrollTop,transcript.scrollHeight);
  await button.onclick();assert.equal(f.panel.wantsPolling(),false);
});

test("welcome voice creates a thread only after a successful utterance, never for permission refusal",async()=>{
  const f=fixture();f.setContext({...f.context,conversation:null});
  f.scenario="mic_denied";await cls(f.root,"voice-session-toggle").onclick();
  assert.equal(f.calls.filter(c=>c.path==="/api/conversations").length,0);
  f.scenario="";await cls(f.root,"voice-session-toggle").onclick();
  assert.equal(f.context.conversation,null);
  f.source.barged=1;f.source.utterance_id=1;f.source.final_text="Bonjour depuis l’accueil.";await f.tick();
  assert.equal(f.context.conversation.id,"created");
  assert.deepEqual(f.sends,[{text:"Bonjour depuis l’accueil.",id:"created"}]);
  assert.equal(f.panel.wantsPolling(),true);
});

test("Escape interrupts audio, switching conversations cancels capture, and late replies cannot restart it",async()=>{
  const f=fixture();await cls(f.root,"voice-session-toggle").onclick();
  f.source.speaking=true;f.keydown({key:"Escape",preventDefault(){},stopImmediatePropagation(){}});await new Promise(r=>setImmediate(r));
  assert.equal(f.source.speaking,false);
  f.setContext({...f.context,conversation:{id:"other",messages:[]}});
  assert.equal(f.source.status,"idle");assert.equal(f.panel.wantsPolling(),false);
  assert.equal(cls(f.root,"voice-session-bar").hidden,true);
});
test("composer voice control stays an icon with a hidden label at every width",()=>{
  const css=readFileSync(new URL("../../ui/panels/voice-conversation.css",import.meta.url),"utf8");
  assert.match(css,/\.voice-session-toggle \{[^}]*width: 32px; height: 32px;/);
  assert.match(css,/\.voice-session-icon \{[^}]*display:\s*flex/);
  assert.match(css,/\.voice-session-label \{[^}]*overflow:\s*hidden/);
  assert.doesNotMatch(css,/@media \(max-width: 900px\)[\s\S]*\.voice-session-label \{ display: none;/);
  assert.match(css,/background: var\(--accent-ink\)/);
  assert.match(css,/@media \(prefers-reduced-motion: reduce\).*animation: none !important/);
});

test("meeting ownership disables conversation voice without cancelling the meeting",async()=>{
  const f=fixture();const toggle=cls(f.root,"voice-session-toggle");f.setMeetingActive(true);
  assert.equal(toggle.disabled,true);assert.match(toggle.title,/réunion/);await toggle.onclick();
  assert.equal(f.calls.filter(c=>c.command==="start").length,0);
  f.setMeetingActive(false);assert.equal(toggle.disabled,false);await toggle.onclick();assert.equal(f.panel.wantsPolling(),true);
});

test("permission denial and transcription errors stop safely and give French recovery",async()=>{
  const f=fixture();f.scenario="mic_denied";await cls(f.root,"voice-session-toggle").onclick();
  assert.match(cls(f.root,"voice-session-state").textContent,/microphone refusé/);
  assert.equal(cls(f.root,"voice-live-transcript").hidden,true,"no listening placeholder under an error");
  assert.equal(f.source.status,"idle");assert.equal(f.panel.wantsPolling(),false);
  const settings=all(f.root).find(e=>e.textContent==="Ouvrir Réglages Système");assert.equal(settings.hidden,false);await settings.onclick();
  assert.equal(f.calls.at(-1).token,"token");assert.equal(f.calls.at(-1).reason,"mic_denied");
  f.scenario="";await cls(f.root,"voice-session-toggle").onclick();f.source.status="failed";await f.tick();
  assert.match(cls(f.root,"voice-session-state").textContent,/transcription a échoué/);assert.equal(f.sends.length,0);
});

test("the voice mode listens to the chosen microphone and never sends it as a voice setting",async()=>{
  const f=fixture(); f.context.state.voice.microphone="usb";
  await cls(f.root,"voice-session-toggle").onclick();
  assert.equal(f.calls.find(c=>c.command==="start").microphone,"usb");
  f.context.conversation.messages.push({id:"r",role:"assistant",kind:"message",state:"streaming",body:"Bonjour. Je vous écoute"});
  await f.tick();
  const speak=f.calls.find(c=>c.command==="speak");
  assert.deepEqual(Object.keys(speak).filter(k=>!["command","token","text","barge","voice","speed","language"].includes(k)),[],"the microphone is not a voice setting");
});
test("a microphone gone when the voice mode starts is announced and the mode still runs",async()=>{
  const f=fixture(); f.context.state.voice.microphone="usb"; f.startExtras={microphone_fallback:true};
  await cls(f.root,"voice-session-toggle").onclick();
  assert.deepEqual(f.notices,["repli:usb"],"only the missing choice is forgotten");
  assert.equal(cls(f.root,"voice-session-toggle").attributes["aria-pressed"],"true");
});

const speaks=f=>f.calls.filter(c=>c.command==="speak").map(({voice,speed,language})=>({voice,speed,language}));
const reply=(f,id,body)=>f.context.conversation.messages.push({id,role:"assistant",kind:"message",state:"streaming",body});
test("each bot's voice mode speaks with the voice, speed and language of its own bot",async()=>{
  const first=fixture(); await cls(first.root,"voice-session-toggle").onclick();
  reply(first,"r1","Bonjour. Je vous écoute"); await first.tick();
  assert.deepEqual(speaks(first),[{voice:"v-a",speed:1,language:""}]);
  const second=fixture(); second.setContext({...second.context,bot:{id:"chef",name:"Chef"}});
  await cls(second.root,"voice-session-toggle").onclick();
  reply(second,"r2","Bonjour. Je vous écoute"); await second.tick();
  assert.deepEqual(speaks(second),[{voice:"v-b",speed:1.8,language:"fr"}]);
});
test("the bot setting shows the voice of its bot, offers speeds up to 2 and a language or Auto",async()=>{
  const f=fixture();
  const {container}=f.section({id:"chef",name:"Chef"});
  const nodes=all(container);
  assert.match(nodes.find(n=>n.className==="voice-bot-current").textContent,/Voix de Chef : Voix B · 1,8× · français/);
  const speed=nodes.find(n=>n.name==="bot-voice-speed");
  assert.deepEqual([speed.min,speed.max,speed.value],["0.5","2",1.8]);
  const language=nodes.find(n=>n.name==="bot-voice-language");
  assert.deepEqual(language.children.map(o=>o.value),["","fr","en"]);
  assert.equal(language.children[0].textContent,"Auto");
  const other=all(f.section({id:"ezerbot",name:"Ezerbot"}).container);
  assert.match(other.find(n=>n.className==="voice-bot-current").textContent,/Voix de Ezerbot : Voix A · 1× · langue Auto/);
  assert.match(other.find(n=>n.className==="voice-bot-hint").textContent,/attribuée automatiquement/);
});
test("a single voice is announced as shared by every bot",()=>{
  const f=fixture(); f.context.state.voice.voices=[f.context.state.voice.voices[0]]; f.context.state.voice.single=true;
  const hint=all(f.section({id:"ezerbot",name:"Ezerbot"}).container).find(n=>n.className==="voice-bot-hint");
  assert.equal(hint.textContent,"Une seule voix est installée sur ce Mac : tous les bots la partagent.");
});
test("saving sends the bot's choice, and a refused speed is said without replacing the saved one",async()=>{
  const f=fixture(); const nodes=all(f.section({id:"chef",name:"Chef"}).container);
  const speed=nodes.find(n=>n.name==="bot-voice-speed"), save=nodes.find(n=>n.textContent==="Enregistrer la voix"), notice=nodes.find(n=>n.className==="voice-settings-notice");
  speed.value="2"; speed.oninput(); await save.onclick();
  assert.deepEqual(f.calls.find(c=>c.path==="/api/voice/bot"),{path:"/api/voice/bot",bot_id:"chef",voice:"v-b",speed:2,language:"fr"});
  assert.match(notice.textContent,/Voix enregistrée/);
  const refusal="Vitesse refusée : choisissez une vitesse de 0,5 à 2. La vitesse du bot reste inchangée.";
  f.refusal=refusal;
  speed.value="2.5"; speed.oninput(); await save.onclick();
  assert.equal(notice.textContent,refusal);
  assert.equal(nodes.find(n=>n.className==="voice-bot-current").textContent.includes("1,8×"),true,"the saved speed is still the one displayed");
});
test("the voices the Mac offers are handed to the engine once, so that bots without a choice get distinct ones",async()=>{
  const f=fixture();
  await new Promise(r=>setImmediate(r));
  assert.deepEqual(f.calls.find(c=>c.path==="/api/voice/voices").voices.map(v=>v.id),["v-a","v-b","v-en"]);
  f.setContext(f.context); await new Promise(r=>setImmediate(r));
  assert.equal(f.calls.filter(c=>c.path==="/api/voice/voices").length,1);
});
test("in a group, each reply is spoken with the voice of the bot that wrote it",async()=>{
  const f=fixture(); await cls(f.root,"voice-session-toggle").onclick();
  f.context.conversation.kind="group";
  f.context.conversation.messages.push(
    {id:"a",role:"assistant",kind:"reply",state:"streaming",body:"Je commence.",payload:{bot_id:"ezerbot"}},
    {id:"b",role:"assistant",kind:"reply",state:"streaming",body:"Je continue.",payload:{bot_id:"chef"}},
  );
  await f.tick();
  assert.deepEqual(f.calls.filter(c=>c.command==="speak").map(({text,voice,speed,language})=>({text,voice,speed,language})),[
    {text:"Je commence.",voice:"v-a",speed:1,language:""},
    {text:"Je continue.",voice:"v-b",speed:1.8,language:"fr"},
  ]);
});

// A sub-task launched from the thread, as the engine publishes it in state.delegation.
const task=(id,status,over={})=>({id,objective:"Résumer le rapport",parent_conversation_id:"chat",status:{status},...over});
const tasksOf=(f,...edges)=>{f.context.conversation.messages.push(...edges.map(payload=>({id:"card-"+payload.id,kind:"delegation",role:"assistant",payload})));return edges;};
const spoken=f=>f.calls.filter(c=>c.command==="speak").map(c=>c.text);
const phrase="La tâche « Résumer le rapport » est terminée.";

test("a task of this thread that finishes during the voice mode is announced once, with its title, and the bar shows it for 5 s",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[],"a running task is not announced");
  edge.status={status:"done",result:"Fait."};f.clock=1000;await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
  assert.equal(cls(f.root,"voice-session-state").textContent,phrase);
  f.source.speaking=false;f.clock=2000;await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[phrase],"two successive polls do not repeat the announcement");
  assert.equal(cls(f.root,"voice-session-state").textContent,phrase);
  f.clock=6001;await f.tick();
  assert.equal(cls(f.root,"voice-session-state").textContent,"À l’écoute · parlez pour envoyer");
});

test("a failed or refused task is announced as failed, a cancelled one is not",async()=>{
  const f=fixture();const edges=tasksOf(f,task("a","running",{objective:"Trier"}),task("b","running",{objective:"Archiver"}),task("c","running",{objective:"Ranger"}));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edges[0].status={status:"failed",reason:"x"};edges[1].status={status:"refused",reason:"y"};edges[2].status={status:"cancelled"};
  for(let i=0;i<3;i++){await f.tick();f.source.speaking=false;}
  assert.deepEqual(spoken(f),["La tâche « Archiver » a échoué.","La tâche « Trier » a échoué."]);
});

test("the announcement waits while Charles speaks and is said once he stops",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.source.transcript="Je voudrais savoir";f.source.barged=1;
  await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[]);
  f.source.transcript="";await f.tick();
  assert.deepEqual(spoken(f),[],"his words just stopped: a short silence first");
  f.clock=2000;await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
});

test("the announcement waits for the end of the sentence being spoken, even while the reply is still being written",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.source.speaking=true;
  f.context.conversation.messages.push({id:"r",role:"assistant",kind:"message",state:"streaming",body:"Bien sûr. Je regarde"});
  await f.tick();assert.deepEqual(spoken(f),["Bien sûr."],"the sentence of the reply is read first");
  f.source.speaking=true;await f.tick();assert.deepEqual(spoken(f),["Bien sûr."]);
  f.source.speaking=false;await f.tick();
  assert.deepEqual(spoken(f),["Bien sûr.",phrase],"a task ended during a long reply is not held until the reply ends");
});

test("a task of another thread is never announced",async()=>{
  const f=fixture();
  f.context.state.delegation={delegations:[task("t1","running",{parent_conversation_id:"autre"})]};
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  f.context.state.delegation.delegations[0].status={status:"done",result:"Fait."};await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[]);
});

test("a task finished before the voice mode opened is not announced",async()=>{
  const f=fixture();tasksOf(f,task("old","done"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[]);
});

test("muted with a card waiting, nothing is announced until the card is answered",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  f.source.muted=true;f.context.conversation.messages.push({id:"approval",kind:"approval",payload:{status:{status:"pending"}}});
  edge.status={status:"done",result:"Fait."};await f.tick();await f.tick();
  assert.equal(f.source.paused,true);assert.deepEqual(spoken(f),[]);
  f.context.conversation.messages.find(m=>m.id==="approval").payload.status.status="approved";await f.tick();
  assert.equal(f.source.paused,false);assert.deepEqual(spoken(f),[phrase]);
});

test("an announcement that Charles cuts off after the bridge took it is said again once he stops",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
  f.source.barged=1;f.source.speaking=false;f.source.transcript="Attends";await f.tick();
  assert.deepEqual(spoken(f),[phrase],"nothing while Charles speaks");
  f.source.transcript="";f.clock=2000;await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase]);
  f.source.speaking=false;await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase],"heard to the end, it is not said a third time");
});

test("an announcement that Charles cut by speaking is said again, and shown only once said",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.bargeOnSpeak=true;await f.tick();
  assert.notEqual(cls(f.root,"voice-session-state").textContent,phrase);
  await f.tick();f.clock=2000;await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase]);
  assert.equal(cls(f.root,"voice-session-state").textContent,phrase);
});

test("an announcement waits for the silence after Charles starts to speak, even before his words are transcribed",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.source.barged=1;
  f.clock=1000;await f.tick();await f.tick();
  assert.deepEqual(spoken(f),[],"barged moved, speaking is false and nothing is transcribed yet");
  f.clock=2400;await f.tick();
  assert.deepEqual(spoken(f),[]);
  f.clock=2600;await f.tick();
  assert.deepEqual(spoken(f),[phrase]);
});

test("an announcement the bridge refused is tried again when the voice mode is relaunched",async()=>{
  const f=fixture();const [edge]=tasksOf(f,task("t1","running"));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  edge.status={status:"done",result:"Fait."};f.refuseSpeak=true;await f.tick();
  assert.equal(f.panel.wantsPolling(),false,"a refused voice stops the session");
  assert.deepEqual(spoken(f),[phrase]);
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase],"never said, so said again");
  f.source.speaking=false;await f.tick();
  await cls(f.root,"voice-session-toggle").onclick();await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  assert.deepEqual(spoken(f),[phrase,phrase],"said, so not said a third time after a relaunch");
});

test("two announcements in a row are each shown for 5 s, one after the other",async()=>{
  const f=fixture();const edges=tasksOf(f,task("a","running",{objective:"Trier"}),task("b","running",{objective:"Archiver"}));
  await cls(f.root,"voice-session-toggle").onclick();await f.tick();
  const state=()=>cls(f.root,"voice-session-state").textContent;
  edges[0].status={status:"done"};f.clock=1000;await f.tick();
  edges[1].status={status:"done"};f.source.speaking=false;f.clock=3000;await f.tick();
  assert.deepEqual(spoken(f),["La tâche « Trier » est terminée.","La tâche « Archiver » est terminée."]);
  f.source.speaking=false;
  f.clock=5500;await f.tick();assert.equal(state(),"La tâche « Trier » est terminée.","the first is not erased by the second");
  f.clock=7000;await f.tick();assert.equal(state(),"La tâche « Archiver » est terminée.");
  f.clock=11001;await f.tick();assert.equal(state(),"À l’écoute · parlez pour envoyer");
});
